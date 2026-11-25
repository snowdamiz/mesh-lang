"""Use the public grouped-hot runner, with observation limited to group zero."""

from contextlib import contextmanager
import os
from pathlib import Path
import runpy
import sys
import time

import backend
from contract import apply, qualify

HERE = Path(__file__).resolve().parent
LANE, MODULE = sys.argv[1:3]
del sys.argv[1:3]
PROBE = LANE.startswith("plan-")
CANDIDATE = (HERE / "candidate.sql").read_text() if LANE in {"plan-key", "key"} else None
assert (MODULE != "-") == PROBE
assert os.environ["PRODUCT_COMPOSITION"] == "product-d2"
assert os.environ["PRODUCT_TOPOLOGY"] == "two-runner"
assert os.environ["PRODUCT_STORAGE"] == "default"


def pg_frame(pid):
    begin = time.time_ns() // 1000
    root = Path("/proc") / str(pid)
    children = (root / "task" / str(pid) / "children").read_text().split()
    rows = {}
    for process in (pid, *map(int, children)):
        try:
            stat = (Path("/proc") / str(process) / "stat").read_text().rsplit(")", 1)[1].split()
        except FileNotFoundError:
            if process == pid:
                raise
        else:
            rows[str(process)] = [int(stat[1]), int(stat[19]), int(stat[11]) + int(stat[12])]
    return dict(begin_us=begin, end_us=time.time_ns() // 1000, rows=rows)


class Cluster(backend.Cluster):
    observations = 0

    def calls(self, engine):
        self.observations += 1
        if self.observations != 3 or PROBE:
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
            if PROBE:
                witness = qualify(cluster, CANDIDATE)
                backend.arm.h.write_json(cluster.evidence / "grant-contract.json", witness)
            meta = apply(cluster, "mesh", CANDIDATE)
            backend.arm.h.write_json(cluster.evidence / "logical-overlay.json", dict(
                lane=LANE, observer=PROBE, **meta))
        return result

    @contextmanager
    def warm_producer(self, env, evidence, deadline):
        with super().warm_producer(env, evidence, deadline) as enqueue:
            if PROBE:
                yield enqueue
                return

            def submit(path, log):
                cluster = self.cluster
                cluster.postmaster = int((cluster.data / "postmaster.pid").read_text().splitlines()[0])
                cluster.before = pg_frame(cluster.postmaster)
                return enqueue(path, log)

            yield submit


backend.Cluster, backend.Subject = Cluster, Subject
if PROBE:
    sys.argv[1:1] = ["sampled", MODULE]
    runpy.run_path(str(HERE / "observe.py"), run_name="__main__")
else:
    runpy.run_path(str(HERE / "product_arm.py"), run_name="__main__")
