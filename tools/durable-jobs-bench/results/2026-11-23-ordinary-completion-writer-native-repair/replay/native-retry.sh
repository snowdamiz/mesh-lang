#!/usr/bin/env bash
set -euo pipefail
stage_sha="${1:?transfer SHA-256}"
key="${2:?unique retry attempt}"
[[ "$stage_sha" =~ ^[a-f0-9]{64}$ && "$key" =~ ^bulk-writer-retry-[a-f0-9]{12}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-$key"
out="$base/runs/$key"
runner="$base/builds/bulk-writer-b1dcd34af288"
broker="$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
tool="$base/toolchain"
export PATH="$tool/otp-29.1.1/bin:$tool/elixir-1.20.4/bin:/usr/sbin:/usr/bin:/bin"
export LC_ALL=C.UTF-8
umask 022
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$stage" "$out"
trap 'rm -f -- "$stage/incoming.tar.gz"' EXIT
cat > "$stage/incoming.tar.gz"
printf '%s  %s\n' "$stage_sha" "$stage/incoming.tar.gz" | sha256sum -c
tar -xzf "$stage/incoming.tar.gz" -C "$stage"
printf '%s  %s\n' \
    55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a "$broker/mesh" \
    7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911 "$broker/mesh.json" \
    93fef96704ae412cf3c55fae73cc2aac405c8b423670826b4876954a44c8b010 "$runner/mesh" \
    361f10d2c2866f694989198bb8359d29c91dd9928dcd95e8f92293293f226d29 "$runner/mesh.json" | sha256sum -c
cp "$runner/mesh.json" "$out/runner-build.json"
python3 -B - "$stage/prior-inventory.json" "$out/host.json" <<'PY'
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys

prior = json.loads(Path(sys.argv[1]).read_text())
boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
device = subprocess.check_output(["findmnt", "-no", "MAJ:MIN", "--target",
                                  "/var/lib/durable-jobs-bench/native"], text=True).strip()
clock_ticks = int(subprocess.check_output(["getconf", "CLK_TCK"], text=True))
host = [boot, platform.release(), device]
assert host == prior["host"] and clock_ticks == prior["clock_ticks"]
assert platform.machine() == "x86_64" and prior["cpus"] == [0, 1]
for label in ("bulk", "river", "oban", "apalis"):
    lane = prior["lanes"][label]
    for path, expected in lane["inventory"]["engines"][lane["engine"]]["verified_files"].items():
        with Path(path).open("rb") as stream:
            assert hashlib.file_digest(stream, "sha256").hexdigest() == expected, path
Path(sys.argv[2]).write_text(json.dumps(dict(host=host, clock_ticks=clock_ticks)) + "\n")
PY

flock -u 9
exec 9<&-
for mode in off zero; do
    mkdir -m 0700 "$out/smoke-$mode"
    COMBINED_WRITER_MODE="$mode" COMBINED_RUNNER_ARTIFACTS="$runner" DURABLE_JOBS_PHASE=diagnostic \
        taskset -c 0,1 "$stage/replay/combined-run" mesh short "$out/smoke-$mode" >"$out/smoke-$mode.log" 2>&1
done

python3 -B - "$stage" "$out" "$runner" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

stage, out, runner = map(Path, sys.argv[1:])
wrapper = stage / "replay/combined-run"
files = [wrapper, stage / "replay/split_arm.py", stage / "replay/identity.py",
         runner / "mesh", runner / "mesh.json"]
with (out / "native-seal.json").open("x") as stream:
    json.dump(dict(runner_directory=str(runner), wrapper=str(wrapper),
                   files={str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in files}), stream)
PY
