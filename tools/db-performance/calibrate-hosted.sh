#!/usr/bin/env bash
set -euo pipefail

stage=$1
evidence="$RUNNER_TEMP/db-calibration"
binary="$RUNNER_TEMP/mesh-db-baseline"
export MESH_TEST_DATABASE_URL="postgresql://postgres@127.0.0.1:$POSTGRES_PORT/postgres"
mkdir -p "$evidence/$stage"

run() {
  local output="$1"
  local mode="$2"
  local blocks="$3"
  local seed="$4"
  shift 4
  printf 'Running %s/%s\n' "$stage" "$output"
  uptime
  timeout 35m python3 tools/db-performance/bench.py \
    --baseline "$binary" --candidate "$binary" --workers 1 \
    --blocks "$blocks" --warmup 4 --seed "$seed" --mode "$mode" \
    --out "$evidence/$stage/$output" "$@" | tee "$evidence/$stage/$output.log"
}

case "$stage" in
  pilot)
    python3 tools/db-performance/bench.py --self-test
    run sqlite_rows sqlite_rows 10 89 --check-noise
    ;;
  full)
    failed=0
    run pg_point pg_point 10 97 --check-noise --iterations 600 || failed=1
    run pg_typed pg_typed 10 97 --check-noise --iterations 600 || failed=1
    run pg_write_single pg_write 10 97 --check-noise --actors 1 --iterations 10000 || failed=1
    run pg_write_multi pg_write 10 97 --check-noise --actors 16 64 --iterations 160 || failed=1
    run sqlite_point sqlite_point 10 97 --check-noise --iterations 6000 || failed=1
    run sqlite_write_single sqlite_write 10 97 --check-noise --actors 1 --iterations 10000 || failed=1
    run sqlite_write_multi sqlite_write 10 97 --check-noise --actors 16 64 --iterations 128 || failed=1
    run pg_rows pg_rows 10 97 --check-noise || failed=1
    run pg_sleep pg_sleep 10 97 --check-noise || failed=1
    run sqlite_rows sqlite_rows 20 97 --check-noise || failed=1
    exit "$failed"
    ;;
  ticker)
    for attempt in {1..60}; do
      uptime | tee -a "$evidence/ticker/host-load.txt"
      if awk '{exit !($1 <= 0.5)}' /proc/loadavg; then
        break
      fi
      if [[ $attempt == 60 ]]; then
        echo "Runner load did not settle before the ticker check" >&2
        exit 1
      fi
      sleep 10
    done
    run idle idle 5 89 --actors 16 --iterations 6400
    run paced pg_sleep 5 89 --actors 16 --iterations 4 --pg-sleep-spacing-ms 100
    python3 - "$evidence/ticker" <<'PY'
import json
from pathlib import Path
from statistics import median
import sys

root = Path(sys.argv[1])
idle, paced = (
    json.loads((root / mode / "run.json").read_text())["trials"]
    for mode in ("idle", "paced")
)
for trials in (idle, paced):
    assert len(trials) == 10 and all("error" not in t for t in trials)
assert all(t["operations"] == t["checksum"] == 64 for t in paced)
idle_p99 = median(t["ticker_p99_ns"] for t in idle)
paced_p99 = median(t["ticker_p99_ns"] for t in paced)
min_wakes = min(t["ticker_samples"] for t in paced)
print(f"Idle ticker p99 {idle_p99 / 1e6:.3f} ms, loaded p99 {paced_p99 / 1e6:.3f} ms, "
      f"at least {min_wakes} actual loaded wakes per trial")
assert idle_p99 < 5_000_000
assert paced_p99 > idle_p99
assert min_wakes >= 1000
PY
    ;;
  *)
    echo "Unknown calibration stage: $stage" >&2
    exit 2
    ;;
esac
