#!/usr/bin/env bash
set -euo pipefail
[[ $# == 5 && "$(uname -sm)" == 'Linux x86_64' && "$(id -u)" == 0 ]]
base=/var/lib/durable-jobs-bench/native
dest="$base/dense-hot-readiness-1e70f18d-logger-off"
out="$base/runs/${dest##*/}"
tool="$base/toolchain"
frozen="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
compiler="$base/builds/mesh-stock-3828f20d/abi/meshc"
rev=1e70f18d624a841740d3bbb3d0fd09b269257f68
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" PG_BIN_DIR="$tool/pgsql-17.11/bin"
export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 LC_ALL=C.UTF-8
export CARGO_TARGET_DIR="$dest/build" TMPDIR="$dest/tmp"
umask 022
ulimit -c 0
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ "$(cat /proc/sys/kernel/random/boot_id)" == "$1" && "$(uname -r)" == "$2" ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base" | xargs)" == "$3" && "$(getconf CLK_TCK)" == 100 ]]
mkdir -m 0711 "$out"
mkdir -m 0755 "$dest" "$dest/artifacts"
mkdir -m 0700 "$TMPDIR" "$dest/scratch"
trap 'code=$?; printf "%s\n" "$code" > "$out/native.exit"; rm -rf -- "$CARGO_TARGET_DIR" "$TMPDIR" "$dest/incoming.tar"' EXIT
cat > "$dest/incoming.tar"
printf '%s  %s\n' "$4" "$dest/incoming.tar" | sha256sum -c
tar -xf "$dest/incoming.tar" -C "$dest" --no-same-owner --no-same-permissions
(cd "$dest" && sha256sum -c installed.sha256) > "$out/installed.log"
cp "$dest/installed.sha256" "$out/"
printf '%s  %s\n' \
    78468180f0c0be9b2b17533b54dc8cbd69e3982d84b239457b15e5beb018b530 "$dest/source.tar.gz" \
    97b8e75f8568191ac2b3a3160ce9c40d0eebc822ba2ec4661259510734816661 "$dest/main-logger-off.mpl" \
    "$5" "$dest/runtime.patch" \
    e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$compiler" \
    b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$PG_BIN_DIR/postgres" \
    | sha256sum -c > "$out/prerequisites.log"
tar -xzf "$dest/source.tar.gz" -C "$dest" --no-same-owner --no-same-permissions
install -m 0644 "$dest/main-logger-off.mpl" "$dest/overlay/fixture/main.mpl"
(cd "$frozen" && sha256sum -c "$dest/overlay/frozen-harness.sha256") > "$out/harness.log"
(
    cd "$dest/source"
    git apply --no-index --check "$dest/runtime.patch"
    git apply --no-index "$dest/runtime.patch"
    sha256sum Cargo.lock
    rustc -vV
    cargo --version
    llvm-config --version
    "$PG_BIN_DIR/postgres" --version
    uptime
    nproc
) > "$out/toolchain.log"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$dest/proof-linker"
cat > "$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER" <<'LINKER'
#!/bin/sh
set -eu
llvm="${LLVM_SYS_211_PREFIX:?}"
exec "$llvm/bin/clang" -fuse-ld=lld --ld-path="$llvm/bin/ld.lld" "$@" \
    -Wl,--lto-O0 -Wl,--strip-debug -Wl,--thinlto-cache-dir="${TMPDIR:?}/thinlto"
LINKER
chmod 0755 "$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"
(cd "$dest/source" && taskset -c 0,1 cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt) > "$out/build.log" 2>&1
runtime="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libmesh_rt.a"
artifact="$dest/artifacts/jobs"
taskset -c 0,1 bash "$dest/overlay/replay/link.sh" "$compiler" "$runtime" "$artifact" "$rev" "$5" > "$out/link.log" 2>&1
cp "$artifact/mesh.json" "$out/mesh.json"
readelf -h -n "$artifact/mesh" > "$out/mesh-elf.txt"
ldd "$artifact/mesh" > "$out/dependencies.txt"
chmod -R a-w "$artifact"
chown nobody:nogroup "$TMPDIR" "$dest/scratch"
mkdir -m 0700 "$out/proof"
chown nobody:nogroup "$out/proof"
cp "$dest/native-proof.py" "$dest/overlay/replay/dense-hot-readiness-proof.py"
runuser -u nobody -- env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" LC_ALL=C.UTF-8 \
    "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$dest/source" "PRODUCT_FROZEN_HARNESS=$frozen" \
    taskset -c 0,1 python3 -B "$dest/overlay/replay/dense-hot-readiness-proof.py" \
        "$out/proof/cases" "$artifact/mesh" > "$out/proof.log" 2>&1
export PRODUCT_ELF_SHA256="$(sha256sum "$artifact/mesh" | cut -d' ' -f1)"
export PRODUCT_MANIFEST_SHA256="$(sha256sum "$artifact/mesh.json" | cut -d' ' -f1)"
mkdir "$out/cold-ready"
CHECKOUT_MODE=cold-ready DURABLE_JOBS_PHASE=diagnostic taskset -c 0,1 \
    bash "$dest/native-run" mesh noop "$out/cold-ready" > "$out/bridge.log" 2>&1
touch "$out/SETUP-PASS"
