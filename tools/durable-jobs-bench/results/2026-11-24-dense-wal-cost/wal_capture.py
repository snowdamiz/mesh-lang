"""Read complete PG17 WAL counters around an unchanged, separately staged arm."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
RESET = "SELECT pg_stat_statements_reset()"
SETTINGS = {
    "server_version_num": "170011", "wal_sync_method": "fdatasync",
    "track_wal_io_timing": "on", "synchronous_commit": "on", "fsync": "on",
    "full_page_writes": "on", "track_commit_timestamp": "on", "ssl": "on",
    "max_connections": "96", "reserved_connections": "0",
    "superuser_reserved_connections": "0", "commit_delay": "0",
    "pg_stat_statements.track": "top",
}
NAMES = ", ".join("'" + key + "'" for key in SETTINGS)
FRAME = f"""
SELECT json_build_object(
  'observed_us', (extract(epoch FROM clock_timestamp()) * 1000000)::bigint,
  'system_id', (SELECT system_identifier::text FROM pg_control_system()),
  'postmaster_started_us', (extract(epoch FROM pg_postmaster_start_time()) * 1000000)::bigint,
  'settings', (SELECT json_object_agg(name, setting) FROM pg_settings WHERE name IN ({NAMES})),
  'observer_tls', (SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()),
  'other_clients', (SELECT count(*) FROM pg_stat_activity
                    WHERE backend_type = 'client backend' AND pid <> pg_backend_pid()),
  'wal_senders', (SELECT count(*) FROM pg_stat_replication),
  'lsn', json_build_object('insert', pg_current_wal_insert_lsn()::text,
                           'write', pg_current_wal_lsn()::text, 'flush', pg_current_wal_flush_lsn()::text),
  'wal', (SELECT row_to_json(w) FROM pg_stat_wal w),
  'checkpoint', (SELECT row_to_json(c) FROM pg_stat_checkpointer c),
  'bgwriter', (SELECT row_to_json(b) FROM pg_stat_bgwriter b),
  'relation_io', (SELECT json_agg(i ORDER BY backend_type, object, context) FROM pg_stat_io i)
)
"""
STATEMENTS = """
SELECT json_build_object(
 'dealloc', (SELECT dealloc FROM pg_stat_statements_info),
 'rows', coalesce((SELECT json_agg(s ORDER BY role, kind) FROM (
   SELECT r.rolname AS role,
     CASE WHEN ltrim(p.query) ~* '^INSERT INTO [^[:space:]]*mesh_jobs_parity_attempts([[:space:](]|$)'
          THEN 'attempt'
          WHEN ltrim(p.query) ~* '^INSERT INTO [^[:space:]]*mesh_jobs_parity_effects([[:space:](]|$)'
          THEN 'effect'
          ELSE 'other'
     END AS kind,
     sum(p.calls) AS calls, sum(p.rows) AS rows,
     sum(p.wal_records) AS wal_records, sum(p.wal_fpi) AS wal_fpi, sum(p.wal_bytes) AS wal_bytes
   FROM pg_stat_statements p JOIN pg_roles r ON r.oid = p.userid
   WHERE p.dbid = (SELECT oid FROM pg_database WHERE datname = '%s')
   GROUP BY 1, 2
 ) s), '[]')
)
"""


def need(condition, message):
    if not condition:
        raise ValueError(message)


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def stamp():
    return dict(epoch_us=time.time_ns() // 1000, monotonic_ns=time.monotonic_ns())


def collect(cluster, first=None, dbname=None):
    # One psql process and one verified TLS backend. Each -c runs separately.
    args, env = cluster.psql("postgres")
    queries = ([first] if first else []) + [FRAME]
    if dbname:
        queries.append(STATEMENTS % dbname)
    begin = stamp()
    raw = subprocess.check_output([*args, "--record-separator-zero",
                                   *(arg for q in queries for arg in ("-c", q))],
                                  env=env, text=True, timeout=15)
    end = stamp()
    records = raw.removesuffix("\0").split("\0")
    if first:
        # pg_stat_statements_reset reports a timestamp on PG17.
        records = records[1:]
    rows = [json.loads(record) for record in records]
    need(len(rows) == (2 if dbname else 1), "wrong WAL observer result count")
    frame = rows[0]
    need(frame["settings"] == SETTINGS and frame["observer_tls"] is True,
         "unexpected PG17 settings or observer TLS")
    need(frame["other_clients"] == frame["wal_senders"] == 0,
         "WAL observer overlapped workload clients")
    need(begin["epoch_us"] <= frame["observed_us"] <= end["epoch_us"],
         "server WAL frame is outside client clock bounds")
    result = dict(begin=begin, end=end, frame=frame)
    if dbname:
        result["statements"] = rows[1]
    return result


def install(module, output, database):
    """Wrap only the frozen cluster setup, its pre-work reset and its cleanup."""
    original = module.Cluster

    class WalCluster(original):
        wal_started = False
        wal_baseline = None

        def configure_tls(self):
            super().configure_tls()
            with (self.data / "postgresql.conf").open("a") as config:
                config.write("track_wal_io_timing = on\n")

        def start(self):
            super().start()
            self.wal_started = True

        def sql(self, db, query):
            if db == "postgres" and query.strip().rstrip(";") == RESET:
                need(self.wal_baseline is None, "repeated original statistics reset")
                self.wal_baseline = collect(self, first=query)
                (output / "wal-before.json").write_text(json.dumps(self.wal_baseline) + "\n")
                return ""
            return super().sql(db, query)

        def stop(self):
            # On a frozen arm error keep its ordinary cleanup and original failure.
            if not self.wal_started or self.wal_baseline is None or sys.exc_info()[0] is not None:
                return super().stop()
            try:
                after = collect(self, dbname=database)
                (output / "wal-live-after.json").write_text(json.dumps(after) + "\n")
                stop_begin = stamp()
                self.call("pg_ctl", "-D", str(self.data), "-m", "fast", "-t", "10", "-w", "stop")
                stop_end = stamp()
                stats_file = self.data / "pg_stat" / "pgstat.stat"
                # A clean shutdown, unlike immediate mode, publishes pending
                # backend statistics and persists the cluster snapshot.
                need(stats_file.is_file(), "missing orderly PG17 stats file")
                digest = hashlib.sha256(stats_file.read_bytes()).hexdigest()
                self.call("pg_ctl", "-D", str(self.data), "-w", "-t", "20", "-l",
                          str(output / "wal-readback.log"), "start")
                restored = collect(self)
                (output / "wal-restored.json").write_text(json.dumps(dict(
                    stop_begin=stop_begin, stop_end=stop_end, stats_file_sha256=digest,
                    restored=restored)) + "\n")
            finally:
                super().stop()

    module.Cluster = WalCluster


def mesh(source, runner, output):
    load("identity", HERE / "identity.py")
    entry = load("wal_dense_checkout", HERE / "native-arm.py")
    sys.path.insert(0, str(source / "tools/durable-jobs-bench/native"))
    import arm

    main = arm.main

    def observed():
        # Dense main installs the frozen R2 Cluster before calling arm.main.
        install(arm, output, "mesh")
        main()

    arm.main = observed
    entry.main("d2-on", source, runner, output)


def peer(engine, path, args, output):
    equal = load("wal_equal_topology", HERE / "peer-arm.py")
    launcher = equal.load(engine, path)
    roster = equal.configure(engine, launcher, output)
    install(launcher, output, {"go": "river", "rust": "apalis", "oban": "oban"}[engine])
    sys.argv = [str(path), *args]
    launcher.main()
    if engine == "rust":
        need(set(roster) == {"runner-0", "runner-1"}
             and len({row["pid"] for row in roster.values()}) == 2, "Apalis runner roster differs")
        launcher.parity.write_json(output / "runner-topology.json", roster)


if __name__ == "__main__":
    args = sys.argv[1:]
    need(sys.platform == "linux" and os.sched_getaffinity(0) == {0, 1} and os.getuid() != 0,
         "diagnostic requires the pinned unprivileged Linux CPU envelope")
    engine, *args = args
    output = Path(args[-1])
    if engine == "mesh":
        source, runner, _ = map(Path, args)
        mesh(source, runner, output)
    else:
        path, *options = args
        need(options[-2] == "noop" and "--smoke" not in options and "--local-smoke" not in options,
             "diagnostic requires the frozen dense arm")
        peer(engine, Path(path), options, output)
