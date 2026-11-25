#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
old="$1"
lane="$2"
module="$3"
shift 3
base=/var/lib/durable-jobs-bench/native
source="$base/source-trusted-jobs-product-623c46b92cc5/source"
[[ "$old" == "$source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay-closure-retry/native-run" ]]
case "$lane" in
    plan-stock|plan-key) [[ "$module" == "$base/dense-live-grant-plans-20261125/auto_explain.so" ]] ;;
    stock-a|stock-b|key) [[ "$module" == - ]] ;;
    *) exit 2 ;;
esac
[[ "$#" == 3 && "$1" == mesh && "$2" == noop && "$3" == /* && -d "$3" && ! -L "$3" ]]
stage="$(mktemp -d /var/tmp/dense-stock-logical-key.XXXXXXXX)"
public=(product_arm.py backend.py identity.py)
private=(stock_arm.py contract.py stock.sql candidate.sql stage.py observe.py settings.py plans.py launcher.py roster.py roster8.py)
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
python3 -B "$stage/stage.py" "$stage" "$old" "$lane" "$module"
timeout --signal=INT --kill-after=10s 85s bash "$stage/native-run" "$@"
