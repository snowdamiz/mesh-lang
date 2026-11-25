#!/usr/bin/env bash
set -euo pipefail
[[ $# == 3 && "$(uname -sm)" == 'Linux x86_64' && "$(id -u)" == 0 ]]
bundle="$1"
revision="$2"
stage_id="$3"
[[ "$bundle" =~ ^[a-f0-9]{64}$ && "$revision" =~ ^[a-f0-9]{40}$ && "$stage_id" =~ ^[a-f0-9]{12}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-dense-producer-1024-$stage_id"
report="$stage/setup"
tools="$base/toolchain"
stock="$base/builds/trusted-jobs-product-623c46b92cc5"
frozen="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
compiler="$base/builds/mesh-stock-3828f20d/abi/meshc"
export PATH="$tools/rust-1.99.0/bin:$tools/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tools/llvm-21.1.8" PG_BIN_DIR="$tools/pgsql-17.11/bin"
export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 LC_ALL=C.UTF-8
umask 022
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$stage" "$report"
trap 'status=$?; printf "%s\n" "$status" > "$report/exit-status"; tar -C "$report" -czf - .; exit "$status"' EXIT
cat > "$stage/source.tar.gz"
printf '%s  %s\n' "$bundle" "$stage/source.tar.gz" | sha256sum -c >&2
tar -xzf "$stage/source.tar.gz" -C "$stage"
printf '%s  %s\n' \
  e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$compiler" \
  b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$PG_BIN_DIR/postgres" \
  af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355 "$stock/mesh" \
  979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55 "$stock/mesh.json" \
  | sha256sum -c > "$report/pins.log"
test "$(cat /proc/sys/kernel/random/boot_id)" = 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a
test "$(uname -r)" = 6.8.0-149-generic
test "$(findmnt -rn -o MAJ:MIN --target "$base")" = 8:0
test "$(getconf CLK_TCK)" = 100
test "$(llvm-config --version)" = 21.1.8
export CARGO_TARGET_DIR="$stage/cargo" TMPDIR="$stage/tmp"
mkdir "$CARGO_TARGET_DIR" "$TMPDIR"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$stage/source/scripts/ci-x86_64-linux-gnu-clang"
(
  rustc -vV
  llvm-config --version
  "$PG_BIN_DIR/postgres" --version
  uname -a
  cat /proc/sys/kernel/random/boot_id
  findmnt -rn -o MAJ:MIN --target "$base"
) > "$report/toolchain.txt"
(cd "$stage/source" && cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt) > "$report/build.log" 2>&1
runtime="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libmesh_rt.a"
bash "$stage/packet/link.sh" "$compiler" "$runtime" "$stage/broker" "$revision" "$bundle" > "$report/link.log" 2>&1
MESH_RT_LIB_PATH="$runtime" "$compiler" build "$stage/packet/probe" --opt-level 2 --output "$stage/probe" > "$report/probe-build.log" 2>&1
cp "$stage/broker/mesh.json" "$report/broker.json"
readelf -h "$stage/broker/mesh" > "$report/broker-elf.txt"
mkdir -m 0700 "$report/proof-owner"
chown nobody:nogroup "$TMPDIR" "$report/proof-owner"
runuser -u nobody -- env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" LC_ALL=C.UTF-8 \
  "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$stage/source" "PRODUCT_FROZEN_HARNESS=$frozen" "PRODUCT_STORAGE=default" \
  taskset -c 0,1 python3 -B "$stage/packet/prove.py" "$report/proof-owner/proof" \
  "$stage/broker/mesh" "$stage/probe" "$stock/mesh" > "$report/proof.log" 2>&1
chmod -R a-w "$stage/broker" "$stage/packet"
touch "$report/SETUP-PASS"
