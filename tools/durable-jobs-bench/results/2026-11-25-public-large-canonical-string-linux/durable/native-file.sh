#!/usr/bin/env bash
# Controller-side link and structural qualification, after the released component gate.
set -euo pipefail
[[ "$(id -u)" == 0 && "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ $# == 4 && "$1" =~ ^(link|export)$ && "$2" =~ ^[0-9a-f]{12}$ && "$3" =~ ^[0-9a-f]{64}$ && "$4" =~ ^[0-9a-f]{64}$ ]]
base=/var/lib/durable-jobs-bench/native
tools="$base/toolchain"
id=public-large-canonical-string-linux-"$2"
stage="$base/source-$id"
component="$base/runs/$id-component"
home="$base/source-$id-file"
out="$base/runs/$id-file-qualification"
product="$base/builds/trusted-jobs-product-623c46b92cc5"
compiler="$base/builds/mesh-stock-3828f20d/abi/meshc"
original="$base/source-public-large-file-session-resume/broker"
export PATH="$tools/rust-1.99.0/bin:$tools/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin" LC_ALL=C.UTF-8
export LLVM_SYS_211_PREFIX="$tools/llvm-21.1.8" CARGO_HOME="$base/cargo-home"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
umask 022
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base")" == 8:0 && "$(getconf CLK_TCK)" == 100 ]]
printf '%s  %s\n' "$3" "$component/summary.json" | sha256sum -c >&2
if [[ "$1" == export ]]; then
    printf '%s  %s\n' "$4" "$out/qualified.json" | sha256sum -c >&2
    exec tar -C "$out" -czf - .
fi
(cd "$stage"; sha256sum -c SHA256SUMS >&2)
python3 -B - "$component" <<'PY'
import json
from pathlib import Path
import sys
p = Path(sys.argv[1])
assert json.loads((p / "summary.json").read_text())["profiles"]["plain"]["earned"] is True
PY
mkdir "$home" "$out" "$home/bin" "$home/tmp"
trap 'rm -rf -- "$home"/work-stock "$home"/work-candidate "$home/cargo" "$home/tmp" "$home/incoming.tar.gz"' EXIT
cat > "$home/incoming.tar.gz"
printf '%s  %s\n' "$4" "$home/incoming.tar.gz" | sha256sum -c >&2
tar -xzf "$home/incoming.tar.gz" -C "$home"
(cd "$home"; sha256sum -c SHA256SUMS >&2)
mkdir "$home/session"
tar -xzf "$home/session-stage.tar.gz" -C "$home/session"
(cd "$home/session"; sha256sum -c SHA256SUMS >&2)
printf '%s  %s\n'   e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19 "$compiler"   b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$tools/pgsql-17.11/bin/postgres"   a5a2d5667c2d7209234d7f1468c40dd4071818010be31364a4e82d04b7ff18a3 "$original" | sha256sum -c >&2
python3 -B - "$home/file-inventory.json" <<'PY' | sha256sum -c >&2
import json
from pathlib import Path
import sys
doc = json.loads(Path(sys.argv[1]).read_text())
for path, sha in doc["engines"]["mesh"]["verified_files"].items():
    print(sha, path)
PY
sha256sum -c "$home/toolchain.sha256" >&2
export TMPDIR="$home/tmp" CARGO_TARGET_DIR="$home/cargo"
rustc --version | tee "$out/rustc.txt"
grep -q '^rustc 1.99.0 (b940084d7 ' "$out/rustc.txt"
[[ "$(llvm-config --version)" == 21.1.8 ]]
for lane in stock candidate; do
    mkdir "$home/work-$lane" "$home/work-$lane/broker"
    tar -xzf "$stage/public-source.tar.gz" -C "$home/work-$lane"
    tree="$home/work-$lane/source"
    (cd "$tree"; sha256sum -c "$stage/public-preimage.sha256")
    if [[ "$lane" == candidate ]]; then
        (cd "$tree"; patch --batch --fuzz=0 -p1 < "$stage/candidate.patch"; sha256sum -c "$stage/candidate-store.sha256")
    fi
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$tree/scripts/ci-x86_64-linux-gnu-clang"
    (cd "$tree"; cargo clean --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt
        cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt
    ) > "$out/$lane-build.log" 2>&1
    runtime="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libmesh_rt.a"
    sha256sum "$runtime" > "$out/$lane-runtime.txt"
    if [[ "$lane" == stock ]]; then
        grep -q '^4e81280d27d218a3c1eaba8b10b33455f06280ca2297b48ad0563ddf96873584 ' "$out/$lane-runtime.txt"
    fi
    stage_broker="$home/work-$lane/broker"
    cp "$product"/{mesh.toml,libworker_identity.a,worker_identity.c,worker_identity.mpl,writes.mpl} "$stage_broker/"
    cp "$home/session/broker-main.mpl" "$stage_broker/main.mpl"
    MESH_RT_LIB_PATH="$runtime" "$compiler" build "$stage_broker" --opt-level 2 --output "$home/bin/$lane" > "$out/$lane-link.log" 2>&1
    chmod a-w "$home/bin/$lane"
    readelf -h -n "$home/bin/$lane" > "$out/$lane-elf.txt"
done
if cmp -s "$home/bin/stock" "$home/bin/candidate"; then exit 1; fi
sha256sum "$home/bin/"* > "$out/brokers.sha256"
flock -u 9
exec 9<&-
python3 -B - "$home" "$out" "$original" "$3" <<'PY'
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

home, out, original = map(Path, sys.argv[1:4])
inventory = json.loads((home / "file-inventory.json").read_text())
launcher = inventory["engines"]["mesh"]["argv"]
bindings = {}
for label, exe in (("exact-file", original), ("stock", home / "bin/stock"), ("candidate", home / "bin/candidate")):
    with exe.open("rb") as source:
        sha = hashlib.file_digest(source, "sha256").hexdigest()
    args = [f"PRODUCT_SESSION_EXECUTABLE={exe}" if item.startswith("PRODUCT_SESSION_EXECUTABLE=")
            else f"PRODUCT_SESSION_SHA256={sha}" if item.startswith("PRODUCT_SESSION_SHA256=")
            else item for item in launcher]
    folder = out / label
    folder.mkdir()
    with (out / f"{label}.log").open("xb") as log:
        subprocess.run(["taskset", "-c", "0,1", *args, "large", folder], stdout=log, stderr=subprocess.STDOUT,
                       check=True, env=dict(os.environ, DURABLE_JOBS_PHASE="diagnostic"))
    bindings[label] = {"path":str(exe),"sha256":sha,"argv":args}
(out / "qualified.json").write_text(json.dumps({"screen_sha256":sys.argv[4],"bindings":bindings},
                                               sort_keys=True, indent=2) + "\n")
PY
sha256sum "$out/qualified.json"
