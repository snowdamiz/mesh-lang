#!/usr/bin/env bash
set -euo pipefail
[[ "$#" == 4 && "$3" =~ ^(noop|short)$ && "$DURABLE_JOBS_PHASE" =~ ^(warmup|measured)$ ]]
mode="$1"
runner="$2"
out="$4"
here=$(cd "$(dirname "$0")" && pwd -P)
base=/var/lib/durable-jobs-bench/native
source="$base/source-ordinary-trusted-v1-23fdb3f5530d"
stock="$base/builds/ordinary-trusted-v1-23fdb3f5530d"
[[ "$out" == "$here"/raw/*/adapter && -d "$out" && ! -L "$out" ]]
case "$mode" in
    v1) [[ "$runner" == "$stock" ]] ;;
    service|direct|service-observed|direct-observed)
        [[ "$runner" =~ ^"$base"/builds/app-pool-[0-9a-f]{12}/"$mode"$ ]] ;;
    *) exit 2 ;;
esac
# The matrix worker holds active-arm.lock. Keep native.lock for the whole arm.
exec 9<"$base/native.lock"
flock -n 9
chmod 0644 "$here/"{native-arm.py,topology_arm.py}
chown nobody:nogroup "$out"
chmod 0700 "$out"
cd "$source/tools/durable-jobs-bench/native"
exec /usr/bin/timeout -k 5 180 /usr/sbin/runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    "PG_BIN_DIR=$base/toolchain/pgsql-17.11/bin" "NATIVE_JOBS_STATE=$base" \
    "NATIVE_JOBS_ARTIFACTS=$stock" "DURABLE_JOBS_PHASE=$DURABLE_JOBS_PHASE" \
    /usr/bin/python3 -B "$here/native-arm.py" "$source" "$runner" "$mode" "$3" "$out"
