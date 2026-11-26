#!/usr/bin/env bash
set -euo pipefail
compiler="$1"
runtime="$2"
fixtures="$3"
out="$4"
mkdir "$out"
export MESH_RT_LIB_PATH="$runtime"
case "$(uname -sm)" in
    'Linux x86_64') target=x86_64-unknown-linux-gnu ;;
    'Darwin arm64') target=aarch64-apple-darwin ;;
    *) exit 2 ;;
esac
for mode in service direct service-observed direct-observed; do
    dest="$out/$mode"
    mkdir "$dest"
    cp "$fixtures/$mode/"* "$dest/"
    cc -O2 -c "$dest/worker_identity.c" -o "$dest/worker_identity.o"
    ar rcs "$dest/libworker_identity.a" "$dest/worker_identity.o"
    checksum=$(shasum -a 256 "$dest/libworker_identity.a" | cut -d' ' -f1)
    cat > "$dest/mesh.toml" <<EOF
[package]
name = "jobs-parity-trusted"
version = "0.1.0"
[native]
abi = 1
bindings = ["worker_identity.mpl"]
[[native.libraries]]
target = "$target"
path = "libworker_identity.a"
sha256 = "$checksum"
EOF
    "$compiler" build "$dest" --opt-level 2 --output "$dest/mesh"
done
