#!/usr/bin/env bash
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd -P)
native=/var/lib/durable-jobs-bench/native
profile=${1:?missing profile}
out=${2:?missing evidence directory}
[[ "$#" == 2 && "$out" == /* && -d "$out" && ! -L "$out" ]]
[[ -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]]
exec 9<"$native/native.lock"
flock -n 9
chown nobody:nogroup "$out"
chmod 0700 "$out"
cd "$here/source"
exec runuser -u nobody -- env -i PATH=/usr/bin:/bin LC_ALL=C.UTF-8 \
    "PG_BIN_DIR=$native/toolchain/pgsql-17.11/bin" \
    "DURABLE_JOBS_PHASE=${DURABLE_JOBS_PHASE:-diagnostic}" \
    "NATIVE_JOBS_STATE=$here" "NATIVE_JOBS_ARTIFACTS=$here/artifact" \
    python3 -B tools/durable-jobs-bench/native/arm.py mesh "$profile" "$out"
