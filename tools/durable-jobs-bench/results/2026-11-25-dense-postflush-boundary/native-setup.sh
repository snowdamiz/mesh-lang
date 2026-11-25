#!/usr/bin/env bash
# Run only within the controller's setup booking. The input stage is hash-verified by the controller.
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
pg="$base/toolchain/pgsql-17.11/bin"
here=$(cd "$(dirname "$0")" && pwd -P)
[[ $# == 1 && "$1" =~ ^[0-9a-f]{12}$ && "$(id -u)" == 0 ]]
dest="$base/postflush-pg17-$1"
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base")" == 8:0 ]]
[[ "$(sha256sum "$pg/postgres" | cut -d ' ' -f1)" == b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 ]]
for option in includedir-server pkglibdir; do
    directory=$("$pg/pg_config" "--$option")
    [[ -d "$directory" && "$(realpath "$directory")" == "${pg%/bin}/"* ]]
done
test ! -e "$dest" && test ! -L "$dest"
work=$(mktemp -d /var/tmp/mesh-postflush.XXXXXXXX)
stage=
cleanup() {
    python3 -B -c 'import shutil, sys; [shutil.rmtree(p) for p in sys.argv[1:] if p]' "$work" "$stage"
}
trap cleanup EXIT
stage=$(mktemp -d "$base/.postflush-pg17.XXXXXXXX")
chmod 0755 "$work" "$stage"
install -d -m 0700 -o nobody -g nogroup "$work/run"
install -m 0444 "$here/"{canary.py,boundary.py} "$work"
bash "$here/build.sh" "$pg/pg_config" "$stage/postflush.so"
chmod 0444 "$stage/postflush.so"
taskset -c 0,1 runuser -u nobody -- env -i PATH=/usr/bin:/bin LC_ALL=C.UTF-8 \
    python3 -B "$work/canary.py" "$pg" "$stage/postflush.so" "$work/run" >"$work/canary.json"
chmod 0555 "$stage"
mv -T -- "$stage" "$dest"
stage=
python3 -B - "$pg/pg_config" "$dest/postflush.so" "$work/canary.json" <<'PY'
import hashlib
import json
from pathlib import Path
import subprocess
import sys

config, module, canary = map(Path, sys.argv[1:])
fields = ("version", "includedir-server", "pkglibdir", "bindir", "configure")
pg = {field: subprocess.check_output([config, f"--{field}"], text=True).strip() for field in fields}
print(json.dumps(dict(pg=pg, module=str(module), sha256=hashlib.sha256(module.read_bytes()).hexdigest(),
                      canary=json.loads(canary.read_text())), sort_keys=True))
PY
