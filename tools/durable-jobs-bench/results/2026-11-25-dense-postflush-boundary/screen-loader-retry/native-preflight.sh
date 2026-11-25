#!/usr/bin/env bash
# Controller correctness preflight. The retained module and the frozen runner stay read-only.
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
source="$base/source-ordinary-trusted-v1-23fdb3f5530d"
pg="$base/toolchain/pgsql-17.11/bin"
matrix=/var/tmp/mesh-durable-jobs-matrix
module="$base/postflush-pg17-65af9b85ab67/postflush.so"
module_sha=fe0d7eb8935483f2a3002aeaecd8584dc1e65ac759ddb8f4cd1a0c29e9aabccf
here=$(cd "$(dirname "$0")" && pwd -P)
[[ $# == 1 && "$1" =~ ^(off-a|off-b|on)$ && "$here" =~ ^"$matrix"/postflush-[0-9a-f]{12}-$1$ ]]
[[ "$(id -u)" == 0 ]]
exec 8<"$matrix/active-arm.lock"
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ "$(uname -smr)" == 'Linux 6.8.0-149-generic x86_64' ]]
[[ "$(cat /proc/sys/kernel/random/boot_id)" == 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base")" == 8:0 ]]
[[ "$(sha256sum "$pg/postgres" | cut -d ' ' -f1)" == b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 ]]
[[ -f "$module" && ! -L "$module" && "$(sha256sum "$module" | cut -d ' ' -f1)" == "$module_sha" ]]
chmod 0711 "$matrix" "$here"
chmod 0644 "$here/"{verify_stage.py,arm_overlay.py,boundary.py,native-arm.py,topology_arm.py,identity.py}
cd "$source/tools/durable-jobs-bench/native"
work=$(mktemp -d "$base/scratch/postflush-stage.XXXXXXXX")
cleanup() {
    # Hold scratch if PostgreSQL failed to stop.
    if [[ -n "$(find "$work" -name postmaster.pid -print -quit)" ]]; then
        printf 'Retained private scratch at %s\n' "$work" >&2
        exit 1
    fi
    /usr/bin/python3 -B -c 'import shutil, sys; shutil.rmtree(sys.argv[1])' "$work"
}
trap cleanup EXIT
chown nobody:nogroup "$work"
mode=off
if [[ "$1" == on ]]; then mode=on; fi
taskset -c 0,1 /usr/bin/timeout -k 5 34 /usr/sbin/runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    "PG_BIN_DIR=$pg" /usr/bin/python3 -B "$here/verify_stage.py" activation "$source" "$module" "$module_sha" "$mode" "$work"
test -z "$(find "$here" -name __pycache__ -print -quit)"
