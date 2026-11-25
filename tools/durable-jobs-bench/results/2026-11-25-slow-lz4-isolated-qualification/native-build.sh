#!/bin/bash
# Sole controller, authenticated root entry point. Compilation and PostgreSQL are nonroot.
set -euo pipefail
umask 077
export PATH=/usr/bin:/bin LC_ALL=C.UTF-8
[[ $# == 1 && "$1" =~ ^[0-9a-f]{64}$ && $(id -u) == 0 && $(uname -m) == x86_64 ]]
base=/var/lib/durable-jobs-bench/native
stock="$base/toolchain/pgsql-17.11"
root="$base/slow-lz4-isolated-${1:0:16}"
work="$root/work"
stage="$root/stage"
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ $(df -Pk "$base" | awk 'END {print $4}') -ge 3145728 ]]
mkdir -m 0711 "$root"
install -d -m 0700 -o nobody -g nogroup "$work"
mkdir -m 0755 "$stage"
cat > "$root/input.tar"
printf '%s  %s\n' "$1" "$root/input.tar" | sha256sum -c >&2
tar --no-same-owner -xf "$root/input.tar" -C "$stage"
# Capture only explicit reports. PostgreSQL cluster contents stay inside the private work tree.
reports="$root/reports"
mkdir "$reports"
finish() {
    code=$?
    trap - EXIT
    printf '%s\n' "$code" > "$reports/exit.txt"
    for report in toolchain.txt installed-abi.txt installed.sha256 codec.sha256; do
        if [[ -f "$work/$report" ]]; then
            cp "$work/$report" "$reports/$report"
        fi
    done
    if [[ -d "$work/bridge" ]]; then
        for report in bridge.txt logical.sha256 stock-version.txt stock-lz4-rejected.log \
                isolated-pglz-version.txt isolated-lz4-version.txt; do
            if [[ -f "$work/bridge/$report" ]]; then cp "$work/bridge/$report" "$reports/$report"; fi
        done
    fi
    tar -czf - -C "$reports" .
    exit "$code"
}
trap finish EXIT
(
    cd "$stock"
    sha256sum -c "$stage/stock.sha256"
    [[ "$(bin/postgres --version)" == 'postgres (PostgreSQL) 17.11' ]]
    [[ "$(bin/pg_config --configure)" == "'--prefix=$stock' '--with-openssl'" ]]
    bin/pg_config --version --configure --cc --cppflags --cflags --ldflags --libs
    readelf -h -d bin/postgres lib/pg_stat_statements.so
    ldd bin/postgres
    ldd lib/pg_stat_statements.so
) > "$reports/stock-readback.txt" 2>&1
printf '%s\n' "$(cat /proc/sys/kernel/random/boot_id)" "$(uname -r)" \
    "$(findmnt -n -o MAJ:MIN -T "$work")" "$(getconf CLK_TCK)" > "$reports/host.txt"
findmnt -J -o SOURCE,TARGET,FSTYPE,MAJ:MIN -T "$work" > "$reports/mount.json"
runuser -u nobody -- env -i PATH=/usr/bin:/bin bash "$stage/build-pg.sh" "$stage" "$work" 8<&- 9<&- \
    > "$reports/build.log" 2>&1
prefix="$work/pgsql-17.11-lz4-537512904744"
libraries() {
    ldd "$1" | awk '/=> / { print $1, $3 }' | sort
}
for file in bin/postgres lib/pg_stat_statements.so; do
    name="${file##*/}"
    libraries "$stock/$file" > "$reports/$name.stock.libraries.txt"
    libraries "$prefix/$file" > "$reports/$name.isolated.libraries.txt"
    cmp "$reports/$name.stock.libraries.txt" "$reports/$name.isolated.libraries.txt"
    awk '$2 ~ /^\// { print $2 }' "$reports/$name.stock.libraries.txt" |
        xargs -r sha256sum > "$reports/$name.library-digests.txt"
done
runuser -u nobody -- env -i PATH=/usr/bin:/bin "TMPDIR=$work/tmp" \
    bash "$stage/bridge.sh" "$stock" "$prefix" "$work/bridge" 8<&- 9<&- > "$reports/bridge.log" 2>&1
runuser -u nobody -- mkdir -m 0700 "$work/scratch" "$work/river" "$work/apalis"
(cd "$stock" && sha256sum -c "$stage/stock.sha256") > "$reports/stock-post-build.txt"
