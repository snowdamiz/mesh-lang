#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
old="$1"
treatment="$2"
shift 2
source=/var/lib/durable-jobs-bench/native/source-trusted-jobs-product-deb2a3bc49fe/source
[[ "$old" == "$source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay/native-run" ]]
[[ "$#" == 3 && "$1" == mesh && "$2" == noop && "$3" == /* && -d "$3" && ! -L "$3" ]]
[[ "${JOBS_GROUPED_WRITER-}" == fixed8ms && ( "$treatment" == stock || "$treatment" == projected ) ]]
stage="$(mktemp -d /var/tmp/dense-enqueue-jsonb-path.XXXXXXXX)"
public=(product_arm.py backend.py identity.py)
private=(contract.py stock.sql candidate.sql overlay.py enqueue-stock.sql)
cleanup() {
    for name in "${public[@]}" "${private[@]}" factorial_arm.py native-run; do
        rm -f -- "$stage/$name"
    done
    rmdir -- "$stage"
}
trap cleanup EXIT
chmod 0755 "$stage"
for name in "${public[@]}"; do install -m 0644 "${old%/*}/$name" "$stage/$name"; done
for name in "${private[@]}"; do install -m 0644 "$here/$name" "$stage/$name"; done
python3 -B "$here/stage_arm.py" "$here/factorial_arm.py" "$treatment" > "$stage/factorial_arm.py"
chmod 0644 "$stage/factorial_arm.py"
python3 -B "$here/render.py" "$old" combined > "$stage/native-run"
timeout --signal=INT --kill-after=10s 85s bash "$stage/native-run" "$@"
