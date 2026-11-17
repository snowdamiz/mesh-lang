#!/bin/sh
set -eu

case ${PERF_WORKERS-} in
  ''|*[!0-9]*)
    echo "PERF_WORKERS must be a positive decimal integer" >&2
    exit 1
    ;;
esac
if ! [ "$PERF_WORKERS" -gt 0 ] || ! [ "$PERF_WORKERS" -le 4294967295 ]; then
  echo "PERF_WORKERS must be in the range 1 to 4294967295" >&2
  exit 1
fi

export MESH_SCHEDULER_MIN_WORKERS="$PERF_WORKERS"
export MESH_SCHEDULER_MAX_WORKERS="$PERF_WORKERS"
exec "${1:?usage: run.sh /absolute/path/to/mesh-benchmark}"
