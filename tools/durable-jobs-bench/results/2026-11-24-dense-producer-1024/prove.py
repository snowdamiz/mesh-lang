"""Drive compiled producer and fault probe against fresh PostgreSQL, without timing."""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

from adapter import Cluster, Subject, arm, patch, producer_counters
from wire import Relay

HERE = Path(__file__).resolve().parent
NS = "parity_0123456789abcdef"
WRITER = "mesh_jobs.enqueue_dense_private_v1(text,jsonb)"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write(path, data):
    path.write_text(json.dumps(data, separators=(",", ":"), sort_keys=True))


def invoke(binary, env, log, *argv, stdin=None, allow_error=False):
    completed = subprocess.run([binary, *argv], env=env, input=stdin, text=True,
                               capture_output=True, timeout=30)
    log.write_text(completed.stdout + completed.stderr)
    if not allow_error:
        completed.check_returncode()
    return completed


@contextmanager
def cluster(out, migrator):
    out.mkdir(mode=0o700)
    scratch = out / "scratch"
    scratch.mkdir()
    class ProofCluster(Cluster):
        def sql(self, database, query):
            if "CREATE ROLE parity_migrator NOLOGIN" in query:
                if local_group := os.environ.get("DENSE_LOCAL_GROUP_SQL"):
                    assert sys.platform == "darwin"
                    super().sql(database, "SET ROLE parity_subject; " + Path(local_group).read_text())
            return super().sql(database, query)

    db = ProofCluster(scratch, out, 20)
    db.trusted_application = True
    db.runners, db.slots, db.lanes = 1, 4, 1
    try:
        db.start()
        db.prepare_database("mesh")
        subject = Subject("mesh", [str(migrator)], migrator.parent, db, db.env)
        env = subject.environment(NS, 4, 4, "dense", 250)
        subject.call("migrate", env, out / "migrate.log")
        sql = (HERE / "install-writer.sql").read_text()
        db.sql("mesh", sql)
        assert db.sql("mesh", f"""SELECT NOT prosecdef
            AND has_function_privilege('parity_producer', oid, 'EXECUTE')
            AND NOT has_function_privilege('parity_subject', oid, 'EXECUTE')
            AND NOT has_function_privilege('compiled_caller', oid, 'EXECUTE')
            FROM pg_proc WHERE oid = '{WRITER}'::regprocedure""") == "t"
        yield db, dict(env, JOBS_PARITY_DB_URL=db.url("mesh", "parity_producer"),
                       MESH_JOBS_SET_BASED_ENQUEUE="1")
    finally:
        db.stop()
        shutil.rmtree(scratch)
        arm.redact(out, db.password)


def stored(db, ns):
    return json.loads(db.sql("mesh", f"""
        SELECT coalesce(json_agg(json_build_object('key', r.request_key, 'receipt', r.receipt,
                   'payload', j.args->>'payload', 'xid', r.xmin::text,
                   'fingerprint', r.fingerprint) ORDER BY r.ctid), '[]')
        FROM mesh_jobs.receipts r JOIN mesh_jobs.jobs j ON j.id=(r.receipt->>'id')::uuid
        WHERE r.namespace='{ns}'
    """))


def checks(out, db, env, broker, probe):
    settings = json.loads(arm.h.CONFIG.read_text())
    batch, = arm.h.inputs(settings["workloads"]["noop"], 64, settings["seed"])
    jobs = batch["jobs"]
    assert len(jobs) == 1024 and {len(j["payload"]) for j in jobs} == {1024}
    results = {}

    def namespace(name):
        assert re.fullmatch(r"[a-z_0-9]+", name)
        return NS + "_" + name

    def counters():
        return producer_counters(db)

    def delta(before):
        after = counters()
        return {key: diff for key, values in after.items()
                if any(diff := [a - b for a, b in zip(values, before.get(key, [0, 0]))])}

    input_path = out / "canonical.json"
    write(input_path, batch)
    fixture_path, = arm.h.batch_files(out, [batch])
    frozen_digest = sha(sha(fixture_path.read_bytes()).encode())
    assert frozen_digest == arm.PROFILES["noop"][-1]
    message = json.dumps({"verb": "enqueue", "batch": batch}, separators=(",", ":"))
    for width in ("128", "1024"):
        ns = namespace("width_" + width)
        before = counters()
        run = invoke(broker, dict(env, JOBS_PARITY_NAMESPACE=ns,
                                  MESH_JOBS_DENSE_PRODUCER_1024=str(int(width == "1024"))),
                     out / f"canonical-{width}.log", "session", stdin=message + "\nstop\n")
        ready, reply, stopped = run.stdout.splitlines()
        receipt = json.loads(reply)
        rows = stored(db, ns)
        assert ready == "SESSION_READY" and stopped == "{}" and receipt["count"] == 1024
        assert [(r["key"], r["payload"]) for r in rows] == [(j["key"], j["payload"]) for j in jobs]
        assert {r["key"]: r["receipt"]["id"] for r in rows} == receipt["ids"]
        assert db.sql("mesh", f"""SELECT count(*) = 1024 AND bool_and(
                   pg_xact_commit_timestamp(r.xmin) = pg_xact_commit_timestamp('{receipt["xid"]}'::xid)
                   AND pg_xact_commit_timestamp(j.xmin) = pg_xact_commit_timestamp('{receipt["xid"]}'::xid))
                FROM mesh_jobs.receipts r JOIN mesh_jobs.jobs j ON j.id=(r.receipt->>'id')::uuid
                WHERE r.namespace='{ns}'""") == "t"
        actual = delta(before)
        assert actual == ({"wide": [1, 1024]} if width == "1024" else {"narrow": [8, 1024]})
        write(out / f"canonical-{width}-durable.json", rows)
        results[width] = dict(statements=actual, receipts=len(rows))

    specs = [dict(worker="parity", args=j, request_key=j["key"], max_attempts=3,
                  backoff_base_ms=1000, backoff_max_ms=1000) for j in jobs]
    assert len(json.dumps(specs, separators=(",", ":")).encode()) > 1024**2

    def send(name, data=specs, ns=None, expected=None, wide=True, rollback=False):
        ns = ns or namespace(name)
        path = out / "probe-input.json"
        if isinstance(data, str):
            path.write_text(data)
        else:
            write(path, data)
        before = counters()
        run = invoke(probe, dict(env, JOBS_PARITY_NAMESPACE=ns, DENSE_PROBE_INPUT=str(path),
                                  DENSE_PROBE_ROLLBACK=str(int(rollback)),
                                  MESH_JOBS_DENSE_PRODUCER_1024=str(int(wide))),
                     out / f"{name}.log")
        reply = json.loads(run.stdout)
        actual = delta(before)
        if expected is not None:
            assert actual == expected, (name, actual)
        results[name] = dict(statements=actual, receipt_count=len(reply) if isinstance(reply, list) else 0)
        if isinstance(reply, dict):
            assert reply["transaction_usable"] and reply["error"]
            results[name]["error"] = reply["error"]
        return ns, reply

    _, error = send("public_count", [dict(worker="parity")] * 129, wide=False, expected={})
    assert "1 to 128" in error["error"]
    _, error = send("public_bytes", wide=False, expected={})
    assert "1 MiB" in error["error"]
    _, error = send("wide_count", [dict(worker="parity")] * 1025, expected={})
    assert "1 to 128" in error["error"]
    _, error = send("wide_bytes", " " * (2 * 1024**2 + 1), expected={})
    assert "2 MiB" in error["error"]

    ns, reply = send("outer_rollback", expected={"wide": [1, 1024]}, rollback=True)
    assert len(reply) == 1024 and not stored(db, ns)
    assert db.sql("mesh", f"SELECT count(*) FROM mesh_jobs.admissions WHERE namespace='{ns}'") == "0"

    # Repeat the exact key and spec across the old batch boundary.
    same = specs.copy()
    same[128] = same[127]
    ns, reply = send("boundary_same", same, expected={"wide": [1, 1], "scalar": [1024, 1024]})
    assert reply[127] == reply[128] and len(stored(db, ns)) == 1023
    changed = same.copy()
    changed[128] = dict(changed[128], args={"payload": "changed"})
    ns, reply = send("boundary_changed", changed)
    assert "request" in reply["error"] and not stored(db, ns)
    for table in ("jobs", "receipts", "admissions", "queues"):
        assert db.sql("mesh", f"SELECT count(*) FROM mesh_jobs.{table} WHERE namespace='{ns}'") == "0"
    for name, items in (("narrow_same", same), ("narrow_changed", changed)):
        ns = namespace(name)
        first, second = out / (name + "-first.json"), out / (name + "-second.json")
        write(first, items[:128])
        write(second, items[128:256])
        before = counters()
        run = invoke(probe, dict(env, JOBS_PARITY_NAMESPACE=ns, DENSE_PROBE_INPUT=str(first),
                                 DENSE_PROBE_TAIL=str(second), MESH_JOBS_DENSE_PRODUCER_1024="0"),
                     out / (name + ".log"))
        receipt = json.loads(run.stdout)
        if name == "narrow_same":
            assert receipt["first"][127] == receipt["tail"][0]
            assert len(stored(db, ns)) == 255
        else:
            assert "different spec" in receipt["error"] and receipt["transaction_usable"] and not stored(db, ns)
        results[name] = dict(statements=delta(before), durable_receipts=len(stored(db, ns)))
    unique = specs.copy()
    for index in (127, 128):
        unique[index] = dict(unique[index], unique="one-dense-job")
    ns, reply = send("uniqueness", unique, expected={"scalar": [1024, 1024]})
    assert reply[127]["id"] == reply[128]["id"] and reply[128]["conflict"]
    assert len(stored(db, ns)) == 1024
    assert db.sql("mesh", f"SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='{ns}'") == "1023"
    ns, reply = send("initial", expected={"wide": [1, 1024]})
    _, replay = send("repeat", ns=ns, expected={"wide": [1, 1], "scalar": [1024, 1024]})
    assert reply == replay and len(stored(db, ns)) == 1024

    sql_cases = (
        ("layout", "ALTER TABLE mesh_jobs.jobs SET (fillfactor=95)",
         "ALTER TABLE mesh_jobs.jobs RESET (fillfactor)", {"wide": [1, 1], "scalar": [1024, 1024]}),
        ("revoke", f"REVOKE EXECUTE ON FUNCTION {WRITER} FROM parity_producer",
         f"GRANT EXECUTE ON FUNCTION {WRITER} TO parity_producer", {"scalar": [1024, 1024]}),
        ("missing", f"ALTER FUNCTION {WRITER} RENAME TO enqueue_dense_hold",
         "ALTER FUNCTION mesh_jobs.enqueue_dense_hold(text,jsonb) RENAME TO enqueue_dense_private_v1",
         {"scalar": [1024, 1024]}),
    )
    for name, begin, end, stats in sql_cases:
        db.sql("mesh", begin)
        ns, reply = send(name, expected=stats)
        assert len(reply) == len(stored(db, ns)) == 1024
        db.sql("mesh", end)

    # This is within the raw input cap but its normalized SQL envelope is too large.
    big = [dict(s, args=dict(s["args"], payload="x" * 1580)) for s in specs]
    ns, reply = send("bind_fallback", big, expected={"scalar": [1024, 1024]})
    assert len(reply) == len(stored(db, ns)) == 1024
    # Wrap the real writer for reply-boundary faults after it has written all rows.
    db.sql("mesh", f"ALTER FUNCTION {WRITER} RENAME TO enqueue_dense_inner")
    for name, ordinal, expression, suffix in (
        ("incomplete_reply", "ordinal", "receipt", "WHERE ordinal <> '129'"),
        ("oversized_reply", "ordinal", "CASE WHEN ordinal = '129' THEN repeat('x', 513) ELSE receipt END", ""),
        ("extra_reply", "ordinal", "receipt", "UNION ALL SELECT '1025', '{}' UNION ALL SELECT '1026', '{}'"),
        ("misordered_reply", "CASE WHEN ordinal = '129' THEN '128' ELSE ordinal END", "receipt", ""),
    ):
        db.sql("mesh", f"""
            CREATE OR REPLACE FUNCTION {WRITER} RETURNS TABLE(ordinal text, receipt text)
            LANGUAGE sql VOLATILE AS $$
                SELECT {ordinal}, {expression} FROM mesh_jobs.enqueue_dense_inner($1, $2) {suffix}
            $$;
            REVOKE EXECUTE ON FUNCTION {WRITER} FROM PUBLIC;
            GRANT EXECUTE ON FUNCTION {WRITER} TO parity_producer;
        """)
        ns, reply = send(name)
        assert "set-based enqueue response" in reply["error"] and not stored(db, ns)
        assert "scalar" not in results[name]["statements"]
        assert db.sql("mesh", f"SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='{ns}'") == "0"
    db.sql("mesh", f"DROP FUNCTION {WRITER}; "
                   "ALTER FUNCTION mesh_jobs.enqueue_dense_inner(text,jsonb) RENAME TO enqueue_dense_private_v1")

    # Verify both serialization boundaries with an open owner session.
    @contextmanager
    def hold(sql):
        command, pg_env = db.psql("mesh")
        with subprocess.Popen(command, env=pg_env, text=True, stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL) as owner:
            owner.stdin.write("BEGIN;\n" + sql + ";\nSELECT 'held';\n")
            owner.stdin.flush()
            assert owner.stdout.readline().strip() == "held"
            try:
                yield
            finally:
                owner.stdin.write("ROLLBACK;\n\\q\n")
                owner.stdin.flush()
                assert owner.wait(timeout=10) == 0

    with hold("DO $$ BEGIN PERFORM pg_advisory_xact_lock(1782495102, 1); END $$"):
        ns, reply = send("migration_lock", expected={"scalar": [1024, 1024]})
        assert len(reply) == len(stored(db, ns)) == 1024

    ns = namespace("held_namespace")
    proof_input = out / "fault-specs.json"
    write(proof_input, specs)

    def spawn(ns, **extra):
        return subprocess.Popen([probe], env=dict(env, JOBS_PARITY_NAMESPACE=ns, DENSE_PROBE_INPUT=str(proof_input),
                              MESH_JOBS_DENSE_PRODUCER_1024="1") | extra,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    child = None
    try:
        with hold(f"INSERT INTO mesh_jobs.admissions(namespace) VALUES ('{ns}')"):
            child = spawn(ns)
            endpoint = time.monotonic() + 4
            while db.sql("mesh", """SELECT EXISTS (
                SELECT FROM pg_stat_activity WHERE usename='parity_producer'
                AND query LIKE 'INSERT INTO mesh_jobs.admissions%'
                AND cardinality(pg_blocking_pids(pid)) > 0)""") != "t":
                assert time.monotonic() < endpoint, "namespace fence did not block the real producer"
                time.sleep(0.01)
            assert not stored(db, ns) and child.poll() is None
        stdout, stderr = child.communicate(timeout=10)
        assert child.returncode == 0 and len(json.loads(stdout)) == len(stored(db, ns)) == 1024
        (out / "held_namespace.log").write_text(stdout + stderr)
        results["namespace_fence"] = dict(blocked=True, receipt_count=1024)
    finally:
        if child is not None and child.poll() is None:
            child.kill()
            child.communicate()

    class ProducerRelay(Relay):
        relay = patch(Relay.relay, [
            ('if data == b"I" and command:', 'if data in (b"I", b"T") and command:'),
            ('(command == "COMMIT" and admitted)', '(command == "COMMIT" and data == b"I")'),
            ('idle=True,', 'idle=(data == b"I"),'),
        ])

    for name, query, committed in (
        ("missing_writer_ack", b"FROM mesh_jobs.enqueue_dense_private_v1", False),
        ("ambiguous_commit_ack", b"COMMIT", True),
    ):
        ns = namespace(name)
        with ProducerRelay(db) as relay:
            gate = relay.arm(query, drop=True)
            with spawn(ns, JOBS_PARITY_DB_URL=db.url("mesh", "parity_producer", port=relay.port)) as child:
                assert gate.hit.wait(6), f"{name} fault did not intercept target completion"
                assert bool(stored(db, ns)) == committed
                gate.release.set()
                stdout, stderr = child.communicate(timeout=10)
                assert child.returncode != 0 and not stdout.strip()
                (out / f"{name}.log").write_text(stderr)
            assert not any("SELECT mesh_jobs.enqueue(" in sql for _, sql in relay.requests)
            assert gate.witness["receipt_dropped"] and gate.witness["idle"] is committed
            if not committed:
                assert gate.witness["rows"] == 1024
        original = stored(db, ns)
        assert len(original) == (1024 if committed else 0)
        _, reply = send(name + "_retry", ns=ns,
                        expected=({"wide": [1, 1], "scalar": [1024, 1024]} if committed else {"wide": [1, 1024]}))
        final = stored(db, ns)
        assert len(final) == 1024 and reply == [r["receipt"] for r in final]
        if committed:
            assert original == final
        results[name] = gate.witness

    return dict(cases=results, canonical_sha256=sha(input_path.read_bytes()), frozen_input_sha256=frozen_digest)


def main():
    out, broker, probe, migrator = (Path(p).resolve() for p in sys.argv[1:])
    with cluster(out, migrator) as (db, env):
        result = checks(out, db, env, broker, probe)
        write(out / "result.json", result)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
