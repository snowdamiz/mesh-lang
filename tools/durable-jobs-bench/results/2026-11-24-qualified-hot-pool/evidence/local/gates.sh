#!/bin/bash
set -euo pipefail
lane="$(cd "$(dirname "$0")" && pwd)"
cd "$lane/source"
export PATH="$HOME/.rustup/toolchains/1.99.0-aarch64-apple-darwin/bin:/opt/homebrew/opt/llvm@21/bin:$lane/../jobs-lean-native-v3/variants/timerfd/retry/tmp/toolchain/zig-aarch64-macos-0.15.2:$PATH"
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21 PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin
export TMPDIR="$lane/tmp" CARGO_TARGET_DIR="$lane/build" CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=5
export PYTHONDONTWRITEBYTECODE=1
unset MESH_TEST_DATABASE_URL MESH_JOBS_TEST_PG_URL
mkdir -p "$lane/logs" "$TMPDIR"
gate() {
  name="$1"
  shift
  code=0
  "$@" >"$lane/logs/$name.log" 2>&1 || code=$?
  echo "$name $code" >> "$lane/logs/status"
  return "$code"
}
gate format cargo fmt --check
gate pool cargo test --offline --locked -p mesh-rt --lib db::pool::tests
gate pg-wire cargo test --offline --locked -p mesh-rt --lib db::pg::tests
gate pg17 bash scripts/verify-postgres-jobs.sh --compatibility 17.11 --parity-driver-check
gate postgres-fence bash scripts/verify-postgres-jobs.sh --effect-fence
gate workspace-build cargo build --locked --offline --workspace
gate workspace-correctness cargo test --locked --offline --workspace -- --skip the_performance_gate_measures_and_reports
