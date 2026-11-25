#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
old="$1"
lane="$2"
shift 2
[[ "$old" =~ ^/var/lib/durable-jobs-bench/native/source-trusted-jobs-product-[a-f0-9]{12}/source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay/native-run$ ]]
[[ "$#" == 3 && "$1" == mesh && "$2" == noop && "$3" == /* && -d "$3" && ! -L "$3" ]]
case "$lane:${JOBS_GROUPED_WRITER-}" in
    bridge:fixed8ms|off-a:fixed8ms|off-b:fixed8ms|positive:fused_fixed8ms|fused:fused_fixed8ms) ;;
    *) exit 2 ;;
esac
stage="$(mktemp -d /var/tmp/dense-fixed8-logical.XXXXXXXX)"
public=(product_arm.py backend.py identity.py)
private=(factorial_arm.py contract.py stock.sql candidate.sql)
cleanup() {
    for name in "${public[@]}" "${private[@]}" native-run; do
        rm -f -- "$stage/$name"
    done
    rmdir -- "$stage"
}
trap cleanup EXIT
chmod 0755 "$stage"
for name in "${public[@]}"; do install -m 0644 "${old%/*}/$name" "$stage/$name"; done
for name in "${private[@]}"; do install -m 0644 "$here/$name" "$stage/$name"; done
python3 -B "$here/render.py" "$old" "$lane" > "$stage/native-run"
timeout --signal=INT --kill-after=10s 85s bash "$stage/native-run" "$@"
