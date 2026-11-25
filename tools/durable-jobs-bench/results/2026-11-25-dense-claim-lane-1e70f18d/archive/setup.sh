#!/usr/bin/env bash
# The sole controller runs this under its dedicated SETUP grant.
set -euo pipefail
[[ $# == 6 && "$(uname -sm)" == 'Linux x86_64' && "$(id -u)" == 0 ]]
base=/var/lib/durable-jobs-bench/native
proof="$base/source-trusted-1e70f18d-jobs-78468180f0c0"
dest="$base/dense-claim-lane-1e70f18d"
out="$base/runs/${dest##*/}"
tools="$base/toolchain"
frozen="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
compiler="$base/builds/mesh-stock-3828f20d/abi/meshc"
rev=1e70f18d624a841740d3bbb3d0fd09b269257f68
payload="$4"
export PATH="$tools/rust-1.99.0/bin:$tools/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tools/llvm-21.1.8" PG_BIN_DIR="$tools/pgsql-17.11/bin"
export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 LC_ALL=C.UTF-8
export CARGO_TARGET_DIR="$dest/cargo" TMPDIR="$dest/tmp"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$proof/proof-linker"
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
trap 'code=$?; printf "%s\n" "$code" > "$out/native.exit"; rm -rf -- "$CARGO_TARGET_DIR" "$TMPDIR" "$dest/incoming.tar" "$dest/source" "$dest/witness-input" "$dest/control-input"' EXIT
cat > "$dest/incoming.tar"
printf '%s  %s\n' "$payload" "$dest/incoming.tar" | sha256sum -c
tar -xf "$dest/incoming.tar" -C "$dest" --no-same-owner --no-same-permissions
(cd "$dest" && sha256sum -c installed.sha256) > "$out/installed.log"
cp "$dest/installed.sha256" "$out/"
awk '$2 == "native-run" || $2 == "adapter.py" || $2 == "identity.py"' \
    "$out/installed.sha256" > "$out/adapter.sha256"
printf '%s  %s\n' \
    78468180f0c0be9b2b17533b54dc8cbd69e3982d84b239457b15e5beb018b530 "$proof/source.tar.gz" \
    "$5" "$proof/artifacts/jobs/mesh" "$6" "$proof/artifacts/jobs/mesh.json" \
    e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$compiler" \
    b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$PG_BIN_DIR/postgres" \
    | sha256sum -c > "$out/prerequisites.log"
sha256sum -c "$dest/proof-toolchain.sha256" > "$out/toolchain.log"
(cd "$frozen" && sha256sum -c "$proof/overlay/frozen-harness.sha256") > "$out/harness.log"

# Recreate from the pinned public source bundle in an isolated stage.
tar -xzf "$proof/source.tar.gz" -C "$dest" --no-same-owner --no-same-permissions source
(cd "$dest/source" && patch --batch --fuzz=0 -p1 < "$dest/claim-lane.patch")
runtime="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libmesh_rt.a"
(cd "$dest/source" && taskset -c 0,1 cargo build --offline --locked --release \
    --target x86_64-unknown-linux-gnu -p mesh-rt) > "$out/build.log" 2>&1
sha256sum "$runtime" > "$out/runtime.sha256"

python3 -B "$dest/overlay/replay/witness_fixture.py" "$dest/overlay" "$dest/witness-input"
python3 -B "$dest/overlay/replay/control_fixture.py" "$dest/overlay" "$dest/control-input"
for label in jobs witness control; do
    inputs="$dest/overlay"
    if [[ "$label" == witness ]]; then inputs="$dest/witness-input"; fi
    if [[ "$label" == control ]]; then inputs="$dest/control-input"; fi
    artifact="$dest/artifacts/$label"
    taskset -c 0,1 bash "$inputs/replay/link.sh" "$compiler" "$runtime" "$artifact" \
        "$rev" "$payload" > "$out/$label-link.log" 2>&1
    cp "$artifact/mesh.json" "$out/$label.json"
    readelf -h -n "$artifact/mesh" > "$out/$label-elf.txt"
    chmod -R a-w "$artifact"
    runuser -u nobody -- test -x "$artifact/mesh"
done
cp "$out/jobs.json" "$out/mesh.json"
chown nobody:nogroup "$TMPDIR" "$dest/scratch"
mkdir -m 0700 "$out/semantic-root"
chown nobody:nogroup "$out/semantic-root"
runuser -u nobody -- env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" LC_ALL=C.UTF-8 \
    "TMPDIR=$TMPDIR" "MESH_JOBS_CLAIM_LANE_LAUNCH=1" "PRODUCT_SOURCE=$proof/source" "PRODUCT_FROZEN_HARNESS=$frozen" \
    taskset -c 0,1 python3 -B "$dest/overlay/replay/proof.py" "$out/semantic-root/semantic" \
        "$dest/artifacts/jobs/mesh" "$dest/artifacts/witness/mesh" > "$out/semantic.log" 2>&1
export PRODUCT_ELF_SHA256="$(sha256sum "$dest/artifacts/jobs/mesh" | cut -d' ' -f1)"
export PRODUCT_MANIFEST_SHA256="$(sha256sum "$dest/artifacts/jobs/mesh.json" | cut -d' ' -f1)"
runuser -u nobody -- env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" LC_ALL=C.UTF-8 \
    "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$proof/source" "PRODUCT_FROZEN_HARNESS=$frozen" \
    taskset -c 0,1 python3 -B "$dest/overlay/replay/held_ack.py" "$out/semantic-root/held" \
        "$dest/artifacts/jobs/mesh" > "$out/held.log" 2>&1
runuser -u nobody -- env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" LC_ALL=C.UTF-8 \
    "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$proof/source" "PRODUCT_FROZEN_HARNESS=$frozen" \
    taskset -c 0,1 python3 -B "$dest/overlay/replay/held_ack.py" "$out/semantic-root/controls" \
        "$dest/artifacts/control/mesh" pause scale > "$out/controls.log" 2>&1
runuser -u nobody -- env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" LC_ALL=C.UTF-8 \
    "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$proof/source" "PRODUCT_FROZEN_HARNESS=$frozen" \
    taskset -c 0,1 python3 -B "$dest/overlay/replay/grant_edges.py" "$out/semantic-root/grant-edges" \
        "$dest/artifacts/jobs/mesh" > "$out/grant-edges.log" 2>&1
for setting in 0 1; do
    mkdir "$out/$setting"
    HOT_CHECKOUT=1 MESH_JOBS_CLAIM_LANE_LAUNCH="$setting" DURABLE_JOBS_PHASE=diagnostic taskset -c 0,1 \
        bash "$dest/native-run" mesh noop "$out/$setting"
done
touch "$out/SETUP-PASS"
