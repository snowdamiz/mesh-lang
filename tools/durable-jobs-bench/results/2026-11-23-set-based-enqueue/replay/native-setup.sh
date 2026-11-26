#!/usr/bin/env bash
set -euo pipefail
archive_hash="${1:?archive SHA256}"
key="${2:?unique setup attempt}"
revision="${3:?exact source revision}"
[[ "$archive_hash" =~ ^[a-f0-9]{64}$ && "$revision" =~ ^[a-f0-9]{40}$ && "$key" =~ ^bulk-enqueue-[a-f0-9]{12}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-$key"
out="$base/runs/$key"
stock="$base/builds/mesh-stock-3828f20d/abi"
tool="$base/toolchain"
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" CARGO_HOME="$base/cargo-home"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0 LC_ALL=C.UTF-8
export PG_BIN_DIR="$tool/pgsql-17.11/bin"
umask 022
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$stage" "$out"
mkdir "$stage/tmp" "$stage/artifacts" "$stage/evidence"
cat > "$stage/incoming.tar.gz"
printf '%s  %s\n' "$archive_hash" "$stage/incoming.tar.gz" | sha256sum -c
tar -xzf "$stage/incoming.tar.gz" -C "$stage"
rm "$stage/incoming.tar.gz"
printf '%s  %s\n' e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$stock/meshc" \
    5111d214dcea114720c838d438686c79e061bf0b582e44239098cb741247ab95 "$stock/libmesh_rt.a" | sha256sum -c
{
    uname -smr
    cat /proc/sys/kernel/random/boot_id
    findmnt -no MAJ:MIN --target "$base"
    getconf CLK_TCK
    rustc --version
    llvm-config --version
    "$PG_BIN_DIR/postgres" --version
} > "$out/setup-identity.txt"
export TMPDIR="$stage/tmp" CARGO_TARGET_DIR="$stage/cargo-target"
trap 'rm -rf -- "$CARGO_TARGET_DIR"; rm -f -- "$stage/incoming.tar.gz"' EXIT
mkdir "$CARGO_TARGET_DIR"
if [[ -d "$base/cargo/release" ]]; then
    cp -a --reflink=auto "$base/cargo/release" "$CARGO_TARGET_DIR/release"
fi
for lane in source ordinary-v1 ordinary-v2; do
    path="$stage/$lane"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$path/scripts/ci-x86_64-linux-gnu-clang"
    (cd "$path" && cargo build --offline --locked --release -p mesh-rt) > "$out/$lane-build.log" 2>&1
    target="$base/builds/$key-$lane"
    export MESH_RT_LIB_PATH="$CARGO_TARGET_DIR/release/libmesh_rt.a"
    if [[ "$lane" == source ]]; then
        "$stock/meshc" build "$stage/fixture" --opt-level 2 --output "$stage/artifacts/proof-new" >"$out/proof-new-link.log" 2>&1
        MESH_RT_LIB_PATH="$stock/libmesh_rt.a" "$stock/meshc" build "$stage/fixture" --opt-level 2 --output "$stage/artifacts/proof-old" >"$out/proof-old-link.log" 2>&1
        mkdir -p "$target/mesh-source"
        cp "$stage/strict-fixture/"* "$target/mesh-source/"
        fixture="$target/mesh-source"
        cc -O2 -c "$fixture/worker_identity.c" -o "$fixture/worker_identity.o"
        ar rcs "$fixture/libworker_identity.a" "$fixture/worker_identity.o"
        digest="$(sha256sum "$fixture/libworker_identity.a" | cut -d' ' -f1)"
        sed -i -E "s/^sha256 = \"[a-f0-9]{64}\"/sha256 = \"$digest\"/" "$fixture/mesh.toml"
        "$stock/meshc" build "$fixture" --opt-level 2 --output "$target/mesh" >"$out/source-link.log" 2>&1
        {
            rustc --version
            llvm-config --version
            printf 'runtime_revision=%s
' "$revision"
            "$stock/meshc" --version
            sha256sum "$stock/meshc" "$MESH_RT_LIB_PATH"
        } >"$target/mesh-toolchain.txt"
        python3 -B "$path/tools/durable-jobs-bench/native/manifest.py" mesh "$target" "$revision" dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17
    else
        "$path/replay/link.sh" "$stock/meshc" "$MESH_RT_LIB_PATH" "$target" >"$out/$lane-link.log" 2>&1
        python3 -B - "$target/mesh.json" "$revision" <<'PY2'
import json
from pathlib import Path
import sys
p = Path(sys.argv[1])
m = json.loads(p.read_text())
m["version"] = sys.argv[2]
p.write_text(json.dumps(m, indent=2, sort_keys=True) + "\n")
PY2
    fi
    sha256sum "$target/mesh" "$target/mesh.json" >>"$out/artifacts.txt"
    sha256sum "$MESH_RT_LIB_PATH" >>"$out/runtimes.txt"
done
chown nobody:nogroup "$stage/tmp" "$stage/evidence"
proof_status=0
runuser -u nobody -- env "PG_BIN_DIR=$PG_BIN_DIR" "PATH=$PATH" \
    taskset -c 0,1 python3 -B "$stage/prove.py" wire-and-mixed >"$out/wire-and-mixed.log" 2>&1 || proof_status=$?
if [[ -d "$stage/evidence/wire-and-mixed" ]]; then
    cp -a "$stage/evidence/wire-and-mixed" "$out/"
fi
test "$proof_status" = 0
test -f "$out/wire-and-mixed/PASS"
flock -u 9
exec 9<&-
for lane in source ordinary-v1 ordinary-v2; do
    target="$base/builds/$key-$lane"
    profiles="short noop"
    if [[ "$lane" == ordinary-v1 ]]; then profiles="short noop slow"; fi
    for profile in $profiles; do
        mkdir -m 0700 "$out/$lane-$profile"
        policy=stock
        if [[ "$lane" == ordinary-v2 && "$profile" == short ]]; then policy=success_cohort; fi
        export NATIVE_JOBS_ARTIFACTS="$target" MESH_JOBS_SET_BASED_ENQUEUE=1 TRUSTED_ADMISSION_POLICY="$policy"
        taskset -c 0,1 "$stage/$lane/tools/durable-jobs-bench/native/run" mesh "$profile" "$out/$lane-$profile" \
            >"$out/$lane-$profile.log" 2>&1
    done
done
