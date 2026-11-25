"""Hook only owner setup and teardown around the frozen public dense hot arm."""
from contextlib import ExitStack
from hashlib import sha256
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import time
from tempfile import TemporaryDirectory

import backend
from layout import GUARD, LANES, guard_sql, install_sql
from wal import reduce
from wal_window import WAL_STATE, WALDUMP_SHA as PINNED_WALDUMP, decode, preserve, wait_flushed

HERE = Path(__file__).resolve().parent
LANE = sys.argv.pop(1)
ARM = backend.arm
SOURCE = (HERE / "enqueue_schema.sql").read_text()
assert LANE in LANES
assert os.environ["PRODUCT_COMPOSITION"] == "product-d2"
assert os.environ["PRODUCT_TOPOLOGY"] == "two-runner"
assert os.environ["PRODUCT_STORAGE"] == "default"

SETTINGS = """
SELECT json_build_object(
    'version', current_setting('server_version_num'),
    'wal_level', current_setting('wal_level'),
    'full_page_writes', current_setting('full_page_writes'),
    'fsync', current_setting('fsync'),
    'synchronous_commit', current_setting('synchronous_commit'),
    'wal_compression', current_setting('wal_compression'),
    'track_wal_io_timing', current_setting('track_wal_io_timing'),
    'observer_tls', (SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()),
    'other_clients', (SELECT count(*) FROM pg_stat_activity
                      WHERE backend_type='client backend' AND pid<>pg_backend_pid()),
    'relation', CASE WHEN c.reltablespace=0 THEN d.dattablespace ELSE c.reltablespace END
                || '/' || d.oid || '/' || pg_relation_filenode(c.oid),
    'reloptions', c.reloptions,
    'heap_bytes', pg_relation_size(c.oid),
    'total_relation_bytes', pg_total_relation_size(c.oid),
    'lsn', pg_current_wal_insert_lsn(),
    'system_id', (SELECT system_identifier::text FROM pg_control_system()),
    'database_stats_reset', (SELECT stats_reset FROM pg_stat_database WHERE datid=d.oid),
    'live_relation_counters', (SELECT to_jsonb(t) FROM pg_stat_user_tables t WHERE t.relid=c.oid)
) FROM pg_class c CROSS JOIN pg_database d
WHERE c.oid='mesh_jobs.jobs'::regclass AND d.datname=current_database()
"""
ACL = f"""
SELECT json_build_array(p.oid, p.proowner::regrole::text, p.proacl, p.prosecdef, p.proconfig,
    has_function_privilege('parity_producer', p.oid, 'EXECUTE'),
    has_function_privilege('parity_subject', p.oid, 'EXECUTE'),
    has_function_privilege('compiled_caller', p.oid, 'EXECUTE'))
FROM pg_proc p WHERE p.oid='{GUARD}'::regprocedure
"""
TERMINAL = """
SELECT count(*)=1024 AND bool_and(state='completed' AND attempt=1
       AND lease_token IS NULL AND lease_expires_at_ms IS NULL AND cancel_requested_at_ms IS NULL)
       AND NOT EXISTS (SELECT FROM mesh_jobs.batch_context_blocks)
FROM mesh_jobs.jobs
"""


class ObservedCluster(backend.Cluster):
    baseline = None

    def frame(self):
        value = json.loads(self.sql("mesh", SETTINGS))
        assert value["observer_tls"] and value["other_clients"] == 0
        assert value["version"] == "170011" and value["wal_level"] == "replica"
        assert value["reloptions"] == (["fillfactor=50"] if LANE == "fill50" else None)
        assert all(value[key] == "on" for key in ("full_page_writes", "fsync", "synchronous_commit"))
        value["captured_us"] = time.time_ns() // 1000
        return value

    def stop(self):
        if self.baseline is None or sys.exc_info()[0] is not None:
            return super().stop()
        baseline = self.baseline
        with ExitStack() as cleanup:
            try:
                snapshot = Path(cleanup.enter_context(TemporaryDirectory(prefix="wal-retention-", dir=self.data.parent)))
                end = self.frame()
                end["terminal_lease_state_ok"] = self.sql("mesh", TERMINAL) == "t"
                assert end["terminal_lease_state_ok"]
                ARM.h.write_json(self.evidence / "layout-end.json", end)
                assert (end["relation"], end["system_id"]) == (baseline["relation"], baseline["system_id"])
                state = wait_flushed(lambda: json.loads(self.sql("mesh", WAL_STATE)), end["lsn"])
                try:
                    retained = preserve(self.data / "pg_wal", snapshot, baseline["lsn"], end["lsn"], state)
                except (OSError, ValueError) as error:
                    ARM.h.write_json(self.evidence / "layout-retention-failure.json", dict(
                        detail=str(error), start_lsn=baseline["lsn"], end_lsn=end["lsn"], wal_state=state))
                    raise
                ARM.h.write_json(self.evidence / "layout-wal-snapshot.json", retained)
            finally:
                super().stop()
            control = self.call("pg_controldata", str(self.data))
            assert any(line.split(":", 1)[1].strip() == "shut down" for line in control.splitlines()
                       if line.startswith("Database cluster state:"))
            dump = decode(self.bin, self.env, snapshot, self.evidence, retained, end["relation"])
        totals = reduce(dump)
        assert (totals["inserts"], totals["updates"]) == (1024, 2048), totals
        ARM.h.write_json(self.evidence / "layout-wal.json", totals)
        stats_file = self.data / "pg_stat" / "pgstat.stat"
        assert stats_file.is_file()
        ARM.h.write_json(self.evidence / "layout-shutdown.json", dict(
            clean=True, persisted_stats_sha256=sha256(stats_file.read_bytes()).hexdigest()))


class ObservedSubject(backend.Subject):
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            cluster = self.cluster
            acl = cluster.sql("mesh", ACL)
            assert json.loads(acl)[1] == "parity_migrator" and json.loads(acl)[5:] == [True, False, False]
            cluster.sql("mesh", install_sql(SOURCE, LANE))
            assert cluster.sql("mesh", ACL) == acl
            cluster.baseline = cluster.frame()
            counters = cluster.baseline["live_relation_counters"]
            assert {"n_tup_ins", "n_tup_upd", "n_tup_newpage_upd", "n_tup_hot_upd"} <= counters.keys()
            assert counters["n_tup_ins"] == counters["n_tup_upd"] == 0
            ARM.h.write_json(cluster.evidence / "layout-before.json", cluster.baseline)
            ARM.h.write_json(cluster.evidence / "layout-identity.json", dict(
                lane=LANE, guard_sha256=sha256(guard_sql(SOURCE).encode()).hexdigest(),
                guard_acl=json.loads(acl)[1:], waldump_sha256=WALDUMP_SHA, waldump_version=WALDUMP_VERSION))
        return result


assert sys.platform == "linux" and sorted(os.sched_getaffinity(0)) == [0, 1]
waldump = Path(os.environ["PG_BIN_DIR"]) / "pg_waldump"
try:
    WALDUMP_VERSION = subprocess.check_output([waldump, "--version"], text=True, timeout=5).strip()
    assert WALDUMP_VERSION == "pg_waldump (PostgreSQL) 17.11"
    WALDUMP_SHA = sha256(waldump.read_bytes()).hexdigest()
    assert WALDUMP_SHA == PINNED_WALDUMP
except (OSError, subprocess.SubprocessError, AssertionError) as error:
    ARM.h.write_json(Path(sys.argv[3]) / "layout-preflight-failure.json", dict(error=str(error)))
    raise
backend.Cluster = ObservedCluster
backend.Subject = ObservedSubject
runpy.run_path(str(HERE / "product_arm.py"), run_name="__main__")
