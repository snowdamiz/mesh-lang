#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

verify() {
  local -a args=(--include-ignored --test-threads=1)
  if [[ -z "${MESH_TEST_DATABASE_CA:-}" ]]; then
    printf 'Skipping the TLS certificate test: MESH_TEST_DATABASE_CA is unset.\n'
    args+=(--skip tls_verifies_the_server_against_sslrootcert)
  fi
  if [[ "${MESH_TEST_DATABASE_MD5:-}" != 1 ]]; then
    printf 'Skipping the MD5 authentication test: MESH_TEST_DATABASE_MD5 is unset.\n'
    args+=(--skip md5_authenticates_a_role_with_an_md5_password)
  fi
  cargo test --locked -p mesh-rt --lib db:: -- "${args[@]}"
  cargo test --locked -p meshc --test e2e_db_values --test e2e_pg_conn_ownership \
    --test e2e_migrate -- --ignored --test-threads=1
  cargo test --locked -p meshc --test e2e_sqlite_db_values --test e2e_sqlite_todo \
    --test e2e_sqlite_built_package
  cargo test --locked -p meshc --test e2e_stdlib e2e_sqlite
}

if [[ -n "${MESH_TEST_DATABASE_URL:-}" ]]; then
  verify
  exit
fi

if [[ -n "${PG_BIN_DIR:-}" ]]; then
  export PATH="$PG_BIN_DIR:$PATH"
elif ! command -v initdb >/dev/null && command -v pg_config >/dev/null; then
  pg_tools="$(pg_config --bindir)"
  export PATH="$pg_tools:$PATH"
fi
for tool in initdb pg_ctl openssl python3; do
  if ! command -v "$tool" >/dev/null; then
    printf 'Required tool missing from PATH: %s (set PG_BIN_DIR for PostgreSQL tools).\n' "$tool" >&2
    exit 1
  fi
done

mkdir -p .tmp
scratch="$(mktemp -d "$PWD/.tmp/verify-db-drivers.XXXXXX")"
data="$scratch/data"
log="$scratch/postgres.log"
cleanup() {
  local result=$?
  if (( result != 0 )) && [[ -f "$log" ]]; then
    tail -n 60 "$log" >&2
  fi
  if pg_ctl -D "$data" status >/dev/null 2>&1; then
    if ! pg_ctl -D "$data" -m fast -t 10 stop >/dev/null &&
       ! pg_ctl -D "$data" -m immediate -t 10 stop >/dev/null; then
      printf 'PostgreSQL is still running. Preserved %s for recovery.\n' "$scratch" >&2
      exit 1
    fi
  fi
  rm -rf -- "$scratch"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
export TMPDIR="$scratch"

initdb -D "$data" -U mesh_driver_proof --auth=trust --no-locale --encoding=UTF8 \
  >"$scratch/initdb.log" 2>&1 || { cat "$scratch/initdb.log" >&2; exit 1; }

if ! {
  openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=mesh-driver-proof \
    -keyout "$scratch/ca.key" -out "$scratch/ca.crt" &&
  openssl req -newkey rsa:2048 -nodes -subj /CN=localhost \
    -keyout "$data/server.key" -out "$scratch/server.csr" &&
  printf 'subjectAltName=IP:127.0.0.1\n' >"$scratch/server.ext" &&
  openssl x509 -req -in "$scratch/server.csr" -CA "$scratch/ca.crt" -CAkey "$scratch/ca.key" \
    -CAcreateserial -days 1 -extfile "$scratch/server.ext" -out "$data/server.crt"
} >"$scratch/openssl.log" 2>&1; then
  cat "$scratch/openssl.log" >&2
  exit 1
fi
chmod 600 "$data/server.key"
cat >>"$data/postgresql.conf" <<'EOF'
listen_addresses = '127.0.0.1'
unix_socket_directories = ''
ssl = on
timezone = 'UTC'
EOF
cat >"$data/pg_hba.conf" <<'EOF'
host all mesh_md5_test 127.0.0.1/32 md5
host all mesh_driver_proof 127.0.0.1/32 trust
EOF

for ((attempt = 1; attempt <= 8; attempt++)); do
  port=$((49152 + RANDOM % 16384))
  if pg_ctl -D "$data" -l "$log" -o "-p $port" -t 10 -w start; then
    break
  fi
  if (( attempt == 8 )) || pg_ctl -D "$data" status >/dev/null 2>&1; then
    exit 1
  fi
done

export MESH_TEST_DATABASE_URL="postgresql://mesh_driver_proof@127.0.0.1:$port/postgres?sslmode=disable"
export MESH_TEST_DATABASE_CA
MESH_TEST_DATABASE_CA="$(python3 -c 'from sys import argv; from urllib.parse import quote; print(quote(argv[1]))' "$scratch/ca.crt")"
export MESH_TEST_DATABASE_MD5=1
verify
