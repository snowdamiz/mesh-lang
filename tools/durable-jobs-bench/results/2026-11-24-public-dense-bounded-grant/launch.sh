#!/usr/bin/env bash
# Invoked only as the adapter for an assigned native matrix arm.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
old="${1:?sealed product launcher}"
mode="${2:?grant experiment mode}"
shift 2
case "$mode" in control|candidate|qualify) ;; *) exit 2 ;; esac
[[ "$old" == /var/lib/durable-jobs-bench/native/source-trusted-jobs-product-623c46b92cc5/source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay-closure-retry/native-run ]]
[[ "$#" == 3 && "$1" == mesh && "$2" == noop && "$3" == /* && ! -L "$3" ]]
stage="$(mktemp -d /var/tmp/public-dense-bounded-grant.XXXXXXXX)"
trap 'rm -f -- "$stage"/{adapter.py,diagnostic.py,stock.sql,product_arm.py,backend.py,identity.py,native-run}; rmdir -- "$stage"' EXIT
chmod 0755 "$stage"
for name in product_arm.py backend.py identity.py; do
    install -m 0644 "${old%/*}/$name" "$stage/$name"
done
for name in adapter.py diagnostic.py stock.sql; do
    install -m 0644 "$here/$name" "$stage/$name"
done
python3 -B - "$old" "$stage/native-run" "$mode" <<'PY'
from pathlib import Path
import sys

old, new, mode = sys.argv[1:]
old, new = Path(old), Path(new)
source = old.read_text()
root = old.parents[5]
for before, after, count in (
    ('source="$(cd "$here/../../../../.." && pwd -P)"', f'source="{root}"', 1),
    ('"$here/product_arm.py" "$@"', f'"$here/adapter.py" {mode} "$@"', 2),
):
    assert source.count(before) == count, before
    source = source.replace(before, after)
new.write_text(source)
PY
bash "$stage/native-run" "$@"
