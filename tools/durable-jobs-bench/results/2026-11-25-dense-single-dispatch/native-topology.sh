#!/usr/bin/env bash
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
here="$(cd "$(dirname "$0")" && pwd -P)"
proof="$base/source-trusted-1e70f18d-jobs-78468180f0c0"
stock="$base/dense-individual-logger-off-1e70f18d"
[[ $# == 5 || ( $# == 6 && "$6" == --stage-check ) ]]
[[ "$1:$2" =~ ^(2:2|1:2|1:4)$ && "$3" == mesh && "$4" == noop ]]
[[ "${DURABLE_JOBS_PHASE:?}" =~ ^(warmup|measured)$ ]]
[[ "${PRODUCT_ELF_SHA256:?}" =~ ^[a-f0-9]{64}$ && "${PRODUCT_MANIFEST_SHA256:?}" =~ ^[a-f0-9]{64}$ ]]
out="$5"
[[ "$(id -u)" == 0 && "$here" == /var/tmp/mesh-durable-jobs-matrix/* && "$out" == "$here"/raw/*/adapter && -d "$out" && ! -L "$out" ]]
[[ -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]]
exec 9<"$base/native.lock"
flock -n 9
chmod 0644 "$here/topology.py"
chown nobody:nogroup "$out"
chmod 0700 "$out"
cd "$out"
exec /usr/sbin/runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    "PG_BIN_DIR=$base/toolchain/pgsql-17.11/bin" "NATIVE_JOBS_STATE=$stock" \
    "PRODUCT_SOURCE=$proof/source" "PRODUCT_FROZEN_HARNESS=$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2" \
    "NATIVE_JOBS_ARTIFACTS=$stock/artifacts/jobs" "DURABLE_JOBS_PHASE=$DURABLE_JOBS_PHASE" HOT_CHECKOUT=1 \
    "PRODUCT_ELF_SHA256=$PRODUCT_ELF_SHA256" "PRODUCT_MANIFEST_SHA256=$PRODUCT_MANIFEST_SHA256" \
    /usr/bin/python3 -B "$here/topology.py" "$@"
