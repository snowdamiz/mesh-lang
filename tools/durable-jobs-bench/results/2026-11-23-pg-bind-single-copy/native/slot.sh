#!/usr/bin/env bash
# Invoke only from the sole matrix controller with its matching UTC grant.
set -euo pipefail
phase="${1:?setup or timing}"
input_sha="${2:?sealed inputs SHA-256}"
attempt="${3:?owned attempt id}"
seconds="${4:?bounded slot duration}"
[[ "$phase" == setup || "$phase" == timing ]]
[[ "$input_sha" =~ ^[a-f0-9]{64}$ && "$attempt" =~ ^pg-bind-single-copy-[a-f0-9]{12}$ ]]
[[ "$seconds" =~ ^[0-9]+$ && "$seconds" -gt 30 ]]
[[ "$(uname -sm)" == "Linux x86_64" && "$(getconf GNU_LIBC_VERSION)" == glibc* ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/builds/pg-bind-single-copy-$input_sha"
out="$base/runs/$attempt"
export LC_ALL=C
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
umask 022
mkdir "$out"
{
    uname -smr
    cat /proc/sys/kernel/random/boot_id
    getconf GNU_LIBC_VERSION
    taskset -c 0,1 grep Cpus_allowed_list /proc/self/status
    printf 'source_archive_sha256=%s\n' "$input_sha"
} > "$out/host.txt"
cat /proc/stat > "$out/proc-stat-before.txt"
trap 'cat /proc/stat > "$out/proc-stat-after.txt"' EXIT

if [[ "$phase" == setup ]]; then
    mkdir "$stage"
    mkdir "$stage/tmp"
    export TMPDIR="$stage/tmp"
    trap 'rm -rf -- "$stage/tmp"; cat /proc/stat > "$out/proc-stat-after.txt"' EXIT
    cat > "$out/source.tar.gz"
    printf '%s  %s\n' "$input_sha" "$out/source.tar.gz" | sha256sum -c
    tar -xzf "$out/source.tar.gz" -C "$stage"
    cd "$stage"
    sha256sum -c SOURCE-SHA256SUMS
    rustc="$base/toolchain/rust-1.99.0/bin/rustc"
    "$rustc" -Vv > "$out/rustc.txt"
    for replica in a b; do
        timeout "$((seconds / 2))s" taskset -c 0,1 "$rustc" --edition=2024 \
            -C opt-level=3 --cfg profile_alloc micro.rs -o "probe-$replica"
        file "probe-$replica" >> "$out/elf.txt"
        timeout 20s taskset -c 0,1 "./probe-$replica" check >> "$out/differential.txt"
    done
    sha256sum probe-a probe-b > ELF-SHA256SUMS
    cp ELF-SHA256SUMS SOURCE-SHA256SUMS "$out"
    cp /proc/sys/kernel/random/boot_id boot-id
    touch qualified
else
    cd "$stage"
    test -f qualified
    cmp boot-id /proc/sys/kernel/random/boot_id
    sha256sum -c SOURCE-SHA256SUMS
    sha256sum -c ELF-SHA256SUMS
    cp ELF-SHA256SUMS SOURCE-SHA256SUMS "$out"
    ulimit -v 1048576
    timeout "$((seconds - 10))s" taskset -c 0,1 python3 -B probe.py "$stage" "$out"
fi
