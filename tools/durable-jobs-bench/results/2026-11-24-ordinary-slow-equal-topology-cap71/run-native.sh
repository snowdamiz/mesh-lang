#!/usr/bin/env bash
set -euo pipefail

engine="$1"
launcher="$2"
artifact="$3"
scratch="$4"
profile="$5"
out="$6"
here="$(cd "$(dirname "$0")" && pwd -P)"
base=/var/lib/durable-jobs-bench/native
tool="$base/toolchain"
[[ "$#" == 6 && "$profile" == slow && "${DURABLE_JOBS_PHASE:-}" =~ ^(warmup|measured)$ ]]
[[ "$out" == "$here"/raw/*/adapter && -d "$out" && ! -L "$out" ]]
[[ -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]]
case "$engine" in
    mesh|oban)
        [[ "$launcher" == "$base"/source-*/tools/durable-jobs-bench/native/arm.py &&
           "$artifact" == "$base"/builds/* && "$scratch" == "$base/scratch" ]]
        args=("$engine")
        ;;
    go|rust)
        [[ "$launcher" == /var/lib/durable-jobs-bench/*/tools/durable-jobs-bench/"$engine"/run.py ||
           "$engine" == rust && "$launcher" == /var/lib/durable-jobs-bench/apalis/*/tools/durable-jobs-bench/rust/run.py ]]
        [[ "$scratch" == /var/lib/durable-jobs-bench/*/scratch ]]
        args=(--binary "$artifact" --pg-bin "$tool/pgsql-17.11/bin" --scratch "$scratch")
        if [[ "$engine" == go ]]; then
            [[ "$artifact" == "$here/river-slow-2x32-cap71" ]]
        fi
        ;;
    *) exit 2 ;;
esac

# The stock recorder holds the native lock itself for River and Apalis.
if [[ "$engine" == mesh || "$engine" == oban ]]; then
    exec 9<"$base/native.lock"
    flock -n 9
fi
chmod 0644 "$here/adapter.py"
if [[ "$engine" == go ]]; then chmod 0755 "$artifact"; fi
chown nobody:nogroup "$out"
chmod 0700 "$out"
cd "$(dirname "$launcher")"
exec /usr/sbin/runuser -u nobody -- env -i \
    "PATH=$tool/otp-29.1.1/bin:$tool/elixir-1.20.4/bin:/usr/sbin:/usr/bin:/bin" \
    LC_ALL=C.UTF-8 ERL_CRASH_DUMP=/dev/null "PG_BIN_DIR=$tool/pgsql-17.11/bin" \
    "NATIVE_JOBS_STATE=$base" "NATIVE_JOBS_ARTIFACTS=$artifact" \
    "DURABLE_JOBS_PHASE=$DURABLE_JOBS_PHASE" \
    /usr/bin/python3 -B "$here/adapter.py" "$engine" "$launcher" "${args[@]}" "$profile" "$out"
