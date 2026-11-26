#!/usr/bin/env bash
set -euo pipefail
base="$1"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export RUSTUP_TOOLCHAIN=1.99.0 CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=3
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21
export TMPDIR="$base/tmp/" CARGO_TARGET_DIR="$base/cargo-v1"
cd "$base/v1/source"
cargo build --locked --offline --release -p mesh-rt
mkdir "$base/v1/reuse"
cp -p "$CARGO_TARGET_DIR/release/libmesh_rt.a" "$base/v1/reuse/libmesh_rt.a"
out="$base/v1/relinked"
mkdir "$out"
cp fixture/{main.mpl,writes.mpl,worker_identity.mpl,worker_identity.c} "$out/"
cc -O2 -c "$out/worker_identity.c" -o "$out/worker_identity.o"
ar rcs "$out/libworker_identity.a" "$out/worker_identity.o"
digest="$(shasum -a 256 "$out/libworker_identity.a" | cut -d' ' -f1)"
cat > "$out/mesh.toml" <<TOML
[package]
name = "jobs-parity-trusted"
version = "0.1.0"
[native]
abi = 1
bindings = ["worker_identity.mpl"]
[[native.libraries]]
target = "aarch64-apple-darwin"
path = "libworker_identity.a"
sha256 = "$digest"
TOML
compiler="$base/bin/meshc"
runtime="$base/v1/reuse/libmesh_rt.a"
MESH_RT_LIB_PATH="$runtime" "$compiler" build "$out" --opt-level 2 --output "$out/mesh"
python3 -B "$base/v1/manifest.py" "$out" "$compiler" "$runtime"
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin python3 -B "$base/probe/v1-count.py" "$base/v1/source" "$out" \
    "$base/v1/state" "$base/bin/native-calls.dylib" "$base/observations/v1-reuse"
