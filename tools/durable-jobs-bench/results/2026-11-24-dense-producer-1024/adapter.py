"""Use the frozen dense arm with an independently pinned compiled broker."""
from contextlib import contextmanager
import inspect
import json
import os
from pathlib import Path
import sys
from textwrap import dedent

from backend import Cluster, Subject, arm, install
from identity import observe, pinned_file, process_stamp

HERE = Path(__file__).resolve().parent


def patch(fn, changes, **extra):
    source = dedent(inspect.getsource(fn))
    for before, after in changes:
        if source.count(before) != 1:
            raise RuntimeError(f"frozen adapter anchor changed: {before}")
        source = source.replace(before, after)
    scope = dict(fn.__globals__, **extra)
    exec(compile(source, fn.__code__.co_filename, "exec"), scope)
    return scope[fn.__name__]


def producer_counters(cluster):
    return json.loads(cluster.sql("mesh", """
        SELECT coalesce(jsonb_object_agg(kind, jsonb_build_array(calls, rows)), '{}')
        FROM (
            SELECT kind, sum(calls) AS calls, sum(rows) AS rows
            FROM (
                SELECT CASE WHEN query LIKE '%FROM mesh_jobs.enqueue_dense_private_v1(%' THEN 'wide'
                       WHEN query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
                       THEN 'narrow' WHEN query LIKE '%SELECT mesh_jobs.enqueue(%' THEN 'scalar' END AS kind,
                       calls, rows
                FROM pg_stat_statements
                WHERE dbid = (SELECT oid FROM pg_database WHERE datname=current_database())
                  AND userid = (SELECT oid FROM pg_roles WHERE rolname='parity_producer')
            ) s WHERE kind IS NOT NULL GROUP BY kind
        ) s
    """))


def capture_writer(cluster, out, batches):
    wide = os.environ["DENSE_WIDTH"] == "1024"
    stats = producer_counters(cluster)
    if stats != ({"wide": [1, 1024]} if wide else {"narrow": [8, 1024]}):
        raise RuntimeError(f"expected all receipts from the assigned writer, observed {stats}")
    rows = json.loads(cluster.sql("mesh", """
        SELECT json_agg(json_build_object('key', request_key, 'receipt', receipt,
                                         'fingerprint', fingerprint) ORDER BY ctid)
        FROM mesh_jobs.receipts
    """))
    batch, = batches
    expected = [job["key"] for job in batch["jobs"]]
    ids = {row["key"]: row["receipt"]["id"] for row in rows}
    transaction, = cluster.receipts
    if [row["key"] for row in rows] != expected or ids != transaction["enqueued_ids"]:
        raise RuntimeError("durable receipts differ from input order or broker IDs")
    xid = int(transaction["xid"])
    assert cluster.sql("mesh", f"""SELECT count(*) = 1024 AND bool_and(
        pg_xact_commit_timestamp(xmin) = pg_xact_commit_timestamp('{xid}'::xid))
        FROM mesh_jobs.receipts""") == "t"
    arm.h.write_json(out / "producer-writer.json",
                     dict(enabled=True, bulk=0 if wide else 8, scalar=0, statements=stats))
    arm.h.write_json(out / "producer-receipts.json", rows)


class DenseSubject(Subject):
    @contextmanager
    def warm_producer(self, env, evidence, deadline):
        env = dict(env, MESH_JOBS_DENSE_PRODUCER_1024="1" if os.environ["DENSE_WIDTH"] == "1024" else "0")
        with super().warm_producer(env, evidence, deadline) as enqueue:
            yield enqueue

    def capture_runners(self, processes, logs):
        arm_subject.capture_runners(self, processes, logs)
        broker, start = self.broker_process
        broker_exe = Path(os.environ["DENSE_BROKER"])
        broker_sha = os.environ["DENSE_BROKER_SHA256"]
        executable = Path(self.command[0])
        cpus = sorted(os.sched_getaffinity(0))
        launches = [observe(broker, broker_exe, "session", broker_sha,
                            pinned_file(broker_exe, broker_sha), start, cpus)]
        launches += [observe(child, executable, "run", self.executable_sha, *child.product_stamp, cpus)
                     for child in processes]
        arm.h.write_json(self.cluster.evidence / "process-identity.json", launches)
        tls = json.loads((self.cluster.evidence / "postgres-tls-sessions.json").read_text())
        roles = [row["role"] for row in tls]
        assert roles.count("compiled_caller") == 32 and roles.count("parity_producer") == 1
        assert 4 <= roles.count("parity_subject") <= 8

    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            self.cluster.sql("mesh", (HERE / "install-writer.sql").read_text())
        return result


arm_subject = arm.Subject


def main():
    assert sys.platform == "linux" and sys.argv[1:3] == ["mesh", "noop"]
    assert sorted(os.sched_getaffinity(0)) == [0, 1] and os.environ["DENSE_WIDTH"] in ("128", "1024")
    artifact = Path(os.environ["NATIVE_JOBS_ARTIFACTS"])
    assert arm.h.digest(artifact / "mesh.json") == os.environ["PRODUCT_MANIFEST_SHA256"]
    broker = Path(os.environ["DENSE_BROKER"])
    assert arm.h.digest(broker) == os.environ["DENSE_BROKER_SHA256"]
    os.environ.update(PRODUCT_TOPOLOGY="two-runner", PRODUCT_STORAGE="default",
                      PRODUCT_COMPOSITION="product-d2", TRUSTED_ADMISSION_POLICY="success_cohort",
                      PRODUCT_CAPTURE_IDENTITY="1", MESH_JOBS_SET_BASED_ENQUEUE="1")
    _, slots, app_sessions, cap, memory, digest = arm.PROFILES["noop"]
    arm.PROFILES["noop"] = (2, slots, 4 * app_sessions, cap, memory, digest)
    install()
    parent = arm.regression.Subject
    original = parent.warm_producer.__wrapped__
    parent.warm_producer = contextmanager(patch(original, [
        ('[*self.command, "session"]', '[os.environ["DENSE_BROKER"], "session"]'),
        ('    ) as broker, output.open("rb") as reader:\n',
         '    ) as broker, output.open("rb") as reader:\n'
         '        self.broker_process = (broker, process_stamp(broker))\n'),
    ], process_stamp=process_stamp))
    source = dedent(inspect.getsource(arm.run_arm))
    begin = source.index('    if engine == "mesh":\n        statements = ')
    end = source.index("    database = database_settings", begin)
    arm.run_arm = patch(arm.run_arm, [(source[begin:end], "    capture_writer(cluster, out, batches)\n")],
                        capture_writer=capture_writer, Cluster=Cluster, Subject=DenseSubject)
    arm.main()


if __name__ == "__main__":
    main()
