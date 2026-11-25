#!/bin/bash
# Logical and client compatibility using only disposable socket-only clusters.
set -euo pipefail
umask 077
export LC_ALL=C
[[ $# == 3 && $(id -u) != 0 && "$1" == /* && "$2" == /* && "$3" == /* ]]
old="$1"
new="$2"
work="$3"
mkdir "$work"
cd "$work"
# Use a short private socket directory for Darwin's Unix socket path limit.
# The controller and the local rehearsal supply a task-owned TMPDIR.
socket="$(mktemp -d "${TMPDIR:?}/bridge.XXXXXX")"
active=
stop() {
    if [[ -n "$active" && -f "$work/data/postmaster.pid" ]]; then
        "$active/bin/pg_ctl" -D "$work/data" -m fast -w stop
        active=
    fi
}
cleanup() {
    stop && rmdir "$socket"
}
trap cleanup EXIT
sql() {
    local client="$1"
    shift
    "$client/bin/psql" -X -qAt -v ON_ERROR_STOP=1 -h "$socket" -p 55437 \
        -U bridge_admin -d postgres "$@"
}
start() {
    local prefix="$1" codec="$2"
    "$prefix/bin/initdb" -D "$work/data" -U bridge_admin --auth=trust --no-sync > "$codec-initdb.log"
    printf "\nshared_preload_libraries = 'pg_stat_statements'\ntrack_commit_timestamp = on\ndefault_toast_compression = '%s'\n" "$codec" >> "$work/data/postgresql.conf"
    active="$prefix"
    "$prefix/bin/pg_ctl" -D "$work/data" -l "$work/postgres-$codec.log" -w start \
        -o "-c listen_addresses= -c unix_socket_directories=$socket -p 55437" >/dev/null
    sql "$prefix" -c 'CREATE EXTENSION pg_stat_statements; CREATE EXTENSION pgcrypto; CREATE EXTENSION hstore'
    [[ "$(sql "$prefix" -c "SELECT digest('abc', 'sha256') = sha256(convert_to('abc','UTF8'))
          AND ('a=>b'::hstore -> 'a') = 'b'")" == t ]]
    [[ "$(sql "$prefix" -c 'SHOW server_version_num')" == 170011 ]]
    [[ "$(sql "$prefix" -c 'SHOW default_toast_compression')" == "$codec" ]]
    sql "$prefix" -c 'SELECT version()'
    sql "$prefix" -c 'SELECT extversion FROM pg_extension WHERE extname = '"'"'pg_stat_statements'"'"''
}
start "$old" pglz > stock-version.txt
if [[ $(uname -s) == Linux ]]; then
    if sql "$old" -c "SET default_toast_compression = 'lz4'" > stock-lz4-rejected.log 2>&1; then
        exit 1
    fi
    grep -q 'invalid value for parameter "default_toast_compression": "lz4"' stock-lz4-rejected.log
fi
sql "$old" -c "CREATE TABLE bridge(payload text); INSERT INTO bridge VALUES (repeat('abcd', 4096))"
[[ "$(sql "$old" -c 'SELECT pg_column_compression(payload) FROM bridge')" == pglz ]]
sql "$old" -c 'COPY bridge TO STDOUT' > logical.tsv
stop
mv data stock-data
# Repeat with fresh clusters, loading the same logical data from the stock server.
for codec in pglz lz4; do
    start "$new" "$codec" > "isolated-$codec-version.txt"
    sql "$old" -c 'CREATE TABLE bridge(payload text)'
    sql "$old" -c 'COPY bridge FROM STDIN' < logical.tsv
    [[ "$(sql "$old" -c 'SELECT pg_column_compression(payload) FROM bridge')" == "$codec" ]]
    sql "$old" -c 'COPY bridge TO STDOUT' | cmp logical.tsv -
    [[ "$(sql "$old" -c 'SELECT count(*) > 0 FROM pg_stat_statements')" == t ]]
    stop
    mv data "isolated-$codec-data"
done
if [[ $(uname -s) == Linux ]]; then
    # Compare the compiler fingerprint reported by the old and private executable.
    cmp stock-version.txt isolated-pglz-version.txt
fi
shasum -a 256 logical.tsv > logical.sha256
printf 'stock logical export, both codec imports, stock psql round trips and pg_stat_statements, pgcrypto, and hstore passed\n' > bridge.txt
