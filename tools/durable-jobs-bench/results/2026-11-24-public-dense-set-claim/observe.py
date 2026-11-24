"""Bracket the public claim on the native adapter using its existing SQL snapshots."""

import json
import os
from pathlib import Path
import runpy
import sys
import time

import backend

MODE = sys.argv.pop(1)
assert MODE in ("off", "trace")
arm = backend.arm
ROLE = "ALTER ROLE parity_subject IN DATABASE mesh SET "
PLANS = Path(os.environ["PG_BIN_DIR"]).parent / "lib/auto_explain.so"
PREFIX = "log_line_prefix = '%m [%p] %l %u %d | '"
TOP = "query LIKE '%FROM mesh_jobs.claim_batch_set_v1(%'"
NESTED = """(query ILIKE '%WITH picked AS MATERIALIZED%'
    OR query ILIKE '%FOR NO KEY UPDATE%' OR query ILIKE '%count(*) FROM mesh_jobs.jobs%'
    OR query ILIKE '%batch_context_blocks%' OR query ILIKE '%pg_try_advisory_xact_lock_shared%'
    OR query ILIKE '%mesh_jobs.queues%')"""
STATS = f"""SELECT coalesce(json_agg(row_to_json(s) ORDER BY s.toplevel DESC, s.queryid), '[]')
FROM (SELECT queryid, toplevel, query, calls, rows, total_exec_time,
    shared_blks_hit, shared_blks_read, temp_blks_read, temp_blks_written, wal_records, wal_bytes
    FROM pg_stat_statements WHERE userid = 'parity_subject'::regrole
    AND dbid = (SELECT oid FROM pg_database WHERE datname = 'mesh')
    AND ((toplevel AND {TOP}) OR (NOT toplevel AND {NESTED}))) AS s"""
OFFLINE = f"""SELECT json_build_object(
  'observed_us', (extract(epoch FROM clock_timestamp())*1000000)::bigint,
  'dealloc', (SELECT dealloc FROM pg_stat_statements_info),
  'statements', ({STATS}),
  'functions', (SELECT coalesce(json_agg(row_to_json(f)), '[]') FROM (
    SELECT funcname, calls, total_time, self_time FROM pg_stat_user_functions
    WHERE schemaname='mesh_jobs' AND funcname IN ('claim_batch_set_v1', 'fence_claim_v1')
  ) AS f),
  'relations', (SELECT coalesce(json_agg(row_to_json(r)), '[]') FROM (
    SELECT relname, seq_scan, seq_tup_read, idx_scan, idx_tup_fetch,
        n_tup_ins, n_tup_upd, n_dead_tup, analyze_count, autoanalyze_count
    FROM pg_stat_user_tables WHERE schemaname='mesh_jobs'
      AND relname IN ('queues', 'jobs', 'batch_context_blocks')
  ) AS r),
  'role_settings', (SELECT setconfig FROM pg_db_role_setting WHERE setrole='parity_subject'::regrole
      AND setdatabase=(SELECT oid FROM pg_database WHERE datname='mesh'))
)"""


def cpu(pids):
    result = {}
    for pid in pids:
        try:
            root = Path("/proc") / str(pid)
            start = int((root / "stat").read_text().rsplit(")", 1)[1].split()[19])
            ns = int((root / "schedstat").read_text().split()[0])
            confirmed = int((root / "stat").read_text().rsplit(")", 1)[1].split()[19])
        except (FileNotFoundError, ProcessLookupError):
            continue
        if start == confirmed:
            result[str(pid)] = [start, ns]
    return result


class ObservedCluster(backend.Cluster):
    capture_calls = False

    def configure_tls(self):
        super().configure_tls()
        with (self.data / "postgresql.conf").open("a") as stream:
            stream.write(PREFIX + "\n")

    def calls(self, engine):
        if not hasattr(self, "frames"):
            self.frames = []
        self.capture_calls = True
        try:
            result = super().calls(engine)
        finally:
            self.capture_calls = False
        assert len(self.frames) in (1, 2, 3)
        return result

    def sql(self, database, query):
        if not self.capture_calls:
            return super().sql(database, query)
        assert "FROM pg_stat_statements JOIN pg_roles" in query and query.count("WHERE dbid =") == 1
        # Preserve original observer counters while obtaining claim elapsed and
        # nested stats in the same round trip. The last call follows settlement.
        original = query.replace("WHERE dbid =", "WHERE toplevel AND dbid =")
        frame = f"""SELECT json_build_object(
          'calls', ({original.strip().rstrip(';')}),
          'statements', ({STATS}),
          'observed_us', (extract(epoch FROM clock_timestamp())*1000000)::bigint,
          'pids', (SELECT coalesce(json_agg(pid ORDER BY pid), '[]') FROM pg_stat_activity
                   WHERE usename='parity_subject' AND datname='mesh'))
        """
        before = time.time_ns() // 1000
        data = json.loads(super().sql(database, frame))
        after = time.time_ns() // 1000
        data.update(begin_us=before, end_us=after, cpu=cpu(data["pids"]))
        self.frames.append(data)
        # One write after the final existing time-window observation.
        if len(self.frames) == 3:
            arm.h.write_json(self.evidence / "claim-frames.json", self.frames)
        return json.dumps(data["calls"])

    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        begin = time.time_ns() // 1000
        data = json.loads(self.sql(engine, OFFLINE))
        data.update(begin_us=begin, end_us=time.time_ns() // 1000)
        arm.h.write_json(self.evidence / "claim-off-clock.json", data)
        return super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)


class ObservedSubject(backend.Subject):
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action != "migrate":
            return result
        settings = {}
        if MODE == "trace":
            settings.update({"pg_stat_statements.track": "all", "track_functions": "pl",
                             "log_min_duration_statement": "0", "log_parameter_max_length": "0",
                             "log_parameter_max_length_on_error": "0", "log_lock_waits": "on",
                             "deadlock_timeout": "1ms"})
            if PLANS.is_file():
                settings.update({"session_preload_libraries": "auto_explain",
                                 "auto_explain.log_min_duration": "0",
                                 "auto_explain.log_analyze": "on", "auto_explain.log_buffers": "on",
                                 "auto_explain.log_timing": "off", "auto_explain.log_triggers": "on",
                                 "auto_explain.log_nested_statements": "on",
                                 "auto_explain.log_parameter_max_length": "0",
                                 "auto_explain.log_format": "json", "auto_explain.sample_rate": "0.25"})
            self.cluster.sql("mesh", "\n".join(f"{ROLE}{k} = '{v}';" for k, v in settings.items()))
        arm.h.write_json(self.cluster.evidence / "claim-observer.json",
                         dict(mode=MODE, settings=settings, plans_available=PLANS.is_file(),
                              plans_sha256=arm.h.digest(PLANS) if PLANS.is_file() else None))
        return result


backend.Cluster = ObservedCluster
backend.Subject = ObservedSubject
runpy.run_path(str(Path(__file__).with_name("product_arm.py")), run_name="__main__")
