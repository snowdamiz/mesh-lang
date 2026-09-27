#!/usr/bin/env python3
"""Line coverage of the Mesh compiler and runtime crates.

    scripts/compiler-coverage.py run [--crates=a,b] [--no-services] [cargo test args...]
    scripts/compiler-coverage.py report [crate-or-path...]   # summarize

`run` runs the crates' tests under `cargo llvm-cov` and writes
target/coverage/lcov.info. Unless `--no-services` is given it runs what the
plain test run leaves out, with local Docker: the database- and
Docker-backed tests (ignored by default), the former against a PostgreSQL
container of its own, and `meshc proof docker-autoscaling` through the
instrumented meshc, its containers running an instrumented runtime and
capacity driver. The runtime crates (mesh-rt, mesh-test-rt) count their
own tests and every Mesh program the tests build: those link an instrumented
runtime (with the profiler runtime a static library does not carry), and
their profiles are read against its objects. `report`
prints each crate's coverage and each file's uncovered lines, leaving out
test code: `tests/` directories and `#[cfg(test)]` modules.

Needs cargo-llvm-cov, and LLVM_COV/LLVM_PROFDATA from the LLVM the Rust
toolchain uses when it is not rustup's (Homebrew's rustc: llvm@21).
"""

import os
import re
import shutil
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LCOV = ROOT / "target/coverage/lcov.info"
RUNTIME_SNAPSHOT = ROOT / "target/coverage/runtime"
RUNTIME_BUILD = ROOT / "target/coverage/runtime-build"
PROFILES = ROOT / "target/llvm-cov-target"
RUNTIME_LIBS = ("libmesh_rt.a", "libmesh_test_rt.a")
# Where the Docker proof's instrumented containers write their profiles, and
# the proof exports the instrumented Linux runtime and driver (objects/).
PROOF_COVERAGE = ROOT / "target/coverage/proof"
CRATES = [
    "mesh-common", "mesh-lexer", "mesh-parser", "mesh-typeck", "mesh-codegen",
    "mesh-fmt", "mesh-lint", "mesh-pkg", "mesh-lsp", "mesh-repl", "meshc", "meshpkg",
    "mesh-rt", "mesh-test-rt",
]


def llvm_env():
    env = dict(os.environ)
    for tool in ("llvm-cov", "llvm-profdata", "llvm-ar"):
        var = tool.upper().replace("-", "_")
        if var not in env:
            brew = Path("/opt/homebrew/opt/llvm@21/bin") / tool
            if brew.exists():
                env[var] = str(brew)
    return env


# A PostgreSQL of the coverage run's own, on a port no other project's test
# database uses (whatsdown's are 5543x, the registry's 55492).
PG_CONTAINER = "mesh-coverage-pg"
PG_PORT = 55530
PG_URL = f"postgres://mesh_test:mesh_test@127.0.0.1:{PG_PORT}/mesh_test?sslmode=disable"
# The server speaks TLS with a certificate for 127.0.0.1, signed at start by a
# CA of its own (exported to PG_CA, for sslrootcert), and md5 authentication
# to a role whose password is md5-hashed. The label names this setup: a
# container without it is an older one, made again.
PG_CA = ROOT / "target/coverage/pg-ca.crt"
PG_SETUP = "tls-md5-1"
PG_START = """set -e
mkdir -p /etc/ssl/mesh && cd /etc/ssl/mesh
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=mesh-test-ca \
  -keyout ca.key -out ca.crt 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj /CN=localhost -keyout server.key \
  -out server.csr 2>/dev/null
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\n' > server.ext
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -days 3650 \
  -extfile server.ext -out server.crt 2>/dev/null
chown postgres server.key server.crt
chmod 600 server.key
exec docker-entrypoint.sh postgres -c ssl=on \
  -c ssl_cert_file=/etc/ssl/mesh/server.crt -c ssl_key_file=/etc/ssl/mesh/server.key
"""


def start_postgres():
    """Start (or reuse) the coverage run's PostgreSQL, wait until it answers,
    and export its CA certificate to PG_CA."""
    state = subprocess.run(["docker", "inspect", "-f",
                            '{{.State.Running}} {{index .Config.Labels "mesh.coverage.pg"}}',
                            PG_CONTAINER], capture_output=True, text=True)
    running, _, setup = state.stdout.strip().partition(" ")
    if state.returncode == 0 and setup != PG_SETUP:
        subprocess.run(["docker", "rm", "-f", PG_CONTAINER], check=True, stdout=subprocess.DEVNULL)
    if state.returncode != 0 or setup != PG_SETUP:
        subprocess.run(["docker", "run", "-d", "--name", PG_CONTAINER,
                        "--label", f"mesh.coverage.pg={PG_SETUP}",
                        "-e", "POSTGRES_USER=mesh_test", "-e", "POSTGRES_PASSWORD=mesh_test",
                        "-e", "POSTGRES_DB=mesh_test", "-e", "POSTGRES_HOST_AUTH_METHOD=md5",
                        "-p", f"127.0.0.1:{PG_PORT}:5432", "--entrypoint", "bash",
                        "postgres:16", "-c", PG_START], check=True, stdout=subprocess.DEVNULL)
    elif running != "true":
        subprocess.run(["docker", "start", PG_CONTAINER], check=True, stdout=subprocess.DEVNULL)
    for _ in range(90):
        ready = subprocess.run(["docker", "exec", PG_CONTAINER, "pg_isready", "-h", "127.0.0.1",
                                "-U", "mesh_test", "-d", "mesh_test"], capture_output=True)
        if ready.returncode == 0:
            PG_CA.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(["docker", "cp", f"{PG_CONTAINER}:/etc/ssl/mesh/ca.crt",
                            str(PG_CA)], check=True, stdout=subprocess.DEVNULL)
            return PG_URL
        time.sleep(1)
    sys.exit(f"{PG_CONTAINER} did not accept connections")


def with_ignored(extra):
    """`extra` with the ignored (database-backed) tests included."""
    if "--" in extra:
        return [*extra, "--include-ignored"]
    return [*extra, "--", "--include-ignored"]


def run(extra):
    crates = CRATES
    services = True
    while extra and extra[0].startswith("--"):
        if extra[0].startswith("--crates="):
            crates = extra[0].split("=", 1)[1].split(",")
        elif extra[0] == "--no-services":
            services = False
        else:
            break
        extra = extra[1:]
    env = llvm_env()
    if services:
        env["MESH_TEST_DATABASE_URL"] = start_postgres()
        env["MESH_TEST_DATABASE_CA"] = str(PG_CA)
        extra = with_ignored(extra)
    linker_dir = build_instrumented_runtime()
    if linker_dir:
        env["PATH"] = linker_dir + os.pathsep + env["PATH"]
    env["MESH_RT_LIB_PATH"] = str(RUNTIME_SNAPSHOT / "libmesh_rt.a")
    env["MESH_TEST_RT_LIB_PATH"] = str(RUNTIME_SNAPSHOT / "libmesh_test_rt.a")
    packages = [arg for crate in crates for arg in ("-p", crate)]
    subprocess.run(["cargo", "llvm-cov", "clean", "--workspace"], cwd=ROOT, env=env, check=True)
    # The clean makes this run rebuild every binary of the workspace.
    built_after = time.time()
    for stale in PROFILES.glob("*.profraw"):
        stale.unlink()
    test = subprocess.run(
        ["cargo", "llvm-cov", "--no-report", "--locked", *packages, "--no-fail-fast", *extra],
        cwd=ROOT, env=env)
    failed = test.returncode
    proof_ran = services and "meshc" in crates
    if proof_ran:
        # The Docker autoscaling proof drives an eleven-container topology from
        # meshc itself; run through the instrumented binary it counts too, and
        # its containers run an instrumented runtime and driver.
        shutil.rmtree(PROOF_COVERAGE, ignore_errors=True)
        proof = subprocess.run(
            ["cargo", "llvm-cov", "run", "--no-report", "--locked", "-p", "meshc", "--",
             "proof", "docker-autoscaling"], cwd=ROOT,
            env={**env, "MESH_PROOF_COVERAGE_DIR": str(PROOF_COVERAGE)})
        failed = failed or proof.returncode
    LCOV.parent.mkdir(parents=True, exist_ok=True)
    # Merges the profiles into PROFILES/<checkout directory name>.profdata,
    # which the per-binary export below reads.
    subprocess.run(["cargo", "llvm-cov", "report", "--lcov", "--output-path", str(LCOV)],
                   cwd=ROOT, env=env, check=True)
    LCOV.write_text(per_binary_lcov(env, built_after))
    with LCOV.open("a") as lcov:
        lcov.write(program_runtime_lcov(env))
        if proof_ran:
            lcov.write(proof_runtime_lcov(env))
    return failed


def per_binary_lcov(env, built_after):
    """The tests' lcov, each test binary exported on its own and the lines
    summed. llvm-cov keeps one record per function name, and a `#[no_mangle]`
    function has the same name in every binary: where two builds compiled it
    differently (a crate's own unit tests, under cfg(test), and every other
    binary that links the crate), one binary's calls were all it counted."""
    from concurrent.futures import ThreadPoolExecutor

    # cargo-llvm-cov names the merged profile after the workspace directory.
    profdata = PROFILES / f"{ROOT.name}.profdata"
    if not profdata.exists():
        sys.exit(f"{profdata} is missing: cargo llvm-cov report wrote no merged profile")
    binaries = [path for directory in (PROFILES / "debug" / "deps", PROFILES / "debug")
                for path in sorted(directory.iterdir())
                if path.is_file() and not path.suffix and os.access(path, os.X_OK)
                # A test target since removed leaves its binary behind, built
                # from sources that are gone; its lines would land on today's.
                and path.stat().st_mtime >= built_after]

    def export(binary):
        return subprocess.run(
            [env.get("LLVM_COV", "llvm-cov"), "export", "-format=lcov",
             f"-instr-profile={profdata}", str(binary)],
            capture_output=True, text=True).stdout

    hits = defaultdict(lambda: defaultdict(int))
    with ThreadPoolExecutor(max_workers=6) as pool:
        for exported in pool.map(export, binaries):
            source = None
            for line in exported.splitlines():
                if line.startswith("SF:"):
                    source = line[3:]
                elif line.startswith("DA:"):
                    number, count = line[3:].split(",")[:2]
                    hits[source][int(number)] += int(count)
    return "".join(
        f"SF:{source}\n" + "".join(f"DA:{number},{count}\n" for number, count in sorted(lines.items()))
        + "end_of_record\n"
        for source, lines in sorted(hits.items()))


def build_instrumented_runtime():
    """The runtime Mesh programs link, in RUNTIME_SNAPSHOT: instrumented, with
    rustc's profiler runtime added (a static library leaves it out), and copied
    aside so a test that rebuilds the runtime cannot swap it mid-run. On Linux,
    where linkers take the profiler runtime only when asked for it, returns a
    directory for the front of PATH whose `cc` links it the way rustc does for
    binaries: last, with `-u`."""
    env = {name: value for name, value in os.environ.items()
           if "LLVM_COV" not in name and not name.startswith("RUSTC_")}
    # mesh_coverage: a program told to stop (SIGTERM) writes its profile first.
    env["RUSTFLAGS"] = "-C instrument-coverage --cfg mesh_coverage"
    env["CARGO_TARGET_DIR"] = str(RUNTIME_BUILD)
    # RUSTFLAGS instrument the proc macros too, which rustc then runs: their
    # profiles would land in the working directory.
    env["LLVM_PROFILE_FILE"] = str(RUNTIME_BUILD / "proc-macro-%p-%m.profraw")
    subprocess.run(["cargo", "build", "--locked", "-p", "mesh-rt", "-p", "mesh-test-rt"],
                   cwd=ROOT, env=env, check=True)
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True,
                             check=True).stdout.strip()
    profiler = next(Path(sysroot).glob("lib/rustlib/*/lib/libprofiler_builtins-*.rlib"))
    RUNTIME_SNAPSHOT.mkdir(parents=True, exist_ok=True)
    for lib in RUNTIME_LIBS:
        out = RUNTIME_SNAPSHOT / lib
        out.unlink(missing_ok=True)
        if sys.platform == "darwin":
            subprocess.run(["libtool", "-static", "-o", str(out), str(RUNTIME_BUILD / "debug" / lib),
                            str(profiler)], check=True, capture_output=True)
        else:
            shutil.copy(RUNTIME_BUILD / "debug" / lib, out)
    if sys.platform == "darwin":
        return None
    cc = RUNTIME_SNAPSHOT / "bin" / "cc"
    cc.parent.mkdir(exist_ok=True)
    cc.write_text(f'#!/bin/sh\nexec /usr/bin/cc "$@" -Wl,-u,__llvm_profile_runtime {profiler}\n')
    cc.chmod(0o755)
    return str(cc.parent)


def exported_lcov(env, profiles, archives, executables, work, source_root=None):
    """lcov of `profiles` read against the runtime objects in `archives` (llvm-cov
    reads objects, not archives; llvm-ar reads the GNU archives macOS ar cannot)
    and `executables`; the sources of objects built under `source_root` are
    renamed to this checkout's."""
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    listing = work / "profiles.txt"
    listing.write_text("".join(f"{profile}\n" for profile in profiles))
    merged = work / "merged.profdata"
    subprocess.run([env.get("LLVM_PROFDATA", "llvm-profdata"), "merge", "-sparse", "-f",
                    str(listing), "-o", str(merged)], check=True)
    objects = list(executables)
    for archive in archives:
        members = work / archive.name
        members.mkdir()
        subprocess.run([env.get("LLVM_AR", "ar"), "x", str(archive)], cwd=members, check=True)
        objects += [member for member in sorted(members.iterdir())
                    if member.name.startswith(("mesh_rt-", "mesh_test_rt-"))]
    args = [env.get("LLVM_COV", "llvm-cov"), "export", "-format=lcov", f"-instr-profile={merged}"]
    for member in objects:
        args += ["-object", str(member)]
    exported = subprocess.run(args, capture_output=True, text=True, check=True).stdout
    shutil.rmtree(work)
    if source_root:
        exported = exported.replace(f"SF:{source_root}/", f"SF:{ROOT}/")
    return exported


def program_runtime_lcov(env):
    """The runtime's coverage from the Mesh programs the tests ran: their
    profiles sit beside the tests' own."""
    profiles = sorted(PROFILES.glob("*.profraw"))
    if not profiles:
        return ""
    archives = [RUNTIME_BUILD / "debug" / lib for lib in RUNTIME_LIBS]
    return exported_lcov(env, profiles, archives, [], RUNTIME_SNAPSHOT / "programs")


def proof_runtime_lcov(env):
    """The runtime's and capacity driver's coverage in the Docker proof's
    containers, built from the image's /app."""
    objects = PROOF_COVERAGE / "objects"
    profiles = sorted(PROOF_COVERAGE.glob("*.profraw"))
    if not profiles or not (objects / "libmesh_rt.a").exists():
        return ""
    return exported_lcov(env, profiles, [objects / "libmesh_rt.a"],
                         [objects / "mesh-capacity-driver"], PROOF_COVERAGE / "work", "/app")


def test_lines(path):
    """The lines of `#[cfg(test)]` items in a source file."""
    lines = path.read_text().splitlines()
    excluded = set()
    i = 0
    while i < len(lines):
        if lines[i].strip() == "#[cfg(test)]":
            depth, j, opened = 0, i, False
            while j < len(lines):
                depth += lines[j].count("{") - lines[j].count("}")
                opened = opened or "{" in lines[j]
                excluded.add(j + 1)
                if (opened and depth <= 0) or (not opened and lines[j].rstrip().endswith(";")):
                    break
                j += 1
            i = j
        i += 1
    return excluded


def load():
    """{file: {line: hits}} for the crates' non-test code."""
    files, current = {}, None
    for line in LCOV.read_text().splitlines():
        if line.startswith("SF:"):
            path = Path(line[3:])
            rel = path.relative_to(ROOT) if path.is_relative_to(ROOT) else path
            parts = rel.parts
            keep = (len(parts) > 2 and parts[0] == "compiler" and parts[1] in CRATES
                    and "tests" not in parts)
            current = files.setdefault(rel, {}) if keep else None
            if current is not None:
                current["__exclude__"] = test_lines(path) if path.exists() else set()
        elif line.startswith("DA:") and current is not None:
            number, hits = line[3:].split(",")[:2]
            if int(number) not in current["__exclude__"]:
                current[int(number)] = current.get(int(number), 0) + int(hits)
    for data in files.values():
        del data["__exclude__"]
    return files


def ranges(numbers):
    out, start, prev = [], None, None
    for n in sorted(numbers):
        if start is None:
            start = prev = n
        elif n == prev + 1:
            prev = n
        else:
            out.append(f"{start}-{prev}" if start != prev else f"{start}")
            start = prev = n
    if start is not None:
        out.append(f"{start}-{prev}" if start != prev else f"{start}")
    return ", ".join(out)


def report(filters):
    files = load()
    crates = defaultdict(lambda: [0, 0])
    for rel, data in files.items():
        crates[rel.parts[1]][0] += sum(1 for hits in data.values() if hits)
        crates[rel.parts[1]][1] += len(data)
    covered = sum(c for c, _ in crates.values())
    total = sum(t for _, t in crates.values())
    print(f"total: {100 * covered / max(total, 1):.2f}% ({covered}/{total} lines)")
    for crate, (c, t) in sorted(crates.items()):
        print(f"  {crate:14} {100 * c / max(t, 1):6.2f}%  {t - c:5} uncovered")
    if filters:
        print()
        for rel, data in sorted(files.items()):
            if any(f in str(rel) for f in filters):
                missed = [n for n, hits in data.items() if not hits]
                if missed:
                    print(f"{rel}: {ranges(missed)}")
    return 0


if __name__ == "__main__":
    command, *rest = sys.argv[1:] or ["report"]
    if command == "run":
        sys.exit(run(rest))
    sys.exit(report(rest))
