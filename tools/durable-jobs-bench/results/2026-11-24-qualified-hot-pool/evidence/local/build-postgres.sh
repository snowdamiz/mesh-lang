#!/bin/bash
set -euo pipefail
lane="$(cd "$(dirname "$0")" && pwd -P)"
version="$1"
cd "$lane/postgres"
archive="postgresql-$version.tar.bz2"
case "$version" in
    18.7) sha=441dfae4280cae3e0425e5f876f80559fb0731c140f1aae22b3e5a54d0411bfd ;;
    19.0) sha=a89fd70efbad6a08c7dbed8d4872e3af30170d9e5ca9dc7906523be95b4e692d ;;
esac
printf '%s  %s\n' "$sha" "$archive" | shasum -a 256 -c
tar xjf "$archive"
cd "postgresql-$version"
ssl=/opt/homebrew/opt/openssl@3
CPPFLAGS="-I$ssl/include" LDFLAGS="-L$ssl/lib" ./configure --prefix="$lane/postgres/$version" --with-ssl=openssl --without-icu --without-readline --without-zlib
make -s -j 8
make -s install
for ext in pgcrypto pg_trgm pg_stat_statements; do
    make -s -C "contrib/$ext" -j 8 install
done
bin="$lane/postgres/$version/bin"
"$bin/postgres" --version
