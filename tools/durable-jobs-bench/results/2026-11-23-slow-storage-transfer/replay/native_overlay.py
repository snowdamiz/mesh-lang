#!/usr/bin/env python3
"""Replay pinned slow adapters under the default or cluster-wide LZ4 policy."""
import json
from pathlib import Path
import platform
import sys
import time

import storage

HERE = Path(__file__).resolve().parent
NATIVE_PINS = {
    "mesh": "f01706da43f294bb1130b95674adf30afe53c9349084cda5ef8cc8f2abdb7554",
    "oban": "95766ac87a9f04dfdd253af52a04d83438d970da7d01c6cdadfea682cebb473e",
    "go": "e97ede452fb9df9c62fc9b12c914dbb2b5c92b2cd64165b7d11ac5e2dcc5d030",
    "rust": "05c94874455c7c48e0ca6e9204b412aa67d41096b6cf314c0d4a9f3a8fdaf0f7",
}
STOCK_HOOKS = {
    "go": (
        ('            cluster.sql("postgres", "SELECT pg_stat_statements_reset()")',
         "            river_install(cluster, policy)\n{}"),
        ('            ledger, _ = verify_run(raw, args.profile, binary_hash, smoke=args.smoke)',
         "{}\n            finish_river(cluster, out, ledger, policy)"
         "\n            if local and ledger['peak_runner_rss_bytes'] == 0:"
         "\n                return"),
    ),
    "rust": (
        ('    parity.checked([str(binary), "migrate"], env=worker_env, log=cluster.scratch / "migrate.log")',
         '{}\n    storage_setup(cluster, "rust", ENGINE)'),
        ('            run_arm(args.profile, output, binary, cluster, profile, build)',
         '{}\n            storage_finish(cluster, "rust", ENGINE, output)'),
    ),
}


def epoch_us():
    return time.time_ns() // 1000


def storage_setup(cluster, engine, database, schema=""):
    if engine == "rust" and int(cluster.sql(database, """
        SELECT count(*) FROM pg_db_role_setting s
        WHERE s.setdatabase IN (0, (SELECT oid FROM pg_database WHERE datname = current_database()))
          AND EXISTS (SELECT FROM unnest(s.setconfig) AS setting
                      WHERE split_part(setting, '=', 1) = 'default_toast_compression')
    """)):
        raise ValueError("Apalis database or roles override the cluster TOAST policy")
    pattern, column, _, _, _ = storage.TABLES[engine]
    if engine == "oban":
        storage.snapshot(engine, schema)  # Validate the schema before embedding it in SQL.
    relation = pattern.format(schema=schema)
    meta = json.loads(cluster.sql(database, storage.metadata(relation, column)))
    if int(cluster.sql(database, f"SELECT count(*) FROM {relation}")):
        raise ValueError(f"{relation}: storage preflight must precede all enqueues")
    cluster.payload_setup = dict(metadata=meta, rows=0, checked_us=epoch_us())
    if engine == "rust":
        cluster.payload_setup["no_database_or_role_override"] = True


def storage_finish(cluster, engine, database, out, schema=""):
    started = epoch_us()
    persisted = json.loads(cluster.sql(database, storage.snapshot(engine, schema)))
    report = dict(engine=engine, setup=cluster.payload_setup, persisted=persisted,
                  verify_start_us=started, verify_end_us=epoch_us())
    if engine == "mesh":
        report["writer"] = json.loads(cluster.sql(database, """
            SELECT coalesce(jsonb_agg(to_jsonb(w) ORDER BY query), '[]') FROM (
                SELECT query, calls, rows, total_exec_time, wal_bytes
                FROM pg_stat_statements
                WHERE dbid = (SELECT oid FROM pg_database WHERE datname=current_database())
                  AND userid = 'parity_producer'::regrole
                  AND (query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
                    OR query LIKE 'SELECT mesh_jobs.enqueue(%')
            ) w
        """))
    storage.write(out / "payload-storage.json", report)


def river_install(cluster, policy):
    sql = (HERE / "slow-river.sql").read_text()
    query = storage.snapshot("go", "river_schema").replace("river_schema", "%1$I")
    cluster.sql("river", sql.replace("SNAPSHOT_QUERY", query).replace("EXPECTED_POLICY", policy))


def finish_river(cluster, out, ledger, policy):
    witness = json.loads(cluster.sql("river", """
        SELECT to_jsonb(w) || jsonb_build_object('triggers_retired',
            NOT EXISTS (SELECT 1 FROM pg_event_trigger
                        WHERE evtname IN ('bench_slow_storage_create','bench_slow_storage_drop')))
        FROM bench_slow_storage.witness w
    """))
    if not witness["triggers_retired"]:
        raise ValueError("River storage witnesses did not retire")
    storage.verify_snapshot("go", witness["persisted"], ledger["jobs"], policy)
    storage.write(out / "payload-storage.json", dict(engine="go", **witness))
    storage.write(out / "ledger.json", ledger)


def execute(engine, policy, path, argv, local):
    source = path.read_bytes()
    expected = NATIVE_PINS[engine]
    if storage.sha(source) != expected:
        raise ValueError(f"{engine}: selected launcher hash differs from {expected}: {path}")
    script = source.decode()
    for anchor, insertion in STOCK_HOOKS.get(engine, ()):
        if script.count(anchor) != 1:
            raise ValueError(f"{engine}: unique stock launcher anchor changed")
        script = script.replace(anchor, insertion.format(anchor))
    sys.path.insert(0, str(path.parent))
    globals_ = dict(__file__=str(path), __name__="slow_storage_stock", local=local, policy=policy,
                    river_install=river_install, finish_river=finish_river,
                    storage_setup=storage_setup, storage_finish=storage_finish)
    exec(compile(script, path, "exec"), globals_)
    stock_cluster = globals_["Cluster"]

    class Cluster(stock_cluster):
        def configure_tls(self):
            super().configure_tls()
            if policy == "lz4":  # Leave the fresh stock cluster configuration untouched.
                with (self.data / "postgresql.conf").open("a") as stream:
                    stream.write("default_toast_compression = 'lz4'\n")

        def start(self):
            super().start()
            if self.sql("postgres", "SHOW default_toast_compression") != policy:
                raise ValueError(f"{engine}: requested cluster default did not take effect")

        def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
            result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
            storage_finish(self, engine, engine, self.evidence, namespace)
            return result

    globals_["Cluster"] = Cluster
    if engine in ("mesh", "oban"):
        stock_subject = globals_["Subject"]

        class Subject(stock_subject):
            def call(self, action, env, log):
                result = super().call(action, env, log)
                if action == "migrate":
                    storage_setup(self.cluster, engine, engine, env["JOBS_PARITY_NAMESPACE"])
                return result

            def capture_runners(self, processes, logs):
                super().capture_runners(processes, logs)
                session_policy = self.cluster.sql(engine, "SELECT current_setting('default_toast_compression')")
                if session_policy != policy:
                    raise ValueError(f"{engine}: producer inherited an unexpected cluster default")
                self.cluster.payload_setup["producer_session_policy"] = session_policy

        globals_["Subject"] = Subject
    sys.argv = [str(path), *argv]
    globals_["main"]()
    out = Path(argv[-1])
    evidence = json.loads((out / "payload-storage.json").read_text())
    ledger = json.loads((out / "ledger.json").read_text())
    storage.verify_snapshot(engine, evidence["persisted"], ledger["jobs"], policy)


def main():
    engine, policy, launcher, *argv = sys.argv[1:]
    if engine not in NATIVE_PINS or policy not in ("pglz", "lz4") or argv[-2] != "slow" or "--smoke" in argv:
        raise SystemExit("use a pinned slow parity adapter and pglz or lz4")
    execute(engine, policy, Path(launcher), argv, local=platform.system() == "Darwin")


if __name__ == "__main__":
    main()
