#!/usr/bin/env bash
# The sole Linode controller invokes this under its SETUP assignment.
set -euo pipefail
[[ $# == 3 && "$(uname -sm)" == 'Linux x86_64' && "$(id -u)" == 0 ]]
attempt="$1"
revision="$2"
source_sha="$3"
[[ "$attempt" =~ ^short16-fused-refill-[a-f0-9]{12}$ && "$revision" =~ ^[a-f0-9]{40}$ && "$source_sha" =~ ^[a-f0-9]{64}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-$attempt"
out="$base/runs/$attempt"
artifact="$base/builds/$attempt"
tools="$base/toolchain"
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
mkdir "$stage" "$out"
export CARGO_TARGET_DIR="$stage/cargo" TMPDIR="$stage/tmp"
mkdir "$CARGO_TARGET_DIR" "$TMPDIR"
trap 'rm -rf -- "$CARGO_TARGET_DIR" "$TMPDIR"' EXIT
cat > "$stage/source.tar.gz"
printf '%s  %s\n' "$source_sha" "$stage/source.tar.gz" | sha256sum -c
tar -xzf "$stage/source.tar.gz" -C "$stage"
printf '%s  %s\n' \
    e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$compiler" \
    b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$PG_BIN_DIR/postgres" \
    | sha256sum -c
test "$(cat /proc/sys/kernel/random/boot_id)" = 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a
test "$(uname -r)" = 6.8.0-149-generic
test "$(findmnt -n -o MAJ:MIN --target "$base" | xargs)" = 8:0
test "$(getconf CLK_TCK)" = 100
test "$(llvm-config --version)" = 21.1.8
source="$stage/source"
packet="$source/tools/durable-jobs-bench/results/2026-11-24-short16-fused-refill"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$source/scripts/ci-x86_64-linux-gnu-clang"
(
    rustc -vV
    cargo --version
    "$LLVM_SYS_211_PREFIX/bin/llvm-config" --version
    "$PG_BIN_DIR/postgres" --version
    sha256sum "$source/Cargo.lock" "$compiler" "$(command -v rustc)" "$(command -v cargo)" \
        "$(command -v cc)" "$LLVM_SYS_211_PREFIX/bin/clang" "$LLVM_SYS_211_PREFIX/bin/ld.lld"
    uname -a
    cat /proc/sys/kernel/random/boot_id
    findmnt -no MAJ:MIN --target "$base"
) > "$out/toolchain.txt"
(cd "$source" && cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt) > "$out/build.log" 2>&1
release="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release"
bash "$packet/replay/link.sh" "$compiler" "$release/libmesh_rt.a" "$artifact" "$revision" "$source_sha" > "$out/link.log" 2>&1
python3 -B "$packet/replay/witness_fixture.py" "$packet" "$TMPDIR/witness-source"
bash "$TMPDIR/witness-source/replay/link.sh" "$compiler" "$release/libmesh_rt.a" "$stage/witness" "$revision" "$source_sha" > "$out/witness-link.log" 2>&1
cp "$stage/witness/mesh.json" "$out/witness.json"
chmod -R a-w "$stage/witness"
readelf -h -n "$artifact/mesh" > "$out/elf.txt"
ldd "$artifact/mesh" > "$out/elf-dependencies.txt"
cp "$artifact/mesh.json" "$out/mesh.json"
chmod -R a-w "$artifact"
chown nobody:nogroup "$TMPDIR"
# Use a writable child of the root-owned evidence directory.
mkdir -m 0700 "$out/semantic-root"
chown nobody:nogroup "$out/semantic-root"
runuser -u nobody -- env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" LC_ALL=C.UTF-8 \
    "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$source" "PRODUCT_FROZEN_HARNESS=$frozen" \
    taskset -c 0,1 python3 -B "$packet/replay/semantic.py" "$out/semantic-root/semantic" "$artifact/mesh" "$stage/witness/mesh" \
    > "$out/semantic.log" 2>&1
runuser -u nobody -- env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" LC_ALL=C.UTF-8 \
    "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$source" "PRODUCT_FROZEN_HARNESS=$frozen" \
    taskset -c 0,1 python3 -B "$packet/replay/refill_proof.py" "$out/semantic-root/refill" "$artifact/mesh" "$stage/witness/mesh" \
    > "$out/refill-proof.log" 2>&1
sha256sum "$artifact/mesh" "$artifact/mesh.json"
