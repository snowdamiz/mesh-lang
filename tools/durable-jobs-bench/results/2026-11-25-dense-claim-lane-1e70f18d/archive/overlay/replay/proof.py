"""Untimed public ordinary Jobs.start checks with real PostgreSQL receipts."""

import hashlib
import json
from pathlib import Path
import sys

from backend import CLAIM, arm, grant_claim, privileges
from semantic import NS, claim_loss, cluster, handler_cut, rows, submit, teardown, wait
from wire import Relay

SET = "claim_batch_set_v1"
CURSOR = "claim_batch_v1"


def require_grant(out, db, subject, env):
    db.sql("mesh", f"REVOKE EXECUTE ON FUNCTION {CLAIM} FROM parity_subject")
    assert not privileges(db)["worker_set"]
    log = out / "grant-denied.log"
    proc = subject.run(env, 0, log)
    try:
        assert proc.wait(timeout=12) == 1
        stderr = log.read_text()
        assert "Jobs set-based batch runner needs EXECUTE on mesh_jobs.claim_batch_set_v1" in stderr
        assert "READY" not in stderr
        return grant_claim(db)
    finally:
        if proc.poll() is None:
            arm.h.stop_runners([proc], require_clean=False)


def check_grant(out, executable, mixed, hot=False):
    with (cluster(out, executable, "dense") as (db, subject, env),
          Relay(db) as queue, Relay(db) as app):
        grant_check = require_grant(out, db, subject, env) if not mixed else None
        # Deterministic, weakly compressible JSON. Exercise default PostgreSQL storage.
        large = hashlib.shake_256(b"e4156740-checked-cursor").hexdigest(65536)
        jobs = [("small", {}), ("second", {"payload": large} if mixed else {})]
        receipt = submit(out, subject, env, "ordinary-grant", jobs)
        physical = json.loads(db.sql("mesh", f"""
            SELECT jsonb_object_agg(args->>'key', jsonb_build_object(
                'payload_bytes', octet_length(args->>'payload'),
                'storage_bytes', pg_column_size(args) + pg_column_size(metadata) + pg_column_size(tags),
                'compression', pg_column_compression(args)))
            FROM mesh_jobs.jobs WHERE namespace='{NS}'
        """))
        assert physical["small"]["storage_bytes"] <= 65536 and physical["small"]["compression"] is None
        if mixed:
            assert physical["second"]["payload_bytes"] == len(large)
            assert physical["second"]["storage_bytes"] > 65536 or physical["second"]["compression"]
        gate = queue.arm(b"COMMIT")
        log = out / "runner.log"
        proc = subject.run(dict(env, JOBS_PARITY_POOL_URL=db.url("mesh", port=queue.port),
                                MESH_EFFECT_APP_URL=db.url("mesh", "compiled_caller", port=app.port) + ("&mesh_pool_hot_checkout=1" if hot else "")),
                           0, log)
        try:
            assert gate.hit.wait(8), "no public batch grant COMMIT"
            held = rows(db)
            assert set(held) == set(receipt["ids"])
            assert gate.witness == dict(command="COMMIT", rows=0, idle=True, receipt_dropped=False)
            assert all(r["state"] == "executing" and r["attempt"] == 1 and r["token"] and r["expiry"]
                       and not r["attempts"] and r["effect"] is None for r in held.values()), held
            assert len({r["token"] for r in held.values()}) == 2
            assert len({r["xid"] for r in held.values()}) == 1
            assert db.sql("mesh", f"""SELECT bool_and(lease_expires_at_ms = attempted_at_ms + 30000)
                FROM mesh_jobs.jobs WHERE namespace='{NS}'""") == "t"
            claims = list(queue.claim_responses)
            observed = [(c["function"], c["statuses"], c["transaction"]) for c in claims if c["statuses"]]
            expected = [(CURSOR if mixed else SET, ["claimed", "claimed"], "T")]
            if mixed:
                expected.insert(0, (SET, ["fallback"], "T"))
            assert observed == expected, observed
            assert not any(sql.startswith("INSERT INTO") for _, sql in app.requests)
            gate.release.set()
            wait("both handlers and individual terminal commits", lambda: all(
                r["state"] == "completed" for r in rows(db).values()))
            arm.h.stop_runners([proc], require_clean=True)
            final = rows(db)
            xids = [receipt["xid"], next(iter(held.values()))["xid"]]
            for row in final.values():
                assert row["attempt"] == 1 and row["token"] is None
                assert len(row["attempts"]) == 1 and row["effect"]
                xids += [row["attempts"][0], row["effect"], row["xid"]]
            assert len(set(xids)) == 8, xids
            app_sql = [sql for _, sql in app.requests]
            inserts = [sql.split()[2] for sql in app_sql if sql.startswith("INSERT INTO")]
            assert sorted(inserts) == ["public.mesh_jobs_parity_attempts"] * 2 + [
                "public.mesh_jobs_parity_effects"] * 2, inserts
            probes = app_sql.count("SELECT 1")
            assert (probes < len(inserts) if hot else probes >= len(inserts)), app_sql
            arm.h.write_json(out / "result.json", dict(grant_check=grant_check, mixed=mixed,
                             physical=physical, held=held, final=final, producer_xid=receipt["xid"],
                             grant_receipt=gate.witness, claim_responses=claims,
                             queue_statements=queue.requests, checked_app_probes=probes, app_inserts=inserts))
        finally:
            gate.release.set()
            if proc.poll() is None:
                arm.h.stop_runners([proc], require_clean=False)


if __name__ == "__main__":
    out, executable, witness = (Path(arg).resolve() for arg in sys.argv[1:])
    out.mkdir(mode=0o700)
    for name, check in (
        ("short-set-grant", lambda dest: check_grant(dest, executable, mixed=False)),
        ("mixed-checked-cursor", lambda dest: check_grant(dest, executable, mixed=True)),
        ("ambiguous-claim-commit", lambda dest: claim_loss(dest, executable)),
        ("ambiguous-application-write", lambda dest: handler_cut(dest, executable, fault=True)),
        ("short-hot-grant", lambda dest: check_grant(dest, executable, mixed=False, hot=True)),
        ("hot-ambiguous-application-write", lambda dest: handler_cut(dest, executable, fault=True, hot=True)),
        ("teardown-release", lambda dest: teardown(dest, executable, witness, forced=False)),
        ("teardown-kill", lambda dest: teardown(dest, executable, witness, forced=True)),
    ):
        print(f"START {name}", flush=True)
        check(out / name)
        print(f"PASS {name}", flush=True)
