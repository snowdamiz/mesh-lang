#!/usr/bin/env bash
set -euo pipefail
stage_sha="${1:?stage SHA-256}"
key="${2:?attempt}"
source_sha="${3:?combined source SHA-256}"
delta_sha="${4:?D2 transfer SHA-256}"
[[ "$stage_sha" =~ ^[a-f0-9]{64}$ && "$key" =~ ^exit-bulk-d2-[a-f0-9]{12}$ ]]
[[ "$source_sha" =~ ^[a-f0-9]{64}$ && "$delta_sha" =~ ^[a-f0-9]{64}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-$key"
out="$base/runs/$key"
candidate="$base/builds/$key"
retained="$base/builds/bulk-writer-b1dcd34af288"
broker="$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
bulk_source="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
stock="$base/builds/mesh-stock-3828f20d/abi"
tool="$base/toolchain"
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" CARGO_HOME="$base/cargo-home"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0 LC_ALL=C.UTF-8
export PG_BIN_DIR="$tool/pgsql-17.11/bin"
umask 022

exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$stage" "$out"
mkdir "$stage/tmp"
export TMPDIR="$stage/tmp/" CARGO_TARGET_DIR="$stage/cargo-target"
trap 'rm -rf -- "$CARGO_TARGET_DIR"; rm -f -- "$stage/incoming.tar"' EXIT
cat > "$stage/incoming.tar"
printf '%s  %s\n' "$stage_sha" "$stage/incoming.tar" | sha256sum -c
tar -xf "$stage/incoming.tar" -C "$stage"
printf '%s  %s\n' \
    "$source_sha" "$stage/combined-source.tar.gz" \
    "$delta_sha" "$stage/d2-transfer.patch" \
    e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$stock/meshc" \
    55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a "$broker/mesh" \
    7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911 "$broker/mesh.json" \
    93fef96704ae412cf3c55fae73cc2aac405c8b423670826b4876954a44c8b010 "$retained/mesh" \
    361f10d2c2866f694989198bb8359d29c91dd9928dcd95e8f92293293f226d29 "$retained/mesh.json" \
    | sha256sum -c
cp "$retained/mesh.json" "$out/baseline-build.json"
python3 -I -B - "$stage/pinned-native.json" "$out/host.json" <<'PY'
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
for path, expected in prior["files"].items():
    with Path(path).open("rb") as stream:
        assert hashlib.file_digest(stream, "sha256").hexdigest() == expected, path
Path(sys.argv[2]).write_text(json.dumps(dict(host=host, clock_ticks=clock_ticks)) + "\n")
PY

tar -xzf "$stage/combined-source.tar.gz" -C "$stage"
mkdir "$CARGO_TARGET_DIR"
if [[ -d "$base/cargo/release" ]]; then
    cp -a --reflink=auto "$base/cargo/release" "$CARGO_TARGET_DIR/release"
fi
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$stage/source/scripts/ci-x86_64-linux-gnu-clang"
(cd "$stage/source" && cargo build --offline --locked --release -p mesh-rt) >"$out/runner-build.log" 2>&1
runtime="$CARGO_TARGET_DIR/release/libmesh_rt.a"
bash "$stage/replay/link-candidate.sh" "$stage/source" "$stock/meshc" "$runtime" "$candidate" \
    "$source_sha" "$delta_sha" >"$out/runner-link.log" 2>&1
cp "$candidate/mesh.json" "$out/runner-build.json"
candidate_elf="$(sha256sum "$candidate/mesh" | cut -d' ' -f1)"
candidate_manifest="$(sha256sum "$candidate/mesh.json" | cut -d' ' -f1)"
chmod a-w "$candidate" "$candidate/mesh" "$candidate/mesh.json"

chown nobody:nogroup "$stage/tmp"
mkdir "$out/proof"
chown nobody:nogroup "$out/proof"
proof_env=(env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" "TMPDIR=$TMPDIR"
           "COMBINED_BULK_SOURCE=$bulk_source" LC_ALL=C.UTF-8)
runuser -u nobody -- "${proof_env[@]}" taskset -c 0,1 python3 -B "$stage/replay/faults.py" \
    "$out/proof/d2" "$broker/mesh" "$candidate/mesh" d2 >"$out/faults-d2.log" 2>&1

flock -u 9
exec 9<&-
for mode in off d2; do
    mkdir -m 0700 "$out/smoke-$mode"
    runner="$retained"
    if [[ "$mode" == d2 ]]; then runner="$candidate"; fi
    EXIT_BULK_CHECKOUT="$mode" COMBINED_RUNNER_ARTIFACTS="$runner" DURABLE_JOBS_PHASE=diagnostic \
        EXIT_BULK_RUNNER_ELF="$candidate_elf" EXIT_BULK_RUNNER_MANIFEST="$candidate_manifest" \
        taskset -c 0,1 "$stage/replay/combined-run" mesh short "$out/smoke-$mode" >"$out/smoke-$mode.log" 2>&1
done
python3 -I -B - "$stage" "$out" "$retained" "$candidate" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

stage, out, retained, candidate = map(Path, sys.argv[1:])
wrapper = stage / "replay/combined-run"
files = [wrapper, stage / "replay/split_arm.py", stage / "replay/identity.py",
         retained / "mesh", retained / "mesh.json", candidate / "mesh", candidate / "mesh.json"]
seal = dict(wrapper=str(wrapper), baseline_directory=str(retained), candidate_directory=str(candidate),
            files={str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in files})
(out / "native-seal.json").write_text(json.dumps(seal, indent=2) + "\n")
PY
