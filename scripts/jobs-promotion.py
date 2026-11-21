#!/usr/bin/env python3
"""Prove J7 lane 7 using a compiled Mesh fixture and two disposable PostgreSQL servers."""

import argparse
import contextlib
import errno
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import uuid
from urllib.parse import quote

sys.dont_write_bytecode = True
from lib.jobs_parity_fault import SettlementProxy

spec = importlib.util.spec_from_file_location("jobs_parity", Path(__file__).with_name("jobs-parity.py"))
parity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parity)

ENGINE = "mesh"
LEASE_MS = 1500  # The compiled parity fixture's "fault" mode.
POLL_MS = 250
RECOVERY_MS = max(100, min(POLL_MS, 1000))  # Runner maintenance interval.
RESUME_MS = LEASE_MS + 2 * RECOVERY_MS


def require(condition, message):
    if not condition:
        raise parity.FixtureError(message)


def report(log, event, **details):
    print(event, json.dumps(details, sort_keys=True), file=log, flush=True)


class Primary(parity.Cluster):
    def configure_tls(self):
        super().configure_tls()
        with (self.data / "pg_hba.conf").open("a") as hba:
            hba.write("hostssl replication parity_replica 127.0.0.1/32 scram-sha-256\n")
            # The one-shot protocol proxy needs to see the frontend and backend frames.
            hba.write("hostnossl mesh parity_subject 127.0.0.1/32 scram-sha-256\n")


def start_standby(primary, standby):
    require(primary.port != standby.port, "disposable PostgreSQL ports collided")
    standby.password = primary.password
    standby.nonce = primary.nonce
    standby.certificate = primary.certificate
    primary.sql("postgres", f"CREATE ROLE parity_replica REPLICATION LOGIN PASSWORD '{primary.password}'")
    passfile = standby.scratch / "replication.pgpass"
    passfile.touch(mode=0o600)
    passfile.write_text(f"127.0.0.1:{primary.port}:replication:parity_replica:{primary.password}\n")
    replication_env = dict(primary.env, PGPASSFILE=str(passfile), PGSSLMODE="verify-full",
                           PGSSLROOTCERT=str(primary.certificate), PGAPPNAME="j7_standby")
    parity.checked([
        str(primary.bin / "pg_basebackup"), "-D", str(standby.data), "-h", "127.0.0.1",
        "-p", str(primary.port), "-U", "parity_replica", "-w", "-X", "stream", "--checkpoint=fast",
    ], env=replication_env, timeout=60, log=standby.evidence / "basebackup.log")
    # Put the TLS and credential-file policy in recovery config as well as in pg_basebackup.
    conninfo = (
        f"postgresql://parity_replica@127.0.0.1:{primary.port}/?"
        f"sslmode=verify-full&application_name=j7_standby&passfile={quote(str(passfile), safe='')}"
        f"&sslrootcert={quote(str(primary.certificate), safe='')}"
    )
    (standby.data / "standby.signal").touch()
    with (standby.data / "postgresql.auto.conf").open("a") as config:
        config.write(f"\nprimary_conninfo = '{conninfo}'\n")
    with (standby.data / "postgresql.conf").open("a") as config:
        config.write(f"\nport = {standby.port}\n")
    (standby.data / "pg_hba.conf").write_text(
        "hostssl all all 127.0.0.1/32 scram-sha-256\n"
    )
    standby.call("pg_ctl", "-D", str(standby.data), "-w", "-t", "20", "-l",
                 str(standby.evidence / "postgres.log"), "start")


def streaming(primary):
    return primary.sql("postgres", """
SELECT count(*) = 1
FROM pg_stat_replication JOIN pg_stat_ssl USING (pid)
WHERE application_name = 'j7_standby' AND usename = 'parity_replica'
  AND client_addr = '127.0.0.1' AND state = 'streaming' AND sync_state = 'async' AND ssl
""") == "t"


def preserve_server_logs(primary, standby, evidence):
    for label, cluster in (("primary", primary), ("standby", standby)):
        source = cluster.evidence / "postgres.log"
        if source.is_file():
            content = source.read_text(errors="replace").replace(primary.password, "[redacted]")
            (evidence / f"postgres-{label}.log").write_text(content)


def replay_barrier(primary, standby):
    # The fixture writer is stopped and its sessions are gone before this sample.
    lsn = primary.sql("postgres", "SELECT pg_current_wal_flush_lsn()")
    require(primary.sql("postgres", "SHOW synchronous_commit") == "on", "local commits must flush")
    require(primary.sql("postgres", """
SELECT count(*) = 4 FROM pg_authid
WHERE rolname IN ('parity_owner', 'parity_subject', 'parity_producer', 'parity_replica')
  AND rolpassword LIKE 'SCRAM-SHA-256$%'
""") == "t", "expected SCRAM verifiers")
    observed = parity.wait_for(
        lambda: standby.sql("postgres", f"""
SELECT pg_last_wal_replay_lsn()
WHERE pg_is_in_recovery() AND pg_last_wal_replay_lsn() >= '{lsn}'::pg_lsn
"""), time.monotonic() + 15, f"standby replay through {lsn}",
    )
    require(streaming(primary), "standby was not streaming over TLS at the replay barrier")
    return lsn, observed


def fenced(primary, pid):
    status = subprocess.run([str(primary.bin / "pg_ctl"), "-D", str(primary.data), "status"],
                            env=primary.env, capture_output=True, timeout=5)
    require(status.returncode == 3, "old PostgreSQL data directory still has a running server")
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        pass
    else:
        raise parity.FixtureError("old PostgreSQL postmaster is still alive")
    with socket.socket() as caller:
        caller.settimeout(1)
        require(caller.connect_ex(("127.0.0.1", primary.port)) == errno.ECONNREFUSED,
                "old primary's TCP endpoint did not refuse the connection")


def promote(standby):
    # The start of the last read-only sample bounds readiness from below. Use it
    # for the deadline so polling and connection startup cannot extend the budget.
    lower = time.monotonic()
    standby.call("pg_ctl", "-D", str(standby.data), "-W", "promote")

    def writable():
        nonlocal lower
        sampled = time.monotonic()
        state = standby.sql("postgres", """
SELECT pg_is_in_recovery() OR current_setting('transaction_read_only')::boolean
""")
        if state == "f":
            return lower, time.monotonic()
        require(state == "t", f"unexpected standby readiness sample {state!r}")
        lower = sampled
        return None

    return parity.wait_for(writable, time.monotonic() + 15, "writable promoted standby", interval=0.025)


def build_fixture(meshc, mesh_rt, scratch, evidence, env):
    project = scratch / "fixture"
    shutil.copytree(parity.MESH_SOURCE, project)
    target = next(line.removeprefix("host: ") for line in parity.checked(["rustc", "-vV"], env=env).splitlines()
                  if line.startswith("host: "))
    parity.checked(["cc", "-std=c11", "-O2", "-c", "worker_identity.c", "-o", "worker_identity.o"],
                   cwd=project, env=env, log=evidence / "native.log")
    parity.checked(["ar", "rcs", "libworker_identity.a", "worker_identity.o"], cwd=project, env=env)
    manifest = (project / "mesh.toml.in").read_text()
    manifest = manifest.replace("@TARGET@", target).replace("@SHA256@", parity.digest(project / "libworker_identity.a"))
    (project / "mesh.toml").write_text(manifest)
    executable = scratch / "mesh-parity-fixture"
    parity.checked([str(meshc), "build", str(project), "--target", target, "--output", str(executable),
                    "--opt-level", "0"], env=dict(env, MESH_RT_LIB_PATH=str(mesh_rt)),
                   timeout=180, log=evidence / "mesh-fixture-build.log")
    return executable


def snapshot(cluster, namespace):
    relation, scope = parity.job_relation(ENGINE, namespace)
    observed = json.loads(cluster.sql(ENGINE, f"""
SELECT json_build_object(
  'jobs', (SELECT coalesce(json_agg(json_build_object(
    'key', j.args->>'key', 'id', j.id, 'effect_key', mesh_jobs.effect_key(j), 'state', j.state,
    'attempt', j.attempt, 'token', j.lease_token, 'expires_ms', j.lease_expires_at_ms,
    'finished_ms', j.finished_at_ms, 'errors', j.errors)), '[]')
    FROM {relation} j WHERE {scope}),
  'attempts', (SELECT coalesce(json_agg(json_build_array(job_key, attempt, effect_key)
    ORDER BY job_key, attempt), '[]') FROM public.mesh_jobs_parity_attempts WHERE {scope}),
  'effects', (SELECT coalesce(json_object_agg(job_key, json_build_array(effect_key, effected_at)), '{{}}')
    FROM public.mesh_jobs_parity_effects WHERE {scope}),
  'receipts', (SELECT coalesce(json_object_agg(request_key, receipt), '{{}}')
    FROM mesh_jobs.receipts WHERE {scope}),
  'business', (SELECT coalesce(json_agg(request_id ORDER BY request_id), '[]')
    FROM public.mesh_jobs_parity_product WHERE {scope})
)
"""))
    jobs = {job.pop("key"): job for job in observed["jobs"]}
    require(len(jobs) == len(observed["jobs"]), "duplicate job keys in fixture")
    observed["jobs"] = jobs
    return observed


def enqueue(subject, env, scratch, evidence, request, jobs):
    batch = {"request_id": request, "rollback": False, "jobs": [
        {"key": key, "payload": "j7", "hold_ms": hold, "fail_first": fail_first}
        for key, hold, fail_first in jobs
    ]}
    path = parity.batch_files(scratch, [batch])[0]
    receipt, committed_ms = subject.enqueue_receipt(env, path, evidence / f"enqueue-{request}.log")
    ids = receipt["ids"]
    require(set(ids) == {key for key, _, _ in jobs} and committed_ms is not None,
            f"{request} enqueue did not commit")
    return ids


def kill_runner(process):
    parity.remember_workers([process], parity.process_table())
    os.kill(process.pid, signal.SIGKILL)
    require(process.wait(timeout=10) == -signal.SIGKILL, "old Mesh runner did not die from SIGKILL")
    parity.wait_for_worker_exit(process.observed_workers, parity.fixture_executables([process]))


def assert_original(snapshot):
    jobs = snapshot["jobs"]
    expected = {"ambiguous": ("completed", 1), "interrupted": ("executing", 1), "queued": ("available", 0)}
    require({key: (row["state"], row["attempt"]) for key, row in jobs.items()} == expected,
            f"unexpected queue at failover: {jobs}")
    require(jobs["interrupted"]["token"] and jobs["interrupted"]["expires_ms"],
            "missing original lease token or expiry")
    require(len({job["id"] for job in jobs.values()}) == len(jobs), "duplicate original job IDs")
    effects = {key: jobs[key]["effect_key"] for key in ("ambiguous", "interrupted")}
    require({key: effect[0] for key, effect in snapshot["effects"].items()} == effects,
            "effects were missing or mismatched before failover")
    require(snapshot["attempts"] == [[key, 1, effect] for key, effect in effects.items()],
            "unexpected handler entries before failover")
    receipts = {key: (row["id"], row["effect_key"]) for key, row in snapshot["receipts"].items()}
    require(receipts == {key: (job["id"], job["effect_key"]) for key, job in jobs.items()},
            "enqueue receipts were missing or mismatched")
    require(snapshot["business"] == ["ambiguous", "interrupted", "queued"], "committed business rows missing")


def before_promotion(primary, subject, env, namespace, scratch, evidence, log):
    require(primary.owned(ENGINE), "fault injection requires the driver's ownership nonce")
    enqueued = enqueue(subject, env, scratch, evidence, "ambiguous", [("ambiguous", 0, False)])
    cut = {"placement": "after_commit", "query_forwarded": True, "command_complete": True, "ready_status": "I"}
    runner_log = evidence / "old-runner.log"
    with SettlementProxy(primary.port, lambda sql: "coalesce(mesh_jobs.settle_state(" in sql,
                         "after_commit") as proxy:
        proxy_url = f"postgresql://parity_subject:{primary.password}@127.0.0.1:{proxy.port}/mesh?sslmode=disable"
        runner = subject.run(dict(env, JOBS_PARITY_DB_URL=proxy_url), 0, runner_log)
        try:
            deadline = time.monotonic() + 20
            require(parity.wait_for(lambda: proxy.fault, deadline, "withheld settlement reply") == cut,
                    "proxy did not withhold a confirmed autocommit reply")
            ambiguous = snapshot(primary, namespace)["jobs"]["ambiguous"]
            require(ambiguous["state"] == "completed" and ambiguous["attempt"] == 1,
                    "the primary did not persist the ambiguous settlement")
            report(log, "settlement_reply_withheld", fault=cut, job=ambiguous)
            enqueued.update(enqueue(subject, env, scratch, evidence, "interrupted", [("interrupted", 30_000, True)]))
            parity.wait_for(lambda: snapshot(primary, namespace)["effects"].get("interrupted"),
                            deadline, "committed effect from interrupted handler")
            enqueued.update(enqueue(subject, env, scratch, evidence, "queued", [("queued", 0, False)]))
            require(runner.poll() is None, "old Mesh runner exited before the cut")
            kill_runner(runner)
        finally:
            if runner.poll() is None:
                kill_runner(runner)
    parity.wait_for(lambda: primary.sql("postgres", """
SELECT count(*) = 0 FROM pg_stat_activity
WHERE datname = 'mesh' AND usename IN ('parity_subject', 'parity_producer')
""") == "t", time.monotonic() + 10, "old writer sessions to disappear")
    original = snapshot(primary, namespace)
    assert_original(original)
    require({key: row["id"] for key, row in original["jobs"].items()} == enqueued,
            "primary job IDs differ from the enqueue replies")
    require(original["jobs"]["ambiguous"] == ambiguous, "ambiguous settlement changed on the old primary")
    report(log, "old_primary_snapshot", **original)
    return original


def recover(standby, subject, env, namespace, scratch, evidence, log, original, lower_ready):
    gate = scratch / "let-replacement-settle"
    runner_log = evidence / "new-runner.log"
    runner = subject.run(dict(env, JOBS_PARITY_RETRY_GATE=str(gate)), 1, runner_log)
    success = False
    try:
        def queued_entered():
            observed = snapshot(standby, namespace)
            return ["queued", 1, original["jobs"]["queued"]["effect_key"]] in observed["attempts"]

        parity.wait_for(queued_entered, lower_ready + RESUME_MS / 1000,
                        f"queued handler entry within {RESUME_MS} ms of writable", interval=0.025)
        elapsed_ms = (time.monotonic() - lower_ready) * 1000
        require(elapsed_ms <= RESUME_MS, f"first resumed handler observed too late: {elapsed_ms:.1f} ms")
        report(log, "eligible_work_resumed", elapsed_upper_ms=round(elapsed_ms, 1), budget_ms=RESUME_MS)

        def reclaimed():
            observed = snapshot(standby, namespace)
            value = observed["jobs"]["interrupted"]
            second_entry = ["interrupted", 2, value["effect_key"]]
            if value["state"] == "executing" and value["attempt"] == 2 and second_entry in observed["attempts"]:
                return value
            return None

        replacement = parity.wait_for(reclaimed, time.monotonic() + 10, "active replacement with a new lease")
        interrupted = original["jobs"]["interrupted"]
        require(replacement["id"] == interrupted["id"] and replacement["effect_key"] == interrupted["effect_key"]
                and replacement["token"] and replacement["token"] != interrupted["token"],
                "replacement did not preserve job/effect identity and rotate its token")
        job_id, token = uuid.UUID(interrupted["id"]), uuid.UUID(interrupted["token"])
        stale = standby.sql(ENGINE, f"""
SET ROLE parity_subject;
SELECT coalesce(mesh_jobs.settle_state('{namespace}', '{job_id}', '{token}',
    '{{"action":"complete"}}'::jsonb, false), 'stale')
""")
        active = reclaimed()
        require(stale == "stale" and active and active["id"] == replacement["id"]
                and active["token"] == replacement["token"],
                "obsolete settlement token affected the active replacement")
        require(standby.sql(ENGINE, """
SELECT count(*) > 0 AND bool_and(ssl) FROM pg_stat_ssl JOIN pg_stat_activity USING (pid)
WHERE usename = 'parity_subject' AND datname = current_database()
""") == "t", "replacement runner must have live TLS sessions")
        report(log, "old_token_rejected", old_token=str(token), replacement_token=active["token"], job_id=str(job_id))
        gate.touch()

        parity.wait_for(lambda: standby.observe(ENGINE, namespace)["terminal"] == 3,
                        time.monotonic() + 10, "all promoted jobs to settle")
        success = True
    finally:
        gate.touch(exist_ok=True)
        parity.stop_runners([runner], require_clean=success)

    if env.get("JOBS_PARITY_MESH_LISTEN") == "1":
        wakeups = parity.mesh_shutdown_report(runner_log).get("wakeups", {})
        require(wakeups.get("subscription_scans", 0) > 0,
                "replacement listener did not scan after subscribing on the promoted primary")
        report(log, "promoted_listener_caught_up", **wakeups)
    final = snapshot(standby, namespace)
    jobs = final["jobs"]
    require(jobs.keys() == original["jobs"].keys(), "job manifest changed across promotion")
    require(final["business"] == original["business"] and final["receipts"] == original["receipts"],
            "promotion changed committed business rows or job receipts")
    attempts = {"ambiguous": 1, "interrupted": 2, "queued": 1}
    for key, old in original["jobs"].items():
        current = jobs[key]
        require((current["id"], current["effect_key"], current["state"], current["attempt"])
                == (old["id"], old["effect_key"], "completed", attempts[key]),
                f"job did not reconcile after promotion: {key}: {current}")
    require(jobs["ambiguous"] == original["jobs"]["ambiguous"], "ambiguous settlement was changed or replayed")
    require(any(error["reason"] == "lease expired" for error in jobs["interrupted"]["errors"]),
            "interrupted claim did not recover through lease expiry")
    require(final["attempts"] == [[key, number, jobs[key]["effect_key"]]
                                  for key, count in attempts.items() for number in range(1, count + 1)],
            "handler attempts or their effect identities did not reconcile")
    require({key: effect[0] for key, effect in final["effects"].items()}
            == {key: row["effect_key"] for key, row in jobs.items()},
            "durable effect identities did not reconcile")
    require(all(final["effects"][key] == effect for key, effect in original["effects"].items()),
            "an effect committed before promotion was replaced")
    enqueued = {key: receipt["id"] for key, receipt in original["receipts"].items()}
    entries = {key: range(1, count + 1) for key, count in attempts.items()}
    writes = standby.reconcile_trial(ENGINE, namespace, enqueued, entries, original["business"])
    report(log, "promoted_final_snapshot", **final, write_counts=writes)


def run(args, scratch, evidence, log):
    env = {k: v for k, v in os.environ.items() if not k.startswith(("JOBS_PARITY_", "PG"))}
    env["TMPDIR"] = str(scratch)
    namespace = f"parity_{secrets.token_hex(8)}"
    report(log, "inputs", git_head=parity.checked(["git", "rev-parse", "HEAD"]),
           compiler_sha256=parity.digest(args.meshc), runtime_sha256=parity.digest(args.mesh_rt),
           postgres=args.postgres_version, namespace=namespace, lease_ms=LEASE_MS,
           recovery_interval_ms=RECOVERY_MS)
    executable = build_fixture(args.meshc, args.mesh_rt, scratch, evidence, env)
    with contextlib.ExitStack() as cleanup:
        primary_path, standby_path = scratch / "primary", scratch / "standby"
        primary_path.mkdir()
        standby_path.mkdir()
        primary = Primary(primary_path, primary_path, 24, args.postgres_version)
        standby = parity.Cluster(standby_path, standby_path, 24, args.postgres_version)
        cleanup.callback(preserve_server_logs, primary, standby, evidence)
        cleanup.callback(primary.stop)
        cleanup.callback(standby.stop)
        primary.start()
        primary.prepare_database(ENGINE)
        subject = parity.Subject(ENGINE, [str(executable)], scratch, primary, env)
        primary_env = subject.environment(namespace, 1, 1, "fault", POLL_MS)
        subject.call("migrate", primary_env, evidence / "migrate.log")
        start_standby(primary, standby)
        parity.wait_for(lambda: streaming(primary), time.monotonic() + 15, "TLS streaming standby")
        original = before_promotion(primary, subject, primary_env, namespace, scratch, evidence, log)

        target, replay = replay_barrier(primary, standby)
        require(standby.owned(ENGINE) and snapshot(standby, namespace) == original,
                "read-only standby did not replay the full fixture state")
        report(log, "replication_barrier", policy="asynchronous physical streaming, wait for replay before fencing",
               primary_flush_lsn=target, standby_replay_lsn=replay, replication_tls=True, scram=True)

        pid = int((primary.data / "postmaster.pid").read_text().splitlines()[0])
        primary.call("pg_ctl", "-D", str(primary.data), "-m", "immediate", "-w", "-t", "15", "stop")
        fenced(primary, pid)
        report(log, "old_primary_fenced", pid=pid, port=primary.port, mode="immediate", tcp="connection refused")
        lower, upper = promote(standby)
        report(log, "promoted_writable", readiness_window_ms=round((upper - lower) * 1000, 1))
        require(standby.owned(ENGINE) and snapshot(standby, namespace) == original,
                "promotion changed replicated job, attempt, effect or receipt identities")
        replacement = parity.Subject(ENGINE, [str(executable)], scratch, standby, env)
        standby_env = replacement.environment(namespace, 1, 1, "fault", POLL_MS)
        if args.listen:
            standby_env["JOBS_PARITY_MESH_LISTEN"] = "1"
        recover(standby, replacement, standby_env, namespace, scratch, evidence, log, original, lower)
        fenced(primary, pid)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--meshc", type=Path, default=parity.ROOT / "target/debug/meshc")
    parser.add_argument("--mesh-rt", type=Path, default=parity.ROOT / "target/debug/libmesh_rt.a")
    parser.add_argument("--postgres-version", default="17.11")
    parser.add_argument("--listen", action="store_true", help="check subscription catch-up on the promoted primary")
    parser.add_argument("--evidence-dir", type=Path, required=True, help="new directory inside the checkout")
    args = parser.parse_args()
    args.meshc = args.meshc.resolve()
    args.mesh_rt = args.mesh_rt.resolve()
    evidence = args.evidence_dir.resolve()
    if not evidence.is_relative_to(parity.ROOT) or evidence == parity.ROOT:
        parser.error("--evidence-dir must be inside the checkout")
    if not parity.POSTGRES_VERSION.fullmatch(args.postgres_version):
        parser.error("--postgres-version must be major.patch")
    if not args.meshc.is_file() or not args.mesh_rt.is_file():
        parser.error("build meshc and mesh-rt first or provide --meshc and --mesh-rt")

    def interrupted(signum, _frame):
        raise SystemExit(128 + signum)

    signal.signal(signal.SIGTERM, interrupted)
    evidence.mkdir(parents=True, exist_ok=False)
    path = evidence / "j7-lane-7.log"
    with path.open("x") as log:
        try:
            with tempfile.TemporaryDirectory(prefix="scratch-", dir=evidence) as scratch:
                run(args, Path(scratch), evidence, log)
        except (Exception, KeyboardInterrupt, SystemExit) as error:
            report(log, "FAIL", error=str(error))
            raise
        report(log, "PASS", old_primary_remained_fenced=True, scratch_removed=True)
    print(f"J7 lane 7 PASS. Evidence in {path}")


if __name__ == "__main__":
    main()
