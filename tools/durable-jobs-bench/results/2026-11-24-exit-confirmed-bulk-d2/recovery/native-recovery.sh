#!/usr/bin/env bash
set -euo pipefail
stage_sha="${1:?transfer digest}"
attempt="${2:?recovery attempt}"
source_sha="${3:?capsule digest}"
[[ "$stage_sha" =~ ^[a-f0-9]{64}$ && "$source_sha" =~ ^[a-f0-9]{64}$ ]]
[[ "$attempt" =~ ^exit-bulk-d2-retry-[a-f0-9]{12}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-$attempt"
out="$base/runs/$attempt"
candidate="$base/builds/exit-bulk-d2-c7c50e726ca0"
baseline="$base/builds/bulk-writer-b1dcd34af288"
broker="$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
bulk_source="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
candidate_elf=972514efac688fd029821152c3bb70a7e62bf1c6227ab8b2d061dc69c47a2dc7
candidate_manifest=ab09bc77affff4496350202150dc6eb0b90d5c252f0a7552767e0070d615fe31
tool="$base/toolchain"
export PATH="$tool/otp-29.1.1/bin:$tool/elixir-1.20.4/bin:/usr/sbin:/usr/bin:/bin"
export PG_BIN_DIR="$tool/pgsql-17.11/bin" LC_ALL=C.UTF-8
umask 022

exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$stage" "$out"
trap 'rm -f -- "$stage/incoming.tar"; if [[ -n "${cert:-}" ]]; then rm -rf -- "$cert"; fi' EXIT
cat > "$stage/incoming.tar"
printf '%s  %s\n' "$stage_sha" "$stage/incoming.tar" | sha256sum -c
tar -xOf "$stage/incoming.tar" stage.py | python3 -I -B - "$stage" "$stage_sha" "$source_sha" > "$out/staged-files.json"
printf '%s  %s\n' \
    "$candidate_elf" "$candidate/mesh" "$candidate_manifest" "$candidate/mesh.json" \
    93fef96704ae412cf3c55fae73cc2aac405c8b423670826b4876954a44c8b010 "$baseline/mesh" \
    361f10d2c2866f694989198bb8359d29c91dd9928dcd95e8f92293293f226d29 "$baseline/mesh.json" \
    55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a "$broker/mesh" \
    7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911 "$broker/mesh.json" \
    | sha256sum -c
cp "$baseline/mesh.json" "$out/baseline-build.json"
cp "$candidate/mesh.json" "$out/runner-build.json"
python3 -I -B - "$stage/pinned-native.json" "$out/host.json" "$PG_BIN_DIR/postgres" <<'PY'
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys

prior = json.loads(Path(sys.argv[1]).read_text())
boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
mount = subprocess.check_output(["findmnt", "-no", "MAJ:MIN", "--target",
                                 "/var/lib/durable-jobs-bench/native"], text=True).strip()
clock_ticks = int(subprocess.check_output(["getconf", "CLK_TCK"], text=True))
host = [boot, platform.release(), mount]
assert host == prior["host"] and clock_ticks == prior["clock_ticks"]
assert platform.machine() == "x86_64" and prior["cpus"] == [0, 1]
assert subprocess.check_output([sys.argv[3], "--version"], text=True).split()[2] == "17.11"
for path, digest in prior["files"].items():
    with Path(path).open("rb") as stream:
        assert hashlib.file_digest(stream, "sha256").hexdigest() == digest, path
Path(sys.argv[2]).write_text(json.dumps(dict(host=host, clock_ticks=clock_ticks)) + "\n")
PY

install -d -m 0700 -o nobody -g nogroup "$stage/tmp" "$out/proof"
export TMPDIR="$stage/tmp/"
cert="$stage/tmp/relay-probe"
mkdir "$cert"
chown nobody:nogroup "$cert"
runuser -u nobody -- openssl req -x509 -nodes -newkey rsa:2048 -days 1 -subj /CN=staged-relay \
    -out "$cert/server.crt" -keyout "$cert/server.key" > "$out/relay-cert.log" 2>&1
cp "$cert/server.crt" "$cert/cert.pem"
proof_env=(env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" "TMPDIR=$TMPDIR"
           "COMBINED_BULK_SOURCE=$bulk_source" LC_ALL=C.UTF-8)
py=(python3 -B -X "pycache_prefix=$stage/no-cache")
runuser -u nobody -- "${proof_env[@]}" taskset -c 0,1 "${py[@]}" \
    "$stage/replay/staged-relay-check.py" "$cert" > "$out/relay-check.json"
rm -rf -- "$cert"
runuser -u nobody -- "${proof_env[@]}" taskset -c 0,1 "${py[@]}" \
    "$stage/replay/faults.py" "$out/proof/d2" "$broker/mesh" "$candidate/mesh" d2 > "$out/faults-d2.log" 2>&1
runuser -u nobody -- "${proof_env[@]}" taskset -c 0,1 "${py[@]}" \
    "$stage/replay/faults.py" "$out/proof/off" "$broker/mesh" "$baseline/mesh" off \
    lost-application-reply lost-claim-reply lost-terminal-reply > "$out/faults-off.log" 2>&1

flock -u 9
exec 9<&-
for mode in off d2; do
    mkdir -m 0700 "$out/smoke-$mode"
    runner="$baseline"
    if [[ "$mode" == d2 ]]; then runner="$candidate"; fi
    EXIT_BULK_CHECKOUT="$mode" COMBINED_RUNNER_ARTIFACTS="$runner" DURABLE_JOBS_PHASE=diagnostic \
        EXIT_BULK_RUNNER_ELF="$candidate_elf" EXIT_BULK_RUNNER_MANIFEST="$candidate_manifest" \
        taskset -c 0,1 "$stage/replay/combined-run" mesh short "$out/smoke-$mode" > "$out/smoke-$mode.log" 2>&1
done

python3 -I -B - "$stage" "$out" "$baseline" "$candidate" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

stage, out, baseline, candidate = map(Path, sys.argv[1:])
replay = stage / "replay"
assert not (stage / "no-cache").exists()
wrapper = replay / "combined-run"
files = [wrapper, replay / "split_arm.py", replay / "identity.py",
         baseline / "mesh", baseline / "mesh.json", candidate / "mesh", candidate / "mesh.json"]
seal = dict(wrapper=str(wrapper), baseline_directory=str(baseline), candidate_directory=str(candidate),
            files={str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in files})
(out / "native-seal.json").write_text(json.dumps(seal, indent=2) + "\n")
PY
