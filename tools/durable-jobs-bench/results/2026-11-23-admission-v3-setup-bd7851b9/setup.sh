#!/usr/bin/env bash
# Invoked by the matrix owner during an active admission-v3 SETUP grant.
set -euo pipefail
sha="${1:?candidate SHA}"
digest="${2:?source archive SHA-256}"
cpus="${3:?two CPU IDs}"
[[ "$sha" =~ ^[a-f0-9]{40}$ && "$digest" =~ ^[a-f0-9]{64}$ && "$cpus" =~ ^[0-3],[0-3]$ ]]
[[ "${cpus%,*}" != "${cpus#*,}" ]]
base=/var/lib/durable-jobs-bench/native
tool="$base/toolchain"
src="$base/source-admission-v3-${sha:0:8}"
out="$base/builds/mesh-admission-v3-${sha:0:8}"
cache="$base/builds/admission-v3-cache-${sha:0:8}"
runs="$base/runs/admission-v3-smoke-${sha:0:8}"
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
"$tool/pgsql-17.11/bin/postgres" --version
mkdir "$src" "$cache"
cleanup() {
    rm -f -- "$src/incoming.tar.gz"
    rm -rf -- "$cache"
}
trap cleanup EXIT
cat > "$src/incoming.tar.gz"
printf '%s  %s\n' "$digest" "$src/incoming.tar.gz" | sha256sum -c
tar -xzf "$src/incoming.tar.gz" -C "$src"
rm "$src/incoming.tar.gz"
test "$(cat "$src/CANDIDATE")" = "$sha"
cp "$src/CANDIDATE" "$src/NATIVE_MESH_REVISION"

export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4
export CARGO_TARGET_DIR="$cache"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$src/scripts/ci-x86_64-linux-gnu-clang"
export NATIVE_JOBS_STATE="$base" LC_ALL=C.UTF-8
export NATIVE_JOBS_MESHC="$stock/meshc"
export NATIVE_JOBS_MESHC_SHA256=e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19
export NATIVE_JOBS_MESHC_REVISION="$sha"
export NATIVE_JOBS_MESHC_SOURCE_REVISION=3828f20d2d6beb683868cce33e6dd1958365914b
if "$src/tools/durable-jobs-bench/native/build" "$out" mesh; then
    cat "$out/mesh-toolchain.txt" "$out/mesh.json"
else
    result=$?
    tail -60 "$out"/mesh-{build,link}.log 2>/dev/null || :
    exit "$result"
fi
# Retain build identities; the cache and incoming source archive are transient.
cleanup
trap - EXIT
date -u +%FT%TZ
flock -u 9
exec 9<&-
mkdir -m 0755 "$runs"
status=0
for arm in strict:noop cohort:noop strict:short cohort:short; do
    mode="${arm%:*}"
    profile="${arm#*:}"
    smoke="$runs/$mode-$profile"
    mkdir -m 0700 "$smoke"
    NATIVE_JOBS_ARTIFACTS="$out" DURABLE_JOBS_PHASE=diagnostic MESH_JOBS_ADMISSION_V3="$mode" \
        taskset -c "$cpus" "$src/tools/durable-jobs-bench/native/run" mesh "$profile" "$smoke" || status=$?
    if [[ -f "$smoke/row.json" ]]; then
        sha256sum "$smoke/"{ledger.json,row.json,admission-ledger.json}
        cat "$smoke/admission-ledger.json" "$smoke/row.json"
    fi
done
date -u +%FT%TZ
exit "$status"
