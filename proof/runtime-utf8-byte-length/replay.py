#!/usr/bin/env python3
"""Compare two Mesh compilers with one runtime on a native Unix host."""
import argparse
import hashlib
import os
from pathlib import Path
import re
import runpy
import shutil
import subprocess

PROOF = Path(__file__).resolve().parent
ROOT = PROOF.parents[1]
BYTES_CALLS = ("mesh_bytes_from_utf8", "mesh_bytes_length")


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def run(command, env, log=None):
    result = subprocess.run(command, capture_output=True, text=True, env=env, timeout=240)
    if log is not None:
        log.write_text(result.stdout + result.stderr)
    return result


def checked(command, env, log=None):
    result = run(command, env, log)
    if result.returncode:
        raise RuntimeError(f"{command} exited {result.returncode}\n{result.stderr[-4000:]}")
    return result.stdout


def native_archive(project, source, bindings, triple, env):
    archive = project / "libnative.a"
    obj = project / "native.o"
    checked(["cc", "-std=c11", "-O2", "-c", source, "-o", obj], env)
    checked(["ar", "rcs", archive, obj], env)
    (project / "mesh.toml").write_text(f"""[package]
name = "{project.name}"
version = "0.1.0"

[native]
abi = 1
bindings = ["{bindings}"]

[[native.libraries]]
target = "{triple}"
path = "{archive.name}"
sha256 = "{digest(archive)}"
""")


def prepare_inputs(out, triple, env):
    fixtures = out / "fixtures"
    shutil.copytree(PROOF / "fixtures", fixtures)
    native = fixtures / "native"
    native_archive(native, native / "native.c", "bindings.mpl", triple, env)

    frozen = fixtures / "frozen"
    frozen.mkdir()
    pinned = ROOT / "proof/jobs-parity/linux-integrated-uncut/fixture"
    for name, sha256 in {
        "main.mpl": "0c72aaa2f0f34c49d07fd2f6cad2d39f491c0857897a0ac7a0facb6b85b4464d",
        "worker_identity.mpl": "e56589a1df5442a8c82b95950314e68776145b2bd6215e22c5014ee14c9d9fca",
    }.items():
        shutil.copyfile(pinned / name, frozen / name)
        assert digest(frozen / name) == sha256, name
    identity = ROOT / "tests/e2e/durable_jobs/.parity/worker_identity.c"
    assert digest(identity) == "2808ca2bd2a286b79c8bf071a99453227395265b761571075a4f6c06780a63a0"
    native_archive(frozen, identity, "worker_identity.mpl", triple, env)

    harness = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))
    settings = harness["json"].loads(harness["CONFIG"].read_text())
    batches = harness["inputs"](settings["workloads"]["large"], 64, settings["seed"])
    inputs = out / "inputs"
    inputs.mkdir()
    assert len(batches) == 1
    harness["batch_files"](inputs, batches)
    case = inputs / "batch_000000.json"
    assert digest(case) == "30f91354336fab21dbe85f2933f3347ec53ab7634af3805169a892536747fa1c"
    return fixtures, case


def body(path, name):
    pattern = rf"^define [^\n]*@(?:Main__)?{re.escape(name)}\([^\n]*\{{\n(.*?)^}}"
    matches = re.findall(pattern, path.read_text(), re.MULTILINE | re.DOTALL)
    assert len(matches) == 1, (path, name)
    return matches[0]


def calls(code, name):
    return len(re.findall(rf"\bcall\b[^\n]*@{re.escape(name)}\(", code))


def check_direct(ir, name, sites, fused):
    code = body(ir, name)
    for symbol in BYTES_CALLS:
        assert calls(code, symbol) == (0 if fused else sites), (ir, name, symbol)
    assert len(re.findall(r"%str_len\d* = load i64, ptr ", code)) == (sites if fused else 0)
    return code


def build(compiler, project, triple, level, binary, env):
    command = [compiler, "build", project, "--target", triple, "--opt-level", level,
               "--emit-llvm", "-o", binary]
    checked(command, env, binary.with_suffix(".build.log"))


def native_checks(compiler, out, fixtures, case, triple, env, fused):
    for level in ("0", "1", "2"):
        for name in ("ordinary", "native", "shadow", "panic"):
            binary = out / f"{name}-O{level}"
            project = fixtures / name
            build(compiler, project, triple, level, binary, env)
            for stress in (False, True):
                run_env = dict(env, MESH_GC_STRESS="1") if stress else env
                result = run([binary], run_env)
                if name == "panic":
                    assert result.returncode != 0, (binary, result)
                    assert result.stdout == "before panic\n", (binary, result)
                    assert "byte length operand failed" in result.stderr, (binary, result)
                else:
                    assert result.returncode == 0, (binary, result)
                    assert result.stdout == (project / "expected.txt").read_text(), (binary, result)

        ordinary = out / f"ordinary-O{level}.ll"
        check_direct(ordinary, "direct", 1, fused)
        escaped = body(ordinary, "escaped")
        assert all(calls(escaped, symbol) == 1 for symbol in BYTES_CALLS)
        assert calls(body(ordinary, "shadowed"), "mesh_bytes_from_utf8") == 0

        native = body(out / f"native-O{level}.ll", "worker")
        for symbol, count in {
            "gate_native_text": 2,
            "gate_native_text_calls": 1,
            "gate_native_convert": 1,
            "gate_native_consume": 1,
            "gate_native_invalid_utf8": 1,
            "mesh_string_length": 1,
            "mesh_bytes_from_utf8": 3 if fused else 6,
        }.items():
            assert calls(native, symbol) == count, (level, symbol)

    for name in ("frozen", "component"):
        build(compiler, fixtures / name, triple, "2", out / name, env)
    check_direct(out / "frozen.ll", "collect_specs", 1, fused)
    check_direct(out / "component.ll", "measure", 3, fused)
    component = checked([out / "component"], dict(env, JOBS_PARITY_INPUT=str(case))).split()
    assert len(component) == 2 and component[0] == "66607436", component
    print(f"{out.name} passed O0/O1/O2, panic, GC stress, frozen producer IR and component byte sum")


def cross_check(baseline, candidate, llvm, out, fixtures, env):
    for triple, is_64bit in [("x86_64-unknown-linux-gnu", True), ("armv7-linux-androideabi", False)]:
        for compiler, label in [(baseline, "baseline"), (candidate, "candidate")]:
            binary = out / f"{label}-{triple}"
            ir = binary.with_suffix(".ll")
            # meshc emits IR before resolving the runtime. The missing archive
            # keeps this check independent of any cross linker or target runtime.
            cross_env = dict(env, MESH_RT_LIB_PATH=str(out / "absent/libmesh_rt.a"))
            result = run(
                [compiler, "build", fixtures / "ordinary", "--target", triple,
                 "--opt-level", "2", "--emit-llvm", "-o", binary],
                cross_env,
                binary.with_suffix(".build.log"),
            )
            assert result.returncode and ir.exists(), (triple, result)
            assert f'target triple = "{triple}"' in ir.read_text()
            check_direct(ir, "direct", 1, label == "candidate" and is_64bit)
            checked([llvm / "opt", "-passes=verify", "-disable-output", ir], env)
            obj = binary.with_suffix(".o")
            checked([llvm / "llc", "-O2", "-filetype=obj", ir, "-o", obj], env)
            dump = checked([llvm / "llvm-objdump", "-dr", obj], env)
            assert "Main__direct" in dump
            binary.with_suffix(".asm").write_text(dump)
            print(f"{label} {triple} verified IR and object {digest(obj)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("baseline", "candidate", "runtime", "out"):
        parser.add_argument(name, type=lambda s: Path(s).resolve())
    parser.add_argument("--target", required=True, help="native target triple (64-bit Unix)")
    parser.add_argument("--llvm-bin", type=lambda s: Path(s).resolve(),
                        help="also emit and verify Linux x86-64 and Android ARMv7 objects")
    args = parser.parse_args()

    args.out.mkdir()
    tmp = args.out / "tmp"
    tmp.mkdir()
    env = dict(
        os.environ,
        TMPDIR=f"{tmp}/",
        MESH_RT_LIB_PATH=str(args.runtime),
        MESH_SCHEDULER_MIN_WORKERS="1",
        MESH_SCHEDULER_MAX_WORKERS="1",
        JOBS_PARITY_MODE="dense",
    )
    env.pop("MESH_GC_STRESS", None)
    for name in ("baseline", "candidate", "runtime"):
        print(f"{name} {digest(getattr(args, name))}")
    print(f"native target {args.target}")

    fixtures, case = prepare_inputs(args.out, args.target, env)
    for compiler, name in [(args.baseline, "baseline"), (args.candidate, "candidate")]:
        out = args.out / name
        out.mkdir()
        native_checks(compiler, out, fixtures, case, args.target, env, name == "candidate")

    for level in ("0", "1", "2"):
        a, b = (args.out / side / f"ordinary-O{level}.ll" for side in ("baseline", "candidate"))
        for name in ("indirect", "shadowed"):
            assert body(a, name) == body(b, name), (level, name)
    if args.llvm_bin:
        cross_check(args.baseline, args.candidate, args.llvm_bin, args.out, fixtures, env)


if __name__ == "__main__":
    main()
