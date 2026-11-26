#!/usr/bin/env python3
"""Replay release Bind microbenchmarks and compiled Mesh against local PG17.11."""
import argparse
import collections
import contextlib
import fcntl
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import platform
import select
import shutil
import signal
import statistics
import subprocess
import tempfile
import time
import traceback

from wire import Bridge

HERE = Path(__file__).resolve().parent
PG = "compiler/mesh-rt/src/db/pg.rs"
BASE = "29abb9cbe97999029dd6383edce67a09fe11de5f"
ARMS = ("stock", "stock-b", "optimized-a", "optimized-b")
ORDERS = tuple(tuple(ARMS[i] for i in order) for order in (
    (0, 1, 3, 2), (1, 2, 0, 3), (2, 3, 1, 0), (3, 0, 2, 1),
    (2, 3, 1, 0)[::-1], (3, 0, 2, 1)[::-1], (0, 1, 3, 2)[::-1], (1, 2, 0, 3)[::-1],
))
FIXTURE = "proof/jobs-parity/linux-integrated-uncut/fixture"
LARGE_INPUT_SHA = "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def emit(path, record):
    with path.open("x") as out:
        json.dump(record, out, sort_keys=True, indent=2)
        out.write("\n")


def section(source, start, end):
    return source[source.index(start):source.index(end, source.index(start))]


def extract(root, candidate, scratch):
    original = run("git", "-C", root, "show", f"{BASE}:{PG}", capture_output=True).stdout
    optimized = (candidate / PG).read_text()
    shared = "\n".join(line for line in original.splitlines() if line.startswith(("pub(crate) const MAX_DB_VALUE_BYTES:", "const MAX_PG_MESSAGE_BYTES:", "const MAX_PG_VALUES:")))
    for start, end in (
        ("fn write_parse(", "/// Write a Bind"),
        ("fn write_describe_portal(", "/// Write a PasswordMessage"),
        ("    fn write_wire_all(", "    pub(super) fn read_wire_message"),
    ):
        piece = section(original, start, end)
        assert piece == section(optimized, start, end)
        if "write_wire_all" in piece:
            piece = f"impl PgConn {{\n{piece}}}"
        shared += "\n" + piece
    helpers = section(optimized, "const EXECUTE_SYNC_BYTES:", "/// Write Describe (Portal)")
    stock = section(original, "fn write_bind_values(", "/// Write Describe (Portal)")
    opt = section(optimized, "fn write_bind_values(", "const EXECUTE_SYNC_BYTES:")
    (scratch / "extracted.rs").write_text(shared + "\n" + helpers + stock.replace("write_bind_values", "stock_bind") + opt.replace("write_bind_values", "optimized_bind"))
    shutil.copyfile(HERE / "micro.rs", scratch / "micro.rs")
    for name, profile in (("cpu", []), ("alloc", ["--cfg", "profile_alloc"])):
        run("rustc", "--edition=2024", "-C", "opt-level=3", *profile, scratch / "micro.rs", "-o", scratch / name)
    return run(scratch / "cpu", "check", capture_output=True).stdout.strip()


@contextlib.contextmanager
def timing(root):
    # Both locks are established by the repository's local timing lanes.
    paths = (root / "target/queue-performance.lock", root / "tools/durable-jobs-bench/.timing.lock")
    with contextlib.ExitStack() as stack:
        locks = [stack.enter_context(p.open("a")) for p in paths]
        while True:
            try:
                for lock in locks:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                for lock in locks:
                    fcntl.flock(lock, fcntl.LOCK_UN)
                time.sleep(1)
        yield


def micros(scratch, job_spec):
    measurements = []
    for group, order in enumerate(ORDERS, 1):
        for arm in order:
            for shape in ("short", "scalar", "mixed", "jobs"):
                for mode in ("cpu", "alloc") if group == 1 else ("cpu",):
                    output = run(scratch / mode, arm, shape, mode, job_spec, capture_output=True).stdout
                    data = {field: int(value) for field, value in (line.split("=") for line in output.splitlines())}
                    measurements.append(dict(group=group, arm=arm, shape=shape, mode=mode, **data))
    return measurements


def load_parity(root):
    spec = importlib.util.spec_from_file_location("parity", root / "scripts/jobs-parity.py")
    parity = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parity)
    return parity


def jobs_fixture(root, project):
    # The pinned uncut fixture plus its native helper, archived for this host.
    project.mkdir()
    pinned = root / FIXTURE
    for name, sha256 in {
        "main.mpl": "0c72aaa2f0f34c49d07fd2f6cad2d39f491c0857897a0ac7a0facb6b85b4464d",
        "worker_identity.mpl": "e56589a1df5442a8c82b95950314e68776145b2bd6215e22c5014ee14c9d9fca",
    }.items():
        shutil.copyfile(pinned / name, project / name)
        assert digest(project / name) == sha256, name
    identity = root / "tests/e2e/durable_jobs/.parity/worker_identity.c"
    assert digest(identity) == "2808ca2bd2a286b79c8bf071a99453227395265b761571075a4f6c06780a63a0"
    run("cc", "-std=c11", "-O2", "-c", identity, "-o", project / "worker_identity.o")
    run("ar", "rcs", project / "libworker_identity.a", project / "worker_identity.o")
    triple = next(line.removeprefix("host: ") for line in run("rustc", "-vV", capture_output=True).stdout.splitlines() if line.startswith("host: "))
    (project / "mesh.toml").write_text(f"""[package]
name = "jobs-parity-mesh"
version = "0.1.0"

[native]
abi = 1
bindings = ["worker_identity.mpl"]

[[native.libraries]]
target = "{triple}"
path = "libworker_identity.a"
sha256 = "{digest(project / 'libworker_identity.a')}"
""")
    return project


def compile_arm(compiler, runtime, source, output):
    env = dict(os.environ, MESH_RT_LIB_PATH=str(runtime))
    run(compiler, "build", source, "-o", output, env=env)


def proc_line(proc, timeout):
    if not select.select([proc.stdout], [], [], timeout)[0]:
        raise TimeoutError(f"Mesh stdout timeout ({timeout}s)")
    line = proc.stdout.readline()
    if not line:
        raise RuntimeError(f"Mesh closed stdout with status {proc.poll()}")
    return line.strip()


def finish(proc, timeout=20):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        pid, status, usage = os.wait4(proc.pid, os.WNOHANG)
        if pid:
            proc.returncode = os.waitstatus_to_exitcode(status)
            assert proc.returncode == 0, proc.returncode
            return dict(cpu_seconds=usage.ru_utime + usage.ru_stime, peak_rss_bytes=usage.ru_maxrss * (1 if platform.system() == "Darwin" else 1024))
        time.sleep(0.002)
    raise TimeoutError("Mesh process failed to exit")


def child(exe, args, env, error_file, drive):
    with error_file.open("w") as stderr:
        proc = subprocess.Popen([exe, *args], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True)
        try:
            receipt = drive(proc)
            receipt.update(finish(proc))
            assert proc.stdout.read() == ""
            assert not error_file.read_text()
            return receipt
        finally:
            if proc.returncode is None:
                proc.kill()
                proc.wait()
            proc.stdin.close()
            proc.stdout.close()


def bind_trial(cluster, database, executable, scale, count, scratch, port=None):
    env = dict(cluster.env, BIND_DATABASE_URL=cluster.url(database, port=port), BIND_SCALE=str(scale), BIND_COUNT=str(count))
    cluster.sql(database, """
        SET ROLE parity_subject;
        CREATE TABLE bind_scalar(id bigint PRIMARY KEY, payload text NOT NULL);
        CREATE TABLE bind_mixed(id bigint PRIMARY KEY, payload bytea NOT NULL, label text NOT NULL, optional text);
    """)

    def drive(proc):
        assert proc_line(proc, 30) == "ready"
        start = time.perf_counter_ns()
        proc.stdin.write("go\n")
        proc.stdin.flush()
        assert proc_line(proc, 120) == f"committed:{count}"
        return dict(wall_ns=time.perf_counter_ns() - start)

    result = child(executable, [], env, scratch / f"{database}.stderr", drive)
    ledger = json.loads(cluster.sql(database, f"""
        SELECT json_build_object('rows', count(*), 'xids', count(DISTINCT s.xmin::text),
            'ids', array_agg(s.id ORDER BY s.id),
            'data', bool_and(s.xmin=m.xmin AND pg_xact_commit_timestamp(s.xmin) IS NOT NULL
                AND sha256(convert_to(s.payload,'UTF8'))=sha256(convert_to(repeat('x', {21 * scale}),'UTF8'))
                AND sha256(m.payload)=sha256(decode(repeat('a5',{17 * scale}),'hex'))
                AND sha256(convert_to(m.label,'UTF8'))=sha256(convert_to(repeat('z',{7 * scale}),'UTF8'))
                AND m.optional IS NULL))
        FROM bind_scalar s JOIN bind_mixed m USING(id) WHERE s.id>0
    """))
    assert ledger == dict(rows=count, xids=count, ids=list(range(1, count + 1)), data=True), ledger
    assert cluster.sql(database, "SELECT count(*) FROM bind_scalar") == str(count + 1)
    assert cluster.sql(database, "SELECT count(*) FROM bind_mixed") == str(count + 1)
    select_calls = json.loads(cluster.sql(database, """
        SELECT json_object_agg(
          CASE WHEN query LIKE 'INSERT INTO bind_scalar %' THEN 'scalar'
               WHEN query LIKE 'INSERT INTO bind_mixed %' THEN 'mixed'
               ELSE 'typed_query' END, json_build_array(calls,total_exec_time))
        FROM pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database())
        AND (query LIKE 'INSERT INTO bind_scalar %' OR query LIKE 'INSERT INTO bind_mixed %' OR query LIKE 'SELECT needle, label, optional, %')
    """))
    assert set(select_calls) == {"scalar", "mixed", "typed_query"}, select_calls
    assert all(calls == count + 1 for calls, _ in select_calls.values()), select_calls
    return dict(scale=scale, committed_rows=count, committed_transactions=count, sql=select_calls, **result)


def job_inputs(parity, scratch):
    settings = json.loads(parity.CONFIG.read_text())
    batches = parity.inputs(settings["workloads"]["large"], 64, settings["seed"])
    inputs = scratch / "frozen-input"
    inputs.mkdir()
    files = parity.batch_files(inputs, batches)
    input_sha = hashlib.sha256("".join(parity.digest(p) for p in files).encode()).hexdigest()
    assert input_sha == LARGE_INPUT_SHA
    assert len(batches) == 1 and len(batches[0]["jobs"]) == 64
    return batches[0]


def jobs_trial(cluster, database, executable, batch, scratch, port=None):
    namespace = "parity_ordered_fixture"
    identity = "SELECT json_build_object('ssl', ssl, 'version', current_setting('server_version_num'), 'fsync', current_setting('fsync'), 'commit', current_setting('synchronous_commit')) AS value FROM pg_stat_ssl WHERE pid=pg_backend_pid()"
    env = dict(cluster.env, JOBS_PARITY_DB_URL=cluster.url(database, "parity_producer", port),
               JOBS_PARITY_NAMESPACE=namespace, JOBS_PARITY_TOTAL_SLOTS="64", JOBS_PARITY_MODE="dense",
               JOBS_PARITY_MESH_LISTEN="1", JOBS_PARITY_MESH_BATCH="1",
               MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2")
    request = json.dumps({"verb": "enqueue", "batch": batch}, separators=(",", ":")) + "\n"
    expected = {job["key"]: hashlib.sha256(job["payload"].encode()).hexdigest() for job in batch["jobs"]}

    def drive(proc):
        assert proc_line(proc, 30) == "SESSION_READY"
        proc.stdin.write(json.dumps({"verb": "query", "sql": identity}) + "\n")
        proc.stdin.flush()
        assert json.loads(proc_line(proc, 30)) == dict(ssl=True, version="170011", fsync="on", commit="on")
        start = time.perf_counter_ns()
        proc.stdin.write(request)
        proc.stdin.flush()
        response = json.loads(proc_line(proc, 120))
        wall = time.perf_counter_ns() - start
        proc.stdin.write("stop\n")
        proc.stdin.flush()
        assert proc_line(proc, 20) == "{}"
        assert response["count"] == 64
        return dict(response=response, wall_ns=wall)

    trial = child(executable, ["session"], env, scratch / f"{database}.stderr", drive)
    response = trial.pop("response")
    ledger = json.loads(cluster.sql(database, """
        SELECT json_build_object(
          'jobs', (SELECT json_object_agg(j.args->>'key', json_build_array(j.id::text,
              encode(sha256(convert_to(j.args->>'payload','UTF8')), 'hex'),
              j.xmin = r.xmin AND pg_xact_commit_timestamp(j.xmin) = (SELECT pg_xact_commit_timestamp(xmin) FROM public.mesh_jobs_parity_product)))
           FROM mesh_jobs.jobs j JOIN mesh_jobs.receipts r ON (r.namespace=j.namespace AND r.request_key=j.args->>'key')),
          'counts', ARRAY[(SELECT count(*) FROM mesh_jobs.jobs), (SELECT count(*) FROM mesh_jobs.receipts), (SELECT count(*) FROM public.mesh_jobs_parity_product)],
          'xid', (SELECT xmin::text FROM public.mesh_jobs_parity_product))
    """))
    assert ledger["counts"] == [64, 64, 1] and ledger["xid"] == response["xid"], ledger["counts"]
    assert ledger["jobs"] == {key: [response["ids"][key], sha, True] for key, sha in expected.items()}
    calls = cluster.sql(database, """
      SELECT calls FROM pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database())
        AND query LIKE 'SELECT mesh_jobs.enqueue($1, $2::jsonb, $3, $4)::text AS receipt%'
    """)
    assert int(calls) == 64, calls
    return dict(jobs=64, enqueue_calls=64, committed_transactions=1, **trial)


def enable_stats(cluster, database):
    cluster.sql(database, "CREATE EXTENSION pg_stat_statements")


def new_db(cluster, name, template=None):
    if template:
        cluster.sql("postgres", f"CREATE DATABASE {name} WITH TEMPLATE {template} OWNER parity_subject")
    else:
        cluster.prepare_database(name)
        enable_stats(cluster, name)


def drop_db(cluster, name):
    cluster.sql("postgres", f"DROP DATABASE {name}")


def keep_wire(bridge):
    # Full individual frame digests live in diagnostic scratch. Group by workload
    # body shape in the portable receipt while retaining a full-sequence digest.
    counts = collections.Counter()
    binds = collections.Counter()
    by_sql = {}
    sql = ""
    for exchange in bridge.transcript:
        counts["".join(frame["tag"] for frame in exchange)] += 1
        for frame in exchange:
            if frame["tag"] == "P":
                sql = frame["sql"]
            if "bind" in frame:
                binds[json.dumps(frame["bind"], sort_keys=True)] += 1
                totals = by_sql.setdefault(sql, collections.Counter())
                totals.update(calls=1, bind_frame_bytes=frame["bytes"], stock_body_copy_bytes=frame["bytes"] - 5,
                              parameter_bytes=sum(max(length, 0) for length in frame["bind"]["lengths"]))
    canonical = json.dumps(bridge.transcript, sort_keys=True, separators=(",", ":")).encode()
    return dict(tls=bridge.tls, exchange_counts=counts, bind_shapes=[dict(count=n, **json.loads(key)) for key, n in sorted(binds.items())],
                frontend_bytes=sum(f["bytes"] for f in itertools.chain.from_iterable(bridge.transcript)),
                transcript_sha256=hashlib.sha256(canonical).hexdigest(), bind_totals_by_sql=by_sql)


def score(cluster, bins, job_bins, template, batch, scratch, score_jobs):
    records = []
    for group, order in enumerate(ORDERS, 1):
        for shape, scale, count in (("short", 1, 384), ("large", 393216, 12), ("jobs", 0, 0)):
            if shape == "jobs" and not score_jobs:
                continue
            for arm in order:
                db = "bind_timed"
                new_db(cluster, db, template if shape == "jobs" else None)
                before = os.getloadavg()
                try:
                    if shape == "jobs":
                        result = jobs_trial(cluster, db, job_bins[arm], batch, scratch)
                    else:
                        result = bind_trial(cluster, db, bins[arm], scale, count, scratch)
                finally:
                    drop_db(cluster, db)
                row = dict(group=group, shape=shape, arm=arm, load_before=before, **result)
                records.append(row)
                print(json.dumps(row), flush=True)
    return records


def main(args):
    root = args.root.resolve()
    out = args.out.resolve()
    args.scratch.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    parity = load_parity(root)
    with tempfile.TemporaryDirectory(prefix="bind-", dir=args.scratch.resolve()) as directory:
        scratch = Path(directory)
        os.environ["TMPDIR"] = str(scratch)
        with (scratch / "differential.txt").open("w") as out_check:
            print(extract(root, args.candidate.resolve(), scratch), file=out_check)
        shutil.copyfile(scratch / "differential.txt", out / "differential.txt")
        compiler = args.arms.resolve() / "stock/meshc"
        runtimes = {arm: args.arms.resolve() / arm / "libmesh_rt.a" for arm in ARMS}
        compiled, jobs = {}, {}
        app = scratch / "fixture"
        app.mkdir()
        shutil.copyfile(HERE / "main.mpl", app / "main.mpl")
        job_app = jobs_fixture(root, scratch / "jobs-fixture")
        for arm, runtime in runtimes.items():
            compiled[arm] = scratch / f"bind-{arm}"
            jobs[arm] = scratch / f"jobs-{arm}"
            compile_arm(compiler, runtime, app, compiled[arm])
            compile_arm(compiler, runtime, job_app, jobs[arm])
        emit(out / "builds.json", dict(base=BASE, compiler=digest(compiler),
             host=platform.platform(), cpus=os.cpu_count(), rustc=run("rustc", "-Vv", capture_output=True).stdout,
             archives={arm: digest(path) for arm, path in runtimes.items()},
             mesh={arm: digest(path) for arm, path in compiled.items()}, jobs={arm: digest(path) for arm, path in jobs.items()}))
        batch = job_inputs(parity, scratch)
        logs = scratch / "pg-logs"
        logs.mkdir()
        cluster = parity.Cluster(scratch, logs, 32, "17.11")
        partial_path = scratch / "job-capture"
        try:
            cluster.start()
            template = "bind_template"
            new_db(cluster, template)
            migrate_env = dict(cluster.env, JOBS_PARITY_DB_URL=cluster.url(template),
                               JOBS_PARITY_NAMESPACE="parity_ordered_fixture", JOBS_PARITY_TOTAL_SLOTS="64")
            run(jobs["stock"], "migrate", env=migrate_env, capture_output=True)
            diagnostics = {}
            for arm in ("stock", "optimized-a"):
                for shape in ("large", "jobs"):
                    db = "bind_wire"
                    new_db(cluster, db, template if shape == "jobs" else None)
                    try:
                        with Bridge(cluster, partial_path if arm == "stock" and shape == "jobs" else None) as bridge:
                            if shape == "jobs":
                                verified = jobs_trial(cluster, db, jobs[arm], batch, scratch, bridge.port)
                            else:
                                verified = bind_trial(cluster, db, compiled[arm], 393216, 2, scratch, bridge.port)
                        wire = keep_wire(bridge)
                        emit(scratch / f"frames-{shape}-{arm}.json", bridge.transcript)
                        diagnostics[(shape, arm)] = (wire, verified)
                    finally:
                        drop_db(cluster, db)
            for shape in ("large", "jobs"):
                assert diagnostics[(shape, "stock")][0] == diagnostics[(shape, "optimized-a")][0], shape
            lengths = [partial_path.with_suffix(f".{i}").stat().st_size for i in range(4)]
            emit(out / "wire.json", dict(input_sha256=LARGE_INPUT_SHA, jobs_enqueue_arg_bytes=lengths,
                 results={f"{shape}-{arm}": dict(wire=wire, verified=verified) for (shape, arm), (wire, verified) in diagnostics.items()}))
            with timing(root):
                emit(out / "micro.json", micros(scratch, partial_path))
                emit(out / "live.json", score(cluster, compiled, jobs, template, batch, scratch, args.score_jobs))
        except BaseException:
            detail = traceback.format_exc()
            for path in scratch.glob("*.stderr"):
                detail += f"\n{path.name}\n" + path.read_text()
            (out / "failure.log").write_text(detail.replace(cluster.password, "<redacted>").replace(cluster.nonce, "<redacted>"))
            raise
        finally:
            cluster.stop()


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda number, _: exit(128 + number))
    p = argparse.ArgumentParser(description=__doc__)
    for flag in ("root", "candidate", "scratch", "arms", "out"):
        p.add_argument(f"--{flag}", type=Path, required=True)
    p.add_argument("--score-jobs", action="store_true")
    main(p.parse_args())
