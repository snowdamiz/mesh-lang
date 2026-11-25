"""Complete SETUP's archive-member checks before the controller opens TIMING."""

import fcntl
from pathlib import Path
import subprocess

from controller import HERE, ID, document, matrix, parked, run, timing_plan

CHECK = r"""
set -o pipefail
stock="$1"
stage="$2"
toolchain_driven_relink="$3"

printf '%s  %s\n' "$4" "$stock/mesh.json" | sha256sum --strict -c
python3 -I -B - "$stock" "$stage" "$toolchain_driven_relink" <<'PIN' | sha256sum --strict -c
import json
from pathlib import Path
import sys

stock, stage = map(Path, sys.argv[1:3])
members = json.loads((stock / "mesh.json").read_text())["files"]
linked = ["broker-source"]
if sys.argv[3] == "true":
    linked.append("relink-source")
for name, digest in sorted(members.items()):
    print(f"{digest}  {stock / name}")
    for directory in linked:
        if name != "mesh" and (directory == "relink-source" or name != "main.mpl"):
            print(f"{digest}  {stage / directory / name}")
PIN
printf '%s  %s\n' 38cf5bc67dfe9a657ff7682226c73634eff9d9b78d735048776dbe7c0aac9577 \
    "$stage/broker-source/main.mpl" | sha256sum --strict -c
printf '%s  %s\n' "$5" "$stage/libmesh_rt.a" "$6" "$stage/broker" | sha256sum --strict -c
if [[ "$toolchain_driven_relink" == true ]]; then
    printf '%s  %s\n' "$7" "$stage/relink-control" | sha256sum --strict -c
fi
"""


def main():
    doc = document()
    release = run.read_json(parked.QUEUE / f"{ID}.released.json")
    assert (release["id"], release["phase"], release["exit_status"]) == (ID, "setup", 0)
    setup = Path(release["receipt"])
    assert setup == HERE / "evidence/native/setup"
    proof = timing_plan(doc, setup)["session_qualification"]
    command = ["bash", "-c", matrix.HOST_IDLE + CHECK, "--", str(parked.STOCK.parent),
               str(parked.REMOTE_BROKER.parent), str(bool(proof["relink_sha256"])).lower(),
               doc["manifest_sha256"], proof["runtime_sha256"], proof["broker_sha256"],
               proof["relink_sha256"] or ""]
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = run.ssh(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=45)
    path = setup.parent / "setup-member-pins.txt"
    with path.open("xb") as stream:
        stream.write(result.stdout)
    result.check_returncode()
    print(path)


if __name__ == "__main__":
    main()
