#!/bin/bash
# Match the prior locally pinned peer releases, with isolated PG scratch.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
packet="$root/tools/durable-jobs-bench/results/2026-11-23-slow-storage-transfer"
lane="$root/target/ordinary-slow-storage-transfer"
previous="$root/target/native-large-payload-storage-peers"
pg="${PG_BIN_DIR:-/opt/homebrew/opt/postgresql@17/bin}"
out="$packet/evidence/local-peers"
mkdir -p "$lane/peer-pg" "$lane/peer-state" "$out"
overlay="$packet/replay/native_overlay.py"
oban="$root/target/linode-jobs-native/local-build/oban-01"
stock_oban="$previous/stock-oban/tools/durable-jobs-bench/native/arm.py"
river="$root/target/linode-river/submit-timing"
apalis="$root/target/linode-apalis/build/release/durable-jobs-apalis-bench"
expected_mesh="$lane/source/tools/durable-jobs-bench/native/arm.py"
export TMPDIR="$lane/tmp" PG_BIN_DIR="$pg" ERL_CRASH_DUMP=/dev/null
shasum -a 256 "$oban/oban" "$stock_oban" "$river/build/river-bench" "$river/source/tools/durable-jobs-bench/go/run.py" \
    "$apalis" "$root/tools/durable-jobs-bench/rust/run.py" "$expected_mesh" > "$out/pins.sha256"
for item in go-lz4 rust-pglz oban-lz4 mesh-pglz go-pglz rust-lz4 oban-pglz mesh-lz4; do
    test -f "$out/$item.PASS" && continue
    engine="${item%-*}"
    policy="${item##*-}"
    trial="$out/$item"
    case "$engine" in
        go) mkdir "$trial"
            args=("$river/source/tools/durable-jobs-bench/go/run.py" --binary "$river/build/river-bench" --pg-bin "$pg" --scratch "$lane/peer-pg") ;;
        rust) args=("$root/tools/durable-jobs-bench/rust/run.py" --binary "$apalis" --pg-bin "$pg" --scratch "$lane/peer-pg") ;;
        oban) export NATIVE_JOBS_STATE="$lane/peer-state" NATIVE_JOBS_ARTIFACTS="$oban"
              args=("$stock_oban" --local-smoke oban) ;;
        mesh) export NATIVE_JOBS_STATE="$lane/peer-state" NATIVE_JOBS_ARTIFACTS="$lane/artifacts-v1" MESH_JOBS_SET_BASED_ENQUEUE=1
              args=("$expected_mesh" --local-smoke mesh) ;;
    esac
    if python3 -B "$overlay" "$engine" "$policy" "${args[@]}" slow "$trial" > "$out/$item.log" 2>&1; then
        touch "$out/$item.PASS"
        printf '%s\n' "$item passed"
    else
        printf '%s\n' "$item failed, see $out/$item.log" >&2
    fi
done
