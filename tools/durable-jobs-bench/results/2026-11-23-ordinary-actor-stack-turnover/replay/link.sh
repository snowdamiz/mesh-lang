#!/usr/bin/env bash
set -euo pipefail
compiler="$1"
runtime="$2"
output="$3"
source="$(cd "$(dirname "$0")/mesh" && pwd -P)"
project="$(dirname "$output")/$(basename "$output")-project"
mkdir "$project"
cp "$source"/{main.mpl,probe.mpl,probe.c} "$project/"
cc -O2 -Wall -Wextra -Werror -c "$project/probe.c" -o "$project/probe.o"
ar rcs "$project/libprobe.a" "$project/probe.o"
digest="$(shasum -a 256 "$project/libprobe.a" | cut -d' ' -f1)"
cat > "$project/mesh.toml" <<TOML
[package]
name = "actor-stack-turnover"
version = "0.1.0"
[native]
abi = 1
bindings = ["probe.mpl"]
[[native.libraries]]
target = "aarch64-apple-darwin"
path = "libprobe.a"
sha256 = "$digest"
TOML
MESH_RT_LIB_PATH="$runtime" "$compiler" build "$project" --opt-level 2 --emit-llvm --output "$output"
