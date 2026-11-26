#!/usr/bin/env bash
set -euo pipefail
archive_hash="${1:?archive SHA-256}"
key="${2:?unique SETUP attempt}"
source_hash="${3:?release-exit patch SHA-256}"
[[ "$archive_hash" =~ ^[a-f0-9]{64}$ && "$source_hash" =~ ^[a-f0-9]{64}$ ]]
[[ "$key" =~ ^bulk-writer-[a-f0-9]{12}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-$key"
out="$base/runs/$key"
runner="$base/builds/$key"
stock="$base/builds/mesh-stock-3828f20d/abi"
export COMBINED_BULK_SOURCE="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
broker="$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2/mesh"
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
mkdir "$stage/tmp" "$stage/witness"
cat > "$stage/incoming.tar.gz"
printf '%s  %s\n' "$archive_hash" "$stage/incoming.tar.gz" | sha256sum -c
tar -xzf "$stage/incoming.tar.gz" -C "$stage"
rm "$stage/incoming.tar.gz"
export TMPDIR="$stage/tmp/" CARGO_TARGET_DIR="$stage/cargo-target"
trap 'rm -rf -- "$CARGO_TARGET_DIR"; rm -f -- "$stage/incoming.tar.gz"' EXIT
printf '%s  %s\n' \
    e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$stock/meshc" \
    7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911 "$broker.json" \
    55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a "$broker" \
    "$source_hash" "$stage/replay/release-exit.patch" | sha256sum -c
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
for label in ("bulk","river","oban","apalis"):
    lane = prior["lanes"][label]
    spec = lane["inventory"]["engines"][lane["engine"]]
    for path, expected in spec["verified_files"].items():
        with Path(path).open("rb") as stream:
            assert hashlib.file_digest(stream, "sha256").hexdigest() == expected, path
Path(sys.argv[2]).write_text(json.dumps(dict(host=host, clock_ticks=clock_ticks))+"\n")
PY
mkdir "$CARGO_TARGET_DIR"
if [[ -d "$base/cargo/release" ]]; then
    cp -a --reflink=auto "$base/cargo/release" "$CARGO_TARGET_DIR/release"
fi
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$stage/source/scripts/ci-x86_64-linux-gnu-clang"
export COMPLETION_WRITER_SOURCE_SHA256="$source_hash"
(cd "$stage/source" && cargo build --offline --locked --release -p mesh-rt) >"$out/runner-build.log" 2>&1
runtime="$CARGO_TARGET_DIR/release/libmesh_rt.a"
bash "$stage/source/replay/link.sh" "$stock/meshc" "$runtime" "$runner" >"$out/runner-link.log" 2>&1
cp "$runner/mesh.json" "$out/runner-build.json"
python3 -B "$stage/replay/teardown-fixture.py" "$stage/source" "$stage/witness/source"
bash "$stage/witness/source/replay/link.sh" "$stock/meshc" "$runtime" "$stage/witness/artifacts" \
    >"$out/teardown-link.log" 2>&1
chown nobody:nogroup "$stage/tmp"
mkdir "$out/proof"
chown nobody:nogroup "$out/proof"
proof_env=(env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" "TMPDIR=$TMPDIR"
           "COMBINED_BULK_SOURCE=$COMBINED_BULK_SOURCE" LC_ALL=C.UTF-8)
runuser -u nobody -- "${proof_env[@]}" taskset -c 0,1 python3 -B "$stage/replay/split-faults.py" \
    "$out/proof/faults" "$broker" "$runner/mesh" >"$out/faults.log" 2>&1
for mode in off zero; do
    runuser -u nobody -- "${proof_env[@]}" taskset -c 0,1 python3 -B "$stage/replay/teardown.py" \
        "$out/proof/teardown-$mode" "$broker" "$stage/witness/artifacts/mesh" "$mode" \
        >"$out/teardown-$mode.log" 2>&1
done
flock -u 9
exec 9<&-
for mode in off zero; do
    mkdir -m 0700 "$out/smoke-$mode"
    COMBINED_WRITER_MODE="$mode" COMBINED_RUNNER_ARTIFACTS="$runner" \
        taskset -c 0,1 "$stage/replay/combined-run" mesh short "$out/smoke-$mode" >"$out/smoke-$mode.log" 2>&1
done
python3 -B - "$stage" "$out" "$runner" <<'PY'
import hashlib
import json
from pathlib import Path
import sys
stage,out,runner=map(Path,sys.argv[1:])
files=[stage/"replay/combined-run",stage/"replay/split_arm.py",runner/"mesh",runner/"mesh.json"]
with (out/"native-seal.json").open("x") as stream:
    json.dump(dict(runner_directory=str(runner), wrapper=str(files[0]),
                   files={str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in files}),stream)
PY
