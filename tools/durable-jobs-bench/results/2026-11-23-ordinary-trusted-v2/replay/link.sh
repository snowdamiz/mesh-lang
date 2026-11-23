#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
meshc="${1:?compiler absolute path}"
runtime="${2:?runtime archive absolute path}"
out="${3:?new artifact directory}"
mkdir "$out"
cp "$root"/fixture/{main.mpl,writes.mpl,worker_identity.mpl,worker_identity.c} "$out/"
case "$(uname -sm)" in
    'Linux x86_64') target=x86_64-unknown-linux-gnu ;;
    'Darwin arm64') target=aarch64-apple-darwin ;;
    *) exit 2 ;;
esac
cc -O2 -c "$out/worker_identity.c" -o "$out/worker_identity.o"
ar rcs "$out/libworker_identity.a" "$out/worker_identity.o"
checksum="$(shasum -a 256 "$out/libworker_identity.a" | cut -d' ' -f1)"
cat > "$out/mesh.toml" <<EOF
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
export MESH_RT_LIB_PATH="$runtime"
"$meshc" build "$out" --opt-level 2 --output "$out/mesh"
python3 -B "$root/replay/manifest.py" "$out" "$meshc" "$runtime"
