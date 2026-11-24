"""Verify the public LARGE session with TLS PostgreSQL, without collecting timings."""

from contextlib import contextmanager
from hashlib import sha256
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

from build import HERE, sha

STOCK, BROKER, TRACE, ADAPTER, OUT = (Path(arg).resolve() for arg in sys.argv[1:6])
ONLY = set(sys.argv[6:])
sys.path.insert(0, str(ADAPTER))
from backend import Subject, arm, install_broker_observer

NS = "parity_0123456789abcdef"
h = arm.h


def select(db, expression):
    return db.sql("mesh", "SELECT " + expression)


def jobs(db):
    return json.loads(select(db, f"""coalesce(jsonb_agg(row ORDER BY key), '[]')::text FROM (
       SELECT args->>'key' AS key, id::text AS id, xmin::text AS xid,
           encode(sha256(convert_to(args->>'payload','UTF8')), 'hex') AS payload,
           args->>'hold_ms' AS hold_ms, args->>'fail_first' AS fail_first,
           worker, max_attempts, backoff_base_ms, backoff_max_ms
       FROM mesh_jobs.jobs WHERE namespace='{NS}') row"""))


def product(db):
    return json.loads(select(db, f"""coalesce(json_agg(request_id ORDER BY request_id), '[]')::text
       FROM public.mesh_jobs_parity_product WHERE namespace='{NS}'"""))


def writer_calls(db):
    return int(select(db, """coalesce(sum(calls), 0)
      FROM pg_stat_statements WHERE query LIKE 'SELECT mesh_jobs.enqueue(%'
      AND userid=(SELECT oid FROM pg_roles WHERE rolname='parity_producer')"""))


def identity(db):
    return json.loads(select(db, """json_build_object('pid', pg_backend_pid(),
        'producer_sessions', (SELECT count(*) FROM pg_stat_activity WHERE usename='parity_producer'),
        'tls', (SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()))"""))


def expect_rows(rows, batch, receipt):
    assert len(rows) == receipt["count"] == 64
    assert len({row["xid"] for row in rows}) == 64
    for row, job in zip(rows, batch["jobs"], strict=True):
        expected = dict(key=job["key"], id=receipt["ids"][job["key"]], xid=row["xid"],
                        payload=sha256(job["payload"].encode()).hexdigest(),
                        hold_ms=str(job["hold_ms"]), fail_first=str(job["fail_first"]).lower(),
                        worker="parity", max_attempts=3, backoff_base_ms=1000, backoff_max_ms=1000)
        assert row == expected, (row, expected)


def reference_frames(batch):
    source = HERE.parent / "2026-11-24-enqueue-spec-preparation/large-capture.json"
    capture = json.loads(source.read_text())
    assert capture["fixture_transport_sha256"] == sha256(
        (json.dumps(batch, indent=2, sort_keys=True) + "\n").encode()).hexdigest()
    frames = [dict(key=job["key"], count=frame["jobs"], bytes=frame["bytes"], sha256=frame["sha256"])
              for job, frame in zip(batch["jobs"], capture["frames"], strict=True)]
    assert all(row["bytes"] <= 1_048_576 for row in frames)
    assert all(a["bytes"] + b["bytes"] - 1 > 1_048_576 for a, b in zip(frames, frames[1:]))
    return frames


def send_command(path):
    return json.dumps(dict(verb="enqueue_file", batch=str(path), request_id=path.stem)).encode() + b"\n"


@contextmanager
def direct(directory, db, env, exe):
    directory.mkdir()
    producer_env = dict(env, JOBS_PARITY_DB_URL=db.url("mesh", "parity_producer"),
                        JOBS_PARITY_FIXTURE_DIR=str(db.scratch))
    with (directory / "stderr.log").open("wb") as errors, \
         (directory / "stdout.log").open("w+b") as output, \
         (directory / "stdout.log").open("rb") as reader, \
         subprocess.Popen([exe, "session"], env=producer_env, stdin=subprocess.PIPE,
                          stdout=output, stderr=errors) as proc:
        def read():
            mark = reader.tell()
            line = reader.readline()
            if line.endswith(b"\n"):
                return line
            reader.seek(mark)
            return None
        assert h.wait_for(read, time.monotonic() + 20, "SESSION_READY", .01) == b"SESSION_READY\n"
        try:
            yield proc
        finally:
            if proc.poll() is None:
                proc.kill()


def rejected(directory, db, env, exe, command, error):
    with direct(directory, db, env, exe) as proc:
        proc.stdin.write(command)
        proc.stdin.flush()
        assert proc.wait(timeout=60) != 0
    assert error in (directory / "stderr.log").read_text()
    assert (directory / "stdout.log").read_text() == "SESSION_READY\n"


def verify_arm(mode, label, exe):
    out = OUT / label
    out.mkdir()
    scratch = out / "scratch"
    scratch.mkdir(mode=0o700)
    db = arm.Cluster(scratch, out, 96)
    db.trusted_application = True
    db.runners, db.slots, db.lanes = 8, 64, 4
    try:
        db.start()
        db.prepare_database("mesh")
        subject = Subject("mesh", [str(STOCK)], STOCK.parent, db, db.env)
        env = subject.environment(NS, 8, 64, "dense", 250)
        env.update(JOBS_PARITY_MESH_LISTEN="1", MESH_JOBS_SET_BASED_ENQUEUE="1")
        subject.call("migrate", env, out / "migrate.log")
        os.environ.update(PRODUCT_SESSION_MODE=mode, PRODUCT_SESSION_EXECUTABLE=str(exe))
        config = json.loads(h.CONFIG.read_text())
        batch, = h.inputs(config["workloads"]["large"], 64, config["seed"])
        path = scratch / f'{batch["request_id"]}.json'
        with subject.warm_producer(env, out, time.monotonic() + 120) as send:
            before = identity(db)
            assert before["producer_sessions"] == 1 and before["tls"]
            assert not path.exists()
            h.write_json(path, batch)
            case = sha256(sha(path).encode()).hexdigest()
            assert case == arm.PROFILES["large"][-1]
            receipt, commit = send(path, out / "enqueue.log")
            assert commit is not None and len(receipt["ids"]) == 64
            accepted = jobs(db)
            expect_rows(accepted, batch, receipt)
            assert writer_calls(db) == 64 and product(db) == [path.stem]
            assert identity(db) == before
            if label == "file":
                alias = scratch / "batch_000001.json"
                alias.symlink_to(path)
                for invalid in (Path(path.name), alias, out / path.name):
                    try:
                        send(invalid, out / "invalid.log")
                    except ValueError as error:
                        assert "private trial scratch" in str(error)
                    else:
                        raise AssertionError(f"unexpected acceptance of {invalid}")
        frames = reference_frames(batch)
        if label.startswith("trace-"):
            lines = [line.split()[1:] for line in (out / "broker-errors.log").read_text().splitlines()
                     if line.startswith("SPEC ")]
            assert len(lines) == 64
            for (count, size, digest), row in zip(lines, frames, strict=True):
                assert (int(count), int(size), digest) == (row["count"], row["bytes"], row["sha256"]), (row, (count, size, digest))
        result = dict(input_sha256=sha(path), fixture_sha256=case, input_bytes=path.stat().st_size,
                      receipt=receipt, backend=before, persisted=accepted, scalar_calls=64)
        if label.startswith("trace-"):
            result["frames"] = frames

        if label == "file":
            path2 = scratch / "batch_000002.json"
            path2.write_bytes(b'{"jobs": [')
            rejected(out / "malformed-file", db, env, exe, send_command(path2), "EOF while parsing")
            rejected(out / "malformed-inline", db, env, exe, b'{"verb":\n', "EOF while parsing")
            rejected(out / "wrong-id", db, env, exe, send_command(path2).replace(
                b'"request_id": "batch_000002"', b'"request_id": "batch_000099"'), "private fixture directory")
            path2.write_text(json.dumps(dict(batch, jobs=[], request_id="batch_000099")))
            rejected(out / "wrong-body", db, env, exe, send_command(path2), "batch file does not match request ID")

            new_jobs = [dict(job, key="proof_" + job["key"]) for job in batch["jobs"]]
            malformed = [*new_jobs[:-1], dict(new_jobs[-1], hold_ms=-1)]
            path2.write_text(json.dumps(dict(batch, jobs=malformed, request_id=path2.stem)))
            before_calls = writer_calls(db)
            rejected(out / "late-invalid", db, env, exe, send_command(path2), "hold_ms must be nonnegative")
            assert writer_calls(db) - before_calls == 62
            assert jobs(db) == accepted and product(db) == [path.stem]

            path2.write_text(json.dumps(dict(batch, jobs=new_jobs[:1], request_id=path2.stem, rollback=True)))
            with direct(out / "explicit-rollback", db, env, exe) as proc:
                proc.communicate(send_command(path2) + b"stop\n", timeout=40)
                assert proc.returncode == 0
            abort, stop = (out / "explicit-rollback/stdout.log").read_text().splitlines()[1:]
            abort = json.loads(abort)
            assert abort["count"] == 1 and json.loads(stop) == {}
            assert select(db, f"pg_xact_commit_timestamp('{abort['xid']}'::xid) IS NULL") == "t"
            assert jobs(db) == accepted and product(db) == [path.stem]

            path2.write_text(json.dumps(dict(batch, jobs=new_jobs, request_id=path2.stem)))
            with direct(out / "lost-reply", db, env, exe) as proc:
                proc.stdin.write(send_command(path2))
                proc.stdin.flush()
                h.wait_for(lambda: path2.stem in product(db), time.monotonic() + 60,
                           "durable commit without consuming the reply", .02)
                first = {row["key"]: row["id"] for row in jobs(db) if row["key"].startswith("proof_")}
                assert len(first) == 64
            (out / "retry").mkdir()
            with subject.warm_producer(env, out / "retry", time.monotonic() + 120) as send:
                retry, retry_commit = send(path2, out / "retry-receipt.log")
                assert retry["ids"] == first and retry["count"] == 64 and retry_commit is not None
                assert len(jobs(db)) == 128 and product(db) == [path.stem, path2.stem]
            lost_receipt = json.loads((out / "lost-reply/stdout.log").read_text().splitlines()[1])
            assert lost_receipt["ids"] == first and retry["xid"] != lost_receipt["xid"]
            result.update(late_rollback_prior_writes=62, explicit_rollback=abort,
                          lost_reply=lost_receipt, retry=retry)
        h.write_json(out / "proof.json", result)
        return result
    finally:
        db.stop()
        shutil.rmtree(scratch)
        arm.redact(out, db.password)


OUT.mkdir()
install_broker_observer()
if sys.platform == "darwin":
    frozen_wait = h.wait_for

    def local_wait(probe, deadline, description, interval):
        # The local host hit the production 15 s watchdog in an open
        # transaction. Give this untimed correctness replay longer.
        if description == "shared producer/observer response":
            deadline = time.monotonic() + 90
        return frozen_wait(probe, deadline, description, interval)

    h.wait_for = local_wait
for mode, label, exe in (("file", "file", BROKER), ("inline", "inline", BROKER),
                         ("inline", "stock", STOCK), ("file", "trace-file", TRACE),
                         ("inline", "trace-inline", TRACE)):
    if ONLY and label not in ONLY:
        continue
    result = verify_arm(mode, label, exe)
    print(label, result["fixture_sha256"], flush=True)
if not ONLY or {"trace-file", "trace-inline"} <= ONLY:
    a, b = [json.loads((OUT / f"trace-{mode}" / "proof.json").read_text()) for mode in ("file", "inline")]
    assert a["frames"] == b["frames"]
