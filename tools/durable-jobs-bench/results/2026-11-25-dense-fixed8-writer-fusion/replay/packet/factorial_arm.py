"""Run the public grouped-refill launcher with only the assigned grant body."""

from contextlib import contextmanager
import os
from pathlib import Path
import re
import runpy
import sys
import time

import backend
from contract import apply, claim, hold, psql, qualify, rows, seed

HERE = Path(__file__).resolve().parent
LANE = sys.argv.pop(1)
SMOKE = LANE == "positive"
CANDIDATE = (HERE / "candidate.sql").read_text()
assert os.environ["PRODUCT_COMPOSITION"] == "product-d2"
assert os.environ["PRODUCT_TOPOLOGY"] == "two-runner"
assert os.environ["PRODUCT_STORAGE"] == "default"
assert os.environ["JOBS_GROUPED_WRITER"] == (
    "fused_fixed8ms" if LANE in {"fused", "positive"} else "fixed8ms")


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


def contraction(cluster):
    seed(cluster)
    psql(cluster, "SELECT mesh_jobs.queue_control('dense','default','scale',2);", role="parity_operator")
    with hold(cluster, f"SELECT count(*) AS grants FROM {claim()} \\gset", commit=True):
        assert psql(cluster, "SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing';") == "0"
    live = """
        SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='dense' AND state='executing'
          AND attempt=1 AND lease_token IS NOT NULL AND lease_expires_at_ms > 0
    """
    assert psql(cluster, live) == "2"
    with hold(cluster, """SELECT mesh_jobs.queue_control('dense','default','scale',1) AS scaled
                         \\gset""", role="parity_operator", commit=True):
        rows(cluster, expect="55P03")
    assert rows(cluster) == [] and psql(cluster, live) == "2"
    for remaining in (1, 0):
        assert psql(cluster, """
            SELECT mesh_jobs.settle_state('dense',id,lease_token,'{"action":"complete"}',false)
            FROM (SELECT id,lease_token FROM mesh_jobs.jobs
                  WHERE namespace='dense' AND state='executing' ORDER BY id LIMIT 1) AS member
        """) == "completed"
        assert int(psql(cluster, live)) == remaining
        assert len(rows(cluster)) == (0 if remaining else 1)
    return dict(held_contraction_sqlstate="55P03", old_capacity=2, new_capacity=1,
                executing_after_contraction=2, grants_at_or_above_cap=0, grants_after_release=1)


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


class Subject(backend.Subject):
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            cluster = self.cluster
            assert cluster.sql("mesh", "SHOW server_version_num") == "170011"
            if SMOKE:
                try:
                    witness = qualify(cluster, CANDIDATE)
                    witness["contraction"] = contraction(cluster)
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
