#!/usr/bin/env python3
"""Prove a mixed-binary Jobs upgrade and a worker-only rollback on disposable PostgreSQL."""

import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
import platform
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.dont_write_bytecode = True
SCRIPT = Path(__file__).resolve()
SPEC = importlib.util.spec_from_file_location("jobs_parity", SCRIPT.with_name("jobs-parity.py"))
parity = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(parity)

ROOT = parity.ROOT
PINNED_MAIN = "7c1620563614a04a573aef806a6d019a2af4dfe9"
FIXTURE = ROOT / "scripts/fixtures/jobs-upgrade/main.mpl"
GRANTS = ROOT / "scripts/fixtures/jobs-role-grants.sql"
DATABASE = "rolling_upgrade"
NAMESPACE = "upgrade"
MIGRATOR = "upgrade_migrator"
PRODUCER = "upgrade_producer"
OLD_WORKER = "upgrade_old_worker"
NEW_WORKER = "upgrade_new_worker"
OPERATOR = "upgrade_operator"
INSPECTOR = "upgrade_inspector"
ROLES = (MIGRATOR, PRODUCER, OLD_WORKER, NEW_WORKER, OPERATOR, INSPECTOR)
BATCH = "claim_batch_v1(text,text,jsonb,bigint,integer,bigint)"


def require(condition, message):
    if not condition:
        raise parity.FixtureError(message)


def toolchain(ref, scratch, evidence, env, prebuilt):
    if prebuilt is not None:
        artifacts = prebuilt / ref / "artifacts"
        require((artifacts / "source-sha").read_text().strip() == ref,
                f"toolchain source SHA mismatch for {ref}")
        return artifacts

    source = scratch / ref
    parity.materialize_commit(ref, source, scratch / "source.tar")
    target = scratch / "cargo"
    parity.checked(
        ["cargo", "build", "--locked", "--jobs", "2", "-p", "meshc", "-p", "mesh-rt"],
        cwd=source, env=dict(env, CARGO_TARGET_DIR=str(target)), timeout=2400,
        log=evidence / f"cargo-{ref}.log",
    )
    artifacts = source / "artifacts"
    artifacts.mkdir()
    for name in ("meshc", "libmesh_rt.a"):
        shutil.copy2(target / "debug" / name, artifacts / name)
    (artifacts / "source-sha").write_text(ref + "\n")
    return artifacts


def build_fixture(ref, scratch, evidence, env, rust_target, prebuilt):
    artifacts = toolchain(ref, scratch, evidence, env, prebuilt)
    project = scratch / f"fixture-{ref}"
    project.mkdir()
    shutil.copy2(FIXTURE, project / "main.mpl")
    binary = scratch / f"mesh-upgrade-{ref}"
    compiler, runtime = (artifacts / name for name in ("meshc", "libmesh_rt.a"))
    parity.checked(
        [str(compiler), "build", str(project), "--target", rust_target,
         "--output", str(binary), "--opt-level", "0"],
        cwd=scratch, env=dict(env, MESH_RT_LIB_PATH=str(runtime)),
        timeout=180, log=evidence / f"fixture-{ref}.log",
    )
    return binary, {
        "source_sha": ref, "compiler_sha256": parity.digest(compiler),
        "runtime_sha256": parity.digest(runtime), "fixture_binary_sha256": parity.digest(binary),
    }


class RollingProof:
    def __init__(self, cluster, scratch, evidence, env, old, new):
        self.cluster = cluster
        self.scratch = scratch
        self.evidence = evidence
        self.env = env
        self.old = old
        self.new = new
        self.gate = scratch / "release-legacy"

    def sql(self, query, database=DATABASE):
        return self.cluster.sql(database, query)

    def settings(self, role, database=DATABASE, **override):
        return dict(self.env, JOBS_UPGRADE_DB_URL=self.cluster.url(database, role),
                    JOBS_UPGRADE_NAMESPACE=NAMESPACE, JOBS_UPGRADE_GATE=str(self.gate),
                    JOBS_UPGRADE_BATCH="0", **override)

    def migrate(self, binary, label, role=MIGRATOR, database=DATABASE):
        return parity.checked([str(binary), "migrate"], cwd=self.scratch, timeout=25,
                              env=self.settings(role, database), log=self.evidence / f"{label}.log")

    def reject(self, binary, action, label, message, env):
        result = subprocess.run([str(binary), action], cwd=self.scratch, env=env,
                                text=True, capture_output=True, timeout=25)
        (self.evidence / f"{label}.log").write_text(result.stdout + result.stderr)
        require(result.returncode == 1 and message in result.stderr and "READY" not in result.stdout,
                f"expected {message!r}; see {label}.log")
        return message

    @contextlib.contextmanager
    def runner(self, binary, role, queues, label, batch=False, *, database=DATABASE):
        env = self.settings(role, database, JOBS_UPGRADE_QUEUES=json.dumps(queues))
        env["JOBS_UPGRADE_BATCH"] = "1" if batch else "0"
        log = self.evidence / f"{label}.log"
        with log.open("wb") as stream:
            process = parity.RunnerProcess([str(binary), "run"], cwd=self.scratch, env=env,
                                           start_new_session=True, stdin=subprocess.DEVNULL,
                                           stdout=stream, stderr=stream)
        completed = False
        try:
            parity.wait_for(lambda: parity.runner_ready([process], [log]),
                            time.monotonic() + 10, f"{label} startup")
            yield process
            completed = True
        finally:
            if not completed:
                self.gate.touch()
            parity.stop_runners([process], require_clean=completed)
        summary = json.loads(log.read_text().splitlines()[-1])
        require(summary["drained"] and summary["stale_acks"] == 0 and summary.get("last_error") is None,
                f"unhealthy runner shutdown; see {log}")

    def enqueue(self, label, queue, *, schedule=None, database=DATABASE):
        spec = {
            "worker": "upgrade", "queue": queue, "args": {"label": label},
            "request_key": label, "max_attempts": 3, "timeout_ms": 120000,
        }
        if schedule is not None:
            spec["scheduled_at_ms"] = schedule
        path = self.scratch / f"{label}.json"
        parity.write_json(path, spec)
        return json.loads(parity.checked(
            [str(self.old), "enqueue"], cwd=self.scratch, timeout=20,
            env=self.settings(PRODUCER, database, JOBS_UPGRADE_SPEC=str(path)),
            log=self.evidence / f"enqueue-{label}.log",
        ))["id"]

    def prepare(self, database):
        self.sql(f"CREATE DATABASE {database} OWNER {MIGRATOR}", "postgres")
        self.sql(f"""
SET ROLE {MIGRATOR};
ALTER DEFAULT PRIVILEGES REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;
""", database)

    def apply_current_grants(self, database=DATABASE):
        cluster = self.cluster
        env = dict(self.env, PGPASSWORD=cluster.password,
                   PGSSLMODE="verify-full" if cluster.expected_version else "disable")
        if cluster.expected_version:
            env["PGSSLROOTCERT"] = str(cluster.certificate)
        args = [str(cluster.bin / "psql"), "-Xqw", "-v", "ON_ERROR_STOP=1",
                "-h", "127.0.0.1", "-p", str(cluster.port), "-U", MIGRATOR, "-d", database,
                "-v", f"producer={PRODUCER}", "-v", f"worker={NEW_WORKER}",
                "-v", f"operator={OPERATOR}", "-v", f"inspector={INSPECTOR}", "-f", str(GRANTS)]
        parity.checked(args, env=env, timeout=15, log=self.evidence / f"{database}-grants.log")

    def setup(self):
        for role in ROLES:
            self.sql(f"CREATE ROLE {role} LOGIN PASSWORD '{self.cluster.password}'", "postgres")
        self.prepare(DATABASE)
        self.migrate(self.old, "migrate-legacy")
        self.grant_legacy(DATABASE)

    def grant_legacy(self, database):
        self.sql(f"""
SET ROLE {MIGRATOR};
CREATE TABLE public.upgrade_attempts (
    job_id uuid NOT NULL, effect_key text NOT NULL, attempt integer NOT NULL, worker_role name NOT NULL
);
CREATE TABLE public.upgrade_effects (
    effect_key text PRIMARY KEY, job_id uuid NOT NULL, worker_role name NOT NULL
);
GRANT INSERT ON public.upgrade_attempts, public.upgrade_effects TO {OLD_WORKER}, {NEW_WORKER};
GRANT USAGE ON SCHEMA mesh_jobs TO {PRODUCER}, {OLD_WORKER};
GRANT SELECT, INSERT (namespace), UPDATE (fence) ON mesh_jobs.admissions TO {PRODUCER};
GRANT SELECT, INSERT (namespace, name) ON mesh_jobs.queues TO {PRODUCER};
GRANT SELECT, INSERT ON mesh_jobs.receipts TO {PRODUCER};
GRANT SELECT ON mesh_jobs.jobs TO {PRODUCER};
GRANT INSERT (namespace, worker, version, queue, args, metadata, tags, state, priority,
    max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms, inserted_at_ms, scheduled_at_ms,
    unique_key) ON mesh_jobs.jobs TO {PRODUCER};
GRANT EXECUTE ON FUNCTION mesh_jobs.enqueue(text,jsonb,text,text), mesh_jobs.now_ms(),
    mesh_jobs.unique_values(jsonb,jsonb), mesh_jobs.effect_key(mesh_jobs.jobs) TO {PRODUCER};
GRANT SELECT ON mesh_jobs.schema_version, mesh_jobs.jobs, mesh_jobs.queues TO {OLD_WORKER};
GRANT UPDATE (concurrency) ON mesh_jobs.queues TO {OLD_WORKER};
GRANT UPDATE (state, attempt, attempted_at_ms, scheduled_at_ms, metadata, finished_at_ms,
    lease_token, lease_expires_at_ms, cancel_requested_at_ms, errors) ON mesh_jobs.jobs TO {OLD_WORKER};
-- The pinned runner always polls cron. Permit its empty-calendar heartbeat and row locks.
GRANT SELECT, INSERT ON mesh_jobs.cron_activation TO {OLD_WORKER};
GRANT UPDATE (epoch, activated_at_ms, activated_by, seen_at_ms) ON mesh_jobs.cron_activation TO {OLD_WORKER};
GRANT SELECT ON mesh_jobs.cron_schedules TO {OLD_WORKER};
GRANT UPDATE (cursor_minute, reboot_epoch) ON mesh_jobs.cron_schedules TO {OLD_WORKER};
GRANT EXECUTE ON FUNCTION mesh_jobs.claim(text,text,jsonb,bigint), mesh_jobs.now_ms(),
    mesh_jobs.effect_key(mesh_jobs.jobs), mesh_jobs.try_renew(text,uuid,uuid,bigint),
    mesh_jobs.settle_state(text,uuid,uuid,jsonb,boolean), mesh_jobs.settle(text,uuid,uuid,jsonb,boolean),
    mesh_jobs.recover(text,integer,jsonb) TO {OLD_WORKER};
INSERT INTO mesh_jobs.queues (namespace,name,concurrency) VALUES ('{NAMESPACE}','legacy',2);
""", database)
        self.no_public_execute(database)

    def no_public_execute(self, database=DATABASE):
        count = self.sql("""
SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
CROSS JOIN LATERAL aclexplode(coalesce(p.proacl, acldefault('f',p.proowner))) a
WHERE n.nspname='mesh_jobs' AND a.grantee=0 AND a.privilege_type='EXECUTE'
""", database)
        require(count == "0", f"PUBLIC has EXECUTE on Jobs functions in {database}")
        return int(count)

    def old_grants(self, database=DATABASE):
        rows = self.sql(f"""
SET ROLE {OLD_WORKER};
SELECT 'schema:USAGE' WHERE has_schema_privilege(current_user, 'mesh_jobs', 'USAGE')
UNION ALL
SELECT table_schema || '.' || table_name || '.' || column_name || ':' || privilege_type
  FROM information_schema.column_privileges WHERE grantee=current_user
    AND table_schema IN ('mesh_jobs','public')
UNION ALL
SELECT table_schema || '.' || table_name || ':' || privilege_type
  FROM information_schema.table_privileges WHERE grantee=current_user
    AND table_schema IN ('mesh_jobs','public')
UNION ALL
SELECT oid::regprocedure::text FROM pg_proc WHERE pronamespace='mesh_jobs'::regnamespace
    AND has_function_privilege(current_user,oid,'EXECUTE')
ORDER BY 1
""", database)
        return rows.splitlines()

    def check_roles(self):
        flags = json.loads(self.sql(f"""
SELECT json_build_object(
    'worker_can_batch', has_function_privilege('{NEW_WORKER}','mesh_jobs.{BATCH}','EXECUTE'),
    'legacy_cannot_batch', NOT has_function_privilege('{OLD_WORKER}','mesh_jobs.{BATCH}','EXECUTE'),
    'producer_cannot_claim', NOT has_function_privilege('{PRODUCER}',
        'mesh_jobs.claim(text,text,jsonb,bigint)','EXECUTE'),
    'worker_cannot_pause_namespace', NOT has_function_privilege('{NEW_WORKER}',
        'mesh_jobs.set_namespace_paused(text,boolean)','EXECUTE'),
    'operator_can_pause_namespace', has_function_privilege('{OPERATOR}',
        'mesh_jobs.set_namespace_paused(text,boolean)','EXECUTE'),
    'inspector_can_read_summary', has_table_privilege('{INSPECTOR}','mesh_jobs.job_summaries','SELECT'),
    'inspector_cannot_read_args', NOT has_column_privilege('{INSPECTOR}','mesh_jobs.jobs','args','SELECT'),
    'migration_role_owns_schema', (SELECT nspowner::regrole::text = '{MIGRATOR}'
        FROM pg_namespace WHERE nspname='mesh_jobs'),
    'lifecycle_roles_independent', NOT EXISTS (
        SELECT FROM pg_roles WHERE rolname IN ('{PRODUCER}','{OLD_WORKER}','{NEW_WORKER}','{OPERATOR}','{INSPECTOR}')
          AND (rolsuper OR rolcreaterole OR rolcreatedb OR pg_has_role(oid,'{MIGRATOR}','MEMBER')
            OR has_schema_privilege(oid,'mesh_jobs','CREATE')))
)
"""))
        require(all(flags.values()), f"Jobs roles were not scoped: {flags}")
        return flags

    def catalog(self, database=DATABASE):
        result = self.sql("""
WITH objects(kind, name, oid, definition, acl) AS (
    SELECT 'function', proname || '(' || pg_get_function_identity_arguments(p.oid) || ')',
        p.oid, md5(pg_get_functiondef(p.oid)), p.proacl::text
    FROM pg_proc p WHERE p.pronamespace='mesh_jobs'::regnamespace
    UNION ALL
    SELECT 'relation', relname, c.oid,
        CASE relkind WHEN 'i' THEN md5(pg_get_indexdef(c.oid))
                     WHEN 'v' THEN md5(pg_get_viewdef(c.oid)) END, c.relacl::text
    FROM pg_class c WHERE c.relnamespace='mesh_jobs'::regnamespace
    UNION ALL
    SELECT 'trigger', tgname, t.oid, md5(pg_get_triggerdef(t.oid)), NULL
    FROM pg_trigger t JOIN pg_class c ON t.tgrelid=c.oid
    WHERE c.relnamespace='mesh_jobs'::regnamespace AND NOT t.tgisinternal
)
SELECT json_object_agg(kind || ' ' || name, json_build_object(
    'oid', oid, 'definition_md5', definition, 'acl', acl))
FROM objects
""", database)
        return json.loads(result)

    def rows(self, database=DATABASE):
        return json.loads(self.sql(f"""
SELECT coalesce(json_object_agg(args->>'label', json_build_object(
    'id',id, 'run',run, 'position',position, 'queue',queue, 'state',state,
    'attempt',attempt, 'max_attempts',max_attempts, 'scheduled_at_ms',scheduled_at_ms,
    'attempted_at_ms',attempted_at_ms, 'finished_at_ms',finished_at_ms,
    'lease_token',lease_token, 'lease_expires_at_ms',lease_expires_at_ms, 'errors',errors,
    'effect_key',mesh_jobs.effect_key(j)
)), '{{}}') FROM mesh_jobs.jobs j WHERE namespace='{NAMESPACE}'
""", database))

    def writes(self, database=DATABASE):
        return json.loads(self.sql("""
SELECT json_build_object(
    'attempts', (SELECT coalesce(json_agg(a ORDER BY job_id, attempt), '[]') FROM public.upgrade_attempts a),
    'effects', (SELECT coalesce(json_agg(e ORDER BY job_id), '[]') FROM public.upgrade_effects e)
)
""", database))

    def request_receipts(self):
        return json.loads(self.sql(f"""
SELECT json_object_agg(request_key, receipt->>'id') FROM mesh_jobs.receipts WHERE namespace='{NAMESPACE}'
"""))

    def wait_row(self, label, state, *processes, database=DATABASE):
        def observe():
            require(all(p.poll() is None for p in processes),
                    f"a runner exited before {label} became {state}")
            row = self.rows(database).get(label)
            return row if row and row["state"] == state else None

        return parity.wait_for(observe, time.monotonic() + 35, f"{label} {state}")

    def lifecycle_calls(self):
        return json.loads(self.sql(f"""
WITH counts AS (
    SELECT rolname, sum(calls) FILTER (WHERE query LIKE '%FROM mesh_jobs.claim(%') AS claim,
        sum(calls) FILTER (WHERE query LIKE '%FROM mesh_jobs.claim_batch_v1(%') AS batch,
        sum(calls) FILTER (WHERE query LIKE 'SELECT mesh_jobs.try_renew(%') AS renew,
        sum(calls) FILTER (WHERE query LIKE 'SELECT coalesce(mesh_jobs.settle_state(%') AS settle
    FROM pg_stat_statements JOIN pg_roles ON userid=pg_roles.oid
    WHERE dbid=(SELECT oid FROM pg_database WHERE datname='{DATABASE}')
      AND rolname IN ('{OLD_WORKER}','{NEW_WORKER}')
    GROUP BY rolname
)
SELECT json_object_agg(rolname, json_build_object('claim',coalesce(claim,0),
    'batch',coalesce(batch,0), 'renew',coalesce(renew,0), 'settle',coalesce(settle,0))) FROM counts
""", "postgres"))

    def owner_replay(self):
        database = "owner_migration_replay"
        self.prepare(database)
        self.migrate(self.old, "replay-initial-legacy", database=database)
        self.grant_legacy(database)
        legacy = self.catalog(database)
        grants = self.old_grants(database)
        self.migrate(self.new, "replay-upgrade", database=database)
        self.apply_current_grants(database)
        upgraded = self.catalog(database)
        legacy_functions = sorted(key for key in legacy if key.startswith("function "))
        changed = [key for key in legacy_functions if key not in upgraded or any(
            legacy[key][field] != upgraded[key][field] for field in ("oid", "definition_md5")
        )]
        require(not changed, f"upgrade replaced legacy functions: {changed}")
        additive = sorted(upgraded.keys() - legacy.keys())
        require(any(name.startswith("function claim_batch_v1(") for name in additive),
                "the current migrator did not install the batch entry point")

        workers = json.dumps([{"worker": "upgrade", "version": 1}])
        held = self.enqueue("replay-held", "legacy", database=database)
        claim = json.loads(self.sql(f"""
SET ROLE {OLD_WORKER};
SELECT json_build_object('id',id,'token',token)
FROM mesh_jobs.claim('{NAMESPACE}','legacy','{workers}',60000)
""", database))
        require(claim["id"] == held and claim["token"], "legacy claim did not take the held job")
        lease = self.rows(database)["replay-held"]
        existing = self.enqueue("replay-existing", "legacy", database=database)
        self.sql(f"""
SET ROLE {OPERATOR};
SELECT mesh_jobs.queue_control('{NAMESPACE}','disabled','configure',3);
SELECT mesh_jobs.queue_control('{NAMESPACE}','disabled','pause',NULL);
SELECT mesh_jobs.set_namespace_paused('{NAMESPACE}',true);
""", database)
        self.migrate(self.old, "replay-owner-legacy", database=database)
        replayed = self.catalog(database)
        require(replayed == upgraded and self.old_grants(database) == grants,
                "old owner replay changed an upgraded object or legacy role grant")

        new = self.enqueue("replay-new", "created-while-paused", database=database)
        queued = {"replay-existing": existing, "replay-new": new}
        queue_names = {"legacy": 2, "created-while-paused": 1}
        before = self.rows(database)
        for queue in queue_names:
            self.sql(f"""
SET ROLE {OLD_WORKER};
DO $$ BEGIN
    PERFORM id FROM mesh_jobs.claim('{NAMESPACE}','{queue}','{workers}',60000);
    RAISE EXCEPTION 'legacy claim escaped the pause';
EXCEPTION WHEN object_not_in_prerequisite_state THEN
    IF SQLERRM <> 'mesh jobs namespace admission is paused or changing' THEN RAISE; END IF;
END $$;
""", database)
            empty = self.sql(f"""
SET ROLE {NEW_WORKER};
SELECT count(*) FROM mesh_jobs.claim_drain_v1('{NAMESPACE}','{queue}','{workers}',60000,NULL,NULL);
SELECT count(*) FROM mesh_jobs.claim_batch_v1('{NAMESPACE}','{queue}','{workers}',60000,1,NULL);
""", database)
            require(empty.splitlines() == ["0", "0"], "current claims escaped the pause")
        require(self.rows(database) == before, "a paused claim changed durable job state")

        renewed = self.sql(f"""
SET ROLE {OLD_WORKER};
SELECT mesh_jobs.try_renew('{NAMESPACE}','{held}','{claim["token"]}',120000)
""", database)
        require(renewed == "active", "legacy renewal failed while paused")
        extended = self.rows(database)["replay-held"]
        self.same_claim(lease, extended)
        require(extended["lease_expires_at_ms"] > lease["lease_expires_at_ms"],
                "legacy renewal did not extend the original lease")
        settled = self.sql(f"""
SET ROLE {OLD_WORKER};
SELECT mesh_jobs.settle_state('{NAMESPACE}','{held}','{claim["token"]}',
    '{{"action":"complete"}}',false)
""", database)
        require(settled == "completed", "legacy settlement failed while paused")

        queues = f"""
SELECT jsonb_object_agg(name, jsonb_build_array(state,concurrency))
FROM mesh_jobs.queues WHERE namespace='{NAMESPACE}' AND name <> ''
"""
        controls = json.loads(self.sql(queues, database))
        require(controls == {"legacy": ["active", 2], "disabled": ["paused", 3],
                             "created-while-paused": ["active", 1]},
                "namespace pause changed a queue's state or limit")
        self.sql(f"""SET ROLE {OPERATOR};
SELECT mesh_jobs.set_namespace_paused('{NAMESPACE}',false)
""", database)
        require(json.loads(self.sql(queues, database)) == controls, "resume changed a queue's controls")
        with self.runner(self.old, OLD_WORKER, queue_names, "replay-old-runner", database=database) as runner:
            for label in queued:
                self.wait_row(label, "completed", runner, database=database)
        final = self.rows(database)
        for label, ident in queued.items():
            row = final[label]
            require(row["id"] == ident and row["state"] == "completed" and row["attempt"] == 1
                    and row["effect_key"] == before[label]["effect_key"] and row["lease_token"] is None,
                    "resumed legacy work lost its job or effect identity")
        writes = self.writes(database)
        require(writes["attempts"] == sorted([
            dict(job_id=ident, effect_key=final[label]["effect_key"], attempt=1, worker_role=OLD_WORKER)
            for label, ident in queued.items()
        ], key=lambda row: row["job_id"]) and writes["effects"] == sorted([
            dict(job_id=ident, effect_key=final[label]["effect_key"], worker_role=OLD_WORKER)
            for label, ident in queued.items()
        ], key=lambda row: row["job_id"]), "resumed legacy work omitted or duplicated a write")
        return {
            "legacy_function_definitions": {key: legacy[key]["definition_md5"] for key in legacy_functions},
            "old_owner_migration_exit": 0, "additive_objects_preserved": additive,
            "upgraded_catalog_and_legacy_grants_unchanged": True,
            "paused_claims_fenced": ["legacy", "current_single", "current_batch"],
            "held_lease_renewed_and_settled_while_paused": True,
            "queues_after_resume": controls, "resumed_jobs": queued, "resumed_writes": writes,
            "public_execute_count": self.no_public_execute(database),
        }

    def run(self):
        self.setup()
        grants = self.old_grants()
        ids = {"retryable": self.enqueue("retryable", "retry")}
        with self.runner(self.old, OLD_WORKER, {"retry": 1}, "old-retry-seed") as seed:
            retry = self.wait_row("retryable", "retryable", seed)
        require(retry["attempt"] == 1 and len(retry["errors"]) == 1
                and retry["scheduled_at_ms"] > retry["attempted_at_ms"],
                "the pinned worker did not seed a durable retry")

        ids["held"] = self.enqueue("held", "legacy")
        with self.runner(self.old, OLD_WORKER, {"legacy": 2}, "old-live") as old_runner:
            self.wait_row("held", "executing", old_runner)
            parity.wait_for(lambda: any(a["job_id"] == ids["held"] for a in self.writes()["attempts"]),
                            time.monotonic() + 15, "entry of the held old handler")
            due = int(self.sql("SELECT mesh_jobs.now_ms()")) + 15000
            ids["scheduled"] = self.enqueue("scheduled", "next", schedule=due)
            before = self.rows()
            held = before["held"]
            require(before["scheduled"]["state"] == "scheduled"
                    and before["scheduled"]["scheduled_at_ms"] == due
                    and before["scheduled"]["attempt"] == 0 and before["retryable"] == retry
                    and held["attempt"] == 1 and held["lease_token"],
                    "pre-upgrade scheduled, retryable or held state differs from the seeded work")

            self.migrate(self.new, "migrate-current")
            self.apply_current_grants()
            upgraded = self.catalog()
            after = self.rows()
            require(all(after[label] == before[label] for label in ("scheduled", "retryable")),
                    "migration changed pending work")
            self.same_claim(held, after["held"])
            require(self.old_grants() == grants, "migration changed the old worker's grants")
            scope = self.check_roles()

            denied = self.reject(self.old, "migrate", "old-worker-migration-denied", "permission denied",
                                 self.settings(OLD_WORKER))
            require(self.catalog() == upgraded, "the old scoped worker's migration changed upgraded SQL")
            batch_denied = self.settings(OLD_WORKER, JOBS_UPGRADE_QUEUES='{"next":1}')
            batch_denied["JOBS_UPGRADE_BATCH"] = "1"
            self.reject(self.new, "run", "batch-with-legacy-grants-denied",
                        "needs EXECUTE on mesh_jobs.claim_batch_v1", batch_denied)
            self.reject(self.new, "run", "single-with-legacy-grants-denied",
                        "needs EXECUTE on mesh_jobs.claim_drain_v1",
                        self.settings(OLD_WORKER, JOBS_UPGRADE_QUEUES='{"legacy":1}'))

            with self.runner(self.new, NEW_WORKER, {"next": 1}, "new-live", batch=True) as new_runner:
                ids["mixed_old"] = self.enqueue("mixed_old", "legacy")
                self.wait_row("mixed_old", "completed", old_runner, new_runner)
                self.wait_row("scheduled", "completed", old_runner, new_runner)
                overlap = self.rows()["held"]
                self.same_claim(held, overlap)
                require(overlap["lease_expires_at_ms"] > held["lease_expires_at_ms"],
                        "held old claim did not renew while both binaries were active")

            rollback = self.rows()
            self.same_claim(held, rollback["held"])
            pending_retry = rollback["retryable"]
            # Namespace-wide recovery may promote a due retry even on a runner serving another queue.
            require(all(pending_retry[k] == retry[k] for k in retry if k != "state")
                    and (pending_retry["state"] == "retryable" or
                         (pending_retry["state"] == "available" and
                          retry["scheduled_at_ms"] <= int(self.sql("SELECT mesh_jobs.now_ms()")))),
                    "retryable identity, history, or eligibility changed before the old-only rollback")
            require(not any(e["job_id"] == ids["held"] for e in self.writes()["effects"]),
                    "the held job ran its effect before release")
            self.gate.touch()
            self.wait_row("held", "completed", old_runner)

        with self.runner(self.old, OLD_WORKER, {"retry": 1}, "old-rollback") as old_rollback:
            self.wait_row("retryable", "completed", old_rollback)
        final = self.rows()
        writes = self.writes()
        self.reconcile(ids, before, final, writes)
        receipts = self.request_receipts()
        require(receipts == ids and self.sql("SELECT count(*) FROM mesh_jobs.jobs") == str(len(ids)),
                "rollback lost request receipts or left extra job rows")
        require(self.catalog() == upgraded and self.old_grants() == grants,
                "worker-only rollback changed the upgraded catalog or the old grants")
        calls = self.lifecycle_calls()
        require(calls[OLD_WORKER]["claim"] >= 4 and calls[OLD_WORKER]["batch"] == 0
                and calls[OLD_WORKER]["renew"] > 0 and calls[OLD_WORKER]["settle"] >= 4
                and calls[NEW_WORKER]["batch"] > 0 and calls[NEW_WORKER]["claim"] == 0
                and calls[NEW_WORKER]["settle"] >= 1,
                f"worker lifecycle calls did not show both compiled paths: {calls}")
        return {
            "j2-lane-9": {
                "scoped_grants": scope, "public_execute_count": self.no_public_execute(),
                "old_grants_unchanged": True, "old_worker_migration_denied": denied,
                "current_batch_start_with_old_grants_denied": True,
                "current_single_start_with_old_grants_denied": True, "lifecycle_calls": calls,
                "owner_replay": self.owner_replay(),
            },
            "j7-lane-5": {
                "before": before, "after_migrate": after, "held_claim_after_concurrent_workers": overlap,
            },
            "j7-lane-6": {
                "after_new_worker_exit": rollback, "final": final, "writes": writes, "request_receipts": receipts,
                "upgraded_catalog_unchanged_by_worker_rollback": True,
            },
        }

    @staticmethod
    def same_claim(before, after):
        stable = ("id", "run", "attempt", "lease_token", "effect_key", "attempted_at_ms")
        require(after["state"] == "executing" and all(after[k] == before[k] for k in stable)
                and after["lease_expires_at_ms"] >= before["lease_expires_at_ms"],
                "old live attempt lost its original claim or effect identity")

    @staticmethod
    def reconcile(ids, before, final, writes):
        roles = {"held": OLD_WORKER, "retryable": OLD_WORKER, "scheduled": NEW_WORKER,
                 "mixed_old": OLD_WORKER}
        require(set(final) == set(ids), "rollback lost rows or introduced extra jobs")
        attempts, effects = [], []
        for label, job_id in ids.items():
            row = final[label]
            budget = 2 if label == "retryable" else 1
            require(row["id"] == job_id and row["state"] == "completed"
                    and row["attempt"] == budget and row["run"] == 1
                    and row["lease_token"] is None and row["lease_expires_at_ms"] is None,
                    f"unexpected terminal row for {label}: {row}")
            if label in before:
                original = before[label]
                require(all(row[k] == original[k] for k in
                            ("position", "queue", "scheduled_at_ms", "max_attempts", "effect_key", "errors"))
                        and row["attempted_at_ms"] >= original["scheduled_at_ms"],
                        f"{label} ran early or lost its original schedule, budget, or identity")
            effect = {"effect_key": f"{job_id}/1", "job_id": job_id, "worker_role": roles[label]}
            effects.append(effect)
            attempts.extend(dict(effect, attempt=attempt) for attempt in range(1, budget + 1))
        require(writes["attempts"] == sorted(attempts, key=lambda a: (a["job_id"], a["attempt"]))
                and writes["effects"] == sorted(effects, key=lambda e: e["job_id"]),
                "handler attempts and committed effects did not reconcile")


def record(args):
    ref = parity.checked(["git", "rev-parse", "HEAD"])
    require(ref != PINNED_MAIN, "the candidate and pinned binaries must be different revisions")
    base = ROOT / "target/jobs-upgrade" / ref
    base.mkdir(parents=True, exist_ok=True)
    evidence = Path(tempfile.mkdtemp(prefix="proof-", dir=base))
    print(evidence, flush=True)
    with parity.workspace(evidence) as scratch:
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("PG", "JOBS_UPGRADE_", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"))}
        env.update(TMPDIR=str(scratch), TZ="UTC", LC_ALL="C", MESH_SCHEDULER_MIN_WORKERS="2",
                   MESH_SCHEDULER_MAX_WORKERS="2", CARGO_BUILD_JOBS="2", CARGO_PROFILE_DEV_DEBUG="0",
                   CARGO_INCREMENTAL="0")
        rust = parity.checked(["rustc", "-vV"], env=env)
        target = next(line.removeprefix("host: ") for line in rust.splitlines() if line.startswith("host: "))
        old, old_build = build_fixture(PINNED_MAIN, scratch, evidence, env, target, args.toolchains)
        new, new_build = build_fixture(ref, scratch, evidence, env, target, args.toolchains)
        cluster = parity.Cluster(scratch, evidence, 24, args.postgres_version)
        try:
            cluster.start()
            metadata = {
                "receipt_version": 2, "result": "pass", "host": platform.system(),
                "rust_target": target, "rustc": rust.splitlines()[0],
                "postgres_version": cluster.sql("postgres", "SHOW server_version"),
                "postgres_version_num": int(cluster.sql("postgres", "SHOW server_version_num")),
                "tls": bool(args.postgres_version),
                "old": old_build, "new": new_build,
                "proof_source_sha256": {str(path.relative_to(ROOT)): parity.digest(path)
                                        for path in (SCRIPT, FIXTURE, GRANTS, Path(parity.__file__))},
            }
            lanes = RollingProof(cluster, scratch, evidence, env, old, new).run()
        finally:
            cluster.stop()
    for lane, observations in lanes.items():
        parity.write_json(evidence / f"{lane}.log", dict(metadata, observations=observations))
    receipt = dict(metadata, lanes=lanes)
    parity.write_json(evidence / "receipt-v2.json", receipt)
    if args.receipt:
        args.receipt.parent.mkdir(parents=True, exist_ok=True)
        parity.write_json(args.receipt, receipt)


def terminate(signum, _frame):
    raise SystemExit(128 + signum)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--postgres-version", help="require this patch version and verified TLS")
    parser.add_argument("--toolchains", type=Path,
                        help="trusted prebuilt compiler/runtime pairs at <dir>/<SHA>/artifacts")
    parser.add_argument("--receipt", type=Path, help="also write the passing receipt to this new path")
    args = parser.parse_args()
    if args.toolchains:
        args.toolchains = args.toolchains.resolve()
    if args.receipt:
        args.receipt = args.receipt.resolve()
        if not args.receipt.is_relative_to(ROOT) or args.receipt.exists():
            parser.error("--receipt must name a new file inside this checkout")
    signal.signal(signal.SIGTERM, terminate)
    try:
        record(args)
    except (parity.FixtureError, OSError, subprocess.SubprocessError) as error:
        sys.exit(str(error))


if __name__ == "__main__":
    main()
