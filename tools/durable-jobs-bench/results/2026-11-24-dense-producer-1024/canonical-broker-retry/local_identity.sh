#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
root="$(cd "$here/../../../../.." && pwd -P)"
stage=/var/lib/durable-jobs-bench/native/source-dense-producer-1024-09b9395cf5c6
frozen=/var/lib/durable-jobs-bench/native/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2
lane="$root/target/dense-producer-1024"
scratch="$(mktemp -d "$lane/canonical-path.XXXXXX")"
trap 'rm -r -- "$scratch"' EXIT
tar xf "$here/../../2026-11-23-set-based-enqueue/native/bulk-enqueue-ce5e551d079e.tar.gz" -C "$scratch" ordinary-v2
docker run --rm --pull never --network none --read-only --cpuset-cpus 0,1 \
    --tmpfs "$stage:rw,exec,mode=755" \
    --mount "type=bind,source=$here,destination=/recovery,readonly" \
    --mount "type=bind,source=$here/..,destination=/original,readonly" \
    --mount "type=bind,source=$scratch/ordinary-v2,destination=$frozen,readonly" \
    -e "DENSE_STAGE=$stage" -e "PRODUCT_FROZEN_HARNESS=$frozen" \
    3dc01f342e31ad8de39338d14ab12de0090c9102b34f75ca9774ff1f4f5e904e \
    sh -ec '
        mkdir "$DENSE_STAGE/packet-canonical-broker"
        cp /recovery/adapter.py /original/backend.py /original/identity.py \
            "$DENSE_STAGE/packet-canonical-broker/"
        exec python3 -B /recovery/identity_probe.py
    '
