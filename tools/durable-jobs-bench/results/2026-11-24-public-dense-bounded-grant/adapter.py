"""Apply the trial-local function overlay after the public ELF migrates."""

import os
from pathlib import Path
import runpy
import sys

import backend
from diagnostic import apply, qualify

MODE = sys.argv.pop(1)


class Subject(backend.Subject):
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            cluster = self.cluster
            assert MODE in {"control", "candidate", "qualify"}
            assert os.environ["PRODUCT_TOPOLOGY"] == "two-runner"
            assert cluster.sql("mesh", "SHOW server_version_num") == "170011"
            if MODE == "qualify":
                qualify(cluster)
            receipt = apply(cluster, "mesh", MODE == "candidate")
            backend.arm.h.write_json(cluster.evidence / "grant-overlay.json", dict(mode=MODE, **receipt))
        return result


class Cluster(backend.Cluster):
    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        # Same off-clock observer in all measured arms. PostgreSQL aggregate
        # counters are asynchronous and include work outside the timed slice.
        data = self.sql(engine, """SELECT json_build_object(
            'jobs', (SELECT row_to_json(s) FROM pg_stat_user_tables s
                     WHERE schemaname='mesh_jobs' AND relname='jobs'),
            'indexes', (SELECT json_agg(row_to_json(s) ORDER BY indexrelname)
                        FROM pg_stat_user_indexes s
                        WHERE schemaname='mesh_jobs' AND relname='jobs'),
            'plan_cache_mode', current_setting('plan_cache_mode'),
            'auto_explain', current_setting('auto_explain.log_min_duration', true))""")
        (self.evidence / "grant-counters.json").write_text(data + "\n")
        return super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)


backend.Subject = Subject
backend.Cluster = Cluster
runpy.run_path(str(Path(__file__).with_name("product_arm.py")), run_name="__main__")
