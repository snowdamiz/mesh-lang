#!/usr/bin/env bash
# Remote argv for the sole authenticated capacity driver.
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
tools="$base/toolchain"
[[ $# -eq 3 && "$2" =~ ^[0-9a-f]{12}$ && "$3" =~ ^[0-9a-f]{64}$ ]]
[[ "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base")" == 8:0 && "$(getconf CLK_TCK)" == 100 ]]
umask 022
export PATH="$tools/rust-1.99.0/bin:$tools/llvm-21.1.8/bin:/usr/sbin:/usr/bin:/bin" LC_ALL=C.UTF-8
export LLVM_SYS_211_PREFIX="$tools/llvm-21.1.8" CARGO_HOME="$base/cargo-home"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
id=public-large-canonical-string-linux-"$2"
home="$base/source-$id"
out="$base/runs/$id-component"
case "$1" in
    screen)
        cd "$home"
        echo "$3  verify/qualification.json" | sha256sum -c >&2
        sha256sum -c SHA256SUMS >&2
        sha256sum -c verify/qualification.sha256 >&2
        cmp escaped-capture.json verify/escaped-capture.json
        python3 -B rotation.py run "$home" "$out"
        sha256sum "$out/summary.json"
        exit
        ;;
    export)
        # The last arg selects the digest of the exact qualification or screen summary.
        for folder in "$out" "$home/verify"; do
            [[ -d "$folder" ]] || continue
            selection=.
            if [[ "$folder" == "$home/verify" ]]; then
                summary=qualification.json
            else
                summary=summary.json
            fi
            if (cd "$folder" && echo "$3  $summary" | sha256sum --status -c); then
                exec tar -C "$folder" -czf - "$selection"
            fi
        done
        exit 1
        ;;
    qualify) ;;
    *) exit 2 ;;
esac

mkdir "$home" "$home/bin" "$home/verify" "$home/tmp"
cd "$home"
trap 'rm -rf -- "$home"/work-stock "$home"/work-move "$home"/work-candidate "$home/cargo" "$home/tmp" "$home/incoming.tar.gz"' EXIT
cat > incoming.tar.gz
echo "$3  incoming.tar.gz" | sha256sum -c >&2
tar -xzf incoming.tar.gz
sha256sum -c SHA256SUMS >&2
export TMPDIR="$home/tmp" CARGO_TARGET_DIR="$home/cargo"
rustc --version | tee verify/rustc.txt
llvm-config --version | tee verify/llvm.txt
grep -q '^rustc 1.99.0 (b940084d7 ' verify/rustc.txt
grep -Fxq 21.1.8 verify/llvm.txt
echo "b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1  $tools/pgsql-17.11/bin/postgres" | sha256sum -c >&2
python3 -B frames.py "$home"
for lane in stock move candidate; do
    mkdir "work-$lane"
    tar -xzf public-source.tar.gz -C "work-$lane"
    tree="$home/work-$lane/source"
    (cd "$tree"; sha256sum -c "$home/public-preimage.sha256")
    if [[ "$lane" != stock ]]; then
        (cd "$tree"; patch --batch --fuzz=0 -p1 < "$home/$lane.patch"; sha256sum -c "$home/$lane-store.sha256")
    fi
    # Keep an exact copy of the production source hash before attaching the probe.
    sha256sum "$tree/compiler/mesh-rt/src/jobs/store.rs" > "verify/$lane-production.sha256"
    python3 -B materialize.py "$tree" "$lane"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$tree/scripts/ci-x86_64-linux-gnu-clang"
    (
        cd "$tree"
        cargo clean --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt
        cargo tree --offline --locked -p mesh-rt -e features
        cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p mesh-rt --example public-large-probe
    ) > "verify/$lane-build.log" 2>&1
    cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/public-large-probe" "bin/$lane"
    readelf -h -n "bin/$lane" > "verify/$lane-elf.txt"
    chmod a-w "bin/$lane"
    for profile in plain escaped; do
        frame_dir="$profile"
        if [[ "$profile" == plain ]]; then frame_dir=large; fi
        "bin/$lane" verify "$home/frames/$frame_dir" "$home/tmp/$lane-$profile.bin" "$profile" > "verify/$lane-$profile.json"
        sha256sum "tmp/$lane-$profile.bin" >> verify/snapshots.sha256
        if [[ "$lane" != stock ]]; then
            cmp "tmp/stock-$profile.bin" "tmp/$lane-$profile.bin"
            cmp "verify/stock-$profile.json" "verify/$lane-$profile.json"
        fi
    done
done
cp escaped-capture.json verify/
mkdir "$home/tmp/wire-owner"
chmod 755 "$home/tmp"
wire_status=0
if [[ "$(id -u)" == 0 ]]; then
    chown nobody:nogroup "$home/tmp/wire-owner"
    runuser -u nobody -- python3 -B wire-check.py "$tools/pgsql-17.11/bin" "$home" "$home/tmp/wire-owner/run" || wire_status=$?
else
    python3 -B wire-check.py "$tools/pgsql-17.11/bin" "$home" "$home/tmp/wire-owner/run" || wire_status=$?
fi
for name in wire.json calls.json initdb.log postgres.log; do
    if [[ -f "$home/tmp/wire-owner/run/$name" ]]; then
        cp "$home/tmp/wire-owner/run/$name" verify/
    fi
done
(( wire_status == 0 ))
python3 -B - "$3" <<'PY'
import json
from pathlib import Path
import sys
from original_rotation import digest, write
profiles = ("plain", "escaped")
verify = Path("verify")
semantic = {p:json.loads((verify / f"stock-{p}.json").read_text()) for p in profiles}
assert all(row["jobs"] == row["frames"] == 64 for row in semantic.values())
assert semantic["plain"]["output_sha256"] == "c98a0cd9500d920dfb07b040ffc7a90eb2628f82681eed196f2ea987db376e18"
assert semantic["plain"]["cases"] >= 190
assert semantic["escaped"]["output_sha256"] != semantic["plain"]["output_sha256"]
pins = {lane:{"binary_sha256":digest(Path("bin") / lane),
              "store_sha256":(verify / f"{lane}-production.sha256").read_text().split()[0]}
        for lane in ("stock", "move", "candidate")}
assert len({row["binary_sha256"] for row in pins.values()}) == len(pins)
write(verify / "semantic.json", semantic)
write(verify / "build-pins.json", pins)
files = {p.name:digest(p) for p in sorted(verify.iterdir())}
write(verify / "qualification.json", {"capsule_sha256":sys.argv[1],"files":files,"passed":True})
print("Qualification result", digest(verify / "qualification.json"))
PY
sha256sum bin/{stock,move,candidate} verify/* | tee verify/qualification.sha256
