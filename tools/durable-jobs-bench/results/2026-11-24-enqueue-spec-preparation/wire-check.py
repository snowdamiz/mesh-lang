#!/usr/bin/env python3
"""Run only semantic comparisons in a newly initialized PostgreSQL cluster."""
import os
from pathlib import Path
import pwd
import shutil
import socket
import subprocess
import sys


def main(pg_bin, bin_dir, work):
    work.mkdir()
    db = work / "postgres"
    pgctl = pg_bin / "pg_ctl"
    env = dict(os.environ, TMPDIR=str(work) + "/")
    user = pwd.getpwuid(os.getuid()).pw_name
    platform_log = work / "postgres-version.txt"
    platform_log.write_bytes(subprocess.check_output([pg_bin / "postgres", "--version"]))
    with (work / "initdb.log").open("xb") as log:
        subprocess.run([pg_bin / "initdb", "-D", db, "--auth=trust", "--encoding=UTF8", "--no-locale"],
                       env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    with (db / "postgresql.conf").open("a") as settings:
        settings.write(f"""
listen_addresses = '127.0.0.1'
unix_socket_directories = ''
port = {port}
fsync = on
synchronous_commit = on
shared_preload_libraries = 'pg_stat_statements'
track_commit_timestamp = on
""")
    started = False
    try:
        subprocess.run([pgctl, "start", "-D", db, "-l", work / "postgres.log", "-w"],
                       env=env, check=True, stdout=subprocess.DEVNULL)
        started = True
        command = [pg_bin / "psql", "-h", "127.0.0.1", "-p", str(port), "-U", user,
                   "-v", "ON_ERROR_STOP=1"]
        for profile, flag in (("large", "0"), ("short", "1")):
            subprocess.run([*command, "-d", "postgres", "-c", f"CREATE DATABASE {profile}"],
                           env=env, check=True, stdout=subprocess.DEVNULL)
            subprocess.run([*command, "-d", profile, "-c", "CREATE EXTENSION pg_stat_statements"],
                           env=env, check=True, stdout=subprocess.DEVNULL)
            run = subprocess.run([bin_dir / profile, "verify-wire"], capture_output=True,
                                 env=dict(env, MESH_JOBS_SET_BASED_ENQUEUE=flag,
                                          ENQUEUE_SPEC_TEST_PG_URL=f"postgresql://{user}@127.0.0.1:{port}/{profile}?sslmode=disable"))
            (work / f"{profile}.json").write_bytes(run.stdout)
            (work / f"{profile}.stderr").write_bytes(run.stderr)
            with (work / f"{profile}-writer-calls.txt").open("xb") as log:
                subprocess.run([*command, "-d", profile, "-Atc",
                                "SELECT query, calls FROM pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND (query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%' OR query LIKE 'SELECT mesh_jobs.enqueue(%') ORDER BY query"],
                               env=env, stdout=log, check=True)
            run.check_returncode()
    finally:
        # Verify shutdown before removing the owned data directory.
        if started:
            subprocess.run([pgctl, "stop", "-m", "fast", "-w", "-D", db],
                           env=env, check=True, stdout=subprocess.DEVNULL)
        shutil.rmtree(db)


if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
