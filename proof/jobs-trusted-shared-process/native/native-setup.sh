#!/usr/bin/env bash
set -euo pipefail
sha="${1:?source bundle SHA-256}"
id="${2:?fresh attempt ID}"
[[ "$sha" =~ ^[a-f0-9]{64}$ && "$id" =~ ^ordinary-trusted-product-db5937e7-[a-f0-9]{12}$ ]]
revision=db5937e7e8988f935e9be16b68a2561c2250cc27
base=/var/lib/durable-jobs-bench/native
src="$base/source-$id"
build="$base/builds/$id"
evidence="$base/runs/$id"
tool="$base/toolchain"
compiler="$base/builds/mesh-stock-3828f20d/abi/meshc"
frozen="$base/builds/ordinary-trusted-v1-23fdb3f5530d"
broker="$base/source-ordinary-trusted-v1-23fdb3f5530d/tools/durable-jobs-bench/native"
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" LC_ALL=C.UTF-8
umask 022
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$src" "$build" "$evidence"
export CARGO_TARGET_DIR="$src/cargo-target" TMPDIR="$src/tmp"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$src/scripts/ci-x86_64-linux-gnu-clang"
trap 'rm -rf -- "$CARGO_TARGET_DIR" "$TMPDIR"; rm -f -- "$src/incoming.tar.gz"' EXIT
cat >"$src/incoming.tar.gz"
printf '%s  %s\n' "$sha" "$src/incoming.tar.gz" | sha256sum -c
tar -xzf "$src/incoming.tar.gz" -C "$src"
rm -- "$src/incoming.tar.gz"
test "$(cat "$src/native-validation/revision")" = "$revision"
sha256sum -c "$src/native-validation/remote.sha256"
{
    uname -smr
    cat /proc/sys/kernel/random/boot_id
    findmnt -no MAJ:MIN --target "$base"
    getconf CLK_TCK
    rustc --version
    llvm-config --version
} >"$evidence/setup-identity.txt"
mkdir "$TMPDIR" "$CARGO_TARGET_DIR"
if [[ -d "$base/cargo/release" ]]; then
    cp -a --reflink=auto "$base/cargo/release" "$CARGO_TARGET_DIR/release"
fi
(
    cd "$src"
    # Keep dependency artifacts from the copied cache and rebuild the exact runtime.
    cargo clean --locked --release -p mesh-rt
    cargo build --locked --release -p mesh-rt
) >"$evidence/runtime-build.log" 2>&1
cp -- "$CARGO_TARGET_DIR/release/libmesh_rt.a" "$src/libmesh_rt.a"
export MESH_RT_LIB_PATH="$src/libmesh_rt.a"
grep -aFq 'trusted job could not accept renewal after its lease expired' "$MESH_RT_LIB_PATH"
cp -- "$frozen"/{main.mpl,writes.mpl,worker_identity.mpl,worker_identity.c,libworker_identity.a,mesh.toml} "$build"
"$compiler" build "$build" --opt-level 2 --output "$build/mesh" >"$evidence/link.log" 2>&1
python3 - "$frozen/mesh.json" "$build" "$revision" "$MESH_RT_LIB_PATH" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

frozen, build, revision, runtime = sys.argv[1:]
build = Path(build)
manifest = json.loads(Path(frozen).read_text())

def digest(path):
    with open(path, "rb") as blob:
        return hashlib.file_digest(blob, "sha256").hexdigest()

for name, expected in manifest["files"].items():
    if name != "mesh" and digest(build / name) != expected:
        raise SystemExit(f"frozen fixture differs: {name}")
manifest.update(version=revision, runtime_sha256=digest(runtime))
manifest["files"]["mesh"] = digest(build / "mesh")
(build / "mesh.json").write_text(json.dumps(manifest, indent=2) + "\n")
PY
sha256sum "$build"/{mesh,mesh.json} >"$evidence/artifacts.txt"

mkdir "$src/fault-fixture"
cp -- "$build"/{main.mpl,writes.mpl,worker_identity.mpl,worker_identity.c,libworker_identity.a,mesh.toml} "$src/fault-fixture"
patch "$src/fault-fixture/main.mpl" <"$src/native-validation/fault-variant.patch" >"$evidence/fault-link.log"
"$compiler" build "$src/fault-fixture" --opt-level 2 --output "$src/fault-fixture/mesh" >>"$evidence/fault-link.log" 2>&1
sha256sum "$src/fault-fixture/mesh" >>"$evidence/artifacts.txt"

# The common wrapper takes native.lock. Keep active-arm.lock for the whole setup.
flock -u 9
exec 9<&-
for profile in short noop; do
    if [[ "$profile" = short ]]; then order=(frozen-v1 product); else order=(product frozen-v1); fi
    for lane in "${order[@]}"; do
        if [[ "$lane" = product ]]; then artifact="$build"; else artifact="$frozen"; fi
        out="$evidence/$profile-$lane"
        mkdir -m 0700 "$out"
        NATIVE_JOBS_ARTIFACTS="$artifact" DURABLE_JOBS_PHASE=diagnostic \
            taskset -c 0,1 "$broker/run" mesh "$profile" "$out" >"$out.log" 2>&1
    done
done
exec 9<"$base/native.lock"
flock -n 9
mkdir -m 0700 "$evidence/faults"
chown nobody:nogroup "$evidence/faults"
(
    cd "$broker"
    taskset -c 0,1 runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
        PG_BIN_DIR="$tool/pgsql-17.11/bin" PYTHONPATH="$broker" \
        python3 -B "$src/native-validation/faults.py" "$evidence/faults/cases" "$src/fault-fixture/mesh"
) >"$evidence/faults.log" 2>&1
