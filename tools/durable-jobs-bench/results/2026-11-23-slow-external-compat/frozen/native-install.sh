
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9

umask 022
destination=/var/lib/durable-jobs-bench/native/slow-external-73a5e27b40064110
work=$(mktemp -d /var/lib/durable-jobs-bench/native/slow-external.partial.XXXXXX)
trap 'rm -rf -- "$work"' EXIT
cat >"$work/package.tar"
printf '%s  %s
' 73a5e27b40064110dd028d54b506ad4347834c7c5bf9e2baa866ac283cc07dac "$work/package.tar" | sha256sum -c
mkdir "$work/overlay"
tar --no-same-owner -xf "$work/package.tar" -C "$work/overlay"
if [[ -e "$destination" ]]; then
    diff -rq -- "$work/overlay" "$destination"
else
    mv -T -- "$work/overlay" "$destination"
fi
printf '%s
' "$destination"
