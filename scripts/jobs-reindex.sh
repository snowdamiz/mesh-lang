#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 INDEX [TIMEOUT_MS]" >&2
  exit 2
fi

index=$1
case "$index" in
  mesh_jobs_ready|mesh_jobs_due|mesh_jobs_running|mesh_jobs_expiring|mesh_jobs_finished|\
  mesh_jobs_unique_candidates|mesh_jobs_unique_key|mesh_jobs_cron_timed|mesh_jobs_cron_reboot) ;;
  *)
    echo "Index is outside the Jobs maintenance allowlist" >&2
    exit 2
    ;;
esac

timeout_ms=${2:-30000}
if [[ ! "$timeout_ms" =~ ^[1-9][0-9]{0,5}$ ]] || (( timeout_ms > 300000 )); then
  echo "TIMEOUT_MS must be between 1 and 300000" >&2
  exit 2
fi

# The session owns the advisory lock through the concurrent index swap.
# Closing psql releases that lock even when a statement fails.
export PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-5}"
database=()
if [[ -n "${PGDATABASE:-}" ]]; then
  database=(--dbname "$PGDATABASE")
fi
psql "${database[@]}" -X -w -v ON_ERROR_STOP=1 -v index="$index" -v timeout_ms="$timeout_ms" <<'SQL'
SET search_path TO pg_catalog, pg_temp;
SELECT set_config('statement_timeout', :'timeout_ms', false) AS timeout_ms \gset
SET lock_timeout = '2s';
DO $$
BEGIN
    IF NOT pg_try_advisory_lock(1782495102, 6) THEN
        RAISE EXCEPTION 'Jobs concurrent reindex already has an owner';
    END IF;
END
$$;
REINDEX INDEX CONCURRENTLY mesh_jobs.:"index";
SQL
