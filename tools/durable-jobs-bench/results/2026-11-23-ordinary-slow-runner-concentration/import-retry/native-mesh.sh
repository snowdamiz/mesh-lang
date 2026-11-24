#!/usr/bin/env bash
set -euo pipefail

base=/var/lib/durable-jobs-bench/native
source="$1"
artifacts="$2"
label="$3"
out="$5"
here="$(cd "$(dirname "$0")" && pwd -P)"
[[ "$#" == 5 && "$4" == slow && "${DURABLE_JOBS_PHASE:-}" =~ ^(warmup|measured)$ ]]
[[ "$source" == "$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v1" &&
   "$artifacts" == "$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v1" ]]
[[ "$out" == "$here"/raw/*/adapter && -d "$out" && ! -L "$out" ]]
[[ -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]]
case "$label" in one-a|one-b|two|four) ;; *) exit 2 ;; esac

# The matrix worker already holds the active-arm lock.
exec 9<"$base/native.lock"
flock -n 9
chmod 0644 "$here/arm.py" "$here/slow_arm.py"
chown nobody:nogroup "$out"
chmod 0700 "$out"
cd "$source/tools/durable-jobs-bench/native"
exec /usr/sbin/runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    "PG_BIN_DIR=$base/toolchain/pgsql-17.11/bin" "NATIVE_JOBS_STATE=$base" \
    "NATIVE_JOBS_ARTIFACTS=$artifacts" "DURABLE_JOBS_PHASE=$DURABLE_JOBS_PHASE" \
    /usr/bin/python3 -B -c '
import importlib.util
from pathlib import Path
import runpy
import sys

entry = Path(sys.argv.pop(1))
spec = importlib.util.spec_from_file_location("arm", entry.with_name("arm.py"))
adapter = importlib.util.module_from_spec(spec)
sys.modules["arm"] = adapter
spec.loader.exec_module(adapter)
runpy.run_path(str(entry), run_name="__main__")
' "$here/slow_arm.py" "$source" "$label" "$out"
