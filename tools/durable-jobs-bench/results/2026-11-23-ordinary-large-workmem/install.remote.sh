#!/usr/bin/env bash
set -euo pipefail
sha="${1:?overlay SHA-256}"
[[ "$sha" =~ ^[a-f0-9]{64}$ ]]
base=/var/lib/durable-jobs-bench/native
overlay="$base/ordinary-large-workmem-${sha:0:16}"
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
umask 022
stage="$(mktemp -d "$base/.ordinary-large-workmem.XXXXXXXX")"
trap 'rm -rf -- "$stage"' EXIT
cat > "$stage/overlay.tar"
printf '%s  %s\n' "$sha" "$stage/overlay.tar" | sha256sum -c
mkdir "$stage/overlay"
tar -xf "$stage/overlay.tar" -C "$stage/overlay"
if [[ -e "$overlay" ]]; then
    diff -qr "$stage/overlay" "$overlay"
else
    mv "$stage/overlay" "$overlay"
fi
printf '%s\n' "$overlay"
