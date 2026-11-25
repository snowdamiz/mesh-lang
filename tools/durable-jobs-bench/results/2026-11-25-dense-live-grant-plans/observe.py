"""Capture the production statement path without altering its SQL or submission."""

import json
import os
from pathlib import Path
import re
import runpy
import sys
import time

import backend
from plans import classify, extract, safe
from settings import PREFIX, ROLE, role_sql

LANE, module_spec = sys.argv[1:3]
del sys.argv[1:3]
ARM = backend.arm
MODULE = Path(module_spec) if LANE == "sampled" else None

# Include effective SECURITY DEFINER users and preserve schema/index/version
# evidence. PostgreSQL counts can lag when backends are busy.
STATS = """
SELECT json_build_object(
  'db', (SELECT oid FROM pg_database WHERE datname = 'mesh'),
  'us_epoch', (extract(epoch FROM clock_timestamp()) * 1000000)::bigint,
  'version_num', current_setting('server_version_num'),
  'dealloc', (SELECT dealloc FROM pg_stat_statements_info),
  'statements_reset', (SELECT stats_reset FROM pg_stat_statements_info),
  'statements', (SELECT coalesce(json_agg(row_to_json(s)), '[]') FROM (
      SELECT userid, userid::regrole::text AS role, dbid, toplevel, queryid, query,
          calls, rows, total_exec_time, shared_blks_hit, shared_blks_read,
          shared_blks_dirtied, shared_blks_written, temp_blks_read, temp_blks_written,
          wal_records, wal_fpi, wal_bytes
      FROM pg_stat_statements WHERE dbid = (SELECT oid FROM pg_database WHERE datname='mesh')
          AND userid::regrole::text IN ('parity_subject','parity_producer','parity_migrator',
                                       'compiled_caller','compiled_definer','parity_owner')) AS s),
  'sessions', (SELECT coalesce(json_agg(row_to_json(a)), '[]') FROM (
      SELECT p.pid, p.usename, p.client_addr, p.client_port, p.state, ssl.ssl
      FROM pg_stat_activity p JOIN pg_stat_ssl ssl USING (pid)
      WHERE p.datname='mesh' AND p.backend_type='client backend') AS a),
  'relation', (SELECT to_jsonb(r) FROM pg_stat_user_tables r
                  WHERE r.relid = 'mesh_jobs.jobs'::regclass),
  'indexes', (SELECT coalesce(json_agg(to_jsonb(i)), '[]') FROM pg_stat_user_indexes i
                 WHERE i.relid = 'mesh_jobs.jobs'::regclass))
"""


def ports(pid, pg_port):
    """Match each live runner's own PostgreSQL sockets to the server TLS view."""
    sockets = set()
    for path in (Path("/proc") / str(pid) / "fd").iterdir():
        try:
            match = re.fullmatch(r"socket:\[(\d+)\]", os.readlink(path))
        except FileNotFoundError:
            continue
        if match:
            sockets.add(match[1])
    out = []
    for line in Path("/proc/net/tcp").read_text().splitlines()[1:]:
        fields = line.split()
        if fields[3] == "01" and fields[9] in sockets and int(fields[2].split(":")[1], 16) == pg_port:
            out.append(int(fields[1].split(":")[1], 16))
    return sorted(out)


def scoped_frame(frame):
    frame = safe(frame)
    frame["statements"] = [row for row in frame["statements"]
                           if row["query"] and classify(row["query"])]
    return frame


class ObservedCluster(backend.Cluster):
    inside_calls = False

    def configure_tls(self):
        super().configure_tls()
        if MODULE:
            with (self.data / "postgresql.conf").open("a") as stream:
                stream.write(f"log_line_prefix = '{PREFIX}'\n")

    def calls(self, engine):
        if not hasattr(self, "frames"):
            self.frames = []
        self.inside_calls = True
        try:
            result = super().calls(engine)
        finally:
            self.inside_calls = False
        if len(self.frames) == 3:
            ARM.h.write_json(self.evidence / "grant-frames.json", self.frames)
        return result

    def sql(self, database, query):
        if not self.inside_calls:
            return super().sql(database, query)
        assert "FROM pg_stat_statements JOIN pg_roles" in query
        # Stock counters count top-level work. Preserve them with nested tracking.
        old = query.strip().rstrip(";").replace("WHERE dbid =", "WHERE toplevel AND dbid =")
        before = time.time_ns() // 1000
        frame = json.loads(super().sql("mesh", f"SELECT json_build_object('stock', ({old}), 'capture', ({STATS}))"))
        capture = scoped_frame(frame["capture"])
        capture.update(begin_us=before, end_us=time.time_ns() // 1000)
        if not self.frames:
            capture["runner_ports"] = {pid: ports(pid, self.port) for pid in self.runner_pids}
        self.frames.append(capture)
        return json.dumps(frame["stock"])

    def stop(self):
        super().stop()
        if MODULE and hasattr(self, "frames") and len(self.frames) == 3:
            # The standard worker export redacts any line resembling a secret.
            # Publish structured plans with SQL literals removed before export.
            log = (self.evidence / "postgres.log").read_text()
            records = list(extract(log))
            assert all(r["session_role"] == ROLE and r["database"] == "mesh" for r in records)
            ARM.h.write_json(self.evidence / "grant-plans.json", safe(records))

    def rows(self, engine, namespace):
        # Both runners and producer have closed by here. Their normal exit
        # flushes per-backend stats. Snapshot before readback adds index work.
        capture = scoped_frame(json.loads(self.sql(engine, STATS)))
        ARM.h.write_json(self.evidence / "grant-closed.json", capture)
        return super().rows(engine, namespace)


class ObservedSubject(backend.Subject):
    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        self.cluster.runner_pids = [child.pid for child in processes]

    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action != "migrate":
            return result
        cluster = self.cluster
        if MODULE:
            cluster.sql("mesh", role_sql(MODULE))
        roles = json.loads(cluster.sql("mesh", """
            SELECT coalesce(json_agg(json_build_array(r.rolname, s.setconfig)), '[]')
            FROM pg_db_role_setting s JOIN pg_roles r ON r.oid=s.setrole
            WHERE s.setdatabase=(SELECT oid FROM pg_database WHERE datname='mesh')
        """))
        observed = [role for role, settings in roles
                    if any(s.startswith("auto_explain.") or s.startswith("session_preload_libraries=")
                           for s in settings)]
        assert observed == ([ROLE] if MODULE else []), roles
        ARM.h.write_json(cluster.evidence / "grant-observer.json",
                         dict(lane=LANE, role_settings=roles, module=str(MODULE) if MODULE else None,
                              module_sha256=ARM.h.digest(MODULE) if MODULE else None))
        return result


assert LANE in ("off-a", "sampled", "off-b") and (MODULE is None or MODULE.is_file())
assert os.environ["PRODUCT_COMPOSITION"] == "product-d2"
assert os.environ["PRODUCT_TOPOLOGY"] == "two-runner"
assert os.environ["PRODUCT_STORAGE"] == "default"
backend.Cluster = ObservedCluster
backend.Subject = ObservedSubject
runpy.run_path(str(Path(__file__).with_name("product_arm.py")), run_name="__main__")
