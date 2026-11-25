#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
old="${1:?pinned public launcher}"
phase="${2:?pilot phase}"
shift 2
base=/var/lib/durable-jobs-bench/native
source="$base/source-trusted-jobs-product-623c46b92cc5/source"
[[ "$old" == "$source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay-closure-retry/native-run" ]]
case "$phase" in gate-stock|stock|candidate) ;; *) exit 2 ;; esac
[[ "$#" == 3 && "$1" == mesh && "$2" == noop && "$3" == /* && ! -L "$3" ]]
[[ "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base")" == 8:0 ]]
stage="$(mktemp -d /var/tmp/empty-recovery-retry.XXXXXXXX)"
trap 'rm -f -- "$stage"/{native-run,product_arm.py,backend.py,identity.py,observe.py,contract.py,probe.py,candidate.sql,prepare-launch.py}; rmdir -- "$stage"' EXIT
chmod 0755 "$stage"
for name in product_arm.py backend.py identity.py; do
    install -m 0644 "${old%/*}/$name" "$stage/$name"
done
for name in observe.py contract.py probe.py candidate.sql prepare-launch.py; do
    install -m 0644 "$here/$name" "$stage/$name"
done
python3 -B "$stage/prepare-launch.py" "$old" "$phase"
bash "$stage/native-run" "$@"
