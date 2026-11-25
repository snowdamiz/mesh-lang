"""Run the frozen compiled broker and runner with the identical logical grant in every arm."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import runpy
import sys
import time

import backend
from contract import apply, qualify

HERE = Path(__file__).resolve().parent
LANE = sys.argv.pop(1)
CANDIDATE = (HERE / "candidate.sql").read_text()
assert os.environ["PRODUCT_COMPOSITION"] == "product-d2"
assert os.environ["PRODUCT_TOPOLOGY"] == "stock"
assert os.environ["PRODUCT_STORAGE"] == "default"
assert sys.argv[1:3] == ["mesh", "short"]


def pg_frame(postmaster):
    begin = time.time_ns() // 1000
    root = Path("/proc") / str(postmaster)
    children = (root / "task" / str(postmaster) / "children").read_text().split()
    samples = {}
    for pid in (postmaster, *map(int, children)):
        try:
            stat = (Path("/proc") / str(pid) / "stat").read_text().rsplit(")", 1)[1].split()
        except FileNotFoundError:
            if pid == postmaster:
                raise
        else:
            samples[str(pid)] = [int(stat[1]), int(stat[19]), int(stat[11]) + int(stat[12])]
    return dict(begin_us=begin, end_us=time.time_ns() // 1000, rows=samples)


class Cluster(backend.Cluster):
    observations = 0

    def calls(self, engine):
        self.observations += 1
        if self.observations != 3:
            return super().calls(engine)
        after = pg_frame(self.postmaster)
        result = super().calls(engine)
        backend.arm.h.write_json(self.evidence / "pg-cpu-bracket.json", dict(
            postmaster=self.postmaster, clock_ticks=os.sysconf("SC_CLK_TCK"),
            before=self.before, after=after))
        return result

    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
        batch = self.sql(engine, """
            SELECT json_build_object('calls', sum(calls), 'rows', sum(rows))
            FROM pg_stat_statements
            WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
              AND userid = 'parity_producer'::regrole
              AND query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
        """)
        backend.arm.h.write_json(self.evidence / "short-producer-batch.json", json.loads(batch))
        return result


class Subject(backend.Subject):
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            cluster = self.cluster
            assert cluster.sql("mesh", "SHOW server_version_num") == "170011"
            if LANE == "proof-fixed":
                try:
                    witness = qualify(cluster, CANDIDATE)
                finally:
                    probe = cluster.evidence / "grant-probe.log"
                    if probe.exists():
                        probe.write_text(re.sub(r'("token"\s*:\s*")[^"]+',
                                                r'\1[redacted]', probe.read_text()))
                backend.arm.h.write_json(cluster.evidence / "grant-contract.json", witness)
            overlay = apply(cluster, "mesh", CANDIDATE)
            backend.arm.h.write_json(cluster.evidence / "logical-overlay.json", dict(
                lane=LANE, observer=False, **overlay))
        return result

    @contextmanager
    def warm_producer(self, env, evidence, deadline):
        with super().warm_producer(env, evidence, deadline) as enqueue:
            def submit(path, log):
                cluster = self.cluster
                cluster.postmaster = int((cluster.data / "postmaster.pid").read_text().splitlines()[0])
                cluster.before = pg_frame(cluster.postmaster)
                return enqueue(path, log)

            yield submit


backend.Cluster, backend.Subject = Cluster, Subject
runpy.run_path(str(HERE / "product_arm.py"), run_name="__main__")
