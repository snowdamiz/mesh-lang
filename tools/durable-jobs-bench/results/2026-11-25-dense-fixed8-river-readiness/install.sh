# The caller prepends the existing remote arm and native SETUP flock guards.
binary=$1
compressed=$2
elf=$3
boot=$4
kernel=$5
disk=$6
clock_ticks=$7
test "$(uname -m)" = x86_64
test "$(uname -r)" = "$kernel"
test "$(cat /proc/sys/kernel/random/boot_id)" = "$boot"
test "$(getconf CLK_TCK)" = "$clock_ticks"
test "$(nproc)" = 4
test "$(findmnt -rn -o MAJ:MIN --target /var/lib/durable-jobs-bench/river/scratch | tr -d " ")" = "$disk"
uptime
sha256sum -c <<'PINS'
@PINNED_FILES@
PINS

parent=${binary%/*}
stage=$(mktemp -d "${parent%/*}/.river-readiness.XXXXXX")
trap 'rm -f -- "$stage/stage.gz" "$stage/river-bench"; rmdir -- "$stage"' EXIT
cat > "$stage/stage.gz"
printf '%s  %s\n' "$compressed" "$stage/stage.gz" | sha256sum -c
gzip -dc "$stage/stage.gz" > "$stage/river-bench"
printf '%s  %s\n' "$elf" "$stage/river-bench" | sha256sum -c
chmod 0755 "$stage/river-bench"
mkdir -m 0755 -p "$parent"
if test ! -f "$binary"; then
    mv -n -- "$stage/river-bench" "$binary"
fi
printf '%s  %s\n' "$elf" "$binary" | sha256sum -c
