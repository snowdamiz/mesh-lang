#!/usr/bin/env bash
set -euo pipefail
umask 022
here=$(cd "$(dirname "$0")" && pwd -P)
native=/var/lib/durable-jobs-bench/native
matrix=/var/tmp/mesh-durable-jobs-matrix
tool="$native/toolchain"
port=3f69b150820fd01ba716d533be4f2eedea25ed27
original=dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17
candidate=$(cat "$here/revision.txt")
[[ "$candidate" =~ ^[a-f0-9]{40}$ ]]
exec 8<>"$matrix/active-arm.lock"
flock -n 8
exec 9<"$native/native.lock"
flock -n 9

base="$native/builds/mesh-v1"
compiler="$native/compiler-artifacts/3f69b150/meshc"
printf '%s  %s\n' 2a824ff2199be1067965bb2ebeebb0ce9e306c3ec6bb4cf73fdd7577a13c753f "$compiler" | sha256sum -c
source_dir="$here/source"
artifact="$here/artifact"
build_home="$here/cargo-home"
mkdir "$source_dir" "$artifact" "$build_home"
cp -a "$native/source-v5/." "$source_dir/"
(cd "$source_dir" && patch --batch --forward -p1 < "$here/runtime.patch")
cp -a "$base/mesh-source" "$artifact/mesh-source"
mkdir "$build_home/registry"
for directory in cache index src; do
    cp -a "$native/cargo-home/registry/$directory" "$build_home/registry/"
done
install -d -m 0700 -o nobody -g nogroup "$here/scratch"
chmod 0755 "$here"
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" CARGO_BUILD_JOBS=4
export CARGO_HOME="$build_home" CARGO_TARGET_DIR="$here/cargo-target"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$source_dir/scripts/ci-x86_64-linux-gnu-clang"
(cd "$source_dir" && cargo build --release --locked --offline -p mesh-rt)
runtime="$CARGO_TARGET_DIR/release/libmesh_rt.a"
MESH_RT_LIB_PATH="$runtime" "$compiler" build "$artifact/mesh-source" --opt-level 2 --output "$artifact/mesh"
{
    rustc --version
    "$LLVM_SYS_211_PREFIX/bin/llvm-config" --version
    printf 'compiler_revision=%s\nruntime_revision=%s\n' "$port" "$candidate"
    "$compiler" --version
    sha256sum "$compiler" "$runtime" "$here/runtime.patch" "$source_dir/compiler/mesh-rt/src/jobs/store.rs"
    file "$artifact/mesh"
} > "$artifact/mesh-toolchain.txt"
python3 -B "$source_dir/tools/durable-jobs-bench/native/manifest.py" mesh "$artifact" "$candidate" "$original"
sha256sum "$artifact/mesh.json" "$artifact/mesh"
