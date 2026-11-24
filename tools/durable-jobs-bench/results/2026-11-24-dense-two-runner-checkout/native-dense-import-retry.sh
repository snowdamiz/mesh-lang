#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
base=/var/lib/durable-jobs-bench/native
[[ "$#" == 7 && "$5" == mesh && "$6" == noop && "${DURABLE_JOBS_PHASE:-}" =~ ^(warmup|measured)$ ]]
mode="$1"
runner="$2"
elf="$3"
manifest="$4"
out="$7"
[[ "$(id -u)" == 0 && "$out" == "$here"/raw/*/adapter && -d "$out" && ! -L "$out" ]]
[[ -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]]
source="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
broker="$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
policy=success_cohort
bulk=1
case "$mode" in
    d2-off|d2-on)
        [[ "$runner" =~ ^"$base"/builds/exit-bulk-d2-[a-f0-9]{12}$ ]]
        [[ "$elf" =~ ^[a-f0-9]{64}$ && "$manifest" =~ ^[a-f0-9]{64}$ ]]
        ;;
    r2-off|r2-zero)
        [[ "$runner" == "$base/builds/bulk-writer-b1dcd34af288" && "$elf" == unused && "$manifest" == unused ]]
        ;;
    v1)
        source="$base/source-ordinary-trusted-v1-23fdb3f5530d"
        broker="$base/builds/ordinary-trusted-v1-23fdb3f5530d"
        [[ "$runner" == "$broker" && "$elf" == unused && "$manifest" == unused ]]
        policy=stock
        bulk=0
        ;;
    *) exit 2 ;;
esac

# The matrix worker already holds the remote active-arm lock.
exec 9<"$base/native.lock"
flock -n 9
chmod 0644 "$here/native-arm.py" "$here/identity.py"
chown nobody:nogroup "$out"
chmod 0700 "$out"
cd "$source/tools/durable-jobs-bench/native"
exec /usr/sbin/runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    "PG_BIN_DIR=$base/toolchain/pgsql-17.11/bin" "NATIVE_JOBS_STATE=$base" \
    "NATIVE_JOBS_ARTIFACTS=$broker" "DURABLE_JOBS_PHASE=$DURABLE_JOBS_PHASE" \
    "TRUSTED_ADMISSION_POLICY=$policy" "MESH_JOBS_SET_BASED_ENQUEUE=$bulk" \
    "DENSE_D2_ELF=$elf" "DENSE_D2_MANIFEST=$manifest" \
    /usr/bin/python3 -B -c '
import importlib.util
from pathlib import Path
import runpy
import sys

entry = Path(sys.argv.pop(1))
spec = importlib.util.spec_from_file_location("identity", entry.with_name("identity.py"))
identity = importlib.util.module_from_spec(spec)
sys.modules["identity"] = identity
spec.loader.exec_module(identity)
runpy.run_path(str(entry), run_name="__main__")
' "$here/native-arm.py" "$mode" "$source" "$runner" "$out"
