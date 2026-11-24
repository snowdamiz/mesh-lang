"""Wrap the retained true-bulk V1 arm with slow topology witnesses."""

import argparse
from collections import Counter
import gzip
import json
import os
from pathlib import Path
import sys
import time

HERE = Path(__file__).resolve().parent
FROZEN = HERE.parent / "2026-11-23-slow-storage-transfer/frozen"
TOPOLOGY = HERE.parent / "2026-11-23-ordinary-runner-topology"
sys.path.insert(0, str(TOPOLOGY))
from arm import CAPS, SESSIONS, load_arm, read, write

PROFILE = "slow"
INPUT = "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
LAYOUTS = {"one-a": (1, 64, 8), "one-b": (1, 64, 8), "two": (2, 32, 4), "four": (4, 16, 2)}
STATEMENTS = """
SELECT coalesce(json_agg(to_jsonb(w) ORDER BY w.query), '[]') FROM (
    SELECT rolname AS role, query, calls, rows, total_exec_time, wal_bytes
    FROM pg_stat_statements JOIN pg_roles ON userid = pg_roles.oid
    WHERE dbid = (SELECT oid FROM pg_database WHERE datname = 'mesh')
      AND rolname IN ('parity_producer', 'parity_subject', 'compiled_caller')
) w
"""
CAP_ACTIVITY = """
SELECT json_build_object(
    'physical_clients', count(*),
    'by_role', (SELECT json_object_agg(rolname, count) FROM (
        SELECT usename AS rolname, count(*) FROM pg_stat_activity
        WHERE backend_type = 'client backend' GROUP BY usename
    ) roles))
FROM pg_stat_activity WHERE backend_type = 'client backend'
"""


def launch(source, label, out, local, wire):
    arm = load_arm(source)
    original = arm.PROFILES[PROFILE]
    if original != (1, 64, 8, 68, 2, INPUT):
        raise ValueError("wrong frozen V1 slow profile")
    runners, slots, apps = LAYOUTS[label]
    arm.PROFILES = {PROFILE: (runners, runners * slots, apps, *original[3:])}
    caps = {"parity_subject": runners * 4, "compiled_caller": runners * apps, "parity_producer": 1}
    if local:
        build = read(Path(os.environ["NATIVE_JOBS_ARTIFACTS"]) / "mesh.json")
        pins = read(FROZEN / "local-pins.json")
        if build["files"] != pins["fixture_files"]:
            raise ValueError("wrong compiled V1 fixture")
        settings = read(arm.h.CONFIG)
        expected = gzip.decompress((FROZEN / "canonical-batch.json.gz").read_bytes())
        batch, = arm.h.inputs(settings["workloads"][PROFILE], 64, settings["seed"])
        actual = (json.dumps(batch, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
        if actual != expected:
            raise ValueError("canonical slow input bytes differ")
    os.environ["MESH_JOBS_SET_BASED_ENQUEUE"] = "1"

    class Cluster(arm.Cluster):
        counter_samples = 0

        def sessions(self, engine):
            return json.loads(self.sql(engine, SESSIONS))

        def audit_tls(self, engine):
            def ready():
                rows = self.sessions(engine)
                return len(rows) == sum(caps.values()) and Counter(
                    row["role"] for row in rows if row["tls"]) == caps

            arm.h.wait_for(ready, time.monotonic() + 12, "exact role and TLS allocation", 0.05)
            super().audit_tls(engine)
            write(out / "sql-capacity.json", json.loads(self.sql(engine, CAPS)))

        def calls(self, engine):
            self.counter_samples += 1
            if self.counter_samples == 2:
                write(out / "sessions-before.json", self.sessions(engine))
                write(out / "statements-before.json", json.loads(self.sql(engine, STATEMENTS)))
            counters = super().calls(engine)
            write(out / f"counters-{self.counter_samples}.json", counters)
            if self.counter_samples == 3:
                write(out / "sessions-end.json", self.sessions(engine))
                write(out / "statements-end.json", json.loads(self.sql(engine, STATEMENTS)))
                write(out / "physical-end.json", json.loads(self.sql(engine, CAP_ACTIVITY)))
            return counters

        def observe(self, engine, namespace):
            query = f"""
                SELECT json_build_object(
                  'connections', (SELECT count(*) FROM pg_stat_activity
                    WHERE usename IN ('parity_subject', 'parity_producer', 'compiled_caller')
                      AND datname = current_database()),
                  'physical_clients', (SELECT count(*) FROM pg_stat_activity
                    WHERE backend_type = 'client backend'),
                  'executing', count(*) FILTER (WHERE state = 'executing'),
                  'terminal', count(*) FILTER (WHERE state IN ('completed','discarded','cancelled')))
                FROM mesh_jobs.jobs WHERE namespace = '{namespace}'
            """
            return json.loads(self.sql(engine, query))

    class Subject(arm.Subject):
        def call(self, action, env, log):
            result = super().call(action, env, log)
            if action == "migrate" and wire:
                self.cluster.sql("mesh", """
                    ALTER ROLE parity_producer IN DATABASE mesh SET log_statement = 'all';
                    ALTER ROLE parity_producer IN DATABASE mesh SET log_parameter_max_length = -1;
                """)
            return result

        def capture_runners(self, processes, logs):
            super().capture_runners(processes, logs)
            write(out / "topology.json", dict(
                label=label, runners=runners, slots_per_runner=slots, app_sessions_per_runner=apps,
                scheduler_min=int(self.env["MESH_SCHEDULER_MIN_WORKERS"]),
                scheduler_max=int(self.env["MESH_SCHEDULER_MAX_WORKERS"]),
                runner_pids_by_index=[process.pid for process in processes], wire=wire))

    arm.Cluster, arm.Subject = Cluster, Subject
    sys.argv = [str(source), *(["--local-smoke"] if local else []), "mesh", PROFILE, str(out)]
    arm.main()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("label", choices=LAYOUTS)
    parser.add_argument("out", type=Path)
    parser.add_argument("--local", action="store_true")
    parser.add_argument("--wire", action="store_true", help="untimed producer-parameter proof")
    args = parser.parse_args()
    if args.wire and not args.local:
        parser.error("wire-proof logging is restricted to the disposable local arm")
    launch(args.source.resolve(), args.label, args.out.resolve(), args.local, args.wire)
