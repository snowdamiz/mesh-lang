#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -sm)" == 'Linux x86_64' && "$(id -u)" == 0 ]]
base=/var/lib/durable-jobs-bench/native
proof="$base/source-trusted-1e70f18d-jobs-78468180f0c0"
dest="$base/dense-individual-hot-pool-1e70f18d"
out="$base/runs/dense-individual-hot-pool-1e70f18d"
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
[[ "$(cat /proc/sys/kernel/random/boot_id)" == "$1" && "$(uname -r)" == "$2" ]]
[[ "$(findmnt -nro MAJ:MIN --target "$base" | xargs)" == "$3" ]]
export PRODUCT_ELF_SHA256="$4" PRODUCT_MANIFEST_SHA256="$5"
backend_sha="$6"
grants_sha="$7"
harness_sha="$8"
shift 8
umask 022
mkdir -m 0711 "$out"
mkdir -m 0755 "$dest"
tar -xf - -C "$dest" --no-same-owner --no-same-permissions
(cd "$dest" && printf '%s  %s\n' "$@" | sha256sum -c)
printf '%s  %s\n' "$@" > "$out/adapter.sha256"
printf '%s  %s\n' \
    "$PRODUCT_ELF_SHA256" "$proof/artifacts/jobs/mesh" \
    "$PRODUCT_MANIFEST_SHA256" "$proof/artifacts/jobs/mesh.json" \
    "$backend_sha" "$proof/overlay/replay/backend.py" \
    "$grants_sha" "$proof/source/scripts/fixtures/jobs-role-grants.sql" \
    "$harness_sha" "$proof/overlay/frozen-harness.sha256" \
    78468180f0c0be9b2b17533b54dc8cbd69e3982d84b239457b15e5beb018b530 "$proof/source.tar.gz" \
    b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1 "$base/toolchain/pgsql-17.11/bin/postgres" \
    | sha256sum -c | tee "$out/proof.sha256"
(cd "$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2" &&
    sha256sum -c "$proof/overlay/frozen-harness.sha256") > "$out/harness.sha256"
cp "$proof/artifacts/jobs/mesh.json" "$out/mesh.json"
mkdir -m 0700 "$dest/scratch"
chown nobody:nogroup "$dest/scratch"
runuser -u nobody -- test -x "$proof/artifacts/jobs/mesh"
for setting in 0 1; do
    mkdir "$out/$setting"
    HOT_CHECKOUT="$setting" DURABLE_JOBS_PHASE=diagnostic taskset -c 0,1 \
        bash "$dest/native-run" mesh noop "$out/$setting"
done
touch "$out/SETUP-PASS"
