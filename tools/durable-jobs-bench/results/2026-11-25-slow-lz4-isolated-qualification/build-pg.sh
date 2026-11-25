#!/bin/bash
# Run as the unprivileged build owner, in a controller-owned private directory.
set -euo pipefail
umask 077
export LC_ALL=C
[[ $(id -u) != 0 && $# == 2 && "$1" == /* && "$2" == /* ]]
sources="$(cd "$1" && pwd -P)"
work="$(cd "$2" && pwd -P)"
[[ "$work" == "$2" && -O "$work" ]]
[[ $(df -Pk "$work" | awk 'END {print $4}') -ge 2097152 ]]
prefix="$work/pgsql-17.11-lz4-537512904744"
build="$work/build"
[[ ! -e "$prefix" && ! -L "$prefix" ]]
mkdir "$build" "$work/tmp"
export TMPDIR="$work/tmp" CONFIG_SITE=/dev/null
unset LD_LIBRARY_PATH DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH MAKEFLAGS
check() {
    printf '%s  %s\n' "$1" "$2" | shasum -a 256 -c
}
pg="$sources/postgresql-17.11.tar.bz2"
lz4="$sources/lz4-1.10.0.tar.gz"
check dd27f2b3c59e73ed14aa3324901242bf69a032a6347805f274e6260322d42979 "$pg"
check 537512904744b35e232912055ccf8ec66d768639ff3abe5788d90d792ec5f48b "$lz4"
case "$(uname -s)" in
    Linux)
        [[ $(uname -m) == x86_64 ]]
        export PATH=/usr/bin:/bin CC=/usr/bin/gcc
        unset CPPFLAGS CFLAGS CXX CXXFLAGS LDFLAGS LIBS PKG_CONFIG_PATH PKG_CONFIG_LIBDIR
        ;;
    Darwin)
        export CC=/usr/bin/clang
        ;;
    *) exit 2 ;;
esac
{
    uname -a
    id
    "$CC" --version
    "$CC" -dumpmachine
    make --version
    ld -v || true
    bison --version
    flex --version
    perl -v
    pkg-config --version
    openssl version -a
    pkg-config --modversion icu-uc icu-i18n openssl zlib
    if [[ $(uname -s) == Linux ]]; then
        getconf GNU_LIBC_VERSION
        sha256sum "$(readlink -f "$CC")" "$(readlink -f "$(command -v ld)")" \
            "$(readlink -f "$(command -v bison)")" "$(readlink -f "$(command -v flex)")" \
            "$(readlink -f "$(command -v pkg-config)")" "$("$CC" -print-prog-name=cc1)"
        dpkg-query -W build-essential gcc gcc-13 binutils libc6 libc6-dev make bison flex \
            perl pkg-config pkgconf libssl-dev libssl3t64 libicu-dev libicu74 zlib1g-dev \
            zlib1g libreadline-dev libreadline8t64 libncurses-dev libtinfo6
    fi
} > "$work/toolchain.txt" 2>&1
tar -xf "$pg" -C "$build"
tar -xf "$lz4" -C "$build"
codec="$build/lz4-1.10.0/lib"
# PostgreSQL's configure adds -llz4 itself. This directory contains only our static archive.
mkdir "$build/codec"
objects=()
for unit in lz4 lz4hc lz4frame xxhash; do
    object="$build/codec/$unit.o"
    "$CC" -O2 -fPIC -I"$codec" -c "$codec/$unit.c" -o "$object"
    objects+=("$object")
done
ar rcs "$build/codec/liblz4.a" "${objects[@]}"
export LZ4_CFLAGS="-I$codec" LZ4_LIBS="-L$build/codec -llz4"
cd "$build/postgresql-17.11"
./configure --prefix="$prefix" --with-openssl --with-lz4
make -j 3
make install
make -C contrib/pg_stat_statements -j 3
make -C contrib/pg_stat_statements install
export PATH="$prefix/bin:$PATH"
[[ "$(postgres --version)" == 'postgres (PostgreSQL) 17.11' ]]
grep -Eq '^#define USE_LZ4 1$' src/include/pg_config.h
cd "$prefix"
extension=lib/pg_stat_statements.so
[[ $(uname -s) != Darwin ]] || extension=lib/pg_stat_statements.dylib
{
    pg_config --version --configure --cc --cppflags --cflags --ldflags --libs
    if [[ $(uname -s) == Linux ]]; then
        readelf -h -d bin/postgres "$extension"
        ldd bin/postgres
        ldd lib/pg_stat_statements.so
        nm bin/postgres | grep ' T LZ4_versionNumber$'
        ! readelf -d bin/postgres | grep 'Shared library:.*lz4'
    else
        otool -L bin/postgres "$extension"
        ! otool -L bin/postgres | grep -q 'liblz4'
    fi
} > "$work/installed-abi.txt"
files=(bin/postgres bin/initdb bin/pg_ctl bin/psql bin/pg_config "$extension"
       share/extension/pg_stat_statements.control)
shasum -a 256 "${files[@]}" > "$work/installed.sha256"
shasum -a 256 "$build/codec/liblz4.a" > "$work/codec.sha256"
