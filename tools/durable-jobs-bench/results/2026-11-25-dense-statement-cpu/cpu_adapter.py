"""Run the frozen adapters with one private postmaster observer."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import sys
import time

from measure import read_report, require

HERE = Path(__file__).resolve().parent
MODULE = HERE / "stmt_cpu.so"


def label(db, sql):
    query = " ".join((sql or "").lower().split()).rstrip(";")
    if re.fullmatch(r"(begin|start transaction|commit|rollback|end)( work| transaction)?", query):
        return "transaction"
    if db == "mesh":
        if re.search(r"mesh_jobs\.settle(?:_state)?\s*\(", query):
            return "settle"
        if "mesh_jobs.claim_batch_set_v1(" in query:
            return "grant"
        if re.search(r"mesh_jobs\.enqueue_batch(?:_layout)?_v1\s*\(", query):
            return "enqueue"
    if db == "river" and re.search(r"\briver_job\b", query):
        if "update " in query and "job_input.finalized_at_do_update" in query:
            return "settle"
        if "skip locked" in query and "update " in query and "attempted_at" in query:
            return "grant"
        if "insert into " in query:
            return "enqueue"
    if "insert into " in query and re.search(r"\bmesh_jobs_parity_(attempts|effects)\b", query):
        return "application-write"
    if re.search(r"\b(pg_stat_activity|pg_stat_statements|pg_stat_ssl|pg_xact_commit_timestamp)\b", query):
        return "observer"
    return "other"


DICTIONARY = """
SELECT json_build_object(
    'cluster_id', (SELECT system_identifier::text FROM pg_control_system()),
    'dealloc', (SELECT dealloc FROM pg_stat_statements_info),
    'rows', coalesce(json_agg(json_build_object('db', dbid, 'role', userid,
               'query', queryid::text, 'database', datname, 'effective_role', rolname,
               'calls', calls, 'sql', query) ORDER BY dbid, userid, queryid), '[]'))
FROM pg_stat_statements
JOIN pg_database ON dbid = pg_database.oid
JOIN pg_roles ON userid = pg_roles.oid
WHERE toplevel
"""


def cpu_cluster(stock, enabled, output):
    class Cluster(stock):
        def configure_tls(self):
            super().configure_tls()
            self.postmaster_result = self.data / "dense-cpu.jsonl"
            self.configured_us = time.time_ns() // 1000
            with (self.data / "postgresql.conf").open("a") as stream:
                stream.write(f"shared_preload_libraries = 'pg_stat_statements,{MODULE}'\n"
                             f"dense_stmt_cpu.enabled = {'on' if enabled else 'off'}\n")

        def stop(self):
            if not (self.data / "postmaster.pid").is_file():
                return super().stop()
            try:
                dictionary_us = time.time_ns() // 1000
                dictionary = json.loads(self.sql("postgres", DICTIONARY))
            finally:
                super().stop()
            raw = self.postmaster_result.read_text()
            read_report(raw, enabled)
            for row in dictionary["rows"]:
                sql = row.pop("sql") or ""
                row["family"] = label(row["database"], sql)
                row["sql_sha256"] = sha256(sql.encode()).hexdigest()
            dictionary.update(configured_us=self.configured_us, dictionary_us=dictionary_us,
                              published_us=time.time_ns() // 1000, module_sha256=sha256(MODULE.read_bytes()).hexdigest())
            (output / "dense-statement-keys.json").write_text(json.dumps(dictionary) + "\n")
            (output / "dense-statement-cpu.jsonl").write_text(raw)
    return Cluster


def replace_once(source, old, new):
    require(source.count(old) == 1, f"frozen adapter anchor changed: {old}")
    return source.replace(old, new, 1)


def mesh_source(source):
    anchor = "    arm.Cluster, arm.Subject = Cluster, Subject"
    return replace_once(source, anchor,
                        "    arm.Cluster, arm.Subject = cpu_cluster(Cluster, cpu_enabled, cpu_out), Subject")


def load_file(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    adapter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(adapter)
    return adapter


def main():
    engine, mode, adapter, *args = sys.argv[1:]
    enabled = mode == "on"
    require(mode in ("off", "on") and engine in ("mesh", "go"), "unknown diagnostic arm")
    adapter = Path(adapter).resolve(strict=True)
    out = Path(args[-1]).resolve(strict=True)
    sys.argv = [str(adapter), *args]
    sys.path.insert(0, str(adapter.parent))
    if engine == "mesh":
        scope = dict(__name__="__main__", __file__=str(adapter), cpu_out=out,
                     cpu_enabled=enabled, cpu_cluster=cpu_cluster)
        exec(compile(mesh_source(adapter.read_text()), str(adapter), "exec"), scope)
        return
    peer = load_file("cpu_stock_peer", adapter)
    load = peer.load

    def load_river(engine, path):
        require(engine == "go", "expected stock River")
        original = load(engine, path)
        original.Cluster = cpu_cluster(original.Cluster, enabled, out)
        return original

    peer.load = load_river
    peer.main()


if __name__ == "__main__":
    main()
