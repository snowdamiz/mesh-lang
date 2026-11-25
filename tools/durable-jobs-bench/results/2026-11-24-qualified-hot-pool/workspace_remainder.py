"""Run the workspace test binaries Cargo did not reach after a failed suite."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys

lane = Path(sys.argv[1]).resolve()
checkout = lane / "source"
logs = lane / "logs"
original = logs / "workspace-correctness.log"
status = logs / "status"
if not re.search(r"^workspace-correctness [1-9][0-9]*$", status.read_text(), re.MULTILINE):
    raise SystemExit("the original workspace command must have exited with a failure")

env = os.environ.copy()
env["PATH"] = os.pathsep.join(map(str, (
    Path.home() / ".rustup/toolchains/1.99.0-aarch64-apple-darwin/bin",
    Path("/opt/homebrew/opt/llvm@21/bin"),
    lane.parent / "jobs-lean-native-v3/variants/timerfd/retry/tmp/toolchain/zig-aarch64-macos-0.15.2",
    env["PATH"],
)))
env.update(
    LLVM_SYS_211_PREFIX="/opt/homebrew/opt/llvm@21",
    PG_BIN_DIR="/opt/homebrew/opt/postgresql@17/bin",
    TMPDIR=str(lane / "tmp"),
    CARGO_TARGET_DIR=str(lane / "build"),
    CARGO_INCREMENTAL="0",
    CARGO_BUILD_JOBS="5",
    PYTHONDONTWRITEBYTECODE="1",
)
for key in ("MESH_TEST_DATABASE_URL", "MESH_JOBS_TEST_PG_URL"):
    env.pop(key, None)

command = ["cargo", "test", "--locked", "--offline"]
skip = ["--", "--skip", "the_performance_gate_measures_and_reports", "--skip", "e2e_performance"]
reached = set(re.findall(r"^\s*Running .+ \((.+)\)$", original.read_text(), re.MULTILINE))
if not reached:
    raise SystemExit("no test binaries found in the original log")

code = 0
with (logs / "workspace-remainder.log").open("x") as log:
    inventory_command = command + ["--workspace", "--no-run", "--message-format=json"]
    inventory = subprocess.check_output(inventory_command, cwd=checkout, env=env, stderr=log)
    metadata = subprocess.check_output(
        ["cargo", "metadata", "--offline", "--locked", "--no-deps", "--format-version", "1"],
        cwd=checkout, env=env, stderr=log,
    )
    package_names = {item["id"]: item["name"] for item in json.loads(metadata)["packages"]}
    by_package = {}
    for line in inventory.splitlines():
        item = json.loads(line)
        binary = item.get("executable")
        if not binary or not item["profile"]["test"] or binary in reached:
            continue
        target = item["target"]
        package = package_names[item["package_id"]]
        kind = target["kind"][0]
        if kind == "test" and target["name"] == "e2e_performance":
            print("Excluded compile timing target e2e_performance", file=log, flush=True)
            continue
        options = by_package.setdefault(package, [])
        if kind == "lib" or "lib" in target["crate_types"]:
            options.append("--lib")
        elif kind in ("test", "bin"):
            options.extend((f"--{kind}", target["name"]))
        else:
            raise ValueError(f"unexpected workspace test target {target}")
    commands = [
        command + ["--no-fail-fast", "-p", package] + options + skip
        for package, options in by_package.items()
    ]
    commands.append(command + ["--workspace", "--no-fail-fast", "--doc"] + skip)
    for args in commands:
        print(f"$ {' '.join(args)}", file=log, flush=True)
        result = subprocess.run(args, cwd=checkout, env=env, stdout=log, stderr=log)
        code = code or result.returncode
with status.open("a") as stream:
    stream.write(f"workspace-remainder {code}\n")
sys.exit(code)
