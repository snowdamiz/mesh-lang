# Sourced by the sole controller under the remote arm and native locks.
stage=$1
frozen=$2
broker_sha=$3
runner_manifest_sha=$4
payload_sha=$5
dest="$stage/packet-timing-context-verified"
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

probe() {
    width=$1
    adapter=$2
    runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
        "DENSE_PROBE_SCRATCH=$tmp/proof" "PRODUCT_SOURCE=$stage/source" \
        PG_BIN_DIR=/var/lib/durable-jobs-bench/native/toolchain/pgsql-17.11/bin \
        "PRODUCT_FROZEN_HARNESS=$frozen" "NATIVE_JOBS_ARTIFACTS=/var/lib/durable-jobs-bench/native/builds/trusted-jobs-product-623c46b92cc5" \
        "PRODUCT_MANIFEST_SHA256=$runner_manifest_sha" "DENSE_BROKER=$stage/broker/mesh" \
        "DENSE_BROKER_SHA256=$broker_sha" "DENSE_WIDTH=$width" \
        taskset -c 0,1 python3 -B "$dest/entry_probe.py" "$adapter"
}
mkdir -m 0700 "$tmp/proof"
chown nobody:nogroup "$tmp/proof"
probe 128 "$dest/adapter.py"
probe 1024 "$dest/adapter.py"
