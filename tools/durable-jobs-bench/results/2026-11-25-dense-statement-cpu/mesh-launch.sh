#!/usr/bin/env bash
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
proof="$base/source-trusted-1e70f18d-jobs-78468180f0c0"
here="$base/dense-individual-logger-off-1e70f18d"
mode="$1"
shift
[[ "$mode" == on || "$mode" == off ]]
[[ "$here" == "$base/dense-individual-logger-off-1e70f18d" ]]
[[ $# == 3 && "$1" == mesh && "$2" == noop && "$3" == /* && ! -L "$3" ]]
[[ "${HOT_CHECKOUT:?}" =~ ^[01]$ ]]
[[ "${PRODUCT_ELF_SHA256:?}" =~ ^[a-f0-9]{64}$ && "${PRODUCT_MANIFEST_SHA256:?}" =~ ^[a-f0-9]{64}$ ]]
phase="${DURABLE_JOBS_PHASE:?}"
if [[ "$phase" == diagnostic ]]; then
    # Setup owns both host locks and only qualifies its own fresh output.
    [[ "$3" == "$base/runs/dense-individual-logger-off-1e70f18d/"* ]]
else
    [[ "$phase" == measured || "$phase" == warmup ]]
    exec 9<"$base/native.lock"
    flock -n 9
fi
[[ "$(id -u)" == 0 && -d "$3" && -z "$(find "$3" -mindepth 1 -maxdepth 1 -print -quit)" ]]
chown nobody:nogroup "$3"
chmod 0700 "$3"
exec runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8     "PG_BIN_DIR=$base/toolchain/pgsql-17.11/bin" "NATIVE_JOBS_STATE=$here"     "PRODUCT_SOURCE=$proof/source" "PRODUCT_FROZEN_HARNESS=$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"     "NATIVE_JOBS_ARTIFACTS=$here/artifacts/jobs" "DURABLE_JOBS_PHASE=$phase"     "HOT_CHECKOUT=$HOT_CHECKOUT" "PRODUCT_ELF_SHA256=$PRODUCT_ELF_SHA256"     "PRODUCT_MANIFEST_SHA256=$PRODUCT_MANIFEST_SHA256"     python3 -B "$base/dense-statement-cpu-20261125/cpu_adapter.py" mesh "$mode" "$here/adapter.py" "$@"
