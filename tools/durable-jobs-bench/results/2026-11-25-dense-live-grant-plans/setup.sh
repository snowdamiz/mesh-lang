#!/usr/bin/env bash
# The sole controller sends this script while holding the workstation timing fence.
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
pg="$base/toolchain/pgsql-17.11"
destination="$base/dense-live-grant-plans-20261125"
test "$(id -u)" = 0
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base")" == 8:0 ]]
[[ "$(taskset -c 0,1 nproc)" == 2 && "$(getconf CLK_TCK)" == 100 ]]
test "$(sha256sum "$pg/bin/postgres" | cut -d ' ' -f1)" = b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1
test "$("$pg/bin/pg_config" --version)" = "PostgreSQL 17.11"
test ! -e "$destination" && test ! -L "$destination"
work="$(mktemp -d /var/tmp/dense-plan-setup.XXXXXXXX)"
private="$(mktemp -d "$base/.dense-live-grant-plans.XXXXXXXX")"
published=no
cleanup() {
    if [[ -f "$work/pg/pg/postmaster.pid" ]]; then
        runuser -u nobody -- "$pg/bin/pg_ctl" -D "$work/pg/pg" -m immediate -w stop >&2 || return
    fi
    python3 -B - "$work" "$private" "$published" <<'PY'
import shutil
import sys
shutil.rmtree(sys.argv[1])
if sys.argv[3] == "no":
    shutil.rmtree(sys.argv[2])
PY
}
trap cleanup EXIT
chmod 0755 "$work" "$private"
mkdir "$work/src"
chmod 0755 "$work/src"
tar -xf - -C "$work/src" --no-same-owner --no-same-permissions
test "$(sha256sum "$work/src/auto_explain.c" | cut -d ' ' -f1)" = ec87b02dab5e42e465d916f724666ed59e2dce9154a3b3cff8518feecb739416
lib="$("$pg/bin/pg_config" --pkglibdir)"
printf 'PG pkglibdir: %s\n' "$lib" >&2
find "$pg" -name 'auto_explain.so' -type f -print >&2
origin=official-17.11-source
for candidate in "$lib/auto_explain.so" "$pg/lib/auto_explain.so" "$pg/lib/postgresql/auto_explain.so"; do
    if [[ -f "$candidate" && "$(realpath "$candidate")" == "$pg/"* ]]; then
        origin="$(realpath "$candidate")"
        install -m 0444 "$origin" "$private/auto_explain.so"
        break
    fi
done
if [[ "$origin" == official-17.11-source ]]; then
    cc -O2 -D_GNU_SOURCE -fPIC -shared \
        -I "$("$pg/bin/pg_config" --includedir-server)" \
        -I "$("$pg/bin/pg_config" --includedir)/internal" \
        "$work/src/auto_explain.c" -o "$private/auto_explain.so"
    chmod 0444 "$private/auto_explain.so"
fi
install -d -m 0700 -o nobody -g nogroup "$work/pg"
taskset -c 0,1 runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    python3 -B "$work/src/canary.py" "$pg/bin" "$private/auto_explain.so" "$work/pg" > "$work/canary.json"
chmod 0555 "$private"
mv -T -- "$private" "$destination"
published=yes
python3 -B - "$origin" "$lib" "$destination/auto_explain.so" "$work/canary.json" <<'PY'
from pathlib import Path
import hashlib, json, sys
origin, pkglibdir, module, receipt = sys.argv[1:]
print(json.dumps(dict(origin=origin, pkglibdir=pkglibdir, remote=module,
                      sha256=hashlib.sha256(Path(module).read_bytes()).hexdigest(),
                      canary=json.loads(Path(receipt).read_text())), indent=2))
PY
