
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9

umask 022
destination=/var/lib/durable-jobs-bench/native/slow-storage-6495443cc4d22bfd
work=$(mktemp -d /var/lib/durable-jobs-bench/native/slow-storage.partial.XXXXXX)
trap 'rm -rf -- "$work"' EXIT
cat >"$work/package.tar"
printf '%s  %s
' 6495443cc4d22bfd7f5f5fa0e3026dfdaf314807e2652a07ab8cb190c2783800 "$work/package.tar" | sha256sum -c
mkdir "$work/overlay"
tar --no-same-owner -xf "$work/package.tar" -C "$work/overlay"
if [[ -e "$destination" ]]; then
    diff -rq -- "$work/overlay" "$destination"
else
    mv -T -- "$work/overlay" "$destination"
fi
printf '%s
' "$destination"
