#!/usr/bin/env bash
# Native correctness for the integrated set-based enqueue product. No timing.
# Usage on the assigned SETUP, over pinned SSH as root:
#   bash -c "$(cat native-correctness.sh)" -- ARCHIVE_SHA256 ATTEMPT REVISION < source.tar.gz
set -euo pipefail
archive_hash="${1:?archive SHA256}"
key="${2:?unique attempt}"
revision="${3:?exact source revision}"
[[ "$archive_hash" =~ ^[a-f0-9]{64}$ && "$revision" =~ ^[a-f0-9]{40}$ \
   && "$key" =~ ^set-based-product-[a-f0-9]{12}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-$key"
out="$base/runs/$key"
tool="$base/toolchain"
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:$tool/pgsql-17.11/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" PG_BIN_DIR="$tool/pgsql-17.11/bin"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 LC_ALL=C.UTF-8
umask 022
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$stage" "$out"
mkdir "$stage/src" "$stage/tmp" "$stage/home" "$stage/cargo-home"
cat > "$stage/incoming.tar.gz"
printf '%s  %s\n' "$archive_hash" "$stage/incoming.tar.gz" | sha256sum -c
tar -xzf "$stage/incoming.tar.gz" -C "$stage/src"
rm "$stage/incoming.tar.gz"
export TMPDIR="$stage/tmp" CARGO_TARGET_DIR="$stage/target"
trap 'rm -rf -- "$stage/target" "$stage/tmp"' EXIT
{
    uname -smr
    cat /proc/sys/kernel/random/boot_id
    getconf CLK_TCK
    rustc --version
    llvm-config --version
    postgres --version
    printf 'revision=%s archive=%s\n' "$revision" "$archive_hash"
} >"$out/identity.txt"

# Root prepares every build with the shared offline crate cache, then the
# disposable PostgreSQL proofs run unprivileged against read-only sources.
cd "$stage/src"
CARGO_HOME="$base/cargo-home" cargo fetch --locked --offline --target x86_64-unknown-linux-gnu \
    >"$out/fetch.log" 2>&1
CARGO_HOME="$base/cargo-home" cargo build --locked --release -p mesh-rt -p meshc >"$out/release-build.log" 2>&1
sha256sum "$CARGO_TARGET_DIR/release/libmesh_rt.a" "$CARGO_TARGET_DIR/release/meshc" >"$out/release-identity.sha256"
CARGO_HOME="$base/cargo-home" bash -c '
  cargo build --locked -p meshc -p mesh-rt
  cargo test --locked -p mesh-rt --lib --no-run
  cargo test --locked -p mesh-rt --test postgres_compatibility --no-run
  cargo test --locked -p meshc --test e2e_durable_jobs --test e2e_pg_conn_ownership \
    --test e2e_db_values --test e2e_migrate --no-run' >"$out/prebuild.log" 2>&1
ln -s "$base/cargo-home/registry" "$stage/cargo-home/registry"
if [[ -d "$base/cargo-home/git" ]]; then ln -s "$base/cargo-home/git" "$stage/cargo-home/git"; fi
chown -R nobody:nogroup "$stage/src" "$stage/target" "$stage/tmp" "$stage/home" "$stage/cargo-home"
unprivileged() {
    runuser -u nobody -- env -i "PATH=$PATH" "HOME=$stage/home" "TMPDIR=$stage/tmp" \
        "CARGO_HOME=$stage/cargo-home" "CARGO_TARGET_DIR=$CARGO_TARGET_DIR" \
        "CARGO_NET_OFFLINE=true" "CARGO_BUILD_JOBS=2" "CARGO_INCREMENTAL=0" \
        "LLVM_SYS_211_PREFIX=$LLVM_SYS_211_PREFIX" "PG_BIN_DIR=$PG_BIN_DIR" "LC_ALL=C.UTF-8" "$@"
}
status=0
gate() {
    local label=$1 code=0
    shift
    (cd "$stage/src" && unprivileged "$@") >"$out/$label.log" 2>&1 || code=$?
    printf '%s %s\n' "$label" "$code" >>"$out/gates.txt"
    if (( code != 0 )); then status=1; fi
}
gate pg-17.11 bash scripts/verify-postgres-jobs.sh --compatibility 17.11 --parity-driver-check
gate effect-compiled bash scripts/verify-postgres-jobs.sh --effect-compiled
grep -E 'set-based enqueue wire .* passed' "$out/pg-17.11.log" >"$out/wire-cases.txt" || true
test "$(wc -l <"$out/wire-cases.txt")" -eq 7 || status=1
exit "$status"
