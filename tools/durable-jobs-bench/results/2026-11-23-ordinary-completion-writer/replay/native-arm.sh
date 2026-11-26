#!/usr/bin/env bash
set -euo pipefail
replay="$(cd "$(dirname "$0")" && pwd)"
mode="${1:?v2, off, zero, hold, or off-repeat}"
source="${2:?absolute source path}"
export NATIVE_JOBS_ARTIFACTS="${3:?sealed native Mesh artifacts}"
export NATIVE_JOBS_STATE="${4:?existing private state directory}"
out="${5:?fresh absolute output directory}"
unset TRUSTED_COMPLETION_WRITER_MS MESH_JOBS_COMPLETION_TRACE
export TRUSTED_ADMISSION_POLICY=success_cohort
case "$mode" in
    v2) expected=3828f20d2d6beb683868cce33e6dd1958365914b ;;
    off|zero|hold|off-repeat)
        expected="$(shasum -a 256 "$replay/completion-writer.patch" | cut -c1-40)"
        ;;
    *) exit 2 ;;
esac
python3 -B - "$NATIVE_JOBS_ARTIFACTS/mesh.json" "$expected" <<'PY'
import json
import sys
from pathlib import Path
manifest = json.loads(Path(sys.argv[1]).read_text())
assert manifest["version"] == sys.argv[2]
assert (manifest["system"], manifest["architecture"]) == ("Linux", "x86_64")
PY
case "$mode" in
    zero) export TRUSTED_COMPLETION_WRITER_MS=0 ;;
    hold) export TRUSTED_COMPLETION_WRITER_MS=1 ;;
esac
driver="$source/tools/durable-jobs-bench/native/arm.py"
if [[ "${WRITER_COMMITTED_TRANSITIONS:-0}" == 1 ]]; then
    export DURABLE_JOBS_PHASE=diagnostic
    export COMPLETION_WRITER_SOURCE="$source"
    export MESH_JOBS_COMPLETION_TRACE=1
    driver="$replay/diagnostic-arm.py"
fi
exec python3 -B "$driver" mesh short "$out"
