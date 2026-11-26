#!/usr/bin/env bash
set -euo pipefail
[[ "$#" == 3 && "$1" =~ ^[0-9a-f]{64}$ && "$2" =~ ^app-pool-[0-9a-f]{12}$ && "$3" =~ ^[0-9]+$ ]]
seal="$1"
attempt="$2"
budget="$3"
base=/var/lib/durable-jobs-bench/native
tool="$base/toolchain"
src="$base/source-$attempt"
build="$base/builds/$attempt"
pg="$tool/pgsql-17.11/bin"
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin"
export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8" CARGO_HOME="$base/cargo-home"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 LC_ALL=C.UTF-8
umask 022

exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
mkdir "$src"
mkdir "$src/evidence"
evidence="$src/evidence"
export CARGO_TARGET_DIR="$src/cargo-target" TMPDIR="$src/tmp"
finish() {
    code=$?
    trap - EXIT
    set +e
    printf '{"exit_code":%d}\n' "$code" > "$evidence/status.json"
    (cd "$evidence" && tar -czf - .)
    rm -rf -- "$CARGO_TARGET_DIR" "$TMPDIR"
    rm -f -- "$src/incoming.tar.gz"
    exit "$code"
}
trap finish EXIT
mkdir "$TMPDIR"
cat > "$src/incoming.tar.gz"
printf '%s  %s\n' "$seal" "$src/incoming.tar.gz" | sha256sum -c >&2
tar -xzf "$src/incoming.tar.gz" -C "$src"
cd "$src"
cp stage/setup-command.txt "$evidence/setup-command.txt"
{
    uname -smr
    cat /proc/sys/kernel/random/boot_id
    findmnt -no MAJ:MIN --target "$base/scratch"
    getconf CLK_TCK
    nproc
    sha256sum "$pg/postgres"
    rustc --version
    llvm-config --version
} > "$evidence/setup-host.txt"
test "$(uname -smr)" = 'Linux 6.8.0-149-generic x86_64'
test "$(cat /proc/sys/kernel/random/boot_id)" = 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a
test "$(findmnt -no MAJ:MIN --target "$base/scratch")" = 8:0
test "$(getconf CLK_TCK)" = 100
test "$(nproc)" = 4
printf '%s  %s\n' b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$pg/postgres" | sha256sum -c >&2
sha256sum -c stage/stock-native.sha256 > "$evidence/stock-native.log"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$src/scripts/ci-x86_64-linux-gnu-clang"
timeout -k 15 "$budget" cargo build --locked --release -p meshc -p mesh-rt > "$evidence/build.log" 2>&1
compiler="$CARGO_TARGET_DIR/release/meshc"
runtime="$CARGO_TARGET_DIR/release/libmesh_rt.a"
python3 -B stage/fixtures.py stage/fixture "$src/fixtures"
bash stage/link-fixtures.sh "$compiler" "$runtime" "$src/fixtures" "$build" > "$evidence/link.log" 2>&1
mkdir "$build/semantics"
cp stage/semantics.mpl "$build/semantics/main.mpl"
MESH_RT_LIB_PATH="$runtime" "$compiler" build "$build/semantics" --output "$build/semantics/mesh" \
    > "$evidence/semantic-build.log" 2>&1
identity=$(mktemp -d "$src/scope.XXXXXXXX")
chown nobody:nogroup "$identity"
code=0
timeout -k 10 75 taskset -c 0,1 /usr/sbin/runuser -u nobody -- env -i \
    PATH=/usr/bin:/bin LC_ALL=C.UTF-8 TMPDIR="$identity" \
    bash "$src/stage/semantic.sh" "$build/semantics/mesh" "$identity/pg" "$pg" > "$evidence/semantics.log" 2>&1 || code=$?
if [[ "$code" == 0 ]]; then
    grep -Fxq APP_POOL_SCOPE_OK "$evidence/semantics.log"
    rm -rf -- "$identity"
else
    printf 'Semantic proof failed. Retained owned diagnostic state at %s\n' "$identity" >&2
    exit "$code"
fi
python3 -B - "$build" "$compiler" "$runtime" "$seal" "$evidence" <<'PY'
import hashlib, json, sys
from pathlib import Path
build, compiler, runtime, seal, evidence = sys.argv[1:]
build, compiler, runtime, evidence = map(Path, (build, compiler, runtime, evidence))

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

common = dict(system="Linux", architecture="x86_64",
              version="db5937e7e8988f935e9be16b68a2561c2250cc27",
              base_revision="dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17",
              execution_contract="ordinary-trusted-shared-process",
              compiler_sha256=digest(compiler), runtime_sha256=digest(runtime),
              source_archive_sha256=seal)
for mode in ("service", "direct", "service-observed", "direct-observed"):
    folder = build / mode
    names = ("mesh", "main.mpl", "mesh.toml", "writes.mpl",
             "worker_identity.c", "worker_identity.mpl", "libworker_identity.a")
    manifest = dict(common, mode=mode, files={name: digest(folder / name) for name in names})
    encoded = json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    (folder / "mesh.json").write_text(encoded)
    (evidence / f"{mode}.json").write_text(encoded)
PY
