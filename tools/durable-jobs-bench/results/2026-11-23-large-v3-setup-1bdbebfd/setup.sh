#!/usr/bin/env bash
# Invoked exclusively by the matrix owner during an active large-v3 SETUP grant.
# stdin is the checksummed staged source archive.
set -euo pipefail
sha="${1:?candidate SHA}"
digest="${2:?source bundle SHA-256}"
cpus="${3:?two CPU IDs}"
[[ "$sha" =~ ^[a-f0-9]{40}$ && "$digest" =~ ^[a-f0-9]{64}$ && "$cpus" =~ ^[0-3],[0-3]$ ]]
base=/var/lib/durable-jobs-bench/native
tool="$base/toolchain"
src="$base/source-large-v3-${sha:0:8}"
out="$base/builds/mesh-large-v3-${sha:0:8}"
stock="$base/builds/mesh-stock-3828f20d/abi"
smoke="$base/runs/large-v3-smoke-${sha:0:8}"

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
"$tool/pgsql-17.11/bin/postgres" --version
mkdir "$src"
cat > "$src/received-source.tar.gz"
printf '%s  %s\n' "$digest" "$src/received-source.tar.gz" | sha256sum -c
tar -xzf "$src/received-source.tar.gz" -C "$src"
test "$(cat "$src/NATIVE_MESH_REVISION")" = "$sha"

export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$src/scripts/ci-x86_64-linux-gnu-clang"
export NATIVE_JOBS_STATE="$base" LC_ALL=C.UTF-8
# The product diff has zero compiler source, manifest, linker or frozen fixture changes.
# Preserve the actual stock compiler's provenance in a separate setup receipt.
if [[ -x "$stock/meshc" ]]; then
    export NATIVE_JOBS_MESHC="$stock/meshc"
    export NATIVE_JOBS_MESHC_SHA256=e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19
    export NATIVE_JOBS_MESHC_REVISION="$sha"
    export NATIVE_JOBS_ACTUAL_MESHC_REVISION=3828f20d2d6beb683868cce33e6dd1958365914b
    printf '%s\n' "actual compiler source=3828f20d2d6beb683868cce33e6dd1958365914b" \
        "candidate runtime source=$sha" "compiler source equivalence verified by bundle author"
fi
if "$src/tools/durable-jobs-bench/native/build" "$out" mesh; then
    cat "$out/mesh-toolchain.txt" "$out/mesh.json"
else
    result=$?
    tail -60 "$out"/mesh-{build,link}.log 2>/dev/null || :
    exit "$result"
fi
date -u +%FT%TZ
# The adapter itself owns the innermost lock during the smoke.
flock -u 9
exec 9<&-
mkdir -m 0700 "$smoke"
NATIVE_JOBS_ARTIFACTS="$out" DURABLE_JOBS_PHASE=diagnostic taskset -c "$cpus" \
    "$src/tools/durable-jobs-bench/native/run" mesh large "$smoke"
date -u +%FT%TZ
sha256sum "$smoke/"{queue-protocol.json,ledger.json,row.json}
cat "$smoke/row.json"
