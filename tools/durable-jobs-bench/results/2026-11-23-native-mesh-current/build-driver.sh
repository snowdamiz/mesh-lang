set -euo pipefail
base=/var/lib/durable-jobs-bench/native
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
echo 'both remote locks acquired'
umask 022
src="$base/source-stock-3828f20d"
out="$base/builds/mesh-stock-3828f20d"
tool="$base/toolchain"
mkdir "$src"
trap 'rm -f -- "$src/incoming.tar.gz"' EXIT
cat > "$src/incoming.tar.gz"
printf '%s  %s
' b630c907274bb7c56174a5360c13c719c8e02ec661479efc711bbd4ca306acda "$src/incoming.tar.gz" | sha256sum -c
tar -xzf "$src/incoming.tar.gz" -C "$src"
rm "$src/incoming.tar.gz"
uname -smr
cat /proc/sys/kernel/random/boot_id
nproc --all
taskset -pc $$
uptime
findmnt --target "$base" -o SOURCE,TARGET,FSTYPE,MAJ:MIN
df -h "$base"
ps -eo pid,comm | awk '$2 ~ /^(cargo|rustc|clang|ld.lld|postgres|beam.smp|mesh|cc1|make)$/'
pg="$tool/pgsql-17.11/bin"
"$pg/postgres" --version
"$pg/pg_config" --configure
sha256sum "$pg/"{postgres,initdb,pg_ctl,psql}
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$src/scripts/ci-x86_64-linux-gnu-clang"
export NATIVE_JOBS_STATE="$base" LC_ALL=C.UTF-8
rustc --version
"$LLVM_SYS_211_PREFIX/bin/llvm-config" --version
date -u +%FT%TZ
if "$src/tools/durable-jobs-bench/native/build" "$out" mesh; then
    cat "$out/mesh-toolchain.txt" "$out/mesh.json"
else
    result=$?
    tail -60 "$out"/mesh-{build,link}.log 2>/dev/null || :
    exit "$result"
fi
date -u +%FT%TZ
