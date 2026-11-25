# Sourced by the sole controller under the remote arm and native locks.
stage=$1
frozen=$2
broker_sha=$3
runner_manifest_sha=$4
payload_sha=$5
dest="$stage/packet-canonical-broker"
test "$(id -u)" = 0
test "$(uname -m)" = x86_64
test "$(uname -r)" = 6.8.0-149-generic
test "$(cat /proc/sys/kernel/random/boot_id)" = 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a
test "$(findmnt -rn -o MAJ:MIN --target "$stage")" = 8:0
sha256sum -c <<'PINS'
@ORIGINAL_PINS@
PINS
tmp=$(mktemp -d "$stage/.timing-splice.XXXXXX")
trap 'rm -r -- "$tmp"' EXIT
chmod 0755 "$tmp"
cat > "$tmp/payload.tar"
printf '%s  %s\n' "$payload_sha" "$tmp/payload.tar" | sha256sum -c
mkdir "$tmp/packet"
tar --no-same-owner -xf "$tmp/payload.tar" -C "$tmp/packet"
for name in backend.py identity.py wire.py install-writer.sql; do
    install -m 0444 "$stage/packet/$name" "$tmp/packet/$name"
done
chmod 0444 "$tmp/packet/"*.py
chmod 0555 "$tmp/packet/native-run" "$tmp/packet"
if [ ! -e "$dest" ]; then
    mv -T --no-clobber "$tmp/packet" "$dest"
fi
test -d "$dest" && test ! -L "$dest"
check_dest() {
    sha256sum -c <<'PINS'
@SIBLING_PINS@
PINS
}
check_dest

printf "%s\n" CANONICAL-BROKER-STAGE-VERIFIED
