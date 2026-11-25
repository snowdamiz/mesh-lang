#!/usr/bin/env bash
# Exercise the original launcher and all five entry paths, without running a Jobs arm.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
root="$(git -C "$here" rev-parse --show-toplevel)"
scratch="$(mktemp -d "$root/target/stock8-stage.XXXXXXXX")"
server=
cleanup() {
    if [[ -n "$server" ]]; then docker stop "$server" >/dev/null; fi
    rm -f -- "$scratch/server.crt" "$scratch/server.key" "$scratch/auto_explain.so"
    rmdir -- "$scratch"
}
trap cleanup EXIT
openssl req -new -x509 -nodes -newkey rsa:2048 -subj /CN=localhost -days 1 \
    -keyout "$scratch/server.key" -out "$scratch/server.crt" 2>/dev/null
module=/var/lib/durable-jobs-bench/native/dense-live-grant-plans-20261125/auto_explain.so
server="$(docker run --rm --pull never --network none -d -e POSTGRES_HOST_AUTH_METHOD=trust \
    --mount "type=bind,src=$scratch,dst=/tls,readonly" --entrypoint /bin/sh postgres:17-alpine -euc '
    install -d -m 0755 "$(dirname "$1")"
    install -m 0644 /usr/local/lib/postgresql/auto_explain.so "$1"
    install -o postgres -m 0600 /tls/server.key /var/tmp/stage.key
    install -o postgres -m 0644 /tls/server.crt /var/tmp/stage.crt
    exec docker-entrypoint.sh postgres -c ssl=on -c ssl_key_file=/var/tmp/stage.key \
        -c ssl_cert_file=/var/tmp/stage.crt -c max_connections=96 -c reserved_connections=0 \
        -c superuser_reserved_connections=0 -c shared_preload_libraries=pg_stat_statements
' pg-stage "$module")"
for attempt in {1..30}; do
    if docker exec "$server" pg_isready -h 127.0.0.1 -q; then break; fi
    sleep 1
done
docker exec "$server" pg_isready -h 127.0.0.1 -q
docker exec "$server" cat "$module" >"$scratch/auto_explain.so"
python3 -B - "$here" "$scratch/auto_explain.so" <<'PY' | docker run --rm --pull never \
    --network "container:$server" -i --entrypoint /bin/bash 3dc01f342e31 -euc '
    cd /
    tar -xf -
    ln -sfn /usr/local/bin/python3 /usr/bin/python3
    install -m 0644 /fixture/stage_probe.py "$(python3 -c "import site; print(site.getsitepackages()[0])")/sitecustomize.py"
    base=/var/lib/durable-jobs-bench/native
    touch "$base/native.lock"
    export PRODUCT_ARTIFACT="$base/builds/trusted-jobs-product-623c46b92cc5"
    export PRODUCT_COMPOSITION=product-bulk PRODUCT_TOPOLOGY=stock PRODUCT_STORAGE=default
    export PRODUCT_MANIFEST_SHA256="$(sha256sum "$PRODUCT_ARTIFACT/mesh.json" | cut -d " " -f 1)"
    export PRODUCT_ELF_SHA256="$(python3 -c "import json; print(json.load(open(\"$PRODUCT_ARTIFACT/mesh.json\"))[\"files\"][\"mesh\"])")"
    old="$base/source-trusted-jobs-product-623c46b92cc5/source/tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay-closure-retry/native-run"
    for lane in plan-stock plan-key stock-a key stock-b; do
        module=-
        case "$lane" in plan-*) module="$base/dense-live-grant-plans-20261125/auto_explain.so";; esac
        mkdir "/fixture/$lane"
        taskset -c 0,1 bash /fixture/launch.sh "$old" "$lane" "$module" mesh noop "/fixture/$lane"
    done
    test -z "$(find /var/tmp -maxdepth 1 -name "dense-stock-logical-key.*" -print -quit)"
'
from io import BytesIO
from pathlib import Path
import subprocess
import sys
import tarfile

sys.path.insert(0, sys.argv[1])
import controller as c

cfg = c.config()
pins = cfg["lanes"]["plan-stock"]["inventory"]["engines"]["mesh"]["verified_files"]
argv = cfg["lanes"]["plan-stock"]["inventory"]["engines"]["mesh"]["argv"]
old = Path(argv[-4])
anchor = "/ordinary-v2/"
frozen = c.ROOT / "target/dense-public-hot-readiness/ordinary-v2"
files = {str(old.parent / name): data for name, data in c.public_files(cfg).items()}
for path, sha in pins.items():
    if anchor in path:
        source = frozen / path.split(anchor)[1]
        assert c.run.digest(source) == sha, source
        files[path] = source.read_bytes()
files.update({f"/fixture/{p.name}": p.read_bytes() for p in (*c.FILES, c.HERE/"stage_probe.py")})
files[argv[-2]] = Path(sys.argv[2]).read_bytes()
files[str(Path(argv[1].split("=",1)[1])/"mesh.json")] = c.MANIFEST_PATH.read_bytes()
files["/fixture/schema.sql"] = subprocess.check_output(
    ["git", "show", f"{c.REVISION}:compiler/mesh-rt/src/jobs/schema.sql"], cwd=c.ROOT)
with tarfile.open(fileobj=sys.stdout.buffer, mode="w|") as tar:
    for name, data in files.items():
        item = tarfile.TarInfo(name.lstrip("/"))
        item.size = len(data)
        item.mode = 0o644
        tar.addfile(item, BytesIO(data))
PY
