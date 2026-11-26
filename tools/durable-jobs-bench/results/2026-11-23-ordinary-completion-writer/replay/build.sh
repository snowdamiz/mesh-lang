#!/usr/bin/env bash
set -euo pipefail
replay="$(cd "$(dirname "$0")" && pwd)"
source="${1:?absolute path to materialized source}"
meshc="${2:?absolute path to the controller-pinned Mesh compiler}"
out="${3:?fresh absolute artifact path}"
case "$source:$meshc:$out" in /*:/*:/*) ;; *) exit 2 ;; esac
export COMPLETION_WRITER_SOURCE_SHA256="$(shasum -a 256 "$replay/completion-writer.patch" | awk '{print $1}')"
: "${CARGO_TARGET_DIR:=$source/target}"
export CARGO_TARGET_DIR
case "$CARGO_TARGET_DIR" in /*) ;; *) exit 2 ;; esac
cd "$source"
cargo build --locked --release -p mesh-rt
bash "$source/replay/link.sh" "$meshc" "$CARGO_TARGET_DIR/release/libmesh_rt.a" "$out"
