#!/bin/bash
# Build every benchmark program with one toolchain.
# usage: build.sh <label> <meshc> <libmesh_rt.a> [opt-level]
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
LABEL="$1"; MESHC="$2"; RT="$3"; OPT="${4:-2}"
OUT="$HERE/bin/$LABEL"
mkdir -p "$OUT"
for dir in "$HERE"/programs/*/; do
  name="$(basename "$dir")"
  if MESH_RT_LIB_PATH="$RT" "$MESHC" build "$dir" --opt-level "$OPT" -o "$OUT/$name" >"$OUT/$name.log" 2>&1; then
    echo "ok   $name"
  else
    echo "FAIL $name"; tail -15 "$OUT/$name.log"
  fi
done
