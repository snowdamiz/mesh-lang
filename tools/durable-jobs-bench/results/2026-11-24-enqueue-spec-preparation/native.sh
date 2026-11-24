#!/usr/bin/env bash
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
tool="$base/toolchain"
stock="$base/builds/mesh-stock-3828f20d/abi"
short="$base/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
large="$base/builds/mesh-direct-producer-v3"
[[ "$#" -ge 3 ]]
step="$1"
id="$2"
[[ "$id" =~ ^enqueue-spec-preparation-[a-f0-9]{12}$ ]]
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 && "$(uname -r)" == 6.8.0-149-generic ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -no MAJ:MIN --target "$base")" == 8:0 ]]
umask 022
export PATH="$tool/rust-1.99.0/bin:$tool/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin" LC_ALL=C.UTF-8
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
home="$base/source-$id"
if [[ "$step" == broker ]]; then
    [[ "$#" == 5 && "$3" =~ ^(short|large)$ && "$4" =~ ^[a-f0-9]{12}$ && "$5" =~ ^[a-f0-9]{12}$ ]]
    cd "$home"
    sha256sum -c SHA256SUMS >&2
    sha256sum -c BINARY-SHA256SUMS >&2
    export LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8"
    export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0
    exec python3 -B broker.py "$3" "$4" "$base/runs/$id-component-$5"
fi
[[ "$#" == 3 ]]
if [[ "$step" == timing ]]; then
    suffix="$3"
    [[ "$suffix" =~ ^[a-f0-9]{12}$ && -f "$home/SETUP-PASS" ]]
    cd "$home"
    sha256sum -c SHA256SUMS >&2
    sha256sum -c BINARY-SHA256SUMS >&2
    exec python3 -B rotation.py run "$home" "$base/runs/$id-component-$suffix"
fi
[[ "$step" == setup ]]
archive="$3"
[[ "$archive" =~ ^[a-f0-9]{64}$ ]]
mkdir "$home" "$home/tmp" "$home/bin" "$home/verify"
cd "$home"
trap 'rm -rf -- "$home/cargo-short" "$home/cargo-large" "$home/tmp" "$home/incoming.tar.gz"' EXIT
cat >incoming.tar.gz
printf '%s  %s\n' "$archive" incoming.tar.gz | sha256sum -c >&2
tar -xzf incoming.tar.gz
sha256sum -c SHA256SUMS >&2
export TMPDIR="$home/tmp" LLVM_SYS_211_PREFIX="$tool/llvm-21.1.8"
export CARGO_HOME="$base/cargo-home" CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
rustc --version | tee verify/rustc.txt
llvm-config --version | tee verify/llvm.txt
grep -q '^rustc 1.99.0 (b940084d7 ' verify/rustc.txt
grep -Fxq 21.1.8 verify/llvm.txt
cat <<EOF | sha256sum -c >&2
e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19  $stock/meshc
5111d214dcea114720c838d438686c79e061bf0b582e44239098cb741247ab95  $stock/libmesh_rt.a
55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a  $short/mesh
e4bab43e5e22662d6ecd4a4d32aaf554978779c1d1f0c88f72284bc99f06e419  $large/broker
EOF
for profile in short large; do
    cp -a "$home/tree-$profile" "$home/work-$profile"
    tree="$home/work-$profile"
    python3 -B capture.py "$profile" "$profile-broker.mpl" "$home/verify/capture-$profile" "$stock/meshc" "$stock/libmesh_rt.a" "$profile-raw.tar.gz"
    python3 -B - "$profile" <<'PY'
import json
from pathlib import Path
import sys
p, = sys.argv[1:]
native = json.loads(Path(f"verify/capture-{p}/capture.json").read_text())
frozen = json.loads(Path(f"{p}-capture.json").read_text())
for key in ("frames", "jobs", "collector_sha256", "broker_source_sha256", "raw_ledger_archive_sha256", "fixture_transport_sha256"):
    assert native[key] == frozen[key], (p, key)
PY
    python3 -B materialize.py "$tree" component "$home/verify/$profile.patch"
    cmp "$profile.patch" "$home/verify/$profile.patch"
    export CARGO_TARGET_DIR="$home/cargo-$profile"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$tree/scripts/ci-x86_64-linux-gnu-clang"
    (
        cd "$tree"
        cargo tree --offline --locked -p mesh-rt -e features
        cargo build --offline --locked --release -p mesh-rt --example enqueue-spec-component
    ) >"$home/verify/$profile-build.log" 2>&1
    cp "$CARGO_TARGET_DIR/release/examples/enqueue-spec-component" "$home/bin/$profile"
    file "$home/bin/$profile" | tee "$home/verify/$profile-elf.txt"
    grep -q 'ELF 64-bit.*x86-64' "$home/verify/$profile-elf.txt"
    "$home/bin/$profile" verify "$home/frames/$profile" >"$home/verify/$profile.json"
    cmp "$home/verify/$profile.json" "$home/$profile-semantic.json"
done
if cmp -s bin/short bin/large; then
    echo "short and large unexpectedly share a component binary" >&2
    exit 1
fi
python3 -B - <<'PY'
import json
from pathlib import Path
from rotation import digest
out = {p: {"component_sha256": digest(Path("bin") / p),
           "lock_sha256": digest(Path(f"tree-{p}/Cargo.lock")),
           "setup_verification_sha256": digest(Path(f"verify/{p}.json")),
           "capture_verification_sha256": digest(Path(f"verify/capture-{p}/capture.json"))}
       for p in ("short", "large")}
Path("build-pins.json").write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")
PY
sha256sum bin/{short,large} verify/{short,large}.json build-pins.json | tee BINARY-SHA256SUMS
mkdir "$home/native-proof"
if [[ "$(id -u)" == 0 ]]; then
    chown nobody:nogroup "$home/native-proof"
    runuser -u nobody -- python3 -B wire-check.py "$tool/pgsql-17.11/bin" "$home/bin" "$home/native-proof/run"
else
    python3 -B wire-check.py "$tool/pgsql-17.11/bin" "$home/bin" "$home/native-proof/run"
fi
touch SETUP-PASS
