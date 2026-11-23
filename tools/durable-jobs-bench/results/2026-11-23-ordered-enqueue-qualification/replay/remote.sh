#!/usr/bin/env bash
set -euo pipefail
umask 077

step=$1
evidence=$2
attempt=$3
base=/var/lib/durable-jobs-bench/native
source="$base/source-ordered-enqueue-$attempt"
artifact="$base/builds/mesh-ordered-enqueue-$attempt"
setup="$base/setup/ordered-enqueue-$attempt"
stock_source="$base/source-v5"
stock="$base/builds/mesh-v1"
tool="$base/toolchain"
rust="$tool/rust-1.99.0"
llvm="$tool/llvm-21.1.8"
pg="$tool/pgsql-17.11"
compiler="$base/compiler-artifacts/3f69b150/meshc"
candidate=77c447f773a4d0af5c5578873a2410301b3ba069
control=3f69b150820fd01ba716d533be4f2eedea25ed27
adapter=tools/durable-jobs-bench/native
source_sha=0c1695ebb875c875095429750731c137656e47b04c59e273d03b813392c5caef
compiler_sha=2a824ff2199be1067965bb2ebeebb0ce9e306c3ec6bb4cf73fdd7577a13c753f
stock_manifest_sha=a562897e895c87251018ec2111e6e61adfb80e822db3a5f19c586581cc7e7dd3
pg_sha=b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1
pg_stats_sha=0cb86ff6113361a6e677309642c97a2779d4234d5b615ef8e740d7d9f8a4d637

check() { printf '%s  %s\n' "$1" "$2" | sha256sum -c -; }

test "$(uname -sm)" = 'Linux x86_64'
test "$(nproc --all)" = 4
test "$("$pg/bin/postgres" --version)" = 'postgres (PostgreSQL) 17.11'
test "$(cat /proc/sys/kernel/random/boot_id)" = "$EXPECTED_BOOT"
test "$(uname -r)" = "$EXPECTED_KERNEL"
test "$(findmnt -T "$base/scratch" -rno MAJ:MIN)" = "$EXPECTED_MOUNT"
check "$pg_sha" "$pg/bin/postgres"
check "$pg_stats_sha" "$pg/lib/pg_stat_statements.so"
check "$stock_manifest_sha" "$stock/mesh.json"

build_env=(env -i "PATH=$rust/bin:$llvm/bin:/usr/bin:/bin" LC_ALL=C.UTF-8
    "LLVM_SYS_211_PREFIX=$llvm" "CARGO_HOME=$setup/cargo-home" "CARGO_TARGET_DIR=$setup/cargo"
    CARGO_BUILD_JOBS=4 "TMPDIR=$setup/tmp"
    "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$source/scripts/ci-x86_64-linux-gnu-clang")
bench_env=(env -i "PATH=/usr/bin:/bin" LC_ALL=C.UTF-8 "PG_BIN_DIR=$pg/bin" DURABLE_JOBS_PHASE=diagnostic
    "ORDERED_SOURCE=$source" "ORDERED_BASE=$base" "ORDERED_ARTIFACT=$artifact")

manifest() {
    python3 -B - "$@" <<'PY'
from pathlib import Path
import hashlib, json, sys
package, revision = Path(sys.argv[1]), sys.argv[2]
data = json.loads((package / "mesh.json").read_text())
assert (data["engine"], data["system"], data["architecture"], data["version"], data["fixture"]) == (
    "mesh", "Linux", "x86_64", revision, "linux-warm-replay-20261122")
expected = {"mesh", "mesh-toolchain.txt",
            *(str(path.relative_to(package)) for path in (package / "mesh-source").iterdir())}
assert data["files"].keys() == expected
for name, digest in data["files"].items():
    with (package / name).open("rb") as stream:
        assert hashlib.file_digest(stream, "sha256").hexdigest() == digest, name
PY
}

manifest "$stock" "$control"
if [[ "$step" == build ]]; then
    check "$source_sha" "$setup/input/source.tar.gz"
    check "$compiler_sha" "$compiler"
    test "$(awk -v h="$compiler_sha" '$1 == h {print $1}' "$stock/mesh-toolchain.txt")" = "$compiler_sha"
    test "$("$llvm/bin/llvm-config" --version)" = 21.1.8
    "$rust/bin/rustc" --version | grep '^rustc 1\.99\.0 '
    mkdir -m 0755 "$source"
    mkdir -m 0755 "$artifact"
    mkdir -m 0700 "$setup/tmp" "$setup/cargo-home" "$setup/cargo-home/registry"
    for dir in cache index src; do
        cp -a "$base/cargo-home/registry/$dir" "$setup/cargo-home/registry/"
    done
    tar -xzf "$setup/input/source.tar.gz" -C "$source" --no-same-owner
    chmod -R u=rwX,go=rX "$source"
    test "$(cat "$source/NATIVE_MESH_REVISION")" = "$candidate"

    # The patch is confined to mesh-rt. Relink the identical compiled fixture
    # with the same stock compiler and rebuilt, explicitly selected runtime.
    (
        cd "$source"
        "${build_env[@]}" cargo build --offline --locked --release -p mesh-rt
    ) >"$evidence/mesh-build.log" 2>&1
    runtime="$setup/cargo/release/libmesh_rt.a"
    test -s "$runtime"
    cp -a "$stock/mesh-source" "$artifact/mesh-source"
    for fixture in main.mpl worker_identity.mpl; do
        tar -xOf "$source/proof/jobs-parity/linux-warm-replay-20261122.tar.gz" "frozen-fixture/$fixture" |
            cmp - "$artifact/mesh-source/$fixture"
    done
    "${build_env[@]}" "MESH_RT_LIB_PATH=$runtime" "$compiler" build "$artifact/mesh-source" \
        --opt-level 2 --output "$artifact/mesh" >"$evidence/mesh-link.log" 2>&1
    (
        "$rust/bin/rustc" --version
        "$llvm/bin/llvm-config" --version
        printf 'compiler_revision=%s\nruntime_revision=%s\n' "$control" "$candidate"
        "$compiler" --version
        sha256sum "$compiler" "$runtime"
        file "$artifact/mesh"
    ) >"$artifact/mesh-toolchain.txt"
    python3 -B "$source/$adapter/manifest.py" mesh "$artifact" "$candidate" dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17
    chmod 0755 "$artifact/mesh"
    chmod 0644 "$artifact/mesh.json" "$artifact/mesh-toolchain.txt"
    for fixture in "$stock/mesh-source/"*; do
        cmp "$fixture" "$artifact/mesh-source/${fixture##*/}"
    done
    manifest "$artifact" "$candidate"
    cp "$artifact"/{mesh.json,mesh-toolchain.txt} "$evidence/"
    cp "$stock/mesh.json" "$evidence/stock-mesh.json"
    sha256sum "$artifact"/mesh{,.json} "$pg/bin/postgres" >"$evidence/build.sha256"
    printf '%s\n%s\n' "$source" "$artifact" >"$evidence/paths.txt"
    printf '%s\n' "$candidate" >"$setup/build-ready"
    exit 0
fi

test "$(cat "$setup/build-ready")" = "$candidate"
manifest "$artifact" "$candidate"
if [[ "$step" == wire-test ]]; then
    cd "$source"
    "${build_env[@]}" cargo test --offline --locked --release -p mesh-rt --lib \
      db::pg::tests::ordered_enqueue_reads_a_backpressured_window_and_retires_a_truncated_connection -- --exact
    exit
fi

data="$evidence/rows"
state="$setup/scratch-${evidence##*/}"
chmod 0711 "$evidence"
install -d -o nobody -g nogroup -m 0700 "$data" "$state" "$state/scratch"
test "$(findmnt -T "$state" -rno MAJ:MIN)" = "$EXPECTED_MOUNT"
if [[ "$step" == semantic-* ]]; then
    choice="${step#semantic-}"
    shape="${choice#*-}"
    if [[ "$choice" == stock-* ]]; then
        binary="$stock"
        runner_source="$stock_source"
    else
        binary="$artifact"
        runner_source="$source"
    fi
    # The parent holds the builder lock across the whole step. The run
    # wrapper would take it again, so invoke its exact underlying adapter.
    cd "$runner_source/$adapter"
    runuser -u nobody -- taskset -c 0,1 "${bench_env[@]}" "NATIVE_JOBS_STATE=$state" \
      "NATIVE_JOBS_ARTIFACTS=$binary" python3 -B "$runner_source/$adapter/arm.py" mesh "$shape" "$data"
    exit
fi

shape="${step#producer-}"
mode=()
if [[ "$shape" == wire-* ]]; then
    shape="${shape#wire-}"
    mode=(--wire --groups 1)
fi
cd "$source"
runuser -u nobody -- taskset -c 0,1 "${bench_env[@]}" "ORDERED_SCRATCH=$state/scratch" \
  python3 -B "$setup/input/producer.py" "$shape" "$data" "${mode[@]}"
