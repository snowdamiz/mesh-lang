#!/usr/bin/env bash
# Controller-owned SETUP. Install one producer-only ELF and prove its two modes.
set -euo pipefail
[[ "$(id -u)" == 0 && "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ $# == 1 && "$1" =~ ^[a-f0-9]{64}$ ]]
base=/var/lib/durable-jobs-bench/native
id=public-large-file-session
attempt=trusted-jobs-product-623c46b92cc5
source="$base/source-$attempt/source"
stock="$base/builds/$attempt"
out="$base/runs/$id"
stage="$base/source-$id"
packet="$source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition"
adapter="$packet/replay-$id"
tools="$base/toolchain"
compiler="$base/builds/mesh-stock-3828f20d/abi/meshc"
runner=af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355
manifest=979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55
runtime_sha=4e81280d27d218a3c1eaba8b10b33455f06280ca2297b48ad0563ddf96873584
export PATH="$tools/rust-1.99.0/bin:$tools/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tools/llvm-21.1.8" LC_ALL=C.UTF-8
export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$source/scripts/ci-x86_64-linux-gnu-clang"
umask 022

exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
test "$(cat /proc/sys/kernel/random/boot_id)" = 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a
test "$(findmnt -nro MAJ:MIN --target "$base")" = 8:0
test "$(getconf CLK_TCK)" = 100
test "$(llvm-config --version)" = 21.1.8
mkdir "$stage" "$out"
cat > "$stage/stage.tar.gz"
printf '%s  %s\n' "$1" "$stage/stage.tar.gz" | sha256sum -c
tar -xzf "$stage/stage.tar.gz" -C "$stage"
{
    printf '%s  %s\n' \
        "$manifest" "$stock/mesh.json" "$runner" "$stock/mesh" \
        623c46b92cc5f94eb19d455e36e6fca1e1b800231441a6de480e2c35aa091b02 "$base/source-$attempt/source.tar.gz" \
        e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$compiler" \
        b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$tools/pgsql-17.11/bin/postgres"
    python3 -I -B - "$stock" <<'PY'
import json
from pathlib import Path
import sys

directory = Path(sys.argv[1])
for path, digest in json.loads((directory / "mesh.json").read_text())["files"].items():
    print(digest, directory / path)
PY
} | sha256sum -c | tee "$out/pins.txt"
(cd "$stage" && sha256sum -c SHA256SUMS) >> "$out/pins.txt"
cmp "$stage/original-main.mpl" "$stock/main.mpl"
cmp "$stage/adapter/product_arm.py" "$packet/replay-closure-retry/product_arm.py"
cmp "$stage/adapter/identity.py" "$packet/replay-closure-retry/identity.py"

# Original public SETUP's exit trap deleted its build tree. Rebuild from its sealed
# archive in this dedicated tree and require byte equality to the original archive.
mkdir "$stage/rebuild" "$stage/tmp" "$stage/broker-source"
export TMPDIR="$stage/tmp" CARGO_TARGET_DIR="$stage/cargo"
tar -xzf "$base/source-$attempt/source.tar.gz" -C "$stage/rebuild"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$stage/rebuild/source/scripts/ci-x86_64-linux-gnu-clang"
(cd "$stage/rebuild/source" && cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt) \
    > "$out/runtime-build.log" 2>&1
runtime="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libmesh_rt.a"
printf '%s  %s\n' "$runtime_sha" "$runtime" | sha256sum -c | tee "$out/runtime-pin.txt"
sha256sum "$runtime" > "$out/runtime.sha256"

cp "$stock"/{mesh.toml,libworker_identity.a,worker_identity.c,worker_identity.mpl,writes.mpl} "$stage/broker-source/"
cp "$stage/broker-main.mpl" "$stage/broker-source/main.mpl"
broker="$stage/broker"
MESH_RT_LIB_PATH="$runtime" "$compiler" build "$stage/broker-source" --opt-level 2 --output "$broker" \
    > "$out/link.log" 2>&1
readelf -h -n "$broker" > "$out/elf.txt"
sha256sum "$broker" > "$out/broker.sha256"
broker_sha="$(cut -d ' ' -f1 "$out/broker.sha256")"
chmod a-w "$broker" "$stage"
mkdir "$adapter"
cp "$stage/adapter/"* "$adapter/"
chmod 755 "$adapter" "$adapter/native-run"
chmod a-w "$adapter" "$adapter/"*
sha256sum "$adapter/"* > "$out/adapter.sha256"

flock -u 9
exec 9<&-
for mode in inline file; do
    mkdir "$out/$mode"
    env PRODUCT_ARTIFACT="$stock" PRODUCT_COMPOSITION=product-bulk PRODUCT_TOPOLOGY=stock \
        PRODUCT_STORAGE=default PRODUCT_ELF_SHA256="$runner" PRODUCT_MANIFEST_SHA256="$manifest" \
        PRODUCT_SESSION_MODE="$mode" PRODUCT_SESSION_EXECUTABLE="$broker" PRODUCT_SESSION_SHA256="$broker_sha" \
        DURABLE_JOBS_PHASE=diagnostic taskset -c 0,1 "$adapter/native-run" mesh large "$out/$mode" \
        > "$out/$mode.log" 2>&1
done
touch "$out/SETUP-PASS"
