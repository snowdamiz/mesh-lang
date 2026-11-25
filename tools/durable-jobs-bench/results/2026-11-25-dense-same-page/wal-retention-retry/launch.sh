#!/usr/bin/env bash
# Native arm wrapper. The original launcher still owns the native lock and UID drop.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
old="$1"
lane="$2"
shift 2
base=/var/lib/durable-jobs-bench/native
source="$base/source-trusted-jobs-product-623c46b92cc5/source"
[[ "$old" == "$source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay-closure-retry/native-run" ]]
case "$lane" in default-a|fill50|default-b) ;; *) exit 2 ;; esac
[[ "$#" == 3 && "$1" == mesh && "$2" == noop && "$3" == /* && ! -L "$3" ]]
stage="$(mktemp -d /var/tmp/dense-same-page.XXXXXXXX)"
trap 'rm -f -- "$stage"/{native-run,product_arm.py,backend.py,identity.py,observe.py,layout.py,wal.py,wal_window.py,launcher.py,enqueue_schema.sql}; rmdir -- "$stage"' EXIT
chmod 0711 "$stage"
for name in product_arm.py backend.py identity.py; do
    install -m 0644 "$(dirname "$old")/$name" "$stage/$name"
done
for name in observe.py layout.py wal.py wal_window.py launcher.py; do
    install -m 0644 "$here/$name" "$stage/$name"
done
install -m 0644 "$source/compiler/mesh-rt/src/jobs/enqueue_schema.sql" "$stage/enqueue_schema.sql"
# Python's imports need directory enumeration under the dropped UID.
chmod 0755 "$stage"
python3 -B - "$stage" "$old" "$stage/native-run" "$lane" <<'PY'
from pathlib import Path
import sys

here, old, new = map(Path, sys.argv[1:4])
sys.path.insert(0, str(here))
from launcher import rewrite
new.write_text(rewrite(old.read_text(), old.parents[5], sys.argv[4]))
PY
bash "$stage/native-run" "$@"
