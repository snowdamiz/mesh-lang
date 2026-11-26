#!/usr/bin/env python3
"""Exercise the compiled fixture with real PostgreSQL and worker lifecycle faults."""

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

LANE = Path(__file__).resolve().parents[5] / "target/ordinary-completion-writer"
sys.path.insert(0, str(LANE / "source/tools/durable-jobs-bench/native"))
import arm

h = arm.h
NS = "parity_0123456789abcdef"


def wait(fn, condition, label, seconds=12):
    return h.wait_for(lambda: (value if condition(value := fn()) else None),
                      time.monotonic() + seconds, label, 0.025)


def quote(value):
    return "'" + value.replace("'", "''") + "'"


def trial(out, executable):
    scratch = out / "scratch"
    scratch.mkdir()
    db = arm.Cluster(scratch, out, 20)
    db.trusted_application = True
    db.runners, db.slots, db.lanes = 1, 3, 2
    commands = []
    results = {}

    def sql(query):
        return db.sql("mesh", query)

    def query(query):
        return json.loads(sql(query))

    def row(key):
        return query(f"""
            SELECT row_to_json(r) FROM (
                SELECT id, state, attempt, run, lease_token, lease_expires_at_ms
                FROM mesh_jobs.jobs WHERE namespace = '{NS}' AND args->>'key' = {quote(key)}
            ) r
        """)

    def writes(key):
        return query(f"""
            SELECT json_build_object(
                'attempts', (SELECT coalesce(json_agg(attempt ORDER BY job_generation, attempt), '[]')
                    FROM public.mesh_jobs_parity_attempts WHERE namespace = '{NS}' AND job_key = {quote(key)}),
                'effects', (SELECT count(*) FROM public.mesh_jobs_parity_effects
                    WHERE namespace = '{NS}' AND job_key = {quote(key)}))
        """)

    def start():
        log = out / f"runner-{len(runners)}.log"
        runner = subject.run(env, len(runners), log)
        runners.append(runner)
        wait(lambda: h.runner_ready([runner], [log], track=False), bool, "runner ready")
        return runner

    def submit(key, hold=0, fail=False):
        args = dict(key=key, hold_ms=hold, payload="valid UTF-8: étage", fail_first=fail)
        path = out / f"{key}.json"
        request = dict(request_id=key, rollback=False, jobs=[args])
        path.write_text(json.dumps(request))
        receipt, _ = subject.enqueue_receipt(env, path, out / f"{key}-enqueue.log")
        assert receipt["count"] == 1
        identity = receipt["ids"][key]
        return identity

    def finished(key, state, expected_attempts, effects):
        val = wait(lambda: row(key), lambda x: x["state"] == state, f"{key} -> {state}")
        observed = writes(key)
        assert observed == {"attempts": expected_attempts, "effects": effects}, (key, val, observed)
        val.pop("lease_token")
        results[key] = dict(val, **observed)
        return val

    runners = []
    clean = False
    try:
        db.start()
        db.prepare_database("mesh")
        subject = arm.Subject("mesh", [str(executable)], executable.parent, db, db.env)
        env = subject.environment(NS, db.slots, db.slots, "fault", 50)
        env.update(JOBS_PARITY_MESH_LISTEN="1", JOBS_PARITY_MESH_BATCH="1",
                   MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2",
                   MESH_EFFECT_LANES=str(db.lanes), MESH_EFFECT_APP_URL=db.url("mesh", "compiled_caller"))
        subject.call("migrate", env, out / "migrate.log")

        # Verify direct SQL rights with real RLS and a redacted connection error.
        sql("INSERT INTO public.mesh_jobs_parity_effects VALUES "
            "('mesh', 'other_tenant', 'foreign', 'foreign', clock_timestamp())")
        role = "SET ROLE compiled_caller; "
        assert sql(role + "SELECT count(effect_key) FROM public.mesh_jobs_parity_effects") == "0"
        for name, statement in (
            ("cross-tenant", "INSERT INTO public.mesh_jobs_parity_effects VALUES "
             "('mesh','other_tenant','foreign','foreign',clock_timestamp()) ON CONFLICT DO NOTHING"),
            ("queue-table", "SELECT count(*) FROM mesh_jobs.jobs"),
            ("rewrite", "UPDATE public.mesh_jobs_parity_effects SET effect_key = 'changed'"),
        ):
            try:
                sql(role + statement)
            except h.FixtureError as error:
                assert re.search(r"row-level security|permission denied", str(error)), str(error)
                results[name] = "denied"
            else:
                raise AssertionError(f"{name}: application role crossed its grants")

        first = start()
        renewing = submit("renew", 2000)
        wait(lambda: writes("renew"), lambda x: x["attempts"] == [1], "renew first write")
        original = row("renew")
        wait(lambda: row("renew"), lambda x: (x["lease_expires_at_ms"] or 0) > original["lease_expires_at_ms"],
             "same-attempt renewal")
        assert row("renew")["lease_token"] == original["lease_token"]
        assert row("renew")["state"] == "executing" and writes("renew")["effects"] == 0
        finished("renew", "completed", [1], 1)

        # Manual rerun advances the queue generation. The business effect key stays fixed.
        sql(f"SELECT mesh_jobs.job_control('{NS}', '{renewing}', 'retry', NULL, NULL)")
        wait(lambda: row("renew"), lambda x: x["run"] == original["run"] + 1, "rerun generation")
        finished("renew", "completed", [1, 1], 1)
        assert results["renew"]["id"] == renewing

        submit("panic")
        finished("panic", "completed", [1, 2], 1)
        assert "trusted proof panic" in (out / "runner-0.log").read_text()
        submit("business-retry", fail=True)
        finished("business-retry", "completed", [1, 2], 1)

        submit("return-hold")
        wait(lambda: writes("return-hold"), lambda x: x["effects"] == 1, "second committed write")
        assert row("return-hold")["state"] == "executing"
        finished("return-hold", "completed", [1], 1)

        cancelled = submit("cancel", 2000)
        wait(lambda: writes("cancel"), lambda x: x["attempts"] == [1], "cancel first write")
        sql(f"SELECT mesh_jobs.job_control('{NS}', '{cancelled}', 'cancel', NULL, NULL)")
        finished("cancel", "cancelled", [1], 0)

        submit("timeout", 2000)
        finished("timeout", "completed", [1, 2], 1)
        error_history = sql(f"""SELECT errors::text FROM mesh_jobs.jobs
            WHERE namespace = '{NS}' AND args->>'key' = 'timeout'""")
        assert "job handler timed out" in error_history
        results["timeout"]["error"] = "job handler timed out"

        # Hold the second SQL call after it is sent to PostgreSQL.
        # Cancellation can stop the actor while the database call still commits.
        pg_command, pg_env = db.psql("mesh")
        for scenario in ("late-effect", "disconnect"):
            with subprocess.Popen(pg_command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, env=pg_env, text=True) as holder:
                holder.stdin.write("BEGIN; LOCK TABLE public.mesh_jobs_parity_effects IN ACCESS EXCLUSIVE MODE;\n"
                                   "SELECT 'LOCK_READY';\n")
                holder.stdin.flush()
                assert holder.stdout.readline().strip() == "LOCK_READY"
                job = submit(scenario)

                def blocked():
                    return sql("""SELECT pid FROM pg_stat_activity
                        WHERE datname = 'mesh' AND usename = 'compiled_caller'
                        AND wait_event_type = 'Lock' AND query LIKE '%mesh_jobs_parity_effects%'""")

                pid = int(wait(blocked, bool, "application backend blocked on proof lock"))
                if scenario == "disconnect":
                    assert sql(f"SELECT pg_terminate_backend({pid})") == "t"
                else:
                    sql(f"SELECT mesh_jobs.job_control('{NS}', '{job}', 'cancel', NULL, NULL)")
                    # Let the queue supervisor see the cancellation while the
                    # held transaction still blocks the effect.
                    time.sleep(0.8)
                    assert row(scenario)["state"] == "executing"
                holder.stdin.write("ROLLBACK;\n\\q\n")
                holder.stdin.flush()
                assert holder.wait(timeout=5) == 0
            if scenario == "disconnect":
                finished(scenario, "completed", [1, 2], 1)
                assert "trusted application write failed:" in (out / "runner-0.log").read_text()
            else:
                finished(scenario, "cancelled", [1], 1)
                results[scenario]["late_effect_committed"] = True

        submit("stop", 2000)
        wait(lambda: writes("stop"), lambda x: x["attempts"] == [1], "stop first write")
        h.stop_runners([first], require_clean=True)
        assert row("stop")["state"] == "retryable" and writes("stop")["effects"] == 0
        second = start()
        finished("stop", "completed", [1, 2], 1)

        submit("crash", 2200)
        wait(lambda: writes("crash"), lambda x: x["attempts"] == [1], "crash first write")
        second.kill()
        second.wait(timeout=5)
        third = start()
        finished("crash", "completed", [1, 2], 1)
        final = row("crash")
        assert final["attempt"] == 2

        submit("crash-after-effect", 50)
        wait(lambda: writes("crash-after-effect"), lambda x: x["effects"] == 1, "committed external effect")
        old = row("crash-after-effect")
        assert old["state"] == "executing"
        old_effect_xid = sql("SELECT xmin::text FROM public.mesh_jobs_parity_effects "
                             "WHERE job_key = 'crash-after-effect'")
        third.kill()
        third.wait(timeout=5)
        fourth = start()
        finished("crash-after-effect", "completed", [1, 2], 1)
        assert old_effect_xid == sql("SELECT xmin::text FROM public.mesh_jobs_parity_effects "
                                     "WHERE job_key = 'crash-after-effect'")

        # Exercise the superseded token while the retry is still executing.
        fenced = submit("fence-replacement", 2200)
        wait(lambda: writes("fence-replacement"), lambda x: x["attempts"] == [1], "old lease attempt")
        old = row("fence-replacement")
        fourth.kill()
        fourth.wait(timeout=5)
        fifth = start()
        replacement = wait(lambda: row("fence-replacement"),
                           lambda r: r["attempt"] == 2 and r["state"] == "executing",
                           "replacement lease")
        assert replacement["lease_token"] != old["lease_token"]
        stale = sql(f"""SELECT mesh_jobs.settle_state('{NS}', '{fenced}', '{old["lease_token"]}',
                                  '{{"action":"discard","reason":"stale token probe"}}', false)""")
        still_executing = row("fence-replacement")
        assert stale == "" and still_executing["state"] == "executing"
        assert still_executing["lease_token"] == replacement["lease_token"]
        finished("fence-replacement", "completed", [1, 2], 1)
        results["stale-token-replay"] = "rejected while replacement token was live"
        h.stop_runners([fifth], require_clean=True)
        clean = True
    finally:
        for runner in runners:
            if runner.poll() is None:
                h.stop_runners([runner], require_clean=False)
        db.stop()
        shutil.rmtree(scratch)
        arm.redact(out, db.password)
        h.write_json(out / "cases.json", dict(clean=clean, cases=results))


if __name__ == "__main__":
    out, executable = map(lambda path: Path(path).resolve(), sys.argv[1:])
    out.mkdir(mode=0o700)
    trial(out, executable)
