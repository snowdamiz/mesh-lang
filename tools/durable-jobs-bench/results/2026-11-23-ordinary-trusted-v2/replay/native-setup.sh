#!/usr/bin/env bash
set -euo pipefail
sha="${1:?source bundle SHA256}"
id="${2:?new attempt ID}"
policy="${3:?selected ordinary queue policy}"
[[ "$policy" == set_based || "$policy" == success_cohort ]]
[[ "$sha" =~ ^[a-f0-9]{64}$ && "$id" =~ ^ordinary-trusted-v2-[a-f0-9]{12}$ ]]
base=/var/lib/durable-jobs-bench/native
src="$base/source-$id"
build="$base/builds/$id"
evidence="$base/runs/$id"
tool="$base/toolchain"
stock="$base/builds/mesh-stock-3828f20d/abi"
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" CARGO_HOME="$base/cargo-home"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 LC_ALL=C.UTF-8
umask 022
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$src" "$evidence"
cat > "$src/incoming.tar.gz"
trap 'rm -f -- "$src/incoming.tar.gz"' EXIT
printf '%s  %s\n' "$sha" "$src/incoming.tar.gz" | sha256sum -c
tar -xzf "$src/incoming.tar.gz" -C "$src"
rm "$src/incoming.tar.gz"
printf '%s  %s\n' e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$stock/meshc" \
                      5111d214dcea114720c838d438686c79e061bf0b582e44239098cb741247ab95 "$stock/libmesh_rt.a" | sha256sum -c
export CARGO_TARGET_DIR="$src/cargo-target" TMPDIR="$src/tmp"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$src/scripts/ci-x86_64-linux-gnu-clang"
mkdir "$TMPDIR"
trap 'rm -rf -- "$src/cargo-target" "$src/tmp"; rm -f -- "$src/incoming.tar.gz"' EXIT
{
    uname -smr
    cat /proc/sys/kernel/random/boot_id
    findmnt -no MAJ:MIN --target "$base"
    getconf CLK_TCK
    rustc --version
    llvm-config --version
    date -u +%FT%TZ
} > "$evidence/setup-identity.txt"
if [[ -d "$base/cargo/release" ]]; then
    mkdir "$CARGO_TARGET_DIR"
    cp -a --reflink=auto "$base/cargo/release" "$CARGO_TARGET_DIR/release"
fi
(
    cd "$src"
    cargo build --locked --release -p mesh-rt
) > "$evidence/runtime-build.log" 2>&1
cp "$CARGO_TARGET_DIR/release/libmesh_rt.a" "$src/libmesh_rt.a"
"$src/replay/link.sh" "$stock/meshc" "$src/libmesh_rt.a" "$build" > "$evidence/link.log" 2>&1
sha256sum "$build"/{mesh,mesh.json} > "$evidence/artifacts.txt"
date -u +%FT%TZ >> "$evidence/setup-identity.txt"
# The stock run wrapper acquires native.lock itself. Retain active-arm.lock.
flock -u 9
exec 9<&-
mkdir -m 0700 "$evidence/short"
TRUSTED_ADMISSION_POLICY="$policy" NATIVE_JOBS_ARTIFACTS="$build" DURABLE_JOBS_PHASE=diagnostic \
    taskset -c 0,1 "$src/tools/durable-jobs-bench/native/run" mesh short "$evidence/short" \
    > "$evidence/short.log" 2>&1
