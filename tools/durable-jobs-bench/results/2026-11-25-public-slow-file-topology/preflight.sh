#!/usr/bin/env bash
# Exercise the actual staged Python and shell entry point as a nonroot Linux
# account. No PostgreSQL, broker, runner or benchmark is started.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
root="$(cd "$here/../../../.." && pwd -P)"
product="$here/../2026-11-24-trusted-jobs-product-composition"
frozen="$root/target/enqueue-spec-preparation/ordinary-v2"
manifest="$product/evidence/native/setup/trusted-jobs-product-623c46b92cc5/mesh.json"
source="$root/target/2026-11-24-trusted-jobs-product-composition/source"
image=sha256:3dc01f342e31ad8de39338d14ab12de0090c9102b34f75ca9774ff1f4f5e904e
docker run -i --rm --pull=never --network none --entrypoint /bin/bash \
    -v "$here/stage:/input:ro" -v "$frozen:/frozen:ro" -v "$source:/source:ro" \
    -v "$manifest:/manifest:ro" "$image" -se <<'DOCKER'
base=/var/lib/durable-jobs-bench/native
src="$base/source-trusted-jobs-product-623c46b92cc5"
frozen="$base/source-bulk-enqueue-3b09dc3d0e23"
artifact="$base/builds/trusted-jobs-product-623c46b92cc5"
stage=/var/tmp/mesh-durable-jobs-matrix/preflight
mkdir -p "$src" "$frozen" "$artifact" "$stage/raw/spec/adapter"
ln -s /frozen "$frozen/ordinary-v2"
ln -s /source "$src/source"
cp /manifest "$artifact/mesh.json"
cp /input/* "$stage/"
chmod 0711 "${stage%/*}" "$stage" "$stage/raw" "$stage/raw/spec"
chmod 0644 "$stage"/{backend.py,identity.py,slow.py,socket_map.py,launch.sh}
chown nobody:nogroup "$stage/raw/spec/adapter"
chmod 0700 "$stage/raw/spec/adapter"
cd "$stage/raw/spec/adapter"
for shape in stock two-runner; do
    for mode in inline file; do
        PUBLIC_SLOW_PREFLIGHT=1 runuser -u nobody -- /bin/bash "$stage/launch.sh" \
            "$mode" "$shape" mesh slow "$PWD"
    done
done
DOCKER
