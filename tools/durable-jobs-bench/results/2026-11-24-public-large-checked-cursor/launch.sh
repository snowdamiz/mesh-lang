#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
old="${1:?sealed product launch path}"
mode="${2:?diagnostic lane}"
shift 2
case "$mode" in trace|thin) ;; *) exit 2 ;; esac
[[ "$old" == /var/lib/durable-jobs-bench/native/source-trusted-jobs-product-623c46b92cc5/source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay-closure-retry/native-run ]]
[[ "$#" == 3 && "$1" == mesh && "$2" == large && "$3" == /* && ! -L "$3" ]]
stage="$(mktemp -d /var/tmp/public-large-cursor.XXXXXXXX)"
trap 'rm -f -- "$stage"/{observe.py,diagnostic.py,product_arm.py,backend.py,identity.py,native-run}; rmdir -- "$stage"' EXIT
chmod 0755 "$stage"
for name in product_arm.py backend.py identity.py; do
    install -m 0644 "${old%/*}/$name" "$stage/$name"
done
for name in observe.py diagnostic.py; do
    install -m 0644 "$here/$name" "$stage/$name"
done
python3 -B - "$old" "$stage/native-run" "$mode" <<'PY'
from pathlib import Path
import sys

old, new, mode = sys.argv[1:]
old, new = Path(old), Path(new)
source = old.read_text()
for before, after, count in (
    ('source="$(cd "$here/../../../../.." && pwd -P)"', f'source="{old.parents[5]}"', 1),
    ('"$here/product_arm.py" "$@"', f'"$here/observe.py" {mode} "$@"', 2),
):
    assert source.count(before) == count, before
    source = source.replace(before, after)
new.write_text(source)
PY
bash "$stage/native-run" "$@"
