#!/usr/bin/env bash
# Only the sole controller runs this, with a dedicated active SETUP grant.
set -euo pipefail
[[ $# == 4 && "$(uname -sm)" == 'Linux x86_64' && "$(id -u)" == 0 ]]
kind="$1"
attempt="$2"
source_sha="$3"
archive_sha="$4"
[[ "$kind" == jobs || "$kind" == standalone ]]
[[ "$attempt" =~ ^trusted-e4156740-(jobs|standalone)-[a-f0-9]{12}$ ]]
[[ "$attempt" == "trusted-e4156740-$kind-${archive_sha:0:12}" ]]
[[ "$source_sha" == e4156740c7409156409cd2f792f4875153d71040 && "$archive_sha" =~ ^[a-f0-9]{64}$ ]]
base=/var/lib/durable-jobs-bench/native
stage="$base/source-$attempt"
out="$base/runs/$attempt"
tools="$base/toolchain"
frozen="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
compiler="$base/builds/mesh-stock-3828f20d/abi/meshc"
export PATH="$tools/rust-1.99.0/bin:$tools/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tools/llvm-21.1.8" PG_BIN_DIR="$tools/pgsql-17.11/bin"
export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 LC_ALL=C.UTF-8
umask 022
ulimit -c 0

exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$stage" "$out"
export CARGO_TARGET_DIR="$stage/cargo" TMPDIR="$stage/tmp"
mkdir "$CARGO_TARGET_DIR" "$TMPDIR"
seal() {
    local code="$?"
    printf '%s\n' "$code" > "$out/native.exit"
    rm -rf -- "$CARGO_TARGET_DIR" "$TMPDIR"
}
trap seal EXIT
cat > "$stage/source.tar.gz"
printf '%s  %s\n' "$archive_sha" "$stage/source.tar.gz" | sha256sum -c
tar -xzf "$stage/source.tar.gz" -C "$stage"
source="$stage/source"
packet="$stage/overlay"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$stage/proof-linker"
cat > "$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER" <<'LINKER'
#!/bin/sh
set -eu
llvm="${LLVM_SYS_211_PREFIX:?}"
exec "$llvm/bin/clang" -fuse-ld=lld --ld-path="$llvm/bin/ld.lld" "$@" \
    -Wl,--lto-O0 -Wl,--strip-debug -Wl,--thinlto-cache-dir="${TMPDIR:?}/thinlto"
LINKER
chmod 0755 "$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"
test "$(cat /proc/sys/kernel/random/boot_id)" = 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a
test "$(uname -r)" = 6.8.0-149-generic
test "$(findmnt -nro MAJ:MIN --target "$base" | xargs)" = 8:0
test "$(getconf CLK_TCK)" = 100
test "$(llvm-config --version)" = 21.1.8
(
    rustc -vV
    cargo --version
    llvm-config --version
    sha256sum "$source/Cargo.lock" "$(command -v rustc)" "$(command -v cargo)" \
        "$(command -v cc)" "$LLVM_SYS_211_PREFIX/bin/clang" "$LLVM_SYS_211_PREFIX/bin/ld.lld" \
        "$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"
    uname -a
    cat /proc/sys/kernel/random/boot_id
    findmnt -nro MAJ:MIN --target "$base"
    printf 'revision=%s\narchive_sha256=%s\nkind=%s\n' "$source_sha" "$archive_sha" "$kind"
) > "$out/toolchain.txt"
cd "$source"

if [[ "$kind" == standalone ]]; then
    # Build once from e4156740. Cargo supplies this exact meshc via CARGO_BIN_EXE_meshc.
    cmd=(cargo test --offline --locked -p meshc --test e2e_proof_gates)
    taskset -c 0,1 "${cmd[@]}" --no-run > "$out/build.log" 2>&1
    exact="$CARGO_TARGET_DIR/debug/meshc"
    mkdir "$stage/artifacts"
    cp "$exact" "$stage/artifacts/meshc"
    chmod a-w "$stage/artifacts/meshc"
    sha256sum "$exact" > "$out/compiler.sha256"
    readelf -h -n "$exact" > "$out/compiler-elf.txt"
    sha256sum "$source/proof/autonomous-gates/performance-budget.json" \
       "$source/compiler/meshc/tests/e2e_proof_gates.rs" > "$out/test-inputs.sha256"
    taskset -c 0,1 "${cmd[@]}" the_performance_gate_measures_and_reports -- \
        --exact --nocapture --test-threads 1 > "$out/standalone-test.log" 2>&1
    sha256sum -c "$out/compiler.sha256"
    grep -F 'test the_performance_gate_measures_and_reports ... ok' "$out/standalone-test.log"
    grep -F 'test result: ok. 1 passed; 0 failed' "$out/standalone-test.log"
    exit
fi

printf '%s  %s\n' \
    e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$compiler" \
    b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$PG_BIN_DIR/postgres" \
    | sha256sum -c | tee "$out/retained-binaries.txt"
(cd "$frozen" && sha256sum -c "$packet/frozen-harness.sha256") > "$out/frozen-harness.txt"
"$PG_BIN_DIR/postgres" --version > "$out/postgres-version.txt"
taskset -c 0,1 cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt \
    > "$out/build.log" 2>&1
runtime="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libmesh_rt.a"
mkdir "$stage/artifacts"
for label in jobs witness; do
    inputs="$packet"
    if [[ "$label" == witness ]]; then
        inputs="$stage/witness-source"
        python3 -B "$packet/replay/witness_fixture.py" "$packet" "$inputs"
    fi
    artifact="$stage/artifacts/$label"
    taskset -c 0,1 bash "$inputs/replay/link.sh" "$compiler" "$runtime" "$artifact" \
        "$source_sha" "$archive_sha" > "$out/$label-link.log" 2>&1
    cp "$artifact/mesh.json" "$out/$label.json"
    readelf -h -n "$artifact/mesh" > "$out/$label-elf.txt"
    ldd "$artifact/mesh" > "$out/$label-dependencies.txt"
    chmod -R a-w "$artifact"
done

chown nobody:nogroup "$TMPDIR"
mkdir -m 0700 "$out/semantic-root"
chown nobody:nogroup "$out/semantic-root"
runuser -u nobody -- env -i "PATH=$PATH" "PG_BIN_DIR=$PG_BIN_DIR" LC_ALL=C.UTF-8 \
    "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$source" "PRODUCT_FROZEN_HARNESS=$frozen" \
    taskset -c 0,1 python3 -B "$packet/replay/proof.py" "$out/semantic-root/semantic" \
        "$stage/artifacts/jobs/mesh" "$stage/artifacts/witness/mesh" > "$out/semantic.log" 2>&1
