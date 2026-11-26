#!/bin/bash
# The matrix owner invokes this only during the worker-execution-v3 SETUP slot.
set -euo pipefail
sha="${1:?candidate SHA}"
digest="${2:?source archive SHA-256}"
cpus="${3:?two CPU IDs}"
[[ "$sha" =~ ^[a-f0-9]{40}$ && "$digest" =~ ^[a-f0-9]{64}$ && "$cpus" =~ ^[0-3],[0-3]$ ]]
base=/var/lib/durable-jobs-bench/native
tool="$base/toolchain"
src="$base/source-worker-execution-v3-${sha:0:8}"
out="$base/builds/mesh-worker-execution-v3-${sha:0:8}"
evidence="$base/runs/worker-execution-v3-setup-${sha:0:8}"
stock="$base/builds/mesh-stock-3828f20d/abi"
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
umask 022
date -u +%FT%TZ
uname -smr
cat /proc/sys/kernel/random/boot_id
uptime
findmnt --target "$base" -o SOURCE,TARGET,FSTYPE,MAJ:MIN
df -h "$base"
mkdir "$src" "$evidence"
bundle="$src/received-source.tar.gz"
trap 'rm -f -- "$bundle"' EXIT
cat > "$bundle"
printf '%s  %s\n' "$digest" "$bundle" | sha256sum -c
tar -xzf "$bundle" -C "$src"
test "$(cat "$src/NATIVE_MESH_REVISION")" = "$sha"

export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8"
export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$src/scripts/ci-x86_64-linux-gnu-clang"
export NATIVE_JOBS_STATE="$base" LC_ALL=C.UTF-8
export NATIVE_JOBS_MESHC="$stock/meshc"
export NATIVE_JOBS_MESHC_SHA256=e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19
# The staged diff has no compiler/codegen changes.
export NATIVE_JOBS_MESHC_REVISION="$sha"
export NATIVE_JOBS_TRUSTED_FIXTURE=1
printf '%s\n' "actual compiler source=3828f20d2d6beb683868cce33e6dd1958365914b" \
    "candidate runtime source=$sha" "compiler source equivalence verified by bundle author"
if ! "$src/tools/durable-jobs-bench/native/build" "$out" mesh; then
    tail -80 "$out"/mesh-{build,link}.log 2>/dev/null || :
    exit 1
fi
cat "$out/mesh-toolchain.txt" "$out/mesh.json"
cp "$out"/mesh-{build,link}.log "$out/mesh-toolchain.txt" "$out/mesh.json" "$evidence/"

proof="$evidence/compiled"
install -d -m 0700 -o nobody -g nogroup "$proof"
MESH_RT_LIB_PATH="$base/cargo/release/libmesh_rt.a" "$stock/meshc" build \
    "$src/scripts/fixtures/jobs-trusted-worker" --opt-level 0 --output "$proof/trusted-effects"
cc -O2 -fPIC -shared "$src/scripts/fixtures/jobs-trusted-worker/drop_receipt.c" \
    -o "$proof/drop-receipt.so" -ldl
sha256sum "$proof"/{trusted-effects,drop-receipt.so}
taskset -c "$cpus" /usr/sbin/runuser -u nobody -- env -i \
    PG_BIN_DIR="$tool/pgsql-17.11/bin" PATH=/usr/bin:/bin \
    bash "$src/scripts/fixtures/jobs-trusted-worker/run-native.sh" "$src" "$proof" |
    tee "$evidence/fault-proof.log"

# Each standard adapter acquires and releases the native lock itself.
flock -u 9
exec 9<&-
for mode in strict trusted-persistent; do
    mkdir -m 0700 "$evidence/noop-$mode"
    NATIVE_JOBS_ARTIFACTS="$out" NATIVE_JOBS_MESH_EXECUTION="$mode" DURABLE_JOBS_PHASE=diagnostic \
        taskset -c "$cpus" "$src/tools/durable-jobs-bench/native/run" mesh noop "$evidence/noop-$mode"
    cat "$evidence/noop-$mode/mesh-execution.json" "$evidence/noop-$mode/row.json"
done
date -u +%FT%TZ
