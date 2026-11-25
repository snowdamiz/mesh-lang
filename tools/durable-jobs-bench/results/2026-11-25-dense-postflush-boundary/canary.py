"""Exercise the actual PostgreSQL 17.11 hook in an isolated correctness cluster."""
import argparse
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

from boundary import US, parse_export, require

EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)


def canary(pg, module, workspace):
    with tempfile.TemporaryDirectory(prefix="postflush-", dir=workspace) as scratch:
        scratch = Path(scratch)
        data = scratch / "pgdata"

        def call(name, *args, **kwargs):
            return subprocess.run([str(pg / name), *args], check=True, timeout=30,
                                  text=True, stdout=subprocess.PIPE, env=dict(os.environ, TZ="UTC"), **kwargs).stdout

        def sql(statement, role="postgres"):
            return call("psql", "-XAtq", "-v", "ON_ERROR_STOP=1", "-h", str(scratch), "-U", role,
                        "-d", "postgres", input=statement).splitlines()

        require(call("postgres", "--version").split()[2] == "17.11", "unexpected canary PG")
        call("initdb", "-D", str(data), "-U", "postgres", "-A", "trust", "--no-locale")
        # Exercise a nonzero epoch with a disposable cluster and no live data.
        call("pg_resetwal", "-e", "7", str(data))
        output = data / "mesh-postflush"
        output.mkdir(mode=0o700)
        call("pg_ctl", "-D", str(data), "-o", f"-c listen_addresses='' -k '{scratch}' -c timezone=UTC "
             "-c synchronous_commit=on -c fsync=on -c track_commit_timestamp=on -c max_prepared_transactions=2",
             "-l", str(scratch / "pg.log"), "-w", "start")
        try:
            require(sql("SELECT current_setting('server_version_num'), current_setting('fsync'), "
                        "current_setting('synchronous_commit'), current_setting('track_commit_timestamp');")
                    == ["170011|on|on|on"], "unexpected canary durability")
            sql(f"""
              CREATE ROLE compiled_caller LOGIN;
              CREATE ROLE unrelated LOGIN;
              CREATE TABLE durable_canary (item integer);
              GRANT SELECT, INSERT ON durable_canary TO compiled_caller, unrelated;
              ALTER ROLE compiled_caller IN DATABASE postgres SET session_preload_libraries TO '{module}';
              ALTER ROLE unrelated IN DATABASE postgres SET session_preload_libraries TO '{module}';
              ALTER ROLE compiled_caller IN DATABASE postgres SET mesh_postflush.enabled TO 'on';
            """)
            wal_start, = sql("SELECT pg_current_wal_insert_lsn();")
            before = time.time_ns()
            pid, read_only, aborted, prepared, xid, state = sql("""
              SELECT pg_backend_pid();
              SELECT pg_current_xact_id_if_assigned() IS NULL;
              BEGIN;
              INSERT INTO durable_canary VALUES (1);
              SELECT pg_current_xact_id();
              ROLLBACK;
              BEGIN;
              INSERT INTO durable_canary VALUES (2);
              SELECT pg_current_xact_id();
              PREPARE TRANSACTION 'canary';
              BEGIN;
              INSERT INTO durable_canary VALUES (3);
              SELECT pg_current_xact_id();
              COMMIT;
              SELECT pg_current_xact_id_if_assigned() IS NULL;
            """, "compiled_caller")
            after = time.time_ns()
            require(read_only == state == "t", "observer assigned an XID")
            record_us, wal_end = sql(f"""
              SELECT (extract(epoch FROM pg_xact_commit_timestamp('{int(xid) % (1 << 32)}'::xid)) * 1000000)::bigint;
              SELECT pg_current_wal_insert_lsn();
            """)
            dump = call("pg_waldump", "-p", str(data / "pg_wal"), "-s", wal_start, "-e", wal_end,
                        "-x", str(int(xid) % (1 << 32)))
            timestamps = re.findall(r"desc: COMMIT (\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+ UTC)", dump)
            require(len(timestamps) == 1, f"expected one timestamped WAL commit: {dump}")
            wall = timestamps[0]
            delta = datetime.fromisoformat(wall.removesuffix(" UTC")).replace(tzinfo=timezone.utc) - EPOCH
            require(int(record_us) == delta // timedelta(microseconds=1),
                    "SQL timestamp differs from WAL record")
            sql("ALTER ROLE compiled_caller IN DATABASE postgres SET mesh_postflush.enabled TO 'off';")
            off_pid, = sql("INSERT INTO durable_canary VALUES (4); SELECT pg_backend_pid();", "compiled_caller")
            other_pid, = sql("INSERT INTO durable_canary VALUES (5); SELECT pg_backend_pid();", "unrelated")
            sql("ROLLBACK PREPARED 'canary';")
            require(sql("SELECT item FROM durable_canary ORDER BY item;") == ["3", "4", "5"], "canary writes differ")
        finally:
            call("pg_ctl", "-D", str(data), "-m", "fast", "-w", "stop")

        require({p.name for p in output.iterdir()} == {f"{pid}.log", f"{off_pid}.log"}
                and pid != other_pid, "unexpected backend export roster")
        _, _, rows = parse_export((output / f"{pid}.log").read_text(), int(pid), True)
        _, _, off_rows = parse_export((output / f"{off_pid}.log").read_text(), int(off_pid), False)
        require(int(xid) >> 32 == 7 and set(rows) == {int(xid)} and not off_rows
                and int(aborted) not in rows and int(prepared) not in rows, "commit event or full XID differs")
        require(before // US <= int(record_us) <= rows[int(xid)] // US <= after // US,
                "WAL, callback and client clock order differs")
        return dict(version="17.11", wal_record_matches_sql=True, callback_between_record_and_client=True,
                    preserved_full_xid_epoch=True, inactive_empty=True, foreign_role_excluded=True,
                    read_only_abort_and_prepare_excluded=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pg_bin", type=Path)
    parser.add_argument("module", type=Path)
    parser.add_argument("workspace", type=Path)
    args = parser.parse_args()
    print(json.dumps(canary(args.pg_bin, args.module, args.workspace), sort_keys=True))
