#!/usr/bin/env bash
set -euo pipefail
[[ $# == 2 && "$1" =~ ^r2-d2-producer-boundary-[a-f0-9]{12}$ && "$2" =~ ^[a-f0-9]{64}$ ]]
id="$1"
packet="$2"
base=/var/lib/durable-jobs-bench/native
tool="$base/toolchain"
stock="$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
abi="$base/builds/mesh-stock-3828f20d/abi"
runner="$base/builds/exit-bulk-d2-c7c50e726ca0"
home="$base/source-$id"
out="$base/runs/$id"
umask 022
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$home" "$out" "$home/tmp"
trap 'rm -rf -- "$home/tree-short" "$home/cargo" "$home/tmp" "$home/packet.tar.gz"' EXIT
cd "$home"
cat > packet.tar.gz
printf '%s  packet.tar.gz\n' "$packet" | sha256sum -c
tar -xzf packet.tar.gz
sha256sum -c native-pins.txt >"$out/resident-pin-check.txt"
[[ "$(uname -s):$(uname -m):$(uname -r)" == Linux:x86_64:6.8.0-149-generic ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt --raw --noheadings --output MAJ:MIN --target "$base")" == 8:0 ]]
[[ "$(getconf CLK_TCK)" == 100 ]]
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin" LC_ALL=C.UTF-8
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" CARGO_HOME="$base/cargo-home"
export TMPDIR="$home/tmp" CARGO_TARGET_DIR="$home/cargo"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$home/tree-short/scripts/ci-x86_64-linux-gnu-clang"
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
rustc --version | tee "$out/rustc.txt"
llvm-config --version | tee "$out/llvm.txt"
grep -q '^rustc 1.99.0 (b940084d7 ' "$out/rustc.txt"
grep -Fxq 21.1.8 "$out/llvm.txt"
python3 -B materialize.py tree-short probe.rs >"$out/runtime-pins.json"
cmp "$out/runtime-pins.json" runtime-pins.json
(
    cd tree-short
    cargo build --release --offline --locked -p mesh-rt
) >"$out/build.log" 2>&1
export MESH_RT_LIB_PATH="$CARGO_TARGET_DIR/release/libmesh_rt.a"
mkdir "$home/broker"
python3 -B - "$stock" "$home/broker" <<'PY'
from pathlib import Path
import hashlib,json,shutil,sys
stock, broker = map(Path,sys.argv[1:])
manifest = json.loads((stock / "mesh.json").read_text())
for name, expected in manifest["files"].items():
    source = stock / name
    assert hashlib.sha256(source.read_bytes()).hexdigest() == expected
    if name != "mesh":
        shutil.copy2(source, broker / name)
PY
"$abi/meshc" build "$home/broker" --opt-level 2 --output "$home/broker/mesh" >"$out/link.log" 2>&1
file "$home/broker/mesh" | tee "$out/broker-elf.txt"
grep -q 'ELF 64-bit.*x86-64' "$out/broker-elf.txt"
python3 -B - "$stock/mesh.json" "$home/broker" "$out" "$MESH_RT_LIB_PATH" <<'PY'
from pathlib import Path
import hashlib,json,sys
stock,broker,out,runtime = map(Path,sys.argv[1:])
def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()
manifest = json.loads(stock.read_text())
manifest["files"]["mesh"] = sha(broker / "mesh")
manifest["runtime_sha256"] = sha(runtime)
manifest["benchmark_observer"] = "r2-d2-producer-boundary"
assert all(sha(broker / f) == digest for f,digest in manifest["files"].items())
path = broker / "mesh.json"
path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n")
image = dict(elf=manifest["files"]["mesh"], manifest=sha(path), runtime=manifest["runtime_sha256"])
(out / "image.json").write_text(json.dumps(image, sort_keys=True, indent=2) + "\n")
(out / "broker-manifest.json").write_bytes(path.read_bytes())
PY
# The exact same native lock works across proof and timing. The staged arm
# takes it itself while the controller's outer remote lock stays held.
flock -u 9
exec 9<&-
mkdir "$out/smoke"
export EXIT_BULK_CHECKOUT=d2 COMBINED_RUNNER_ARTIFACTS="$runner"
export EXIT_BULK_RUNNER_ELF=972514efac688fd029821152c3bb70a7e62bf1c6227ab8b2d061dc69c47a2dc7
export EXIT_BULK_RUNNER_MANIFEST=ab09bc77affff4496350202150dc6eb0b90d5c252f0a7552767e0070d615fe31
export R2_OBSERVED_BROKER="$home/broker/mesh"
export R2_OBSERVED_BROKER_SHA="$(sha256sum "$home/broker/mesh" | cut -d' ' -f1)"
export R2_OBSERVED_MANIFEST_SHA="$(sha256sum "$home/broker/mesh.json" | cut -d' ' -f1)"
DURABLE_JOBS_PHASE=diagnostic R2_OBSERVATION=nested taskset --cpu-list 0,1 "$home/replay/combined-run" mesh short "$out/smoke"
touch "$out/SETUP-PASS"
