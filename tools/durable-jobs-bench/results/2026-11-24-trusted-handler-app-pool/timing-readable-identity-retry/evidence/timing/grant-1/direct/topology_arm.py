"""Run the frozen ordinary V1 arm with a labeled runner allocation."""

import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

LAYOUTS = {"stock-a": (8, 8, 4), "stock-b": (8, 8, 4),
           "tuned-2x32": (2, 32, 16), "tuned-1x64": (1, 64, 32)}
ORDERS = (
    ("stock-a", "tuned-2x32", "tuned-1x64", "stock-b"),
    ("stock-b", "tuned-1x64", "tuned-2x32", "stock-a"),
    ("tuned-2x32", "stock-b", "stock-a", "tuned-1x64"),
    ("tuned-1x64", "stock-a", "stock-b", "tuned-2x32"),
    ("stock-b", "tuned-2x32", "tuned-1x64", "stock-a"),
)
INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"

SESSIONS = """
SELECT coalesce(json_agg(json_build_object(
    'role', a.usename, 'backend', a.pid, 'tls', coalesce(s.ssl, false),
    'client_port', a.client_port, 'state', a.state,
    'query_us', (extract(epoch FROM a.query_start) * 1000000)::bigint,
    'query', a.query) ORDER BY a.pid), '[]')
FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s USING (pid)
WHERE a.datname = current_database()
  AND a.usename IN ('parity_subject', 'parity_producer', 'compiled_caller')
"""

CAPS = """
SELECT json_build_object(
    'sql_queues', (SELECT json_agg(to_jsonb(q)) FROM mesh_jobs.queues q),
    'role_caps', (SELECT json_object_agg(rolname, rolconnlimit) FROM pg_roles
                  WHERE rolname IN ('parity_subject', 'parity_producer', 'compiled_caller')))
"""


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_text())


def write(path, data):
    with path.open("x") as stream:
        json.dump(data, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def load_arm(source):
    path = source / "tools/durable-jobs-bench/native/arm.py"
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("ordinary_v1", path)
    arm = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(arm)
    return arm


def run(source, label, out, local, sample_activity):
    arm = load_arm(source)
    canonical = arm.PROFILES["noop"]
    if canonical != (8, 64, 4, 96, 2, INPUT):
        raise ValueError("unexpected V1 noop contract")
    runners, slots_each, app_each = LAYOUTS[label]
    arm.PROFILES = {"noop": (runners, runners * slots_each, app_each, *canonical[3:])}
    caps = {"parity_subject": runners * 4, "compiled_caller": runners * app_each,
            "parity_producer": 1}

    class TopologyCluster(arm.Cluster):
        counter_samples = 0

        def sessions(self, engine):
            return json.loads(self.sql(engine, SESSIONS))

        def audit_tls(self, engine):
            def warmed():
                rows = self.sessions(engine)
                counts = Counter(row["role"] for row in rows if row["tls"])
                if counts == caps and sum(counts.values()) == len(rows):
                    return rows
                return None

            arm.h.wait_for(warmed, time.monotonic() + 10, "exact warm TLS allocation", 0.05)
            super().audit_tls(engine)
            arm.h.write_json(self.evidence / "sql-capacity.json", json.loads(self.sql(engine, CAPS)))

        def calls(self, engine):
            self.counter_samples += 1
            if self.counter_samples == 2:
                arm.h.write_json(self.evidence / "sessions-before.json", self.sessions(engine))
            counters = super().calls(engine)
            arm.h.write_json(self.evidence / f"counters-{self.counter_samples}.json", counters)
            if self.counter_samples == 3:
                arm.h.write_json(self.evidence / "sessions-end.json", self.sessions(engine))
            return counters

        def sql(self, database, query):
            if sample_activity and "'executing', count(*) FILTER" in query:
                query = query.replace("'executing', count(*) FILTER",
                    """'app_active', (SELECT coalesce(json_agg(pid ORDER BY pid), '[]')
                       FROM pg_stat_activity WHERE usename = 'compiled_caller'
                         AND datname = current_database() AND state = 'active'),
                       'executing', count(*) FILTER""", 1)
            return super().sql(database, query)

    class TopologySubject(arm.Subject):
        def capture_runners(self, processes, logs):
            super().capture_runners(processes, logs)
            arm.h.write_json(self.cluster.evidence / "topology.json", {
                "label": label, "runners": runners, "slots_per_runner": slots_each,
                "app_sessions_per_runner": app_each,
                "scheduler_min": int(self.env["MESH_SCHEDULER_MIN_WORKERS"]),
                "scheduler_max": int(self.env["MESH_SCHEDULER_MAX_WORKERS"]),
                "runner_pids_by_index": [process.pid for process in processes],
                "sample_activity": sample_activity,
            })

    arm.Cluster, arm.Subject = TopologyCluster, TopologySubject
    sys.argv = [str(source), *(["--local-smoke"] if local else []), "mesh", "noop", str(out)]
    arm.main()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("label", choices=LAYOUTS)
    parser.add_argument("out", type=Path)
    parser.add_argument("--local", action="store_true")
    parser.add_argument("--sample-activity", action="store_true")
    args = parser.parse_args()
    run(args.source.resolve(), args.label, args.out.resolve(), args.local, args.sample_activity)
