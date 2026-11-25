#!/usr/bin/env bash
# Entrypoint for the existing matrix worker. The worker owns active-arm.lock.
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
source="$base/source-ordinary-trusted-v1-23fdb3f5530d"
stock="$base/builds/ordinary-trusted-v1-23fdb3f5530d"
here=$(cd "$(dirname "$0")" && pwd -P)
[[ $# == 5 && "$4" == noop && "$DURABLE_JOBS_PHASE" =~ ^(warmup|measured)$ ]]
mode="$1"
runner="$2"
module="$3"
out="$5"
[[ "$mode" =~ ^(on|off)$ && "$runner" == "$base/builds/app-pool-39e79f4abb76/direct-observed" ]]
[[ "$module" =~ ^"$base"/postflush-pg17-[0-9a-f]{12}/postflush\.so$ && -f "$module" && ! -L "$module" ]]
[[ "$out" == "$here"/raw/*/adapter && -d "$out" && ! -L "$out" ]]
exec 9<"$base/native.lock"
flock -n 9
[[ "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base")" == 8:0 ]]
helpers=("$here/"{native-arm.py,topology_arm.py,identity.py,arm_overlay.py,boundary.py})
chmod 0644 "${helpers[@]}"
/usr/sbin/runuser -u nobody -- env -i PATH=/usr/bin:/bin LC_ALL=C.UTF-8 \
    /usr/bin/python3 -B -c 'from pathlib import Path; import sys
for path in sys.argv[1:]:
    Path(path).read_bytes()' "${helpers[@]}" "$module"
printf '%s\n' 'POSTFLUSH_STAGED_HELPERS_READABLE'
chown nobody:nogroup "$out"
chmod 0700 "$out"
cd "$source/tools/durable-jobs-bench/native"
exec /usr/bin/timeout -k 5 35 /usr/sbin/runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    "PG_BIN_DIR=$base/toolchain/pgsql-17.11/bin" "NATIVE_JOBS_STATE=$base" \
    "NATIVE_JOBS_ARTIFACTS=$stock" "DURABLE_JOBS_PHASE=$DURABLE_JOBS_PHASE" \
    /usr/bin/python3 -B "$here/arm_overlay.py" "$source" "$runner" "$out" "$module" "$mode"
