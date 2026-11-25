#!/usr/bin/env bash
# Run only PostgreSQL and libpq connection controls on an isolated Docker loopback.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -P)"
root="$(git -C "$here" rev-parse --show-toplevel)"
dense="$here/../../2026-11-25-dense-live-grant-plans/procfd-recovery"
frozen="$root/target/enqueue-spec-preparation/ordinary-v2"
scratch="$(mktemp -d "$root/target/slow-roster-tls.XXXXXXXX")"
server=
cleanup() {
    if [[ -n "$server" ]]; then docker stop "$server" >/dev/null; fi
    python3 -B - "$scratch" <<'PY'
from pathlib import Path
import sys
directory = Path(sys.argv[1])
for name in ("server.crt", "server.key"):
    (directory / name).unlink(missing_ok=True)
directory.rmdir()
PY
}
trap cleanup EXIT
chmod 0755 "$scratch"
openssl req -new -x509 -nodes -newkey rsa:2048 -days 1 -subj /CN=127.0.0.1 \
    -addext subjectAltName=IP:127.0.0.1 -keyout "$scratch/server.key" \
    -out "$scratch/server.crt" 2>/dev/null
server="$(docker run --rm --pull=never --network none -d -e POSTGRES_HOST_AUTH_METHOD=trust \
    -v "$scratch:/tls:ro" --entrypoint /bin/sh 3540c0362c80 -eu -c '
    install -o postgres -m 0600 /tls/server.key /var/tmp/slow.key
    install -o postgres -m 0644 /tls/server.crt /var/tmp/slow.crt
    exec docker-entrypoint.sh postgres -c ssl=on -c ssl_key_file=/var/tmp/slow.key \
        -c ssl_cert_file=/var/tmp/slow.crt -c max_connections=68 -c reserved_connections=0 \
        -c superuser_reserved_connections=0
')"
for attempt in {1..30}; do
    if docker exec "$server" pg_isready -h 127.0.0.1 -q; then break; fi
    sleep 1
done
docker exec "$server" pg_isready -h 127.0.0.1 -q
docker exec "$server" psql -Xq -v ON_ERROR_STOP=1 -U postgres -c 'CREATE DATABASE mesh CONNECTION LIMIT 68'
docker exec "$server" psql -Xq -v ON_ERROR_STOP=1 -U postgres -c '
    CREATE ROLE parity_subject LOGIN CONNECTION LIMIT 4;
    CREATE ROLE parity_producer LOGIN CONNECTION LIMIT 1;
    CREATE ROLE compiled_caller LOGIN CONNECTION LIMIT 8;
    GRANT pg_read_all_stats TO parity_producer;'
for runners in 1 2; do
    docker exec "$server" psql -Xq -v ON_ERROR_STOP=1 -U postgres \
        -c "ALTER ROLE parity_subject CONNECTION LIMIT $((runners * 4))"
    docker run --rm --pull=never --network "container:$server" \
        -v "$here:/packet:ro" -v "$here/../stage:/original-stage:ro" -v "$dense:/dense:ro" -v "$frozen:/frozen:ro" \
        -v "$scratch/server.crt:/tls/server.crt:ro" --entrypoint /bin/bash 3dc01f342e31 -eu -c '
        stage=/var/tmp/slow-roster-stage
        mkdir -p "$stage/raw"
        cp /packet/stage/* "$stage/"
        chmod 0711 "$stage"
        chown nobody:nogroup "$stage/raw"
        chmod 0700 "$stage/raw"
        PRODUCT_FROZEN_HARNESS=/frozen runuser -u nobody -- /usr/local/bin/python3 -B \
            /packet/lifecycle.py "$stage" "$stage/raw" "$1"
    ' slow-roster "$runners"
done
