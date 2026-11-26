#!/usr/bin/env bash
set -euo pipefail
lane="$(cd "$(dirname "$0")" && pwd -P)"
base="$lane/../linode-ordered-enqueue"
compiler="$base/build/release/meshc"
export TMPDIR="$lane/tmp"
runtime="$lane/build/release/libmesh_rt.a"
MESH_RT_LIB_PATH="$runtime" "$compiler" build "$lane/fixture" --opt-level 2 --output "$lane/artifacts/proof-new"
MESH_RT_LIB_PATH="$runtime" "$compiler" build "$lane/artifacts/strict" --opt-level 2 --output "$lane/artifacts/strict/mesh"
shasum -a 256 "$compiler" "$runtime" "$lane/artifacts/"{proof-new,strict/mesh} >"$lane/evidence/compiled-02.sha256"
