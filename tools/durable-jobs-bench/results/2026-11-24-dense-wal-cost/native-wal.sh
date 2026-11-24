#!/usr/bin/env bash
set -euo pipefail
stage="$(cd "$(dirname "$0")" && pwd -P)"
base=/var/lib/durable-jobs-bench/native
pg="$base/toolchain/pgsql-17.11/bin"
out="${!#}"
engine="$1"
shift
[[ "${DURABLE_JOBS_PHASE:-}" =~ ^(warmup|measured)$ && "$out" == "$stage"/raw/*/adapter ]]
[[ -d "$out" && ! -L "$out" && -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]]
case "$engine" in
    go|rust)
        # worker.py has already acquired both host locks and dropped privileges.
        [[ "$(id -u)" == 65534 ]]
        exec /usr/bin/python3 -B "$stage/wal_capture.py" "$engine" "$@"
        ;;
    mesh|oban)
        [[ "$(id -u)" == 0 ]]
        exec 9<"$base/native.lock"
        flock -n 9
        chown nobody:nogroup "$out"
        chmod 0700 "$out"
        ;;
    *) exit 2 ;;
esac
common=(env -i LC_ALL=C.UTF-8 "PG_BIN_DIR=$pg" "NATIVE_JOBS_STATE=$base"
    "DURABLE_JOBS_PHASE=$DURABLE_JOBS_PHASE")
if [[ "$engine" == mesh ]]; then
    [[ "$#" == 6 && "$4" == mesh && "$5" == noop ]]
    source="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
    broker="$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
    cd "$source/tools/durable-jobs-bench/native"
    exec /usr/sbin/runuser -u nobody -- "${common[@]}" PATH=/usr/sbin:/usr/bin:/bin \
        "NATIVE_JOBS_ARTIFACTS=$broker" TRUSTED_ADMISSION_POLICY=success_cohort \
        MESH_JOBS_SET_BASED_ENQUEUE=1 "DENSE_D2_ELF=$2" "DENSE_D2_MANIFEST=$3" \
        /usr/bin/python3 -B "$stage/wal_capture.py" mesh "$source" "$1" "$out"
fi
[[ "$#" == 4 && "$2" == oban && "$3" == noop ]]
arm="$1"
cd "$(dirname "$arm")"
tool="$base/toolchain"
exec /usr/sbin/runuser -u nobody -- "${common[@]}" \
    "PATH=$tool/otp-29.1.1/bin:$tool/elixir-1.20.4/bin:/usr/bin:/bin" \
    ERL_CRASH_DUMP=/dev/null "NATIVE_JOBS_ARTIFACTS=$base/builds/oban-v2" \
    /usr/bin/python3 -B "$stage/wal_capture.py" oban "$@"
