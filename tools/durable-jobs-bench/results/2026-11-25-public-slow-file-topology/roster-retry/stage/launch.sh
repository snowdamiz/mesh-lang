#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
base=/var/lib/durable-jobs-bench/native
source="$base/source-trusted-jobs-product-623c46b92cc5/source"
artifact="$base/builds/trusted-jobs-product-623c46b92cc5"
frozen="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
broker="$base/source-public-large-file-session-resume/broker"
runner_sha=af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355
broker_sha=a5a2d5667c2d7209234d7f1468c40dd4071818010be31364a4e82d04b7ff18a3
manifest_sha=979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55

mode="$1"
shape="$2"
shift 2
[[ "$#" == 3 && "$1" == mesh && "$2" == slow && "$3" == /* && -d "$3" && ! -L "$3" ]]
case "$mode:$shape" in inline:stock|inline:two-runner|file:stock|file:two-runner) ;; *) exit 2 ;; esac
preflight=0
if [[ "${PUBLIC_SLOW_PREFLIGHT:-}" == 1 && "$(id -u)" != 0 ]]; then
    preflight=1
else
    [[ "$(id -u)" == 0 && "$3" == "$here"/raw/*/adapter ]]
    [[ -z "$(find "$3" -mindepth 1 -maxdepth 1 -print -quit)" ]]
    printf '%s  %s\n' "$runner_sha" "$artifact/mesh" "$broker_sha" "$broker" | sha256sum -c >&2
    exec 9<"$base/native.lock"
    flock -n 9
    chmod 0644 "$here"/{backend.py,identity.py,slow.py,roster.py,launch.sh}
    chown nobody:nogroup "$3"
    chmod 0700 "$3"
fi
args=(env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8
    "PG_BIN_DIR=$base/toolchain/pgsql-17.11/bin" "NATIVE_JOBS_STATE=$base"
    "PRODUCT_SOURCE=$source" "PRODUCT_FROZEN_HARNESS=$frozen"
    PRODUCT_COMPOSITION=product-bulk PRODUCT_STORAGE=default "PRODUCT_TOPOLOGY=$shape"
    "PRODUCT_SESSION_MODE=$mode" "PRODUCT_SESSION_EXECUTABLE=$broker" "PRODUCT_SESSION_SHA256=$broker_sha"
    "PRODUCT_ELF_SHA256=$runner_sha" "PRODUCT_MANIFEST_SHA256=$manifest_sha"
    "NATIVE_JOBS_ARTIFACTS=$artifact" "DURABLE_JOBS_PHASE=${DURABLE_JOBS_PHASE:-diagnostic}"
    "PUBLIC_SLOW_PREFLIGHT=$preflight")
cd "$source"
if [[ "$preflight" == 1 ]]; then
    exec "${args[@]}" /usr/bin/python3 -B "$here/slow.py" "$@"
fi
exec /usr/sbin/runuser -u nobody -- "${args[@]}" /usr/bin/python3 -B "$here/slow.py" "$@"
