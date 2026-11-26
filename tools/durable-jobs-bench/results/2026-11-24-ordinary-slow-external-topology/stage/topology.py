"""Add the retained slow topology witnesses to the EXTERNAL Mesh adapter."""
from collections import Counter
import json
import os
from pathlib import Path
import re
import time

import storage

INPUT = "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
LAYOUTS = {"one-a": (1, 64, 8), "one-b": (1, 64, 8), "two": (2, 32, 4)}

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


def established_clients(table, server_port):
    """Map kernel socket inode to its client port for the fixture's loopback DB."""
    peer = f"0100007F:{server_port:04X}"
    result = {}
    for line in table.splitlines()[1:]:
        fields = line.split()
        if len(fields) > 9 and fields[2] == peer and fields[3] == "01":
            result[fields[9]] = int(fields[1].split(":")[1], 16)
    return result


def runner_backend_map(pids, postgres_port, sessions):
    ports = {item["client_port"]: item["backend"] for item in sessions if item["role"] == "compiled_caller"}
    clients = established_clients(Path("/proc/net/tcp").read_text(), postgres_port)
    mapping = {}
    for pid in pids:
        out = {}
        for fd in Path(f"/proc/{pid}/fd").iterdir():
            try:
                target = os.readlink(fd)
            except FileNotFoundError:
                continue
            match = re.fullmatch(r"socket:\[(\d+)\]", target)
            if match and (port := clients.get(match[1])) in ports:
                out[port] = {"client_port": port, "backend": ports[port]}
        mapping[str(pid)] = sorted(out.values(), key=lambda entry: entry["backend"])
    return mapping


def install(arm, out):
    label = os.environ["MESH_EXTERNAL_TOPOLOGY"]
    runners, slots, apps = LAYOUTS[label]
    original = arm["PROFILES"]["slow"]
    if original != (1, 64, 8, 68, 2, INPUT):
        raise ValueError("wrong frozen V1 slow contract")
    arm["PROFILES"] = {"slow": (runners, runners * slots, apps, *original[3:])}
    caps = {"parity_subject": runners * 4, "compiled_caller": 8, "parity_producer": 1}
    harness = arm["h"]
    base_cluster, base_subject = arm["Cluster"], arm["Subject"]

    class Cluster(base_cluster):
        counter_samples = 0

        def sessions(self, engine):
            return json.loads(self.sql(engine, SESSIONS))

        def audit_tls(self, engine):
            def ready():
                rows = self.sessions(engine)
                return len(rows) == sum(caps.values()) and Counter(
                    row["role"] for row in rows if row["tls"]
                ) == caps

            harness.wait_for(ready, time.monotonic() + 12, "exact role and TLS allocation", 0.05)
            super().audit_tls(engine)
            storage.write(out / "sql-capacity.json", json.loads(self.sql(engine, CAPS)))

        def calls(self, engine):
            self.counter_samples += 1
            if self.counter_samples == 2:
                storage.write(out / "sessions-before.json", self.sessions(engine))
                storage.write(out / "statements-before.json", json.loads(self.sql(engine, STATEMENTS)))
            counters = super().calls(engine)
            storage.write(out / f"counters-{self.counter_samples}.json", counters)
            if self.counter_samples == 3:
                storage.write(out / "sessions-end.json", self.sessions(engine))
                storage.write(out / "statements-end.json", json.loads(self.sql(engine, STATEMENTS)))
                storage.write(out / "physical-end.json", json.loads(self.sql(engine, CAP_ACTIVITY)))
            return counters

        def observe(self, engine, namespace):
            harness.job_relation(engine, namespace)
            return json.loads(self.sql(engine, f"""
                SELECT json_build_object(
                  'connections', (SELECT count(*) FROM pg_stat_activity
                    WHERE usename IN ('parity_subject', 'parity_producer', 'compiled_caller')
                      AND datname = current_database()),
                  'physical_clients', (SELECT count(*) FROM pg_stat_activity
                    WHERE backend_type = 'client backend'),
                  'executing', count(*) FILTER (WHERE state = 'executing'),
                  'terminal', count(*) FILTER (WHERE state IN ('completed','discarded','cancelled')))
                FROM mesh_jobs.jobs WHERE namespace = '{namespace}'
            """))

    class Subject(base_subject):
        def capture_runners(self, processes, logs):
            super().capture_runners(processes, logs)
            pids = [process.pid for process in processes]
            mapping = runner_backend_map(pids, self.cluster.port, self.cluster.sessions("mesh"))
            backends = {item["backend"] for items in mapping.values() for item in items}
            if len(backends) != 8 or any(len(items) != apps for items in mapping.values()):
                raise ValueError("could not attribute the exact eight warmed application backends")
            storage.write(out / "topology.json", dict(
                label=label, runners=runners, slots_per_runner=slots, app_sessions_per_runner=apps,
                scheduler_min=int(self.env["MESH_SCHEDULER_MIN_WORKERS"]),
                scheduler_max=int(self.env["MESH_SCHEDULER_MAX_WORKERS"]),
                runner_pids_by_index=pids, warm_app_backends_by_runner=mapping, wire=False))

    arm["Cluster"], arm["Subject"] = Cluster, Subject
