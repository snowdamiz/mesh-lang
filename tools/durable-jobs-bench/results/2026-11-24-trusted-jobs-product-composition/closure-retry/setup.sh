#!/usr/bin/env bash
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
product=trusted-jobs-product-623c46b92cc5
source="$base/source-$product/source"
packet="$source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition"
old="$packet/replay"
stage="$packet/replay-closure-retry"
artifact="$base/builds/$product"
out="$base/runs/$product-closure-retry"
frozen="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2/tools/durable-jobs-bench/native/arm.py"
[[ $# == 4 && "$1" =~ ^[a-f0-9]{64}$ && "$2" =~ ^[a-f0-9]{64}$ &&
   "$3" =~ ^[a-f0-9]{64}$ && "$4" =~ ^[a-f0-9]{64}$ ]]
umask 022
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base")" == 8:0 ]]
mkdir "$out" "$stage"
printf '%s  %s\n' "$1" "$artifact/mesh" "$2" "$artifact/mesh.json" "$3" "$old/backend.py" \
   d9f984dad988e2380b22e971ae5abf122619f246bf592a9402720d49996685ad "$frozen" \
   b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$base/toolchain/pgsql-17.11/bin/postgres" \
   | sha256sum -c | tee "$out/pins.txt"
cp "$old/native-run" "$old/product_arm.py" "$old/identity.py" "$stage/"
cat > "$stage/backend.py"
printf '%s  %s\n' "$4" "$stage/backend.py" | sha256sum -c
chmod a-w "$stage" "$stage"/*
sha256sum "$stage"/{backend.py,identity.py,native-run,product_arm.py} > "$out/adapter.sha256"
cp "$artifact/mesh.json" "$out/mesh.json"
flock -u 9
exec 9<&-
for mode in product-bulk product-group; do
    mkdir "$out/$mode"
    env "PRODUCT_ARTIFACT=$artifact" "PRODUCT_COMPOSITION=$mode" "PRODUCT_TOPOLOGY=stock" \
        "PRODUCT_STORAGE=default" "PRODUCT_ELF_SHA256=$1" "PRODUCT_MANIFEST_SHA256=$2" \
        DURABLE_JOBS_PHASE=diagnostic taskset -c 0,1 "$stage/native-run" mesh short "$out/$mode" \
        > "$out/$mode.log" 2>&1
done
touch "$out/SETUP-PASS"
