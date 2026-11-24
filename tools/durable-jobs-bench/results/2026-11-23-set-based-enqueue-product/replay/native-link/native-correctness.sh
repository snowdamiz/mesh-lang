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
pg="$tool/pgsql-17.11"
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:$pg/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 LC_ALL=C.UTF-8
umask 022
ulimit -c 0
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
trap 'rm -rf -- "$stage/target" "$stage/tmp" "$stage/pgsql-compat"' EXIT
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$stage/proof-linker"
cat > "$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER" <<'LINKER'
#!/bin/sh
set -eu
llvm="${LLVM_SYS_211_PREFIX:?}"
exec "$llvm/bin/clang" -fuse-ld=lld --ld-path="$llvm/bin/ld.lld" "$@" \
    -Wl,--lto-O0 -Wl,--strip-debug -Wl,--thinlto-cache-dir="${TMPDIR:?}/thinlto"
LINKER
chmod 0755 "$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"
{
    uname -smr
    cat /proc/sys/kernel/random/boot_id
    getconf CLK_TCK
    rustc --version
    llvm-config --version
    "$LLVM_SYS_211_PREFIX/bin/ld.lld" --version
    "$pg/bin/postgres" --version
    sha256sum "$LLVM_SYS_211_PREFIX/bin/clang" "$LLVM_SYS_211_PREFIX/bin/ld.lld" \
        "$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"
    printf 'revision=%s archive=%s\n' "$revision" "$archive_hash"
} >"$out/identity.txt"

# Only the proof copy supplies extensions beyond the benchmark installation.
pg_archive="$base/cache/postgresql-17.11.tar.bz2"
pg_compat="$stage/pgsql-compat"
export PG_BIN_DIR="$pg_compat/bin"
{
    test "$(uname -sm)" = 'Linux x86_64'
    test "$(llvm-config --version)" = 21.1.8
    printf '%s  %s\n' b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 \
        "$pg/bin/postgres" | sha256sum -c
    printf '%s  %s\n' dd27f2b3c59e73ed14aa3324901242bf69a032a6347805f274e6260322d42979 \
        "$pg_archive" | sha256sum -c
    cp -a --reflink=auto "$pg" "$pg_compat"
    test "$("$PG_BIN_DIR/pg_config" --sharedir)" = "$pg_compat/share"
    test "$("$PG_BIN_DIR/pg_config" --pkglibdir)" = "$pg_compat/lib"
    mkdir "$TMPDIR/pg-extensions"
    tar -xf "$pg_archive" -C "$TMPDIR/pg-extensions" --strip-components=2 \
        postgresql-17.11/contrib/pg_trgm postgresql-17.11/contrib/pgcrypto
    for extension in pg_trgm pgcrypto; do
        make -C "$TMPDIR/pg-extensions/$extension" -j2 USE_PGXS=1 PG_CONFIG="$PG_BIN_DIR/pg_config"
        make -C "$TMPDIR/pg-extensions/$extension" USE_PGXS=1 PG_CONFIG="$PG_BIN_DIR/pg_config" install
    done
    sha256sum "$PG_BIN_DIR/postgres" "$pg_compat/lib/pg_trgm.so" "$pg_compat/lib/pgcrypto.so"
} >"$out/pg-extension-setup.log" 2>&1

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
sha256sum "$CARGO_TARGET_DIR/debug/libmesh_rt.a" "$CARGO_TARGET_DIR/debug/meshc" >"$out/proof-identity.sha256"
ln -s "$base/cargo-home/registry" "$stage/cargo-home/registry"
if [[ -d "$base/cargo-home/git" ]]; then ln -s "$base/cargo-home/git" "$stage/cargo-home/git"; fi
chown -R nobody:nogroup "$stage/src" "$stage/target" "$stage/tmp" "$stage/home" "$stage/cargo-home"
unprivileged() {
    runuser -u nobody -- env -i "PATH=$PATH" "HOME=$stage/home" "TMPDIR=$stage/tmp" \
        "CARGO_HOME=$stage/cargo-home" "CARGO_TARGET_DIR=$CARGO_TARGET_DIR" \
        "CARGO_NET_OFFLINE=true" "CARGO_BUILD_JOBS=2" "CARGO_INCREMENTAL=0" \
        "LLVM_SYS_211_PREFIX=$LLVM_SYS_211_PREFIX" "PG_BIN_DIR=$PG_BIN_DIR" "LC_ALL=C.UTF-8" \
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER" \
        "MESH_RT_LIB_PATH=$CARGO_TARGET_DIR/debug/libmesh_rt.a" "$@"
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
