#!/usr/bin/env python3
"""Line coverage of the Mesh compiler crates.

    scripts/compiler-coverage.py run [--crates=a,b] [cargo test args...]  # measure
    scripts/compiler-coverage.py report [crate-or-path...]   # summarize

`run` runs the compiler crates' tests under `cargo llvm-cov` and writes
target/coverage/lcov.info. Mesh programs built by the tests link a runtime
built without instrumentation (an instrumented one fails to link), so the
runtime crates (mesh-rt, mesh-test-rt) are measured by their own tests, not
here. `report` prints each crate's coverage and each file's uncovered lines,
leaving out test code: `tests/` directories and `#[cfg(test)]` modules.

Needs cargo-llvm-cov, and LLVM_COV/LLVM_PROFDATA from the LLVM the Rust
toolchain uses when it is not rustup's (Homebrew's rustc: llvm@21).
"""

import os
import re
import shutil
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LCOV = ROOT / "target/coverage/lcov.info"
RUNTIME_SNAPSHOT = ROOT / "target/coverage/runtime"
CRATES = [
    "mesh-common", "mesh-lexer", "mesh-parser", "mesh-typeck", "mesh-codegen",
    "mesh-fmt", "mesh-lint", "mesh-pkg", "mesh-lsp", "mesh-repl", "meshc", "meshpkg",
]


def llvm_env():
    env = dict(os.environ)
    for tool in ("llvm-cov", "llvm-profdata"):
        var = tool.upper().replace("-", "_")
        if var not in env:
            brew = Path("/opt/homebrew/opt/llvm@21/bin") / tool
            if brew.exists():
                env[var] = str(brew)
    return env


def run(extra):
    crates = CRATES
    if extra and extra[0].startswith("--crates="):
        crates = extra[0].split("=", 1)[1].split(",")
        extra = extra[1:]
    env = llvm_env()
    # The runtime programs link, built as usual and copied aside so a test
    # that rebuilds it cannot swap it mid-run.
    subprocess.run(["cargo", "build", "--locked", "-p", "mesh-rt", "-p", "mesh-test-rt"],
                   cwd=ROOT, check=True)
    RUNTIME_SNAPSHOT.mkdir(parents=True, exist_ok=True)
    for lib in ("libmesh_rt.a", "libmesh_test_rt.a"):
        shutil.copy2(ROOT / "target/debug" / lib, RUNTIME_SNAPSHOT / lib)
    env["MESH_RT_LIB_PATH"] = str(RUNTIME_SNAPSHOT / "libmesh_rt.a")
    env["MESH_TEST_RT_LIB_PATH"] = str(RUNTIME_SNAPSHOT / "libmesh_test_rt.a")
    packages = [arg for crate in crates for arg in ("-p", crate)]
    subprocess.run(["cargo", "llvm-cov", "clean", "--workspace"], cwd=ROOT, env=env, check=True)
    test = subprocess.run(
        ["cargo", "llvm-cov", "--no-report", "--locked", *packages, "--no-fail-fast", *extra],
        cwd=ROOT, env=env)
    LCOV.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["cargo", "llvm-cov", "report", "--lcov", "--output-path", str(LCOV)],
                   cwd=ROOT, env=env, check=True)
    return test.returncode


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
    """{file: {line: hits}} for the compiler crates' non-test code."""
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
    print(f"compiler: {100 * covered / max(total, 1):.2f}% ({covered}/{total} lines)")
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
