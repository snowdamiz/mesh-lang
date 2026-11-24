#!/usr/bin/env bash
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
here="$(cd "$(dirname "$0")" && pwd -P)"
[[ "$#" == 9 && "$7" == mesh && "$8" == slow && "$9" == /var/tmp/mesh-durable-jobs-matrix/slow-cap71-confirmed-refill-v1-*/raw/*/adapter && -d "$9" && ! -L "$9" ]]
image="$1" attempt="$2" fused="$3" mesh_sha="$4" manifest_sha="$5" producer_sha="$6"
[[ "$mesh_sha" =~ ^[0-9a-f]{64}$ && "$manifest_sha" =~ ^[0-9a-f]{64}$ && "$producer_sha" =~ ^[0-9a-f]{64}$ ]]
public=trusted-jobs-product-623c46b92cc5
case "$image/$fused" in
    public/0) [[ "$attempt" == "$public" ]] ;;
    candidate/0|candidate/1) [[ "$attempt" == short16-fused-refill-c4c5cc2243b6 ]] ;;
    *) exit 2 ;;
esac
exec 9<"$base/native.lock"
flock -n 9
args=(env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8
    "PG_BIN_DIR=$base/toolchain/pgsql-17.11/bin" "NATIVE_JOBS_STATE=$base"
    "PRODUCT_SOURCE=$base/source-$attempt/source" "NATIVE_JOBS_ARTIFACTS=$base/builds/$attempt"
    "PRODUCT_ELF_SHA256=$mesh_sha" "PRODUCT_MANIFEST_SHA256=$manifest_sha"
    "SLOW_PRODUCER_PATH=$base/builds/$public/mesh" "SLOW_PRODUCER_SHA=$producer_sha"
    "PRODUCT_FROZEN_HARNESS=$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
    "SLOW_IMAGE=$image" "SLOW_FUSED=$fused" "DURABLE_JOBS_PHASE=${DURABLE_JOBS_PHASE:-diagnostic}")
if [[ "$(id -u)" == 0 ]]; then
    [[ -d "$9" && -z "$(find "$9" -mindepth 1 -maxdepth 1 -print -quit)" ]]
    chown nobody:nogroup "$9"
    chmod 0700 "$9"
    exec runuser -u nobody -- "${args[@]}" python3 -B "$here/runtime.py" "${@:7}"
fi
exec "${args[@]}" python3 -B "$here/runtime.py" "${@:7}"
