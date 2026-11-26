#!/usr/bin/env bash
set -euo pipefail
exe=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
scratch="$2"
pg="$3"
mkdir -m 0700 "$scratch"
scratch=$(cd "$scratch" && pwd)
trap 'if "$pg/pg_ctl" -D "$scratch/data" status >/dev/null 2>&1; then "$pg/pg_ctl" -D "$scratch/data" -m immediate -w stop; fi' EXIT
"$pg/initdb" -D "$scratch/data" -A trust --no-locale -U proof > "$scratch/initdb.log"
# This loopback cluster fails closed if another process wins the ephemeral port.
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
"$pg/pg_ctl" -D "$scratch/data" -l "$scratch/postgres.log" -o "-h 127.0.0.1 -p $port -c unix_socket_directories=''" -w start
for db in queue app_a app_b; do
    "$pg/createdb" -U proof -h 127.0.0.1 -p "$port" "$db"
done
export PROOF_QUEUE="postgresql://proof@127.0.0.1:$port/queue?sslmode=disable"
export PROOF_APP_A="postgresql://proof@127.0.0.1:$port/app_a?sslmode=disable"
export PROOF_APP_B="postgresql://proof@127.0.0.1:$port/app_b?sslmode=disable"
"$exe"
