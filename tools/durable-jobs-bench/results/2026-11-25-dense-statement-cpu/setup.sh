#!/usr/bin/env bash
# Called only by the sole controller while it holds the checkout timing fence.
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
pg="$base/toolchain/pgsql-17.11"
dest="$base/dense-statement-cpu-20261125"
test "$(id -u)" = 0
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base" | tr -d '[:space:]')" == 8:0 ]]
[[ "$(nproc --all)" == 4 && "$(taskset -c 0,1 nproc)" == 2 && "$(getconf CLK_TCK)" == 100 ]]
test "$(sha256sum "$pg/bin/postgres" | cut -d ' ' -f 1)" = b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1
test "$("$pg/bin/pg_config" --version)" = 'PostgreSQL 17.11'
test ! -e "$dest" && test ! -L "$dest"
private="$(mktemp -d "$base/.dense-statement-cpu.XXXXXXXX")"
work="$(mktemp -d /var/tmp/dense-cpu-setup.XXXXXXXX)"
cleanup() {
    # Any canary interrupted before its finally block needs a clean stop first.
    for control in "$work"/smoke-*/*/pgdata/postmaster.pid; do
        if [[ -f "$control" ]]; then
            runuser -u nobody -- "$pg/bin/pg_ctl" -D "${control%/*}" -m immediate -w stop >&2 || return
        fi
    done
    rm -rf -- "$work" "$private"
}
trap cleanup EXIT
chmod 0755 "$private"
chown nobody:nogroup "$work"
tar -xf - -C "$private" --no-same-owner --no-same-permissions
(cd "$private" && sha256sum -c input.sha256 >&2)
cc -O2 -D_GNU_SOURCE -fPIC -shared -Wall -Wextra -Werror -Wno-unused-parameter \
    -I "$("$pg/bin/pg_config" --includedir-server)" \
    -I "$("$pg/bin/pg_config" --includedir)/internal" \
    "$private/stmt_cpu.c" -o "$private/stmt_cpu.so"
chmod 0444 "$private"/*
taskset -c 0,1 runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    python3 -B "$private/smoke.py" "$pg/bin" "$private/stmt_cpu.so" "$work" > "$work/smoke.json"
# The same private artifact is used in every arm.
chmod 0555 "$private"
mv -T -- "$private" "$dest"
python3 -B - "$dest" "$work/smoke.json" <<'PY'
from hashlib import sha256
import json
from pathlib import Path
import sys
path, smoke = map(Path, sys.argv[1:])
print(json.dumps(dict(remote=str(path), version="17.11", platform="linux-x86_64",
                      files={p.name: sha256(p.read_bytes()).hexdigest() for p in sorted(path.iterdir())},
                      smoke=json.loads(smoke.read_text()))))
PY
