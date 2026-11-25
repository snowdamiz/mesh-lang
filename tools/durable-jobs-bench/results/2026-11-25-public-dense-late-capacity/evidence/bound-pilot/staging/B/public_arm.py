"""Reuse the released PostgreSQL CPU observation around the public arm."""
from collections import Counter
from contextlib import contextmanager
from hashlib import sha256
import json
from pathlib import Path
import os
import runpy
import sys
import time

REPLAY = Path(sys.argv.pop(1))
sys.path.insert(0, str(REPLAY))
import backend

LANE = os.environ["PUBLIC_LANE"]
if LANE == "bounded-grouped-cold":
    backend.MODES["product-d2"] = ("success_cohort", False)
assert backend.arm.PROFILES["noop"][:4] == (8, 64, 4, 96)


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


class Subject(backend.Subject):
    def run(self, env, index, log):
        supply = [env[key] for key in ("MESH_SCHEDULER_MIN_WORKERS", "MESH_SCHEDULER_MAX_WORKERS",
                                       "MESH_EFFECT_LANES")]
        assert supply == ["2", "2", "4"]
        child = super().run(env, index, log)
        if index == 0:
            self.supply = {}
        self.supply[str(child.pid)] = supply
        return child

    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        self.runner_pids = [child.pid for child in processes]
        assert len(self.runner_pids) == len(set(self.runner_pids)) == 8
        assert all(child.poll() is None for child in processes)

    def ready_roster(self):
        expected = Counter(compiled_caller=32, parity_subject=32, parity_producer=1)

        def probe():
            sessions = json.loads(self.cluster.sql("mesh", """
                SELECT COALESCE(json_agg(json_build_object(
                    'backend', a.pid, 'role', a.usename, 'tls', s.ssl
                ) ORDER BY a.pid), '[]')
                FROM pg_stat_activity a JOIN pg_stat_ssl s USING (pid)
                WHERE a.datname = 'mesh'
                    AND a.usename IN ('compiled_caller', 'parity_subject', 'parity_producer')
            """))
            return sessions if Counter(row["role"] for row in sessions) == expected else None

        return backend.arm.h.wait_for(probe, time.monotonic() + 12, "canonical 65 TLS sessions", 0.05)

    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            routes = json.loads(self.cluster.sql("mesh", """
                SELECT jsonb_object_agg(p.proname, jsonb_build_object(
                    'source', p.prosrc, 'security_definer', p.prosecdef,
                    'owner', pg_get_userbyid(p.proowner),
                    'configuration', p.proconfig,
                    'worker', has_function_privilege('parity_subject', p.oid, 'EXECUTE'),
                    'producer', has_function_privilege('parity_producer', p.oid, 'EXECUTE'),
                    'app', has_function_privilege('compiled_caller', p.oid, 'EXECUTE')))
                FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
                WHERE n.nspname='mesh_jobs'
                    AND p.proname IN ('claim_batch_set_v1', 'claim_batch_bounded_v1')
            """))
            for route in routes.values():
                route["body_sha256"] = sha256(route.pop("source").encode()).hexdigest()
            backend.arm.h.write_json(self.cluster.evidence / "public-grants.json", routes)
        return result

    @contextmanager
    def warm_producer(self, env, evidence, deadline):
        with super().warm_producer(env, evidence, deadline) as enqueue:
            def submit(path, log):
                cluster = self.cluster
                roster = self.ready_roster()
                backend.arm.h.write_json(evidence / "canonical-ready.json",
                                         dict(sessions=roster, workers=self.supply, lane=LANE))
                cluster.postmaster = int((cluster.data / "postmaster.pid").read_text().splitlines()[0])
                cluster.before = pg_frame(cluster.postmaster)
                return enqueue(path, log)

            yield submit


backend.Cluster, backend.Subject = Cluster, Subject
runpy.run_path(str(REPLAY / "product_arm.py"), run_name="__main__")
