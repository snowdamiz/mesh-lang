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
    off-a|off-b) [[ "$module" == - ]] ;;
    sampled) [[ "$module" == "$base/dense-live-grant-plans-20261125/auto_explain.so" ]] ;;
    *) exit 2 ;;
esac
[[ "$#" == 3 && "$1" == mesh && "$2" == noop && "$3" == /* && ! -L "$3" ]]
stage="$(mktemp -d /var/tmp/dense-live-grant-plans.XXXXXXXX)"
trap 'rm -f -- "$stage"/{observe.py,settings.py,plans.py,launcher.py,adapt.py,roster.py,product_arm.py,backend.py,identity.py,native-run}; rmdir -- "$stage"' EXIT
chmod 0755 "$stage"
for name in product_arm.py backend.py identity.py; do
    install -m 0644 "${old%/*}/$name" "$stage/$name"
done
for name in observe.py settings.py plans.py launcher.py adapt.py roster.py; do
    install -m 0644 "$here/$name" "$stage/$name"
done
python3 -B - "$stage" "$old" "$lane" "$module" <<'PY'
from pathlib import Path
import sys

stage, old = map(Path, sys.argv[1:3])
sys.path.insert(0, str(stage))
from adapt import rewrite_observer
from launcher import rewrite
observer = stage / "observe.py"
observer.write_text(rewrite_observer(observer.read_text()))
(stage / "native-run").write_text(rewrite(old.read_text(), old.parents[5], *sys.argv[3:]))
PY
timeout --signal=INT --kill-after=10s 85s bash "$stage/native-run" "$@"
