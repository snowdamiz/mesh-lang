#!/usr/bin/env bash
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
bundle=${1:?sealed transfer SHA-256}
destination=${2:?owned overlay}
[[ "$bundle" =~ ^[0-9a-f]{64}$ ]]
[[ "$destination" =~ ^/var/lib/durable-jobs-bench/native/ordinary-broker-once-[0-9a-f]{16}$ ]]
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
umask 022
export PATH="$base/toolchain/llvm-21.1.8/bin:/usr/bin:/bin"
export TMPDIR="$base/tmp" LC_ALL=C.UTF-8
stage="$(mktemp -d "$base/tmp/ordinary-broker-once.XXXXXX")"
trap 'rm -rf -- "$stage"' EXIT
cat > "$stage/overlay.tar"
printf '%s  %s\n' "$bundle" "$stage/overlay.tar" | sha256sum -c >&2
export TMPDIR="$stage"
mkdir "$stage/files"
tar --no-same-owner -xf "$stage/overlay.tar" -C "$stage/files"
python3 -B "$stage/files/link.py" "$stage/files" "$destination"
