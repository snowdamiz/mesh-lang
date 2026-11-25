#!/usr/bin/env bash
set -euo pipefail
[[ $# == 8 && "$(id -u)" == 0 ]]
engine="$1"; policy="$2"; launcher="$3"; artifact="$4"; work="$5"; bin="$6"; profile="$7"; out="$8"
here="$(cd "$(dirname "$0")" && pwd -P)"
base=/var/lib/durable-jobs-bench/native
tool="$base/toolchain"
[[ "$profile" == slow && "${DURABLE_JOBS_PHASE:-}" =~ ^(diagnostic|warmup|measured)$ ]]
[[ "$policy" == pglz || "$policy" == lz4 ]]
[[ "$work" == "$base"/slow-lz4-isolated-*/work && "$bin" == "$work"/pgsql-17.11-lz4-*/bin ]]
[[ "$out" == /var/tmp/mesh-durable-jobs-matrix/*/raw/*/adapter && -d "$out" && ! -L "$out" ]]
[[ -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]]
case "$engine" in
    mesh|oban)
        [[ "$launcher" == "$base"/source-*/tools/durable-jobs-bench/native/arm.py && "$artifact" == "$base"/builds/* ]]
        exec 9<"$base/native.lock"
        flock -n 9
        args=("$engine")
        ;;
    go|rust)
        [[ "$launcher" == /var/lib/durable-jobs-bench/*/tools/durable-jobs-bench/*/run.py && -x "$artifact" ]]
        scratch=river
        [[ "$engine" == go ]] || scratch=apalis
        args=(--binary "$artifact" --pg-bin "$bin" --scratch "$work/$scratch")
        ;;
    *) exit 2 ;;
esac
chown nobody:nogroup "$out"
chmod 0700 "$out"
cd "$(dirname "$launcher")"
exec /usr/sbin/runuser -u nobody -- env -i \
    "PATH=$tool/otp-29.1.1/bin:$tool/elixir-1.20.4/bin:/usr/sbin:/usr/bin:/bin" \
    LC_ALL=C.UTF-8 ERL_CRASH_DUMP=/dev/null "PG_BIN_DIR=$bin" \
    "NATIVE_JOBS_STATE=$work" "NATIVE_JOBS_ARTIFACTS=$artifact" \
    "DURABLE_JOBS_PHASE=$DURABLE_JOBS_PHASE" \
    /usr/bin/python3 -B "$here/adapter.py" "$engine" "$policy" "$launcher" "${args[@]}" "$profile" "$out"
