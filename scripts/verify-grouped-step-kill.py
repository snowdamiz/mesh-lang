#!/usr/bin/env python3
"""Build and exercise generated grouped actor entries with and without step lowering."""

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

FIXTURE = Path(__file__).resolve().parents[1] / "compiler/meshc/tests/fixtures/grouped_step_kill"
CASES = [
    ("direct", "1"), ("direct", "7"), ("linked", "1"), ("linked", "7"),
    ("prestart",), ("concurrent",), ("fair",),
]


def verify(args, work, step):
    env = dict(os.environ, MESH_GROUPED_STEP=str(step), MESH_RT_LIB_PATH=str(args.runtime))
    for name in ("MESH_SCHEDULER_MIN_WORKERS", "MESH_SCHEDULER_MAX_WORKERS"):
        env.pop(name, None)

    def run(command, **kwargs):
        return subprocess.run(command, cwd=work, env=env, check=True, timeout=60, **kwargs)

    binary = work / "fixture"
    run([args.meshc, "build", "--opt-level", "2", "--emit-llvm", FIXTURE, "-o", binary])
    if run([binary], capture_output=True, text=True).stdout.splitlines() != [
        "1", "4", "3", "7", "10", "10"
    ]:
        raise RuntimeError("ordinary generated fixture output changed")

    ir = binary.with_suffix(".ll").read_text()
    launcher = re.search(r"^define i64 @Main__start_victim\(.*?^}", ir, re.M | re.S)
    spawn = "mesh_actor_spawn_step" if step else "mesh_actor_spawn_grouped"
    if not launcher or f"call i64 @{spawn}(ptr @rotate_words," not in launcher[0]:
        raise RuntimeError(f"expected {spawn} in generated fixture launcher")

    # Use the real generated launch site (including its shape, bound and entry).
    # Expose a barrier only at the send and let the C driver own main().
    if ir.count("define i32 @main(") != 1 or "call i64 @mesh_actor_send(" not in ir:
        raise RuntimeError("expected a generated main and a scalar send")
    ir = ir.replace("define i32 @main(", "define i32 @fixture_main(")
    instrumented = work / "observed.ll"
    instrumented.write_text(ir.replace("@mesh_actor_send(", "@observed_send("))
    harness = work / "harness"
    libraries = ["-lm", "-lpthread", "-ldl"]
    if sys.platform == "darwin":
        libraries.extend(["-framework", "Security", "-framework", "CoreFoundation"])
    run([args.clang, "-O2", "-std=c11", "-Wall", "-Wextra", "-Werror", "-Wno-override-module", FIXTURE / "harness.c",
         instrumented, args.runtime, *libraries, "-o", harness])

    failures = []
    for case in CASES:
        label = f"MESH_GROUPED_STEP={step} {' '.join(case)}"
        try:
            run([harness, *case])
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
            failures.append(label)
            print(f"FAIL {label}: {exc}", file=sys.stderr, flush=True)
        else:
            print(f"PASS {label}", flush=True)
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--meshc", required=True, type=Path)
    parser.add_argument("--runtime", required=True, type=Path)
    parser.add_argument("--clang", default="clang", help="clang compatible with meshc's LLVM")
    args = parser.parse_args()
    args.meshc = args.meshc.resolve(strict=True)
    args.runtime = args.runtime.resolve(strict=True)
    failures = []
    with tempfile.TemporaryDirectory(prefix="mesh-grouped-kill-") as tmp:
        for step in (0, 1):
            failures.extend(verify(args, Path(tmp), step))
    return int(bool(failures))


if __name__ == "__main__":
    sys.exit(main())
