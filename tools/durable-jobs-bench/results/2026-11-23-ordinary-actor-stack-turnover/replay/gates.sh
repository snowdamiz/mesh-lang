#!/usr/bin/env bash
set -euo pipefail
base="$1"
mkdir "$base/gates"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export RUSTUP_TOOLCHAIN=1.99.0 CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=5
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21
export MESH_VERIFY_Z3="$base/bin/z3" CARGO_TARGET_DIR="$base/cargo" TMPDIR="$base/tmp/"
export PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin PYTHONDONTWRITEBYTECODE=1
unset MESH_TEST_DATABASE_URL MESH_JOBS_TEST_PG_URL
ulimit -c 0
cd "$base/ordinary"

gate() {
    local name="$1"
    shift
    local status=0
    printf 'START %s\n' "$name"
    "$@" > "$base/gates/$name.log" 2>&1 || status=$?
    printf '%s\t%s\n' "$name" "$status" | tee -a "$base/gates/exits.tsv"
}

gate stack-runtime cargo test --locked --offline -p mesh-rt --lib actor::stack::tests -- --test-threads=1
gate exit-order cargo test --locked --offline -p mesh-rt --lib owned_actor_reports_exit_after_its_stack_and_registration_are_cleaned_up
gate workspace-build cargo build --locked --offline --workspace
gate workspace-tests cargo test --locked --offline --workspace --no-fail-fast -- --test-threads=4
gate laws cargo test --locked --offline -p meshc --bin meshc verify_solver::tests::pinned_z3 -- --ignored
gate postgres-jobs bash scripts/verify-postgres-jobs.sh
