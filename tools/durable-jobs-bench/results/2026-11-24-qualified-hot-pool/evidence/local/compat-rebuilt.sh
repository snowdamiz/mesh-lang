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

for version in 18.7 19.0; do
  export PG_BIN_DIR="$lane/postgres/$version/bin"
  test "$("$PG_BIN_DIR/postgres" --version)" = "postgres (PostgreSQL) $version"
  gate "pg-$version-retry" bash scripts/verify-postgres-jobs.sh --compatibility "$version" --j6-roles
  gate "fence-$version" bash scripts/verify-postgres-jobs.sh --effect-fence
done
