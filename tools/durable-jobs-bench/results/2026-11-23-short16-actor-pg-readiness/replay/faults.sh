#!/usr/bin/env bash
set -uo pipefail
bundle="$(cd "$(dirname "$0")/.." && pwd -P)"
lane="${1:?absolute build directory}"
out="${2:?new absolute fault evidence directory}"
mkdir "$out" || exit 1
export PG_BIN_DIR="/opt/homebrew/opt/postgresql@17/bin" TMPDIR="$lane/tmp/" PYTHONDONTWRITEBYTECODE=1
export TRUSTED_ADMISSION_POLICY=success_cohort MESH_JOBS_SET_BASED_ENQUEUE=1
overall=0
fault() {
  local label="$1" code=0
  shift
  "$@" > "$out/$label.log" 2>&1 || code=$?
  printf '%s %s\n' "$label" "$code" | tee -a "$out/status.txt"
  if (( code != 0 )); then overall=1; fi
}
for mode in readiness off; do
  fault "queue-$mode" python3 -B "$bundle/replay/queue-faults.py" \
    "$lane" "$out/queue-$mode" "$lane/queue-fault-artifacts/mesh" "$mode"
done
fault actor python3 -B "$lane/faults.py" "$lane/actor-fault" "$out/actor"
exit "$overall"
