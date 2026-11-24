#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
base=/var/lib/durable-jobs-bench/native
runner="$base/builds/exit-bulk-d2-c7c50e726ca0"
broker="$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
source="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
[[ "$#" == 4 && "$1" =~ ^[24]$ && "$2" == mesh && "$3" == noop ]]
[[ "${DURABLE_JOBS_PHASE:-}" =~ ^(warmup|measured)$ ]]
out="$4"
[[ "$(id -u)" == 0 && "$out" == "$here"/raw/*/adapter && -d "$out" && ! -L "$out" ]]
[[ -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]]
exec 9<"$base/native.lock"
flock -n 9
chmod 0644 "$here/stage-arm.py" "$here/native-arm.py" "$here/identity.py"
chown nobody:nogroup "$out"
chmod 0700 "$out"
cd "$source/tools/durable-jobs-bench/native"
exec /usr/sbin/runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    "PG_BIN_DIR=$base/toolchain/pgsql-17.11/bin" "NATIVE_JOBS_STATE=$base" \
    "NATIVE_JOBS_ARTIFACTS=$broker" "DURABLE_JOBS_PHASE=$DURABLE_JOBS_PHASE" \
    TRUSTED_ADMISSION_POLICY=success_cohort MESH_JOBS_SET_BASED_ENQUEUE=1 \
    DENSE_D2_ELF=972514efac688fd029821152c3bb70a7e62bf1c6227ab8b2d061dc69c47a2dc7 \
    DENSE_D2_MANIFEST=ab09bc77affff4496350202150dc6eb0b90d5c252f0a7552767e0070d615fe31 \
    /usr/bin/python3 -B "$here/stage-arm.py" "$1" "$source" "$runner" "$out"
