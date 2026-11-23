"""Configure and observe a single disposable PostgreSQL server."""

import json
from pathlib import Path
import subprocess
import sys
import threading
import time

SETTINGS = (
    "work_mem", "hash_mem_multiplier", "maintenance_work_mem", "temp_buffers",
    "temp_file_limit", "shared_buffers", "max_connections", "fsync", "full_page_writes",
    "synchronous_commit", "track_commit_timestamp", "ssl", "reserved_connections",
    "superuser_reserved_connections", "jit", "max_parallel_workers_per_gather",
    "shared_preload_libraries", "pg_stat_statements.track", "block_size", "log_temp_files",
    "effective_cache_size", "autovacuum", "max_worker_processes", "max_parallel_workers",
    "wal_level", "wal_compression", "wal_buffers", "track_io_timing",
)
DATABASES = {"mesh": "mesh", "oban": "oban", "go": "river", "rust": "apalis"}
PERIOD = 0.25


def expected_setting(mb):
    return dict(value=str(mb * 1024), unit="kB", source="configuration file", cluster_conf=True)


def settings(cluster, database):
    names = ", ".join(f"'{name}'" for name in SETTINGS)
    return json.loads(cluster.sql(database, f"""
        SELECT json_build_object(
            'settings', (SELECT json_object_agg(name, json_build_object(
                'value', setting, 'unit', unit, 'source', source,
                'cluster_conf', sourcefile = current_setting('config_file')))
                FROM pg_settings WHERE name IN ({names})),
            'database', current_database(),
            'database_cap', (SELECT datconnlimit FROM pg_database WHERE datname=current_database()),
            'tls', (SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()),
            'overrides', (SELECT count(*) FROM pg_db_role_setting
                WHERE EXISTS (SELECT 1 FROM unnest(setconfig) s WHERE s LIKE 'work_mem=%'))
        )
    """))


def io_stats(cluster, database):
    # River retains pg_stat_statements only in the maintenance database.
    observer = "postgres" if database == "river" else database
    return json.loads(cluster.sql(observer, f"""
        SELECT json_build_object(
          'database', (SELECT json_build_object('temp_files', temp_files, 'temp_bytes', temp_bytes)
                         FROM pg_stat_database WHERE datname='{database}'),
          'roles', (SELECT coalesce(json_object_agg(rolname, row_to_json(r)), '{{}}'::json) FROM (
              SELECT rolname, sum(calls) AS calls, sum(temp_blks_read) AS read_blocks,
                     sum(temp_blks_written) AS written_blocks,
                     count(*) FILTER (WHERE temp_blks_read > 0 OR temp_blks_written > 0) AS spilling_statements
              FROM pg_stat_statements s JOIN pg_roles ON userid=oid
              WHERE dbid = (SELECT oid FROM pg_database WHERE datname='{database}')
              GROUP BY rolname
          ) r),
          'mesh_claim', (SELECT coalesce(json_agg(row_to_json(q)), '[]'::json) FROM (
              SELECT queryid::text, rows, calls, total_exec_time AS execution_ms,
                     temp_blks_read AS read_blocks, temp_blks_written AS written_blocks
              FROM pg_stat_statements
              WHERE dbid = (SELECT oid FROM pg_database WHERE datname='{database}')
                AND userid = 'parity_subject'::regrole
                AND query LIKE '%FROM mesh_jobs.claim%'
              ORDER BY queryid
          ) q),
          'statements_evicted', (SELECT dealloc FROM pg_stat_statements_info)
        )
    """))


def linux_memory(root):
    proc = Path("/proc")
    children = (proc / f"{root}/task/{root}/children").read_text().split()
    rows = {}
    for pid in (str(root), *children):
        try:
            with (proc / pid / "smaps_rollup").open() as smaps:
                fields = {name: int(value.split()[0]) for line in smaps if ":" in line
                          for name, value in (line.split(":", 1),) if name in {"Rss", "Pss"}}
            if fields.keys() == {"Rss", "Pss"}:
                rows[pid] = {"rss_kib": fields["Rss"], "pss_kib": fields["Pss"]}
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    return rows, len(children) + 1


def mac_memory(root):
    output = subprocess.check_output(["ps", "-A", "-o", "pid=,ppid=,rss="], text=True, timeout=2)
    rows = {pid: {"rss_kib": int(rss), "pss_kib": None}
            for pid, ppid, rss in (line.split() for line in output.splitlines())
            if str(root) in (pid, ppid)}
    return rows, len(rows)


class MemorySamples:
    def __init__(self, pgdata):
        self.pid = int((pgdata / "postmaster.pid").read_text().splitlines()[0])
        self.points = []
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.sample, name="postgres-memory", daemon=True)
        self.thread.start()

    def sample(self):
        collect = linux_memory if sys.platform == "linux" else mac_memory
        while not self.stop.is_set():
            stamp = time.time_ns() // 1000
            try:
                rows, expected = collect(self.pid)
            except (OSError, subprocess.SubprocessError):
                rows, expected = {}, 0
            if str(self.pid) in rows and len(rows) == expected:
                self.points.append(dict(epoch_us=stamp, capture_us=time.time_ns() // 1000 - stamp,
                                        processes=len(rows),
                                        rss_sum_kib=sum(r["rss_kib"] for r in rows.values()),
                                        pss_sum_kib=sum(r["pss_kib"] for r in rows.values())
                                        if sys.platform == "linux" else None))
            self.stop.wait(PERIOD)

    def finish(self):
        self.stop.set()
        self.thread.join(timeout=3)
        return {"postmaster_pid": self.pid, "period_seconds": PERIOD, "points": self.points}


def write(path, document):
    with path.open("x") as stream:
        json.dump(document, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def instrument(base, mb, engine, out):
    database = DATABASES[engine]

    class Cluster(base):
        def configure_tls(self):
            super().configure_tls()
            with (self.data / "postgresql.conf").open("a") as stream:
                stream.write(f"\nwork_mem = '{mb}MB'\n")

        def start(self):
            super().start()
            self.workmem_before = settings(self, "postgres")
            if self.workmem_before["settings"]["work_mem"] != expected_setting(mb):
                raise RuntimeError("the disposable server did not load its assigned work_mem")
            self.workmem_memory = MemorySamples(self.data)

        def stop(self):
            if not hasattr(self, "workmem_memory"):
                return super().stop()
            memory = self.workmem_memory.finish()
            try:
                # The stock adapter has finished its run and stopped the runners.
                # Always bypass the warmed broker and use the stock TLS observer.
                after = settings(self, database)
                stats = io_stats(self, database)
                write(out / "workmem.json", dict(
                    expected_mb=mb, before=self.workmem_before, after=after, io=stats, memory=memory,
                ))
            except Exception as error:
                # The original adapter retains the primary failure. Replay
                # requires the full sidecar for any successful qualification.
                write(out / "workmem-probe-failure.json", dict(kind=type(error).__name__, memory=memory,
                          stage="post-settlement settings or observer counters"))
            finally:
                super().stop()

    return Cluster
