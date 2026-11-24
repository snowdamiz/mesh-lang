#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
root="$(cd "$here/../../../.." && pwd -P)"
scratch="$root/target/exit-confirmed-short-refill"
original="$root/target/ordinary-completion-writer/combined/workspace"
workspace="$scratch/workspace-gates"
out="$scratch/gates"
[[ ! -e "$workspace" && ! -e "$out" ]]
mkdir "$workspace" "$out"
# The preexisting expanded r2 tree supplies the workspace-only test assets.
python3 -B - "$original" "$workspace" <<'PY'
from pathlib import Path
import shutil
import sys
original, fresh = map(Path, sys.argv[1:])
for item in original.iterdir():
    if item.name in {".git", ".tmp", "target"}:
        continue
    if item.is_dir():
        shutil.copytree(item, fresh / item.name, symlinks=True)
    elif item.is_file():
        shutil.copy2(item, fresh / item.name)
PY
runner_path=compiler/mesh-rt/src/jobs/runner.rs
cmp "$scratch/runner.original" "$workspace/$runner_path"
cmp "$scratch/source/Cargo.lock" "$workspace/Cargo.lock"
cp "$scratch/source/$runner_path" "$workspace/$runner_path"
export PATH="${REFILL_RUST_BIN:-$HOME/.rustup/toolchains/1.99.0-aarch64-apple-darwin/bin}:$PATH"
export TMPDIR="$scratch/tmp/" PG_BIN_DIR="${PG_BIN_DIR:-/opt/homebrew/opt/postgresql@17/bin}"
export CARGO_HOME="$scratch/cargo-home" CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4
export LLVM_SYS_211_PREFIX="${LLVM_SYS_211_PREFIX:-/opt/homebrew/opt/llvm@21}"
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
unset CARGO_TARGET_DIR MESH_JOBS_TEST_PG_URL MESH_TEST_DATABASE_URL MESH_SHORT_REFILL
cd "$workspace"
run_case() {
  local label="$1"
  shift
  local code=0
  "$@" >"$out/$label.log" 2>&1 || code=$?
  printf '{"returncode": %d}\n' "$code" >"$out/$label.status.json"
  printf '%s %d\n' "$label" "$code"
}
run_case workspace-build cargo build --locked --workspace
run_case refill-wait cargo test --locked -p mesh-rt --lib refill_wait_is_bounded_and_only_aggregates_under_live_demand
run_case scheduler-drop cargo test --locked -p mesh-rt --lib owned_actor_reports_exit_after_its_stack_and_registration_are_cleaned_up
exec 9<"$root/target/queue-performance.lock"
flock 9
run_case postgres-jobs bash scripts/verify-postgres-jobs.sh
