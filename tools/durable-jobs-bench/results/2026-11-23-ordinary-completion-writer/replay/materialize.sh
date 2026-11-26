#!/usr/bin/env bash
set -euo pipefail
replay="$(cd "$(dirname "$0")" && pwd)"
archive="${1:?absolute path to the frozen ordinary-trusted-v2.tar.gz}"
source="${2:?fresh absolute source path}"
case "$archive:$source" in /*:/*) ;; *) exit 2 ;; esac
expected=d87114a2332d8e3dd56da8615367ffb7b68c16945a5879c101191a09c15377b6
test "$(shasum -a 256 "$archive" | awk '{print $1}')" = "$expected"
mkdir "$source"
tar xzf "$archive" -C "$source"
cd "$source"
export GIT_CEILING_DIRECTORIES="$(dirname "$source")"
git apply --check "$replay/completion-writer.patch"
git apply "$replay/completion-writer.patch"
