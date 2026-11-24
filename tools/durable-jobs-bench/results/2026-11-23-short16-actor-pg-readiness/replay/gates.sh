#!/usr/bin/env bash
set -uo pipefail
lane="${1:?absolute experiment directory}"
evidence="${2:?absolute gate output directory}"
mkdir "$evidence" || exit 1
cd "$lane/ordinary-v2" || exit 1
export PATH="$lane/inputs/proof-bin:/opt/homebrew/opt/rustup/bin:/opt/homebrew/opt/llvm@21/bin:$PATH"
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21 CARGO_TARGET_DIR="$lane/cargo"
export TMPDIR="$lane/tmp/" PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=2 PYTHONDONTWRITEBYTECODE=1
unset MESH_TEST_DATABASE_URL MESH_JOBS_TEST_PG_URL PGHOST PGHOSTADDR PGSERVICE PGOPTIONS
overall=0
gate() {
  local name="$1" code=0
  shift
  "$@" > "$evidence/$name.log" 2>&1 || code=$?
  printf '%s %s\n' "$name" "$code" | tee -a "$evidence/status.txt"
  if (( code != 0 )); then overall=1; fi
}
if [[ "${3:-}" == "--postgres-only" ]]; then
  gate postgres bash scripts/verify-postgres-jobs.sh --compatibility 17.11 --parity-driver-check
  exit "$overall"
fi
gate workspace cargo build --workspace --locked
gate postgres bash scripts/verify-postgres-jobs.sh --compatibility 17.11 --parity-driver-check
gate pool cargo test --locked -p mesh-rt --lib db::pool::tests
gate scheduler cargo test --locked -p mesh-rt --lib actor::scheduler::tests
exit "$overall"
