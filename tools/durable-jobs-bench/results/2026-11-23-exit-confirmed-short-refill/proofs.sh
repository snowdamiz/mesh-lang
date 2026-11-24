#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
root="$(cd "$here/../../../.." && pwd -P)"
combined="$here/../2026-11-23-ordinary-completion-writer/combined/replay"
scratch="$root/target/exit-confirmed-short-refill"
original="$root/target/ordinary-completion-writer/combined"
runner="$scratch/refill-artifacts/mesh"
broker="$original/broker-artifacts/mesh"
out="${1:?new private proof directory}"
[[ "$out" == "$scratch"/* && ! -e "$out" ]]
mkdir "$out"
export TMPDIR="$scratch/tmp/" PG_BIN_DIR="${PG_BIN_DIR:-/opt/homebrew/opt/postgresql@17/bin}"
export COMBINED_BULK_SOURCE="$original/bulk-base/ordinary-v2" MESH_SHORT_REFILL=1
export COMPLETION_WRITER_SOURCE_SHA256="$(shasum -a 256 "$here/refill.patch" | cut -d' ' -f1)"
exec 9<"$root/target/queue-performance.lock"
flock 9
run_case() {
  local label="$1"
  shift
  local code=0
  "$@" >"$out/$label.log" 2>&1 || code=$?
  printf '{"returncode": %d}\n' "$code" >"$out/$label.status.json"
  printf '%s %d\n' "$label" "$code"
}
run_case split-faults python3 -B "$combined/split-faults.py" "$out/split-faults" "$broker" "$runner"
run_case sparse python3 -B "$here/prove-sparse.py" "$out/sparse" "$runner"
run_case witness-source python3 -B "$combined/teardown-fixture.py" "$scratch/source" "$out/witness-source"
run_case witness-link bash "$out/witness-source/replay/link.sh" "$scratch/cargo/release/meshc" \
  "$scratch/cargo/release/libmesh_rt.a" "$out/witness-artifacts"
run_case teardown python3 -B "$combined/teardown.py" "$out/teardown" "$broker" "$out/witness-artifacts/mesh" zero
