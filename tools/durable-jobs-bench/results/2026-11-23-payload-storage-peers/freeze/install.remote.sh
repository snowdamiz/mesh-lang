#!/bin/bash
set -euo pipefail
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
umask 022
destination=/var/lib/durable-jobs-bench/native/payload-storage-peers-811120acf887d24d
work=$(mktemp -d /var/lib/durable-jobs-bench/native/payload-storage-peers.partial.XXXXXX)
trap 'rm -rf -- "$work"' EXIT
cat >"$work/package.tar"
printf '%s  %s\n' 811120acf887d24dbe9798db9877089aa7f21ecd81d4c44b54cb1f14cde418b1 "$work/package.tar" | sha256sum -c
mkdir "$work/overlay"
tar --no-same-owner -xf "$work/package.tar" -C "$work/overlay"
if [[ -e "$destination" ]]; then
    diff -rq -- "$work/overlay" "$destination"
else
    mv -T -- "$work/overlay" "$destination"
fi
printf '%s\n' "$destination"
