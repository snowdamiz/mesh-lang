#!/usr/bin/env bash
# Install and prove the adapter with two already-qualified ELFs under SETUP.
set -euo pipefail
[[ $# == 5 && "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' && "$(id -u)" == 0 ]]
for sha in "$@"; do [[ "$sha" =~ ^[0-9a-f]{64}$ ]]; done
base=/var/lib/durable-jobs-bench/native
public=trusted-jobs-product-623c46b92cc5
candidate=short16-fused-refill-c4c5cc2243b6
stage="$base/slow-cap71-confirmed-refill-v1"
out="$base/runs/slow-cap71-confirmed-refill-v1"
pg="$base/toolchain/pgsql-17.11/bin"
frozen="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base")" == 8:0 && "$(getconf CLK_TCK)" == 100 ]]
umask 022
mkdir "$stage" "$out"
cat > "$out/adapter.tar.gz"
printf '%s  %s\n' "$1" "$out/adapter.tar.gz" "$2" "$base/builds/$candidate/mesh.json" \
    "$3" "$base/builds/$candidate/mesh" "$4" "$base/builds/$public/mesh.json" \
    "$5" "$base/builds/$public/mesh" \
    b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$pg/postgres" \
    d9f984dad988e2380b22e971ae5abf122619f246bf592a9402720d49996685ad "$frozen/tools/durable-jobs-bench/native/arm.py" \
    db8e121ab9b1f6aed2293b26c79a4fbfa8ac8aba6ff228be54b360bd32048a8e "$frozen/proof/jobs-parity/linux-regression/trial.py" \
    09aad1d1c6cb5c0d46408bf314879812bb24fa504cf26a00e728b5f21ad1c5c8 "$frozen/scripts/jobs-parity.py" \
    | sha256sum -c | tee "$out/pins.txt"
tar -xzf "$out/adapter.tar.gz" --no-same-owner -C "$stage"
chmod -R a-w "$stage"
mkdir -m 0700 "$out/proof"
chown nobody:nogroup "$out/proof"
for pair in public candidate; do
    if [[ "$pair" == public ]]; then attempt="$public"; else attempt="$candidate"; fi
    runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
        "PG_BIN_DIR=$pg" "PRODUCT_FROZEN_HARNESS=$frozen" "PRODUCT_SOURCE=$base/source-$attempt/source" \
        taskset -c 0,1 python3 -B "$stage/proof.py" "$out/proof/$pair" \
        "$base/builds/$attempt/mesh" "$base/builds/$public/mesh" > "$out/$pair.log" 2>&1
done
touch "$out/SETUP-PASS"
