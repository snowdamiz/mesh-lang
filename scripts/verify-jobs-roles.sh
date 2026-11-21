#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
if [[ "${MESH_JOBS_DISPOSABLE_ROLE_PROOF:-}" != 1 ]]; then
  echo "Run verify-postgres-jobs.sh --j6-roles to launch a disposable role-proof cluster" >&2
  exit 2
fi

upgrade_from=7c1620563614a04a573aef806a6d019a2af4dfe9
git cat-file -e "$upgrade_from:compiler/mesh-rt/src/jobs/schema.sql"
git cat-file -e "$upgrade_from:compiler/mesh-rt/src/jobs/cron_schema.sql"
psql_proof() {
  psql -XqAtw -v ON_ERROR_STOP=1 -v VERBOSITY=verbose "$MESH_TEST_DATABASE_URL" "$@"
}

psql_proof -v role_password="$MESH_JOBS_TEST_ROLE_PASSWORD" <<'SQL'
CREATE ROLE jobs_migrator LOGIN PASSWORD :'role_password';
CREATE ROLE jobs_producer LOGIN PASSWORD :'role_password';
CREATE ROLE jobs_worker LOGIN PASSWORD :'role_password';
CREATE ROLE jobs_operator LOGIN PASSWORD :'role_password';
CREATE ROLE jobs_inspector LOGIN PASSWORD :'role_password';
CREATE ROLE jobs_index_admin;
CREATE DATABASE jobs_role_fresh OWNER jobs_migrator;
CREATE DATABASE jobs_role_upgrade OWNER jobs_migrator;
SQL

as_role() {
  local role=$1
  {
    printf '\\connect -reuse-previous=on %s\nSET SESSION AUTHORIZATION %s;\nSET client_min_messages TO warning;\n' "$database" "$role"
    cat
  } | psql_proof -v producer=jobs_producer -v worker=jobs_worker \
    -v operator=jobs_operator -v inspector=jobs_inspector
}

statement() {
  printf '%s\n' "$2" | as_role "$1"
}

expect() {
  local role=$1 sql=$2 expected=$3 actual
  actual="$(statement "$role" "$sql")"
  if [[ "$actual" != "$expected" ]]; then
    printf 'Role %s returned %s, expected %s\n' "$role" "$actual" "$expected" >&2
    exit 1
  fi
}

denied() {
  local role=$1 sql=$2 output
  if output="$(statement "$role" "$sql" 2>&1)" || [[ "$output" != *42501* ]]; then
    printf 'Expected insufficient privilege for %s running %s\n%s\n' "$role" "$sql" "$output" >&2
    exit 1
  fi
}

prepared='{"worker":"proof","version":1,"queue":"default","args":{"proof":1},"metadata":{},"tags":[],"priority":0,"max_attempts":2,"timeout_ms":60000,"backoff_base_ms":1000,"backoff_max_ms":60000,"suspended":false,"unique":null,"replace":{}}'
workers='[{"worker":"proof","version":1}]'
enqueue_sql="SELECT mesh_jobs.enqueue('proof', '$prepared', 'receipt', 'fingerprint') ->> 'id';"

for database in jobs_role_fresh jobs_role_upgrade; do
  statement jobs_migrator "
    ALTER DEFAULT PRIVILEGES REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;
    CREATE SCHEMA business;
    CREATE TABLE business.writes (id integer PRIMARY KEY);
    GRANT USAGE ON SCHEMA business TO jobs_producer;
    GRANT INSERT ON business.writes TO jobs_producer;"

  if [[ "$database" == jobs_role_upgrade ]]; then
    {
      printf 'BEGIN;\n'
      git show "$upgrade_from:compiler/mesh-rt/src/jobs/schema.sql"
      git show "$upgrade_from:compiler/mesh-rt/src/jobs/cron_schema.sql"
      printf 'COMMIT;\n'
    } | as_role jobs_migrator >/dev/null
    statement jobs_migrator "
      GRANT USAGE ON SCHEMA mesh_jobs TO jobs_worker;
      GRANT SELECT ON mesh_jobs.jobs, mesh_jobs.queues TO jobs_worker;
      GRANT UPDATE ON mesh_jobs.jobs, mesh_jobs.queues TO jobs_worker;
      GRANT EXECUTE ON FUNCTION mesh_jobs.claim(text,text,jsonb,bigint),
        mesh_jobs.now_ms(), mesh_jobs.effect_key(mesh_jobs.jobs),
        mesh_jobs.renew(text,uuid,uuid,bigint), mesh_jobs.settle(text,uuid,uuid,jsonb,boolean)
        TO jobs_worker;
      SELECT mesh_jobs.enqueue('upgrade', '$prepared', 'old-receipt', 'old-spec');" >/dev/null
    expect jobs_migrator "SELECT mesh_jobs.queue_control('upgrade','default','pause',NULL)->>'state';" paused
    expect jobs_worker "SELECT count(*) FROM mesh_jobs.claim('upgrade','default','$workers',60000);" 0
    expect jobs_migrator "SELECT mesh_jobs.queue_control('upgrade','default','resume',NULL)->>'state';" active
    legacy="$(statement jobs_worker "SELECT id || ' ' || token FROM
      mesh_jobs.claim('upgrade','default','$workers',60000);")"
    read -r legacy_id legacy_token <<<"$legacy"
    [[ "$legacy_id" =~ ^[0-9a-f-]{36}$ && "$legacy_token" =~ ^[0-9a-f-]{36}$ ]]
  fi

  {
    printf 'BEGIN; SELECT pg_advisory_xact_lock(1782495102, 1);\n'
    cat compiler/mesh-rt/src/jobs/schema.sql
    cat compiler/mesh-rt/src/jobs/drain_schema.sql
    printf 'COMMIT; BEGIN;\n'
    cat compiler/mesh-rt/src/jobs/cron_schema.sql
    printf 'COMMIT;\n'
  } | as_role jobs_migrator >/dev/null

  if [[ "$database" == jobs_role_upgrade ]]; then
    expect jobs_worker "SELECT mesh_jobs.renew('upgrade','$legacy_id','$legacy_token',60000);" active
    expect jobs_worker "SELECT mesh_jobs.settle('upgrade','$legacy_id','$legacy_token',
      '{\"action\":\"complete\"}',false);" t
    denied jobs_worker "SELECT mesh_jobs.claim_batch_v1('upgrade','default','$workers',60000,1,NULL);"
    statement jobs_migrator "REVOKE UPDATE ON mesh_jobs.jobs, mesh_jobs.queues FROM jobs_worker;"
  fi
  as_role jobs_migrator < scripts/fixtures/jobs-role-grants.sql

  expect jobs_migrator "SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
    CROSS JOIN LATERAL aclexplode(coalesce(p.proacl, acldefault('f', p.proowner))) a
    WHERE n.nspname='mesh_jobs' AND a.grantee=0 AND a.privilege_type='EXECUTE';" 0

  receipt_id="$(statement jobs_producer "
    BEGIN; INSERT INTO business.writes VALUES (1);
    INSERT INTO mesh_jobs.admissions AS a(namespace) VALUES ('proof')
      ON CONFLICT (namespace) DO UPDATE SET fence = NOT a.fence;
    $enqueue_sql
    COMMIT;")"
  [[ "$receipt_id" =~ ^[0-9a-f-]{36}$ ]]
  statement jobs_producer "
    BEGIN; INSERT INTO business.writes VALUES (2);
    SELECT mesh_jobs.enqueue('proof', '$prepared', 'rolled-back', 'different');
    ROLLBACK;" >/dev/null
  expect jobs_migrator "SELECT count(*) FROM business.writes; SELECT count(*) FROM
    mesh_jobs.jobs WHERE namespace='proof'; SELECT count(*) FROM mesh_jobs.receipts
    WHERE namespace='proof';" $'1\n1\n1'
  denied jobs_producer "SELECT mesh_jobs.claim('proof','default','$workers',60000);"
  denied jobs_producer "SELECT mesh_jobs.claim_drain_v1('proof','default','$workers',60000,NULL,NULL);"
  denied jobs_producer "SELECT mesh_jobs.queue_control('proof','default','pause',NULL);"
  denied jobs_worker "SELECT mesh_jobs.set_namespace_paused('proof',true);"

  expect jobs_operator "SELECT mesh_jobs.queue_control('proof','default','pause',NULL)->>'state';" paused
  expect jobs_worker "SELECT count(*) FROM mesh_jobs.claim('proof','default','$workers',60000);" 0
  expect jobs_operator "SELECT mesh_jobs.queue_control('proof','default','resume',NULL)->>'state';" active

  attempt="$(statement jobs_worker "SELECT id || ' ' || token FROM
    mesh_jobs.claim_batch_v1('proof','default','$workers',60000,1,NULL) WHERE status='claimed';")"
  read -r job_id job_token <<<"$attempt"
  [[ "$job_id" == "$receipt_id" && "$job_token" =~ ^[0-9a-f-]{36}$ ]]
  expect jobs_worker "SELECT mesh_jobs.try_renew('proof','$job_id','$job_token',60000);" active
  expect jobs_inspector "SELECT state || ':' || running FROM mesh_jobs.queue_summaries
    WHERE namespace='proof';" active:1
  expect jobs_inspector "SELECT state FROM mesh_jobs.job_summaries WHERE namespace='proof';" executing
  denied jobs_inspector "SELECT args FROM mesh_jobs.jobs WHERE namespace='proof';"
  denied jobs_inspector "UPDATE mesh_jobs.jobs SET priority=1 WHERE namespace='proof';"
  denied jobs_inspector "SELECT mesh_jobs.enqueue('proof','$prepared','','');"
  denied jobs_inspector "SELECT mesh_jobs.settle_state('proof','$job_id','$job_token',
    '{\"action\":\"complete\"}',false);"
  denied jobs_worker "CREATE TABLE mesh_jobs.denied (id int);"
  denied jobs_worker "ALTER TABLE mesh_jobs.jobs ADD COLUMN denied int;"
  denied jobs_worker "ALTER ROLE jobs_operator CREATEROLE;"
  denied jobs_worker "REINDEX INDEX mesh_jobs.mesh_jobs_ready;"
  denied jobs_operator "REINDEX INDEX mesh_jobs.mesh_jobs_ready;"
  if [[ "$(statement jobs_migrator 'SHOW server_version_num;')" -ge 170000 ]]; then
    statement jobs_migrator "
      GRANT USAGE ON SCHEMA mesh_jobs TO jobs_index_admin;
      GRANT MAINTAIN ON mesh_jobs.jobs TO jobs_index_admin;"
    statement jobs_index_admin "REINDEX INDEX CONCURRENTLY mesh_jobs.mesh_jobs_ready;"
  fi

  expect jobs_worker "SELECT mesh_jobs.settle_state('proof','$job_id','$job_token',
    '{\"action\":\"complete\"}',false); SELECT mesh_jobs.recover('proof',1,NULL);" $'completed\n0'
  expect jobs_operator "SELECT mesh_jobs.job_control('proof','$job_id','retry',NULL,NULL)->>'state';" available
  expect jobs_inspector "SELECT effect_key FROM mesh_jobs.job_summaries WHERE namespace='proof';" "$receipt_id/2"
  expect jobs_producer "$enqueue_sql" "$receipt_id"
  printf 'Scoped Jobs role proof passed on %s\n' "$database"
done
