"""Replay compiled producer, direct SQL contracts and stock plans without timing."""

from contextlib import contextmanager
from copy import deepcopy
from hashlib import sha256
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from tempfile import TemporaryDirectory

import overlay

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
FIXTURE = "tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition"


def dump(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def literal(value):
    return "'" + value.replace("'", "''") + "'"


def sql(db, query):
    return db.sql("mesh", query)


def read(db, query):
    return json.loads(sql(db, query))


def delta(db, before):
    return {key: diff for key, stats in overlay.counts(db).items()
            if any(diff := [x-y for x, y in zip(stats, before.get(key, [0, 0]))])}


def snapshot(db, namespace):
    ns = literal(namespace)
    return read(db, f"""
        SELECT jsonb_build_object(
           'jobs', (SELECT coalesce(jsonb_agg(to_jsonb(j) ORDER BY position), '[]')
                    FROM mesh_jobs.jobs j WHERE namespace={ns}),
           'receipts', (SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY ctid), '[]')
                        FROM mesh_jobs.receipts r WHERE namespace={ns}),
           'queues', (SELECT coalesce(jsonb_agg(name ORDER BY ctid), '[]')
                      FROM mesh_jobs.queues WHERE namespace={ns}),
           'fence', (SELECT count(*) FROM mesh_jobs.admissions WHERE namespace={ns}))
    """)


def normalize(snapshot):
    jobs = snapshot["jobs"]
    assert len({job["id"] for job in jobs}) == len(jobs)
    assert [job["position"] for job in jobs] == sorted({job["position"] for job in jobs})
    ids = {job["id"]: position for position, job in enumerate(jobs, 1)}
    for job in jobs:
        job["position"] = job["id"] = ids[job["id"]]
        clock = job.pop("inserted_at_ms")
        schedule = job.pop("scheduled_at_ms")
        job["schedule"] = schedule if abs(schedule - clock) > 86400000 else schedule - clock
    for row in snapshot["receipts"]:
        receipt = row["receipt"]
        uid = receipt["id"]
        assert receipt["effect_key"] == uid + "/1"
        receipt["id"] = ids[uid]
        receipt["effect_key"] = receipt["id"]
    return snapshot


def plans(db, out, writer, label):
    merge = writer.split("    RETURN QUERY\n    WITH inserted AS (", 1)[1].split("    GET DIAGNOSTICS", 1)[0]
    queue = writer.split("    INSERT INTO mesh_jobs.queues", 1)[1].split("    RETURN QUERY", 1)[0]
    for stem, query in (("merge", "WITH inserted AS (" + merge),
                        ("queue", "INSERT INTO mesh_jobs.queues" + queue)):
        query = query.strip().removesuffix(";").replace("elements(batch)", "elements($2)").replace("ns,", "$1,")
        plan = read(db, "SET ROLE parity_producer; SET plan_cache_mode=force_generic_plan; "
                    f"PREPARE witness(text,jsonb) AS {query};"
                    "EXPLAIN (VERBOSE, COSTS OFF, FORMAT JSON) EXECUTE witness('proof','[]');")
        (out / f"{label}-{stem}-plan.json").write_text(json.dumps(plan, indent=2) + "\n")


def unusual_specs():
    rows = []
    for i in range(32):
        key = {"request_key": f"k{i:02}"} if i != 13 else {}
        fields = (
            {},
            {"scheduled_at_ms": None, "delay_ms": None, "unique": None, "replace": None},
            {"version": 4294967295, "priority": 9, "max_attempts": 100, "timeout_ms": 31536000000,
             "backoff_base_ms": 0, "backoff_max_ms": 31536000000, "delay_ms": 1234},
            {"queue": "second", "tags": ["a", "b-1", "a"], "scheduled_at_ms": 4102444800000,
             "metadata": {"n": None, "array": [{"numeric": 1.25}, True, False]}},
            {"queue": "third", "suspended": True, "delay_ms": 99, "priority": 8},
            {"meta": {"nested": [{"obj": {"n": -1024.25, "u": "💧雪\\\"'", "s": ""}}]},
             "args": {"nested": {"children": [None, 18446744073709551615, 0.25, {"s": "\n\t"}]}}},
            {"scheduled_at_ms": 0, "backoff_base_ms": 1, "backoff_max_ms": 1},
            {"queue": "second", "worker": "other-worker", "version": 3},
        )[i % 8]
        rows.append({"worker": "parity", **key, **fields})
    return rows


def run_compiled(db, env, artifact, probe, scratch, batch):
    raw = {}
    expected = {"bulk": [8, 1024]}
    before = overlay.counts(db)
    broker = subprocess.run([artifact, "session"], env=dict(env, JOBS_PARITY_NAMESPACE="dense"),
                            input=dump({"verb": "enqueue", "batch": batch}) + "\nstop\n",
                            text=True, capture_output=True, timeout=30)
    broker.check_returncode()
    assert not broker.stderr, broker.stderr
    ready, receipt, stop = broker.stdout.splitlines()
    receipt = json.loads(receipt)
    assert ready == "SESSION_READY" and stop == "{}" and receipt["count"] == 1024
    assert delta(db, before) == expected
    dense = snapshot(db, "dense")
    assert [r["request_key"] for r in dense["receipts"]] == [j["key"] for j in batch["jobs"]]
    assert [(j["args"]["key"], j["args"]["payload"]) for j in dense["jobs"]] == [
        (j["key"], j["payload"]) for j in batch["jobs"]]
    assert {r["request_key"]: r["receipt"]["id"] for r in dense["receipts"]} == receipt["ids"]
    xid = literal(receipt["xid"])
    commits = read(db, f"""
      SELECT jsonb_build_array(
        (SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='dense'
          AND pg_xact_commit_timestamp(xmin)=pg_xact_commit_timestamp({xid}::xid)),
        (SELECT count(*) FROM mesh_jobs.receipts r JOIN mesh_jobs.jobs j
           ON j.namespace=r.namespace AND j.id=(r.receipt->>'id')::uuid
          WHERE r.namespace='dense' AND r.xmin=j.xmin
          AND pg_xact_commit_timestamp(r.xmin)=pg_xact_commit_timestamp({xid}::xid)),
        (SELECT count(*) FROM public.mesh_jobs_parity_product WHERE namespace='dense' AND xmin::text={xid}),
        pg_xact_commit_timestamp({xid}::xid) IS NOT NULL)
    """)
    assert commits == [1024, 1024, 1, True], commits
    raw["dense"] = dict(broker=receipt, durable=dense, calls=expected, commit_witness=commits)

    empty = dict(jobs=[], receipts=[], queues=[], fence=0)
    specs = unusual_specs()

    def send(label, items=specs, ns=None, expected=None, rollback=False, tail=None):
        namespace = ns or label
        path = scratch / "specs.json"
        path.write_text(dump(items))
        extra = {}
        if tail is not None:
            other = scratch / "tail.json"
            other.write_text(dump(tail))
            extra["DENSE_PROBE_TAIL"] = str(other)
        before = overlay.counts(db)
        process = subprocess.run([probe], text=True, capture_output=True, timeout=30,
                                 env=dict(env, DENSE_PROBE_INPUT=str(path), JOBS_PARITY_NAMESPACE=namespace,
                                          DENSE_PROBE_ROLLBACK=str(int(rollback))) | extra)
        process.check_returncode()
        assert not process.stderr, process.stderr
        reply, stats = json.loads(process.stdout), delta(db, before)
        if expected is not None:
            assert stats == expected, (label, stats)
        stored = snapshot(db, namespace)
        raw[label] = dict(reply=reply, durable=stored, calls=stats)
        return reply, stored

    reply, stored = send("unusual", expected={"bulk": [1, 32]})
    assert len(reply) == len(stored["jobs"]) == 32
    assert [r["id"] for r in reply] == [j["id"] for j in stored["jobs"]]
    assert stored["queues"] == ["default", "second", "third"] and len(stored["receipts"]) == 31
    for index, job in enumerate(stored["jobs"]):
        spec = specs[index]
        assert job["args"] == spec.get("args", {})
        assert job["metadata"] == spec.get("metadata", spec.get("meta", {}))
        assert job["tags"] == spec.get("tags", [])
        assert job["timeout_ms"] == spec.get("timeout_ms", 60000)
        assert job["scheduled_at_ms"] == spec.get("scheduled_at_ms") or (
            spec.get("scheduled_at_ms") is None
            and job["scheduled_at_ms"] == job["inserted_at_ms"] + (spec.get("delay_ms") or 0))
        assert job["state"] == reply[index]["state"] == (
            "suspended" if spec.get("suspended") else
            "scheduled" if job["scheduled_at_ms"] > job["inserted_at_ms"] else "available")
    assert [j["inserted_at_ms"] for j in stored["jobs"]] == sorted(j["inserted_at_ms"] for j in stored["jobs"])

    all_keys = [dict(item, request_key=f"retry{i}") for i, item in enumerate(specs)]
    reply, stored = send("rolled_back", all_keys, ns="retry", rollback=True, expected={"bulk": [1, 32]})
    assert len(reply) == 32 and stored == empty
    initial, stored = send("retry_fresh", all_keys, ns="retry", expected={"bulk": [1, 32]})
    assert set(r["id"] for r in initial).isdisjoint(r["id"] for r in reply)
    replay, retried = send("retry_receipt", all_keys, ns="retry", expected={"bulk": [1, 1], "scalar": [32, 32]})
    assert initial == replay and len(stored["jobs"]) == len(retried["jobs"]) == 32
    assert stored["receipts"] == retried["receipts"] and stored["jobs"] == retried["jobs"]
    changed = deepcopy(all_keys)
    changed[-1]["priority"] = 3
    error, got = send("retry_conflict", changed, ns="retry")
    assert error["transaction_usable"] and "different spec" in error["error"]
    assert got == retried

    # A second Jobs call fails at SQL bind after its SAVEPOINT. The probe
    # verifies the connection still executes SQL and rolls the outer unit back.
    nul = [dict(all_keys[0], args={"bad": "\0"})]
    error, got = send("savepoint", all_keys, tail=nul)
    assert error["transaction_usable"] and got == empty and raw["savepoint"]["calls"] == {"bulk": [1, 32]}

    for label, enter, leave, expected in (
        ("legacy", "ALTER FUNCTION " + overlay.WRITER + " RENAME TO enqueue_batch_hold",
         "ALTER FUNCTION mesh_jobs.enqueue_batch_hold(text,jsonb) RENAME TO enqueue_batch_v1", {"scalar": [32, 32]}),
        ("acl", "REVOKE EXECUTE ON FUNCTION " + overlay.WRITER + " FROM parity_producer",
         "GRANT EXECUTE ON FUNCTION " + overlay.WRITER + " TO parity_producer", {"scalar": [32, 32]}),
        ("layout", "ALTER TABLE mesh_jobs.jobs SET (fillfactor=95)",
         "ALTER TABLE mesh_jobs.jobs RESET (fillfactor)", {"bulk": [1, 1], "scalar": [32, 32]}),
    ):
        sql(db, enter)
        try:
            response, got = send(label, all_keys, expected=expected)
            assert len(response) == len(got["jobs"]) == 32
        finally:
            sql(db, leave)

    unique = [dict(item, unique="same-job") for item in all_keys]
    response, got = send("unique", unique, expected={"scalar": [32, 32]})
    assert len(response) == 32 and any(r["conflict"] for r in response) and len(got["jobs"]) < 32

    for index, fields in enumerate((
        {"args": None}, {"metadata": []}, {"tags": [1]}, {"worker": None}, {"worker": ""},
        {"priority": 2.5}, {"priority": "2"}, {"priority": -1}, {"version": 0},
        {"version": 4294967296}, {"timeout_ms": 0}, {"scheduled_at_ms": 1, "delay_ms": 1},
        {"scheduled_at_ms": -1}, {"delay_ms": 31536000001}, {"unknown": True},
        {"args": {"nested": [1]}, "meta": {}, "metadata": {}}, {"suspended": None},
    )):
        label = f"mesh_invalid_{index}"
        bad = [dict(item, **fields) for item in specs]
        error, got = send(label, bad, expected={})
        assert error["transaction_usable"] and error["error"] and got == empty
        raw[label]["fields"] = fields
    return raw


def direct_sql(db):
    baseline = dict(worker="direct", version=1, queue="test", args={}, metadata={}, tags=[],
                    priority=0, max_attempts=20, timeout_ms=60000, backoff_base_ms=1000,
                    backoff_max_ms=3600000, suspended=False, unique=None, replace={})
    items = {
        "absent_spec": {}, "null_spec": {"spec": None}, "array_spec": {"spec": [{}]},
        "scalar_spec": {"spec": 1}, "absent_key": {"spec": baseline, "fingerprint": "fp"},
    }
    vectors = {
        "ordinary": {}, "null_unique": {"unique": None}, "bad_unique": {"unique": {}},
        "missing_unique": {"unique": ...}, "missing_replace": {"replace": ...},
        "null_replace": {"replace": None}, "null_queue": {"queue": None},
        "missing_queue": {"queue": ...}, "number_queue": {"queue": 2.5},
        "object_queue": {"queue": {"b": [1, None], "a": "雪"}}, "array_queue": {"queue": [1, 2]},
        "empty_queue": {"queue": ""}, "null_args": {"args": None}, "missing_args": {"args": ...},
        "null_metadata": {"metadata": None}, "null_tags": {"tags": None},
        "fractional_priority": {"priority": 1.5}, "numeric_string": {"priority": "  +3 "},
        "decimal_numeric": {"priority": 2.0}, "priority_overflow": {"priority": 2147483648},
        "version_overflow": {"version": 9223372036854775808}, "invalid_boolean": {"suspended": "maybe"},
        "null_clock": {"scheduled_at_ms": None, "delay_ms": None},
        "short_circuit": {"scheduled_at_ms": "0", "delay_ms": "ignored"},
        "short_circuit_suspended": {"scheduled_at_ms": "0", "delay_ms": "ignored", "suspended": True},
        "delay_error": {"scheduled_at_ms": None, "delay_ms": "bad"},
        "addition_overflow": {"delay_ms": 9223372036854775807},
        "multiple_errors": {"version": "bad version", "delay_ms": "bad delay", "priority": "bad priority"},
    }
    for name, change in vectors.items():
        spec = dict(baseline, **change)
        items[name] = {"spec": {key: value for key, value in spec.items() if value is not ...},
                       "request_key": "", "fingerprint": "fp"}
    cases = {name: [item] * 16 for name, item in items.items()}
    cases.update({"batch_json_null": None, "batch_object": {}, "batch_empty": [],
                  "batch_null_element": [None] * 16, "batch_array_element": [[]] * 16})
    observations = {}
    args, env = db.psql("mesh")
    for name, batch in cases.items():
        value = literal(dump(batch)) + "::jsonb"
        query = ("BEGIN; SET LOCAL ROLE parity_producer; "
                 f"SELECT ordinal, CASE WHEN ordinal='0' THEN receipt ELSE (receipt::jsonb->>'state') END "
                 f"FROM mesh_jobs.enqueue_batch_v1('direct', {value}); "
                 "RESET ROLE; SELECT coalesce(jsonb_agg(name ORDER BY ctid),'[]') "
                 "FROM mesh_jobs.queues WHERE namespace='direct'; ROLLBACK;")
        result = subprocess.run([*args, "-v", "VERBOSITY=verbose", "-c", query],
                                env=env, text=True, capture_output=True, timeout=10)
        if result.returncode:
            match = re.search(r"ERROR:\s+(\w+): (.+)", result.stderr)
            assert match and snapshot(db, "direct") == dict(jobs=[], receipts=[], queues=[], fence=0)
            observed = dict(sqlstate=match[1], message=match[2])
        else:
            assert not result.stderr
            lines = result.stdout.splitlines()
            observed = dict(rows=lines[:-1], queues=json.loads(lines[-1]))
        observations[name] = dict(input=batch[0] if isinstance(batch, list) and batch else batch, **observed)
    return observations


@contextmanager
def fresh(backend, broker, folder):
    folder.mkdir()
    with TemporaryDirectory(dir=folder) as tmp:
        db = backend.Cluster(Path(tmp), folder, 96)
        db.runners, db.slots, db.lanes, db.hold_ms = 2, 32, 16, 0
        try:
            db.start()
            db.prepare_database("mesh")
            subject = backend.Subject("mesh", [str(broker)], broker.parent, db, db.env)
            env = subject.environment("dense", 32, 64, "dense", 250)
            subject.call("migrate", env, folder / "migrate.log")
            yield db, dict(env, JOBS_PARITY_DB_URL=db.url("mesh", "parity_producer"),
                           JOBS_PARITY_MESH_LISTEN="1", MESH_JOBS_SET_BASED_ENQUEUE="1")
        finally:
            db.stop()


def main(source, broker, probe, out):
    os.environ.update(PRODUCT_FROZEN_HARNESS=str(ROOT), PRODUCT_SOURCE=str(source), PRODUCT_STORAGE="default",
                      PRODUCT_TOPOLOGY="two-runner", PRODUCT_COMPOSITION="product-d2")
    sys.path.insert(0, str(source / FIXTURE / "replay"))
    import backend

    settings = json.loads(backend.arm.h.CONFIG.read_text())
    batch, = backend.arm.h.inputs(settings["workloads"]["noop"], 64, settings["seed"])
    out.mkdir()

    fixture_dir = out / "fixture"
    fixture_dir.mkdir(exist_ok=True)
    fixture, = backend.arm.h.batch_files(fixture_dir, [batch])
    pinned = overlay.digest(sha256(fixture.read_bytes()).hexdigest())
    assert pinned == "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
    results = {}
    for label in ("stock", "candidate"):
        with fresh(backend, broker, out / label) as (db, env):
            state = overlay.apply_writer(db, label == "candidate")
            plans(db, out, overlay.source() if label == "stock" else overlay.candidate(), label)
            compiled = run_compiled(db, env, broker, probe, out / label, batch)
            direct = direct_sql(db)
            result = dict(state=state, compiled=compiled, direct=direct)
            (out / label / "raw.json").write_text(dump(result) + "\n")
            results[label] = result
    a, b = (results[k] for k in ("stock", "candidate"))
    assert a["direct"] == b["direct"]
    assert a["compiled"].keys() == b["compiled"].keys()
    for label, left in a["compiled"].items():
        right = b["compiled"][label]
        assert left["calls"] == right["calls"], label
        assert normalize(deepcopy(left["durable"])) == normalize(deepcopy(right["durable"])), label
        if "fields" in left or label in ("retry_conflict", "savepoint"):
            assert left["reply"] == right["reply"], label
    assert read_candidate_sql() == overlay.candidate()
    print(dump(dict(input_sha256=pinned, server_version="17.11",
                    compiled_cases=list(a["compiled"]), direct_cases=list(a["direct"]),
                    broker_sha256=sha256(broker.read_bytes()).hexdigest(),
                    probe_sha256=sha256(probe.read_bytes()).hexdigest())))


def read_candidate_sql():
    return (HERE / "candidate.sql").read_text()


if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
