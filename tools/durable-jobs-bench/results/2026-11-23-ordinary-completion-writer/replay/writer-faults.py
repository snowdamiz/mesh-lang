#!/usr/bin/env python3
"""Exercise the compiled writer, its lifecycle receipt and its local slot fence."""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

from wire import Relay

ROOT = Path(__file__).resolve().parents[5]
LANE = ROOT / "target/ordinary-completion-writer"
sys.path.insert(0, str(LANE / "source/tools/durable-jobs-bench/native"))
import arm

h = arm.h
NS = "parity_0123456789abcdef"
COHORT = b"FROM mesh_jobs.complete_admitted_v3("
SCALAR = b"mesh_jobs.settle_state($1"
CLAIM_COMMIT = b"COMMIT"
RENEW = b"SELECT mesh_jobs.try_renew("


def wait(fn, description, seconds=10):
    return h.wait_for(fn, time.monotonic() + seconds, description, 0.015)


def run_case(out, executable, case):
    out.mkdir(mode=0o700)
    scratch = out / "scratch"
    scratch.mkdir()
    db = arm.Cluster(scratch, out, 20)
    db.trusted_application = True
    db.runners, db.slots, db.lanes = 1, 4, 2
    proc = None
    log = out / "runner.log"
    status = scratch / "runner-status"
    results = {}

    def sql(query):
        return db.sql("mesh", query)

    def snapshot():
        return json.loads(sql(f"""
          SELECT coalesce(json_object_agg(k, to_jsonb(j) - 'k'), '{{}}')
          FROM (
            SELECT args->>'key' AS k, id, state, attempt, xmin::text AS xid,
                  lease_expires_at_ms AS lease_expires_at_ms,
                  (SELECT count(*) FROM public.mesh_jobs_parity_effects e
                   WHERE e.namespace = jobs.namespace AND e.job_key=jobs.args->>'key') AS effects,
                  (SELECT coalesce(json_agg(attempt ORDER BY attempt), '[]')
                   FROM public.mesh_jobs_parity_attempts a
                   WHERE a.namespace=jobs.namespace AND a.job_key=jobs.args->>'key') AS attempts
            FROM mesh_jobs.jobs WHERE namespace = '{NS}'
          ) j
        """))

    def has(key, **expectations):
        row = snapshot().get(key, {})
        return row if row and all(row.get(k) == v for k, v in expectations.items()) else None

    def connections():
        return json.loads(sql("""
          SELECT json_agg(json_build_array(a.usename,a.pid,s.ssl) ORDER BY a.pid)
          FROM pg_stat_activity a JOIN pg_stat_ssl s USING(pid)
          WHERE a.datname='mesh' AND a.usename IN ('parity_subject','compiled_caller')
        """))

    def get_status():
        status.unlink(missing_ok=True)
        status.with_suffix(".request").touch()
        wait(status.exists, "runner status")
        return json.loads(status.read_text())

    def submit(label, jobs):
        path = out / f"input-{label}.json"
        path.write_text(json.dumps(dict(request_id=label, rollback=False, jobs=[
            dict(key=key, hold_ms=hold, fail_first=retry, payload="writer-fault/été")
            for key, hold, retry in jobs
        ])))
        receipt, _ = subject.enqueue_receipt(env, path, out / f"enqueue-{label}.log")
        assert receipt["count"] == len(jobs)

    def done(key, attempts=None, state="completed"):
        return wait(lambda: has(key, state=state, attempts=attempts or [1]), f"{key} -> {state}")

    def receipt(gate):
        assert gate.hit.wait(6), f"receipt never withheld: {gate.query!r}"
        return gate.witness

    def start(relay):
        nonlocal proc
        proc = subject.run(dict(env, JOBS_PARITY_POOL_URL=db.url("mesh", port=relay.port)), 0, log)
        wait(lambda: h.runner_ready([proc], [log], track=False), "compiled runner ready")

    try:
        db.start()
        db.prepare_database("mesh")
        subject = arm.Subject("mesh", [str(executable)], executable.parent, db, db.env)
        env = subject.environment(NS, db.slots, db.slots, "fault", 50)
        env.update(JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_EVENTS="logged",
                   JOBS_PARITY_STATUS_PATH=str(status), MESH_JOBS_COMPLETION_TRACE="1",
                   TRUSTED_ADMISSION_POLICY="success_cohort", TRUSTED_COMPLETION_WRITER_MS="1",
                   MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2",
                   MESH_EFFECT_LANES=str(db.lanes), MESH_EFFECT_APP_URL=db.url("mesh", "compiled_caller"))
        subject.call("migrate", env, out / "migrate.log")
        with Relay(db) as relay:
            try:
                start(relay)
                if case == "ambiguous-claim":
                    gate = relay.arm(CLAIM_COMMIT, drop=True)
                    submit("claims", [("claim-a", 0, False), ("claim-b", 0, False)])
                    results["wire"] = receipt(gate)
                    before = snapshot()
                    assert all(r["state"] == "executing" and r["attempts"] == [] for r in before.values()), before
                    gate.release.set()
                    for k in before:
                        assert done(k, [2])["effects"] == 1
                    results["before_recovery"] = before
                else:
                    if case == "renew-priority":
                        submit("renew", [("renew", 2000, False)])
                        running = wait(lambda: has("renew", state="executing", attempts=[1]),
                                       "long-lived handler starts")
                    prime = relay.arm(SCALAR)
                    submit("prime", [("prime", 0, False)])
                    results["prime_wire"] = receipt(prime)
                    assert done("prime")["effects"] == 1
                    initial_connections = connections()
                    assert len(initial_connections) == 5 and all(r[2] for r in initial_connections)
                    if case == "stop":
                        proc.send_signal(signal.SIGTERM)
                        time.sleep(0.12)
                        assert proc.poll() is None, "runner stopped before durable receipt"
                        submit("after-stop", [("after-stop", 0, False)])
                        assert snapshot()["after-stop"]["attempts"] == []
                        prime.release.set()
                        assert proc.wait(timeout=10) == 0
                    elif case == "renew-priority":
                        submit("peers", [("left", 0, False), ("right", 0, False)])
                        wait(lambda: all(has(k, state="executing", attempts=[1], effects=1)
                                         for k in ("left", "right")), "peers collect while receipt is held")
                        wait(lambda: int(sql("SELECT mesh_jobs.now_ms()")) >= running["lease_expires_at_ms"] - 985,
                             "live handler renewal becomes due")
                        renew = relay.arm(RENEW)
                        prime.release.set()
                        results["renew_wire"] = receipt(renew)
                        snapshot_during_renew = snapshot()
                        assert all(snapshot_during_renew[k]["state"] == "executing" for k in ("left", "right"))
                        assert snapshot_during_renew["renew"]["lease_expires_at_ms"] > running["lease_expires_at_ms"]
                        results["renew_before_successes"] = snapshot_during_renew
                        renew.release.set()
                        assert done("left")["xid"] == done("right")["xid"]
                        assert done("renew")["effects"] == 1
                    else:
                        submit("peers", [("left", 0, False), ("middle", 0, False), ("right", 0, False),
                                         ("later-retry", 0, True)])
                        wait(lambda: all(has(k, state="executing", attempts=[1], effects=1)
                                         for k in ("left", "middle", "right")), "three handlers returned writes")
                        # The reporter still holds the prime slot, even though the
                        # broker can already observe its durable completion.
                        held = get_status()
                        assert held["queues"]["default"]["in_flight"] == 4, held
                        assert snapshot()["later-retry"]["attempts"] == []
                        results["during_receipt_hold"] = held
                        assert connections() == initial_connections
                        results["held_physical_backends"] = initial_connections
                        if case == "ambiguous-completion":
                            cohort = relay.arm(COHORT, drop=True)
                            prime.release.set()
                            results["wire"] = receipt(cohort)
                            before = {k: done(k) for k in ("left", "middle", "right")}
                            assert len({r["xid"] for r in before.values()}) == 1
                            assert cohort.witness["rows"] == 3
                            assert get_status()["queues"]["default"]["in_flight"] >= 3
                            cohort.release.set()
                            wait(lambda: log.read_text().count('"ack_uncertain"') >= 3,
                                 "original success tokens marked uncertain")
                            wait(lambda: log.read_text().count('"ack_stale"') >= 3,
                                 "original success tokens reconciled by scalar ack")
                            results["committed_before_receipt_loss"] = before
                        elif case == "cancelled-member":
                            middle = snapshot()["middle"]
                            sql(f"SELECT mesh_jobs.job_control('{NS}', '{middle['id']}', 'cancel', NULL, NULL)")
                            prime.release.set()
                            left, right = done("left"), done("right")
                            cancelled = done("middle", state="cancelled")
                            assert left["xid"] == right["xid"] != cancelled["xid"]
                            assert cancelled["effects"] == 1
                        else:
                            command, pg_env = db.psql("mesh")
                            with subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                                  stderr=subprocess.PIPE, text=True, env=pg_env) as holder:
                                holder.stdin.write(
                                    f"BEGIN; SELECT id FROM mesh_jobs.jobs WHERE namespace='{NS}' "
                                    "AND args->>'key'='middle' FOR NO KEY UPDATE;\nSELECT 'LOCK_READY';\n")
                                holder.stdin.flush()
                                assert holder.stdout.readline().strip()
                                assert holder.stdout.readline().strip() == "LOCK_READY"
                                try:
                                    prime.release.set()
                                    left, right = done("left"), done("right")
                                    assert left["xid"] == right["xid"]
                                    assert has("middle", state="executing", attempts=[1], effects=1)
                                    results["peers_while_middle_locked"] = {"left": left, "right": right}
                                    if case == "expired-middle":
                                        mid = snapshot()["middle"]
                                        wait(lambda: int(sql("SELECT mesh_jobs.now_ms()")) >= mid["lease_expires_at_ms"],
                                             "held member's PostgreSQL lease expires")
                                        local = get_status()
                                        assert mid["id"] in local["undrained_jobs"]
                                        proc.send_signal(signal.SIGTERM)
                                        results["local_slot_after_expiry"] = local
                                finally:
                                    holder.stdin.write("ROLLBACK;\n\\q\n")
                                    holder.stdin.flush()
                                    assert holder.wait(timeout=5) == 0
                            if case == "expired-middle":
                                assert proc.wait(timeout=10) == 0
                                mid = snapshot()["middle"]
                                assert mid["state"] != "completed" and mid["attempts"] == [1], mid
                            else:
                                assert done("middle")["xid"] != left["xid"]
                        if case != "expired-middle":
                            assert done("later-retry", [1, 2])["effects"] == 1
                if proc.poll() is None:
                    h.stop_runners([proc], require_clean=True)
            finally:
                if proc and proc.poll() is None:
                    for gate in relay.gates:
                        gate.release.set()
                    h.stop_runners([proc], require_clean=False)
        results["final"] = snapshot()
        (out / "result.json").write_text(json.dumps(results, indent=2) + "\n")
    finally:
        if proc and proc.poll() is None:
            h.stop_runners([proc], require_clean=False)
        db.stop()
        shutil.rmtree(scratch)
        arm.redact(out, db.password)


if __name__ == "__main__":
    cases = ("held-middle", "stop", "ambiguous-completion", "ambiguous-claim", "expired-middle",
             "renew-priority", "cancelled-member")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument("executable", type=Path)
    parser.add_argument("--cases", choices=cases, nargs="+", default=cases)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(mode=0o700)
    umask = os.umask(0o077)
    try:
        for case in args.cases:
            run_case(out / case, args.executable.resolve(), case)
            print(case, "passed", flush=True)
    finally:
        os.umask(umask)
