#!/usr/bin/env bash
# Run inside a disposable local PostgreSQL 17.11 Docker container.
set -euo pipefail
root=/checkout
results="$root/tools/durable-jobs-bench/results"
retry="$results/2026-11-25-empty-recovery-native-retry"
original="$results/2026-11-25-empty-recovery-native"
product=2026-11-24-trusted-jobs-product-composition
archive="$results/$product/source.tar.gz"
base=/var/lib/durable-jobs-bench/native
source="$base/source-trusted-jobs-product-623c46b92cc5/source"
old="$source/tools/durable-jobs-bench/results/$product/replay-closure-retry"
artifact="$base/builds/trusted-jobs-product-623c46b92cc5"
stage=/var/tmp/recovery-intake
mkdir -p /fixture/shims "$old" "$artifact" "$stage" "$base/toolchain/pgsql-17.11/bin" /var/tmp/arm/adapter

apk add --no-cache python3 util-linux shadow openssl >/dev/null
for name in initdb pg_ctl postgres psql createdb; do
    ln -s "$(command -v "$name")" "$base/toolchain/pgsql-17.11/bin/$name"
done
touch "$base/native.lock"

# Preserve the original product shell and entry point. Supply only the local
# fixture backend, since the actual pinned Mesh ELF is a Linode artifact.
for name in native-run product_arm.py identity.py; do
    tar -xOzf "$archive" "source/tools/durable-jobs-bench/results/$product/replay/$name" > "$old/$name"
done
cp "$retry/local-proof-backend.py" "$old/backend.py"
for name in schema drain_schema cron_schema enqueue_schema; do
    tar -xOzf "$archive" "source/compiler/mesh-rt/src/jobs/$name.sql"
done > /fixture/schema.sql
tar -xOzf "$archive" source/scripts/fixtures/jobs-role-grants.sql |
    sed -e '/-- Optional producer fast path\./,$d' \
        -e 's/:"producer"/parity_producer/g' -e 's/:"worker"/parity_subject/g' \
        -e 's/:"operator"/parity_operator/g' -e 's/:"inspector"/parity_inspector/g' > /fixture/roles.sql
for name in launch.sh prepare-launch.py contract.py observe.py; do
    cp "$retry/$name" "$stage/$name"
done
cp "$original/probe.py" "$results/2026-11-25-empty-recovery/candidate.sql" "$stage/"
setup="$results/$product/evidence/native/setup/trusted-jobs-product-623c46b92cc5-closure-retry"
tar -xOzf "$setup/raw.tar.gz" ./mesh.json > "$artifact/mesh.json"
printf '%s  %s\n' \
    c5e0351810985f4fb21487843dd6e95258cac9d9951989493b86e6d6c57de17a "$old/native-run" \
    979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55 "$artifact/mesh.json" |
    sha256sum -c >/dev/null

# Exercise the real launcher bytes. This private test PATH supplies the
# expected answers for the three Linode-specific host identity checks.
cat > /fixture/shims/uname <<'EOF'
#!/bin/sh
test "$*" = "-smr" || exec /bin/uname "$@"
printf '%s\n' 'Linux 6.8.0-149-generic x86_64'
EOF
cat > /fixture/shims/cat <<'EOF'
#!/bin/sh
test "$*" = "/proc/sys/kernel/random/boot_id" || exec /bin/cat "$@"
printf '%s\n' '90ec1c24-08c8-4bc1-aa28-79b6db1bd13a'
EOF
cat > /fixture/shims/findmnt <<'EOF'
#!/bin/sh
test "$*" = "-nro MAJ:MIN --target /var/lib/durable-jobs-bench/native"
printf '%s\n' '8:0'
EOF
chmod 755 /fixture/shims/*
export PATH="/fixture/shims:$PATH"
export PRODUCT_ARTIFACT="$artifact" PRODUCT_COMPOSITION=product-d2
export PRODUCT_TOPOLOGY=two-runner PRODUCT_STORAGE=default
export PRODUCT_ELF_SHA256=af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355
export PRODUCT_MANIFEST_SHA256=979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55
cd /var/tmp/arm/adapter

bash "$stage/launch.sh" "$old/native-run" gate-stock mesh noop "$PWD"

# Corrupt one rewrite anchor inside this disposable container and check that
# the same outer shell records the initiating exception against the real file.
sed 's/product_arm.py/invalid-anchor.py/g' "$old/native-run" > "$old/rejected-run"
mv "$old/rejected-run" "$old/native-run"
if bash "$stage/launch.sh" "$old/native-run" gate-stock mesh noop "$PWD" 2>/fixture/entry-error.log; then
    exit 1
fi
grep -q 'prepare-launch.py' /fixture/entry-error.log
! grep -q '/adapter/-' /fixture/entry-error.log
python3 -B -c '
import json
from pathlib import Path
result = json.loads(Path("local-proof.json").read_bytes())
result["failure_uses_named_script"] = True
print(json.dumps(result, indent=2, sort_keys=True))
'
