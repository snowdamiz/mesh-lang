#!/usr/bin/env bash
# Invoked by the sole controller during the public candidate SETUP.
set -euo pipefail
[[ $# == 3 && "$(uname -sm)" == 'Linux x86_64' && "$(id -u)" == 0 ]]
bundle=$1 revision=$2 id=$3
[[ "$bundle" =~ ^[a-f0-9]{64}$ && "$revision" =~ ^[a-f0-9]{40}$ && "$id" =~ ^[a-f0-9]{12}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-public-large-exact-prepare-$id"
report="$stage/setup"
tools="$base/toolchain"
stock="$base/builds/trusted-jobs-product-623c46b92cc5"
compiler="$base/builds/mesh-stock-3828f20d/abi/meshc"
broker="$base/source-public-large-file-session-resume/broker"
broker_sha=a5a2d5667c2d7209234d7f1468c40dd4071818010be31364a4e82d04b7ff18a3
export PATH="$tools/rust-1.99.0/bin:$tools/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tools/llvm-21.1.8" PG_BIN_DIR="$tools/pgsql-17.11/bin"
export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 LC_ALL=C.UTF-8
export CARGO_TARGET_DIR="$stage/cargo" TMPDIR="$stage/tmp"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$stage/source/scripts/ci-x86_64-linux-gnu-clang"
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
test "$(cat "$stage/revision.txt")" = "$revision"
printf '%s  %s\n' \
  e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$compiler" \
  b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$PG_BIN_DIR/postgres" \
  af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355 "$stock/mesh" \
  979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55 "$stock/mesh.json" \
  "$broker_sha" "$broker" | sha256sum -c > "$report/pins.log"
test "$(cat /proc/sys/kernel/random/boot_id)" = 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a
test "$(uname -r)" = 6.8.0-149-generic
test "$(findmnt -rn -o MAJ:MIN --target "$base")" = 8:0
test "$(getconf CLK_TCK)" = 100
test "$(llvm-config --version)" = 21.1.8
mkdir "$CARGO_TARGET_DIR" "$TMPDIR"
p="$stage/source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition"
for file in main.mpl writes.mpl worker_identity.mpl worker_identity.c; do
    cmp "$p/fixture/$file" "$stock/$file"
done
jobs="$stage/source/compiler/mesh-rt/src/jobs"
test "$("$PG_BIN_DIR/pg_config" --version)" = 'PostgreSQL 17.11'
cc -O2 -fPIC -shared -I "$("$PG_BIN_DIR/pg_config" --includedir-server)" \
    "$jobs/pg17_lock_exact.c" -o "$stage/pg17_lock_exact.so" > "$report/pg17-build.log" 2>&1
(cd "$stage/source" && cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt) \
    > "$report/build.log" 2>&1
runtime="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libmesh_rt.a"
bash "$p/replay/link.sh" "$compiler" "$runtime" "$stage/artifact" "$revision" "$bundle" > "$report/link.log" 2>&1
cp "$stage/artifact/mesh.json" "$report/artifact.json"
readelf -h -n "$stage/artifact/mesh" > "$report/elf.txt"
readelf -h -Ws "$stage/pg17_lock_exact.so" > "$report/pg17-module.txt"
sha256sum "$stage"/adapter/* "$jobs"/prepare_claim_{install,schema,remove}.sql \
    "$stage/pg17_lock_exact.so" "$stage/artifact/mesh.json" > "$report/installed.sha256"
chmod -R a-w "$stage/artifact" "$stage/adapter"
chmod a-w "$stage/pg17_lock_exact.so" "$jobs"/prepare_claim_{install,schema,remove}.sql
elf="$(sha256sum "$stage/artifact/mesh" | cut -d ' ' -f1)"
manifest="$(sha256sum "$stage/artifact/mesh.json" | cut -d ' ' -f1)"
# Keep the outer lock during the isolated smoke. The adapter owns the inner one.
flock -u 9
exec 9<&-
mkdir "$report/smoke"
env PRODUCT_ARTIFACT="$stage/artifact" PRODUCT_ELF_SHA256="$elf" PRODUCT_MANIFEST_SHA256="$manifest" \
    PRODUCT_COMPOSITION=product-bulk PRODUCT_TOPOLOGY=stock PRODUCT_STORAGE=default \
    PRODUCT_CLAIM_MODE=prepared PRODUCT_SESSION_MODE=file PRODUCT_SESSION_EXECUTABLE="$broker" \
    PRODUCT_SESSION_SHA256="$broker_sha" DURABLE_JOBS_PHASE=diagnostic taskset -c 0,1 \
    "$stage/adapter/native-run" mesh large "$report/smoke" > "$report/smoke.log" 2>&1
touch "$report/SETUP-PASS"
