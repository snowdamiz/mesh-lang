#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
unset MESH_JOBS_DISPOSABLE_ROLE_PROOF MESH_JOBS_TEST_ROLE_PASSWORD
postgres_version=
parity_driver_check=false
j6_roles=false
while (( $# > 0 )); do
  case "$1" in
    --compatibility)
      if [[ $# -lt 2 || -n "$postgres_version" || ! "$2" =~ ^[0-9]+\.[0-9]+$ ]]; then
        printf 'Expected one --compatibility VERSION with a major and patch number.\n' >&2
        exit 2
      fi
      postgres_version="$2"
      shift 2
      ;;
    --parity-driver-check)
      parity_driver_check=true
      shift
      ;;
    --j6-roles)
      j6_roles=true
      shift
      ;;
    *)
      printf 'Usage: %s [--compatibility VERSION] [--parity-driver-check] [--j6-roles]\n' "$0" >&2
      exit 2
      ;;
  esac
done

if "$parity_driver_check"; then
  python3 scripts/jobs-parity.py --self-check
  PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/tests -p test_jobs_parity_fault.py
fi

verify() {
  export MESH_JOBS_TEST_PG_URL="$MESH_TEST_DATABASE_URL"
  if [[ -n "$postgres_version" ]]; then
    MESH_TEST_POSTGRES_VERSION="$postgres_version" \
      cargo test --locked -p mesh-rt --test postgres_compatibility -- --ignored --nocapture
    cargo test --locked -p meshc --test e2e_pg_conn_ownership --test e2e_db_values \
      --test e2e_migrate -- --ignored --test-threads=1
  fi
  cargo test --locked -p mesh-rt --lib db::pg::tests
  cargo test --locked -p mesh-rt --lib jobs:: -- --include-ignored --test-threads=1
  cargo test --locked -p meshc --test e2e_durable_jobs -- --nocapture
}

if [[ -n "${PG_BIN_DIR:-}" ]]; then
  export PATH="$PG_BIN_DIR:$PATH"
elif ! command -v initdb >/dev/null && [[ -d /opt/homebrew/opt/postgresql@17/bin ]]; then
  export PATH="/opt/homebrew/opt/postgresql@17/bin:$PATH"
elif ! command -v initdb >/dev/null && command -v pg_config >/dev/null; then
  pg_tools="$(pg_config --bindir)"
  export PATH="$pg_tools:$PATH"
fi

if [[ -n "${MESH_TEST_DATABASE_URL:-}" ]]; then
  if "$j6_roles"; then
    echo "--j6-roles requires a new disposable cluster. Unset MESH_TEST_DATABASE_URL." >&2
    exit 2
  fi
  verify
  exit
fi

for tool in initdb pg_ctl createdb psql; do
  if ! command -v "$tool" >/dev/null; then
    printf 'PostgreSQL 14+ required. Set PG_BIN_DIR to a directory containing %s.\n' "$tool" >&2
    exit 1
  fi
done

mkdir -p .tmp
scratch="$(mktemp -d "$PWD/.tmp/verify-postgres-jobs.XXXXXX")"
data="$scratch/data"
log="$scratch/postgres.log"
cleanup() {
  local result=$?
  if (( result != 0 )) && [[ -f "$scratch/openssl.log" && ! -f "$data/server.crt" ]]; then
    tail -n 20 "$scratch/openssl.log" >&2
  fi
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
unset PGHOSTADDR PGSERVICE PGOPTIONS

credentials=mesh_jobs_proof
connection_options=sslmode=disable
auth=(--auth=trust)
if [[ -n "$postgres_version" ]]; then
  password="$(openssl rand -hex 16)"
  printf '%s\n' "$password" >"$scratch/password"
  credentials="mesh_jobs_proof:$password"
  auth=(--auth-local=trust --auth-host=scram-sha-256 --pwfile="$scratch/password")
fi

if ! initdb -D "$data" -U mesh_jobs_proof "${auth[@]}" --no-locale --encoding=UTF8 >"$scratch/initdb.log" 2>&1; then
  cat "$scratch/initdb.log" >&2
  exit 1
fi
cat >>"$data/postgresql.conf" <<'EOF'
listen_addresses = '127.0.0.1'
unix_socket_directories = ''
EOF

if [[ -n "$postgres_version" ]]; then
  rm -- "$scratch/password"
  openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=mesh-probe-ca \
    -keyout "$scratch/ca.key" -out "$scratch/ca.crt" 2>"$scratch/openssl.log"
  openssl req -newkey rsa:2048 -nodes -subj /CN=localhost \
    -keyout "$data/server.key" -out "$scratch/server.csr" 2>>"$scratch/openssl.log"
  printf 'subjectAltName=IP:127.0.0.1\n' >"$scratch/server.ext"
  openssl x509 -req -in "$scratch/server.csr" -CA "$scratch/ca.crt" -CAkey "$scratch/ca.key" \
    -CAcreateserial -days 1 -extfile "$scratch/server.ext" -out "$data/server.crt"
  chmod 600 "$data/server.key"
  printf "ssl = on\ntimezone = 'UTC'\n" >>"$data/postgresql.conf"
  printf 'hostssl all all 127.0.0.1/32 scram-sha-256\n' >"$data/pg_hba.conf"
  rootcert="$(python3 -c 'from sys import argv; from urllib.parse import quote; print(quote(argv[1]))' "$scratch/ca.crt")"
  connection_options="sslmode=verify-full&sslrootcert=$rootcert"
fi

for ((attempt = 1; attempt <= 8; attempt++)); do
  port=$((49152 + RANDOM % 16384))
  if pg_ctl -D "$data" -l "$log" -o "-p $port" -t 10 -w start; then
    break
  fi
  if (( attempt == 8 )) || pg_ctl -D "$data" status >/dev/null 2>&1; then
    exit 1
  fi
done

server_url="postgres://$credentials@127.0.0.1:$port"
createdb -w --maintenance-db="$server_url/postgres?$connection_options" mesh_jobs_proof
export MESH_TEST_DATABASE_URL="$server_url/mesh_jobs_proof?$connection_options"
server_version="$(psql -X -w -qAt -v ON_ERROR_STOP=1 "$MESH_TEST_DATABASE_URL" -c 'SHOW server_version')"
printf 'Disposable Jobs proof server: PostgreSQL %s\n' "$server_version"
if "$j6_roles"; then
  export MESH_JOBS_DISPOSABLE_ROLE_PROOF=1
  MESH_JOBS_TEST_ROLE_PASSWORD="$(psql -X -w -qAt -v ON_ERROR_STOP=1 "$MESH_TEST_DATABASE_URL" \
    -c "SELECT replace(gen_random_uuid()::text, '-', '')")"
  export MESH_JOBS_TEST_ROLE_PASSWORD
  bash scripts/verify-jobs-roles.sh
fi
verify
