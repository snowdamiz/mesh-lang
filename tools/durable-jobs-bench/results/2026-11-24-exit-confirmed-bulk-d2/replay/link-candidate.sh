#!/usr/bin/env bash
set -euo pipefail
source="${1:?source tree}"
compiler="${2:?compiler}"
runtime="${3:?runtime archive}"
out="${4:?new artifact directory}"
archive_sha="${5:?combined source archive SHA-256}"
delta_sha="${6:?D2 patch SHA-256}"
[[ "$archive_sha" =~ ^[a-f0-9]{64}$ && "$delta_sha" =~ ^[a-f0-9]{64}$ ]]
export COMPLETION_WRITER_SOURCE_SHA256=3618be08a9dda3122f7161148fc5ad810cd2ebc4a6233f84775e046c988e14ee
bash "$source/replay/link.sh" "$compiler" "$runtime" "$out"
python3 -B - "$out/mesh.json" "$archive_sha" "$delta_sha" <<'PY'
import json
from pathlib import Path
import sys
path = Path(sys.argv[1])
manifest = json.loads(path.read_text())
archive, delta = sys.argv[2:]
manifest.update(version=archive[:40], combined_source_sha256=archive, d2_patch_sha256=delta,
                checkout_env="MESH_PG_HOT_CHECKOUT=1", checkout_max_idle_ms=500)
path.write_text(json.dumps(manifest, indent=2) + "\n")
PY
