#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd -P)"
meshc="${1:?absolute meshc path}"
runtime="${2:?absolute libmesh_rt.a path}"
out="${3:?new absolute artifact directory}"
revision="${4:?product commit}"
source_sha="${5:?source archive SHA-256}"
[[ "$revision" =~ ^[0-9a-f]{40}$ && "$source_sha" =~ ^[0-9a-f]{64}$ ]]
mkdir "$out"
cp "$here"/fixture/{main.mpl,writes.mpl,worker_identity.mpl,worker_identity.c} "$out/"
case "$(uname -sm)" in
    'Linux x86_64') target=x86_64-unknown-linux-gnu ;;
    'Darwin arm64') target=aarch64-apple-darwin ;;
    *) exit 2 ;;
esac
cc -O2 -c "$out/worker_identity.c" -o "$out/worker_identity.o"
ar rcs "$out/libworker_identity.a" "$out/worker_identity.o"
rm "$out/worker_identity.o"
checksum="$(shasum -a 256 "$out/libworker_identity.a" | cut -d' ' -f1)"
cat > "$out/mesh.toml" <<TOML
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
TOML
MESH_RT_LIB_PATH="$runtime" "$meshc" build "$out" --opt-level 2 --output "$out/mesh"
python3 -I -B - "$out" "$meshc" "$runtime" "$revision" "$source_sha" <<'PY'
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys

out, compiler, runtime = map(Path, sys.argv[1:4])
revision, source_sha = sys.argv[4:]

def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def command(*args):
    return subprocess.check_output(args, text=True).strip()

manifest = dict(
    system=platform.system(), architecture=platform.machine(), version=revision,
    source_sha256=source_sha, compiler_sha256=sha(compiler), runtime_sha256=sha(runtime),
    rustc=command("rustc", "-vV"), cargo=command("cargo", "--version"),
    cc=command("cc", "--version"), meshc=command(str(compiler), "--version"),
    execution_contract="ordinary-trusted-shared-process-v2",
    compiler_source_revision="3828f20d2d6beb683868cce33e6dd1958365914b",
    files={name: sha(out / name) for name in (
        "mesh", "main.mpl", "writes.mpl", "worker_identity.mpl",
        "worker_identity.c", "libworker_identity.a", "mesh.toml")},
)
(out / "mesh.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
PY
