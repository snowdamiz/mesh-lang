#!/usr/bin/env bash
set -uo pipefail
lane="$(cd "$(dirname "$0")" && pwd -P)"
cd "$lane/source" || exit 1
export PATH="/opt/homebrew/opt/rustup/bin:/opt/homebrew/opt/llvm@21/bin:/opt/homebrew/opt/openssl@3/bin:$PATH"
export LLVM_SYS_211_PREFIX="/opt/homebrew/opt/llvm@21"
export CARGO_TARGET_DIR="$lane/gates-target" TMPDIR="$lane/tmp" CARGO_INCREMENTAL=0
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=3 RUST_TEST_THREADS=4 PYTHONDONTWRITEBYTECODE=1
export MESH_JOBS_SET_BASED_ENQUEUE=1
unset MESH_TEST_DATABASE_URL MESH_JOBS_TEST_PG_URL PGHOST PGHOSTADDR PGSERVICE PGOPTIONS
logs="$lane/evidence/${GATES_RUN:-gates-01}"
mkdir "$logs" || exit 2
uname -sm > "$logs/host.txt"
rustc --version >> "$logs/host.txt"
git status --short > "$logs/source.status"
git diff > "$logs/source.diff"
shasum -a 256 compiler/mesh-rt/src/jobs/{store.rs,store_tests.rs,schema.sql,enqueue_schema.sql} > "$logs/source.sha256"
overall=0
gate() {
    local label="$1" code=0
    shift
    "$@" >"$logs/$label.log" 2>&1 || code=$?
    printf '%s\n' "$code" >"$logs/$label.exit"
    printf '%s %s\n' "$label" "$code"
    if (( code != 0 )); then overall=1; fi
}
gate rustfmt cargo fmt --all -- --check
gate workspace-build cargo build --workspace --locked
export PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin
gate postgres-17.11 bash scripts/verify-postgres-jobs.sh --compatibility 17.11 --parity-driver-check --j6-roles
gate workspace-tests cargo test --workspace --locked
gate diagnostics-minimal cargo check --locked -p mesh-diagnostics --no-default-features
gate effect-fence bash scripts/verify-postgres-jobs.sh --effect-fence
gate effect-transaction bash scripts/verify-postgres-jobs.sh --effect-transaction
exit "$overall"
