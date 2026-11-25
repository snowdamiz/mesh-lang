#!/bin/bash
# Darwin functional rehearsal. Uses private work and the retained Darwin artifacts.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
root="$(git -C "$here" rev-parse --show-toplevel)"
lane="$root/target/slow-lz4-isolated-qualification"
old="$here/../2026-11-23-slow-storage-transfer"
stock="$(pg_config --bindir)"
isolated="$lane/local-pg/pgsql-17.11-lz4-537512904744/bin"
export PYTHONPATH="$old/replay" TMPDIR="$lane/tmp" ERL_CRASH_DUMP=/dev/null DURABLE_JOBS_PHASE=diagnostic
stock_oban="$root/target/native-large-payload-storage-peers/stock-oban/tools/durable-jobs-bench/native/arm.py"
river="$root/target/linode-river/submit-timing/source/tools/durable-jobs-bench/go/run.py"
apalis="$root/tools/durable-jobs-bench/rust/run.py"
mesh="$lane/mesh-harness/tools/durable-jobs-bench/native/arm.py"
run_name="${LOCAL_RUN_ID:-functional}"
[[ "$run_name" =~ ^functional(-[a-z0-9-]+)?$ ]]
output="$lane/$run_name"
mkdir "$output"
mkdir "$output/state" "$output/scratch"
if [[ $# == 0 ]]; then set -- mesh go oban rust; fi
run() (
    local build="$1" engine="$2" policy="$3" out="$output/$1-$2-$3"
    local pg="$isolated"
    [[ "$build" != stock ]] || pg="$stock"
    export PG_BIN_DIR="$pg" NATIVE_JOBS_STATE="$output/state" MESH_JOBS_SET_BASED_ENQUEUE=1
    case "$engine" in
        mesh) export NATIVE_JOBS_ARTIFACTS="$lane/mesh-artifacts"
              args=("$mesh" --local-smoke mesh) ;;
        oban) export NATIVE_JOBS_ARTIFACTS="$root/target/linode-jobs-native/local-build/oban-01"
              args=("$stock_oban" --local-smoke oban) ;;
        go) mkdir "$out"
            args=("$river" --binary "$lane/river-bench" --pg-bin "$pg" --scratch "$output/scratch") ;;
        rust) args=("$apalis" --binary "$lane/rust-target/debug/durable-jobs-apalis-bench" --pg-bin "$pg" --scratch "$output/scratch") ;;
    esac
    python3 -B "$here/overlay.py" "$engine" "$policy" "${args[@]}" slow "$out"
)
failed=0
for engine in "$@"; do
    for cell in stock-pglz isolated-pglz isolated-lz4; do
        if run "${cell%-*}" "$engine" "${cell##*-}" > "$output/$engine-$cell.log" 2>&1; then
            printf '%s %s PASS\n' "$engine" "$cell"
        else
            printf '%s %s FAIL\n' "$engine" "$cell"
            failed=1
        fi
    done
done
exit "$failed"
