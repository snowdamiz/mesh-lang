#!/usr/bin/env bash
# Executed as the unprivileged proof user inside the native SETUP fence.
set -euo pipefail
[[ $# == 2 && "$(id -u)" == 65534 ]]
binary="$1"
scratch="$2"
data="$scratch/late-capacity-postgres"
cleanup() {
    if pg_ctl -D "$data" status >/dev/null 2>&1; then
        pg_ctl -D "$data" -m fast -w stop
    fi
}
trap cleanup EXIT
initdb -D "$data" -A trust -U mesh --no-locale --encoding UTF8
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
pg_ctl -D "$data" -l "$scratch/late-capacity-postgres.log" \
    -o "-h 127.0.0.1 -p $port -c unix_socket_directories=''" -w start
export MESH_TEST_DATABASE_URL="postgresql://mesh@127.0.0.1:$port/postgres?sslmode=disable"
for mode in baseline grow shrink global-cap pause namespace-pause stop; do
    MESH_TEST_LATE_CAPACITY_CASE="$mode" "$binary" \
        postgres_grouped_receipts_free_seats_while_one_checked_claim_waits --ignored --nocapture
done
