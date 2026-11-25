# The controller prepends HOST_IDLE, which holds both remote campaign fences.
set -euo pipefail
umask 077
export LC_ALL=C
destination=$1
compiler=$2
compiler_sha=$3
module_cache=$4
boot=$5
kernel=$6
disk=$7
stock=$8
work_mount=$9
[[ "$(id -u)" == 0 && "$(uname -m)" == x86_64 && "$(nproc)" == 4 ]]
[[ "$(uname -r)" == "$kernel" && "$(cat /proc/sys/kernel/random/boot_id)" == "$boot" ]]
[[ "$(findmnt -rn -o MAJ:MIN --target "$work_mount")" == "$disk" ]]
[[ -d "$module_cache" && -x "$compiler" && ! -e "$destination" ]]
printf '%s  %s\n' "$compiler_sha" "$compiler" | sha256sum -c - >&2
if pgrep -x 'postgres|go|cargo|rustc|cc1|cc1plus|clang|gcc|ld' >/dev/null; then
    exit 1
fi
work=$(mktemp -d "${destination%/*}/.slow-readiness-build.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
tar -xf - -C "$work"
(cd "$work" && sha256sum --strict -c source-sha256.txt >&2)
mkdir "$work/tmp" "$work/cache"
GO=(/usr/bin/env -i PATH=/usr/bin:/bin LC_ALL=C
     GOTOOLCHAIN=local GOENV=off GOWORK=off GOFLAGS=
     GOOS=linux GOARCH=amd64 GOAMD64=v1 CGO_ENABLED=0
     GOPROXY=off GOSUMDB=off 'GOVCS=*:off' GOTELEMETRY=off
     "GOPATH=$work/gopath" "GOMODCACHE=$module_cache" "GOCACHE=$work/cache" "GOTMPDIR=$work/tmp"
     "$compiler")
"${GO[@]}" version > "$work/compiler.txt"
grep -Fx 'go version go1.26.4 linux/amd64' "$work/compiler.txt" >&2
(
    cd "$work/stock"
    "${GO[@]}" mod verify >&2
    "${GO[@]}" build -mod=readonly -trimpath -buildvcs=false -o "$work/stock-elf" .
)
printf '%s  %s\n' "$stock" "$work/stock-elf" | sha256sum -c - >&2
(
    cd "$work/candidate"
    "${GO[@]}" build -mod=readonly -trimpath -buildvcs=false -o "$work/river-bench" .
)
"${GO[@]}" version -m "$work/river-bench" | tail -n +2 > "$work/build-info.txt"
cmp "$work/build-info.txt" "$work/expected-build-info.txt"
(cd "$work" && sha256sum --strict -c source-sha256.txt >&2)
(cd "$work" && sha256sum stock-elf river-bench > elf-sha256.txt)
install -m 0555 "$work/river-bench" "$work/install-elf"
mkdir -m 0755 -- "$destination"
mv -- "$work/install-elf" "$destination/river-bench"
chmod 0555 -- "$destination"
tar -C "$work" -czf - compiler.txt build-info.txt elf-sha256.txt source-sha256.txt river-bench
