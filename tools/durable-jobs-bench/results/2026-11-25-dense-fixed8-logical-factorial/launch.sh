#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
old="$1"
lane="$2"
shift 2
source=/var/lib/durable-jobs-bench/native/source-trusted-jobs-product-deb2a3bc49fe/source
[[ "$old" == "$source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay/native-run" ]]
[[ "$#" == 3 && "$1" == mesh && "$2" == noop && "$3" == /* && -d "$3" && ! -L "$3" ]]
case "$lane:${JOBS_GROUPED_WRITER-}" in
    smoke-stock:|stock-a:|stock-b:|key:|smoke-combined:fixed8ms|fixed8:fixed8ms|combined:fixed8ms) ;;
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
