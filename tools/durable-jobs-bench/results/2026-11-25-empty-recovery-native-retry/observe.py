"""Install one recover() body and reuse the frozen native timing boundaries."""
import json
import os
from pathlib import Path
import re
import runpy
import sys
import time

import backend
from contract import ELF, MANIFEST, POSTGRES, definitions, digest
from probe import FUNCTION, GUARD, proc_identity, run, stats_query

HERE = Path(__file__).resolve().parent
PHASE = sys.argv.pop(1)
VARIANT = "stock" if PHASE == "gate-stock" else PHASE
ARM = backend.arm
SQL = definitions((HERE / "candidate.sql").read_bytes())

FIXTURE = """
SELECT json_build_object(
 'version',current_setting('server_version_num'),
 'fsync',current_setting('fsync'),
 'full_page_writes',current_setting('full_page_writes'),
 'synchronous_commit',current_setting('synchronous_commit'),
 'track_commit_timestamp',current_setting('track_commit_timestamp'),
 'wal_level',current_setting('wal_level'),
 'wal_compression',current_setting('wal_compression'),
 'max_connections',current_setting('max_connections')::int,
 'tls',(SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()),
 'layout',mesh_jobs.enqueue_batch_layout_v1(),
 'reloptions',(SELECT reloptions FROM pg_class WHERE oid='mesh_jobs.jobs'::regclass),
 'indexes',(SELECT json_agg(pg_get_indexdef(indexrelid) ORDER BY indexrelid)
            FROM pg_index WHERE indrelid='mesh_jobs.jobs'::regclass),
 'triggers',(SELECT json_agg(pg_get_triggerdef(oid) ORDER BY oid)
             FROM pg_trigger WHERE tgrelid='mesh_jobs.jobs'::regclass AND NOT tgisinternal),
 'guard_definition',pg_get_functiondef('mesh_jobs.enqueue_batch_layout_v1()'::regprocedure),
 'guard_acl',(SELECT proacl::text FROM pg_proc WHERE oid='mesh_jobs.enqueue_batch_layout_v1()'::regprocedure),
 'settle_definition',pg_get_functiondef('mesh_jobs.settle(text,uuid,uuid,jsonb,boolean)'::regprocedure),
 'roles',(SELECT coalesce(json_object_agg(setrole::regrole::text,setconfig ORDER BY setrole),'{}'::json)
          FROM pg_db_role_setting
          WHERE setdatabase=(SELECT oid FROM pg_database WHERE datname=current_database())
            AND setrole<>0))
"""


def capture_cpu(pids):
    stamps = {}
    for pid in pids:
        process = Path("/proc") / str(pid)
        try:
            pre = process.joinpath("stat").read_text().rsplit(")", 1)[1].split()[19]
            cpu = int(process.joinpath("schedstat").read_text().split()[0])
            post = process.joinpath("stat").read_text().rsplit(")", 1)[1].split()[19]
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
        if pre == post:
            stamps[str(pid)] = [pre, cpu]
    return stamps


class ObservedCluster(backend.Cluster):
    calls_capture = False

    def calls(self, engine):
        if not hasattr(self, "frames"):
            self.frames = []
        self.calls_capture = True
        try:
            return super().calls(engine)
        finally:
            self.calls_capture = False

    def sql(self, database, query):
        if not self.calls_capture:
            return super().sql(database, query)
        assert query.count("WHERE dbid =") == 1 and "FROM pg_stat_statements JOIN pg_roles" in query
        stock = query.strip().rstrip(";").replace("WHERE dbid =", "WHERE toplevel AND dbid =")
        before = time.time_ns() // 1000
        frame = json.loads(super().sql(database, f"""
SELECT json_build_object('stock',({stock}),'recovery',({stats_query("mesh")}),
    'epoch_us',(extract(epoch FROM clock_timestamp())*1000000)::bigint,
    'pids',(SELECT coalesce(json_agg(pid),'[]') FROM pg_stat_activity
            WHERE datname='mesh' AND usename='parity_subject'))
"""))
        capture = dict(frame["recovery"], epoch_us=frame["epoch_us"], begin_us=before,
                       end_us=time.time_ns() // 1000, cpu=capture_cpu(frame["pids"]), pids=frame["pids"])
        self.frames.append(capture)
        if len(self.frames) == 3:
            ARM.h.write_json(self.evidence / "recovery-frames.json", self.frames)
        return json.dumps(frame["stock"])

    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        receipt = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
        assert len(self.frames) == 3
        assert self.sql(engine, GUARD) == "t"
        tail = json.loads(self.sql("postgres", stats_query(engine)))
        tail["writer"] = json.loads(self.sql("postgres", """
SELECT coalesce(json_agg(row_to_json(s)),'[]')
FROM (SELECT query,calls,rows,wal_records,wal_bytes
      FROM pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname='mesh')
      AND userid='parity_producer'::regrole AND toplevel
      AND query ILIKE '%mesh_jobs.enqueue_batch_v1(%') s
"""))
        # Count committed durable jobs independently of response rows from the SQL writer.
        assert re.fullmatch(r"parity_[0-9a-f]{16}", namespace), namespace
        tail["committed_rows"] = int(self.sql(engine, f"""
SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='{namespace}' AND state='completed'
  AND attempt=1 AND pg_xact_commit_timestamp(xmin) IS NOT NULL
"""))
        ARM.h.write_json(self.evidence / "recovery-closed.json", tail)
        writer = tail["writer"]
        assert tail["committed_rows"] == sum(s["rows"] for s in writer) == 1024, tail
        assert sum(s["calls"] for s in writer) == 8 and sum(s["wal_records"] for s in writer) > 0, tail
        return receipt


def write_plan_gate(evidence, value):
    target = evidence / "recovery-plan-gate.json"
    pending = target.with_suffix(".pending")
    with pending.open("w") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")
    pending.replace(target)


class ObservedSubject(backend.Subject):
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action != "migrate":
            return result
        cluster = self.cluster
        query = lambda statement: cluster.sql("mesh", statement)
        before = json.loads(query(FIXTURE))
        assert before["layout"] and before["tls"] and before["reloptions"] is None, before
        assert before["version"] == "170011" and before["max_connections"] == 96, before
        assert all(before[key] == "on" for key in
                   ("synchronous_commit", "track_commit_timestamp", "fsync", "full_page_writes")), before
        assert query("SELECT count(*) FROM mesh_jobs.jobs") == "0"
        stock = proc_identity(query, SQL["stock"])
        assert stock[1] == "parity_migrator"
        if PHASE == "gate-stock":
            run(cluster, SQL, lambda value: write_plan_gate(cluster.evidence, value))
        query("BEGIN; SET LOCAL ROLE parity_migrator;\n" + SQL[VARIANT] + "\nCOMMIT;")
        identity = proc_identity(query, SQL[VARIANT])
        assert identity[:6] == stock[:6] and identity[7:] == stock[7:]
        after = json.loads(query(FIXTURE))
        assert after == before
        query("ALTER ROLE parity_subject IN DATABASE mesh SET pg_stat_statements.track='all'")
        traced = json.loads(query(FIXTURE))
        assert traced["roles"].pop("parity_subject") == ["pg_stat_statements.track=all"]
        assert traced == after
        ARM.h.write_json(cluster.evidence / "recovery-fixture.json",
                         dict(phase=PHASE, fixture=after, procedure=identity,
                              installed_sha256=digest(SQL[VARIANT].encode())))
        return result


assert sys.platform == "linux" and sorted(os.sched_getaffinity(0)) == [0, 1]
assert os.environ["PRODUCT_COMPOSITION"] == "product-d2"
assert os.environ["PRODUCT_TOPOLOGY"] == "two-runner"
assert os.environ["PRODUCT_STORAGE"] == "default"
assert os.environ["PRODUCT_ELF_SHA256"] == ELF
assert os.environ["PRODUCT_MANIFEST_SHA256"] == MANIFEST
assert digest((Path(os.environ["PG_BIN_DIR"]) / "postgres").read_bytes()) == POSTGRES
backend.Cluster = ObservedCluster
backend.Subject = ObservedSubject
runpy.run_path(str(HERE / "product_arm.py"), run_name="__main__")
