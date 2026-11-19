#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
verify() {
  export MESH_JOBS_TEST_PG_URL="$MESH_TEST_DATABASE_URL"
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

if ! initdb -D "$data" -U mesh_jobs_proof --auth=trust --no-locale --encoding=UTF8 >"$scratch/initdb.log" 2>&1; then
  cat "$scratch/initdb.log" >&2
  exit 1
fi
cat >>"$data/postgresql.conf" <<'EOF'
listen_addresses = '127.0.0.1'
unix_socket_directories = ''
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

createdb -w -h 127.0.0.1 -p "$port" -U mesh_jobs_proof mesh_jobs_proof
export MESH_TEST_DATABASE_URL="postgres://mesh_jobs_proof@127.0.0.1:$port/mesh_jobs_proof?sslmode=disable"
verify
