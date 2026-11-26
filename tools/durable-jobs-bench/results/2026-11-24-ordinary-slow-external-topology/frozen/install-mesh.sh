
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9

umask 022
destination=/var/lib/durable-jobs-bench/native/slow-external-topology-ba920836536623e7
work=$(mktemp -d /var/lib/durable-jobs-bench/native/slow-external.partial.XXXXXX)
trap 'rm -rf -- "$work"' EXIT
cat >"$work/package.tar"
printf '%s  %s
' ba920836536623e79287621e6d04f6f2ba2144027e4dec2117571aced76b8095 "$work/package.tar" | sha256sum -c
mkdir "$work/overlay"
tar --no-same-owner -xf "$work/package.tar" -C "$work/overlay"
if [[ -e "$destination" ]]; then
    diff -rq -- "$work/overlay" "$destination"
else
    mv -T -- "$work/overlay" "$destination"
fi
printf '%s
' "$destination"
