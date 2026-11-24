"""Reproduce the archived 42703 and verify the controller's repair, untimed."""

import argparse
from collections import Counter
import json
from pathlib import Path
import re
import runpy
import socket
import subprocess
import tarfile
import tempfile

import packet

HERE = packet.PACKET
RETRY = runpy.run_path(str(HERE / "sql-audit-retry.py"))
OLD = runpy.run_path(str(HERE / "adapter.py"))["STATEMENTS"]
FIXED = runpy.run_path(str(RETRY["ADAPTER"]))["STATEMENTS"]


def classify():
    final_pins = RETRY["pins"]()
    archive_path = RETRY["FAILED"]
    portable = HERE / "evidence/first-setup-failed"
    release = packet.run.read_json(portable / "controller-release.json")
    packet.require(release["id"] == packet.ID + "-setup" and release["exit_status"] == 1,
                   "incorrect failed controller receipt")
    with tarfile.open(archive_path) as archive:
        def read(name):
            return packet.run.file_json(archive, name)

        statement_file = archive.extractfile("adapter/physical-postgres.log")
        log = statement_file.read().decode()
        error = re.search(r' \| ERROR:  column "current_database" does not exist at character (\d+)\n'
                          r'[^\n]+ \| STATEMENT:  (.+(?:\n\t[^\n]*)*)', log)
        packet.require(error is not None, "server did not log the failed SQL")
        sql = "\n".join(line.removeprefix("\t") for line in error[2].splitlines())
        expected = f"SELECT ({OLD.strip()})::text AS value"
        packet.require(sql == expected and sql[int(error[1]) - 1:].startswith("CURRENT_DATABASE"),
                       "logged SQL differs from the pinned audit statement")
        failure = read("adapter/failure.json")
        invocation = read("invocation.json")
        database = read("adapter/database.json")
        roles = read("adapter/postgres-tls-sessions.json")
        lifecycle = read("adapter/sql-lifecycle.json")
        members = archive.getnames()
        packet.require("42703" in failure["message"] and failure["stage"][1][2] == "run_trial"
                       and failure["stage"][2] == ["adapter.py", 131, "calls"]
                       and invocation["engine"] == "mesh" and invocation["profile"] == "slow"
                       and read("status.json")["exit_code"] == 1, "failure is not the original Mesh SQL audit")
        packet.require(not any("enqueue-" in name or name.endswith("/ledger.json") for name in members)
                       and "adapter/statements-before.json" not in members
                       and read("adapter/counters-1.json")["application"] == 0
                       and not any(re.search(r"\binsert\s+into.*mesh_jobs_parity_(attempts|effects)",
                                             row["query"], re.I) for row in lifecycle["statements"]),
                       "workload or completed snapshot is present in the failed arm")
        warm = Counter(row["role"] for row in roles if row["tls"])
        packet.require(len(roles) == 17 and warm == {"parity_subject": 8, "compiled_caller": 8, "parity_producer": 1}
                       and database["max_connections"] == database["database_connection_limit"] == 71
                       and lifecycle["post_arm"] == {"cap": 71, "dealloc": 0, "sessions": {"parity_owner": 1}},
                       "original connection envelope differs")
    config = packet.design()
    setup = packet.plan(config, "setup")
    timing = packet.plan(config, "timing")
    packet.require(len(setup) == 1 and len(setup[0]["order"]) == 4 and setup[0]["phases"] == ["warmup"]
                   and len(timing) == 8 and all(row["phases"] == ["warmup", "measured"] for row in timing)
                   and timing[0]["order"] == timing[1]["order"][::-1], "native gate or timing plan changed")
    seals = {}
    for mode in ("setup", "timing"):
        request = HERE / f"request-sql-retry-{mode}.json"
        inbox = packet.QUEUE / f"{RETRY['ID']}-{mode}.json"
        packet.require(packet.run.read_json(request)["local_sha256"] == final_pins, "retry request inputs drifted")
        if inbox.exists():
            packet.require(request.read_bytes() == inbox.read_bytes(), "controller holds a different retry request")
        seals[mode] = packet.run.digest(request)
    return sql, dict(attempt=invocation["attempt"], archive_sha256=packet.run.digest(archive_path),
                     server_sqlstate="42703", server_error_position=int(error[1]), failure=failure,
                     failed_before_submit=True, native_warm_roles=dict(warm), post_arm=lifecycle["post_arm"],
                     preserved_native_plan=dict(setup=setup, timing=timing), retry_request_sha256=seals)


def regression(bin_dir, original_sql):
    version = subprocess.check_output([bin_dir / "postgres", "--version"], text=True).strip()
    packet.require(re.match(r"postgres \(PostgreSQL\) 17\.11(?:\s|$)", version) is not None,
                   "expected PostgreSQL 17.11 for the untimed semantic check")
    lane = packet.TARGET
    lane.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="sql-audit-unit-", dir=lane) as location:
        path = Path(location)
        data = path / "data"

        def pg(program, *args, check=True):
            return subprocess.run([bin_dir / program, *map(str, args)], cwd=path, text=True,
                                  capture_output=True, check=check)

        pg("initdb", "-D", data, "-U", "parity_owner", "--no-locale", "--encoding=UTF8", "--auth=trust")
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        with (data / "postgresql.conf").open("a") as stream:
            stream.write(f"\nlisten_addresses = '127.0.0.1'\nport = {port}\nunix_socket_directories = ''\n"
                         "shared_preload_libraries = 'pg_stat_statements'\n")
        try:
            pg("pg_ctl", "-D", data, "-l", path / "postgres.log", "-w", "-t", 15, "start")

            def query(database, user, sql, check=True):
                return pg("psql", "-X", "-Atq", "-h", "127.0.0.1", "-p", port, "-d", database, "-U", user,
                          "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=verbose", "-c", sql, check=check)

            admin = "parity_owner"
            producer = "parity_producer"
            query("postgres", admin, "CREATE DATABASE mesh")
            for name in ("postgres", "mesh"):
                query(name, admin, "CREATE EXTENSION pg_stat_statements")
            query("postgres", admin, "CREATE ROLE parity_producer LOGIN; CREATE ROLE parity_subject LOGIN; "
                  "CREATE ROLE compiled_caller LOGIN; GRANT pg_read_all_stats TO parity_producer")
            query("mesh", "parity_subject", "SELECT 3 AS audit_runner_mesh")
            query("mesh", "compiled_caller", "SELECT 4 AS audit_application_mesh")
            query("mesh", admin, "SELECT 5 AS audit_excluded_admin")
            query("postgres", "parity_subject", "SELECT 7 AS audit_other_database")
            rejected = query("mesh", producer, original_sql, check=False)
            packet.require(rejected.returncode != 0 and re.search(
                r'ERROR:\s+42703: column "current_database" does not exist', rejected.stderr),
                "PostgreSQL did not reproduce the archived server error")
            fixed_sql = f"SELECT ({FIXED.strip()})::text AS value"
            packet.require(fixed_sql == original_sql.replace("CURRENT_DATABASE)", "CURRENT_DATABASE())"),
                           "repaired broker statement changed beyond the function call")
            broker = json.loads(query("mesh", producer, fixed_sql).stdout)
            teardown = json.loads(query("postgres", admin, FIXED.replace("CURRENT_DATABASE()", "'mesh'")).stdout)
            other = json.loads(query("postgres", admin, FIXED).stdout)
            expected = {"audit_runner_mesh": ("parity_subject", 1, 1),
                        "audit_application_mesh": ("compiled_caller", 1, 1)}

            def witnesses(rows):
                return {key: (row["role"], row["calls"], row["rows"])
                        for row in rows for key in expected if key in row["query"]}

            packet.require(witnesses(broker) == witnesses(teardown) == expected and not witnesses(other)
                           and all("audit_excluded_admin" not in row["query"]
                                   and "audit_other_database" not in row["query"] for row in broker + teardown)
                           and any(row["role"] == "parity_subject" and "audit_other_database" in row["query"]
                                   for row in other), "role or database scoping changed")
            return dict(postgres_version=version, old_statement_sqlstate="42703",
                        corrected_broker_witnesses=witnesses(broker), corrected_teardown_witnesses=witnesses(teardown),
                        other_database_and_admin_excluded=True, mode="untimed SQL correctness only")
        finally:
            if (data / "postmaster.pid").exists():
                pg("pg_ctl", "-D", data, "-m", "immediate", "-w", "-t", 15, "stop")


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--pg-bin", type=Path, help="also replay the exact SQL against a disposable PostgreSQL 17.11")
    args = cli.parse_args()
    original, result = classify()
    if args.pg_bin:
        result["untimed_regression"] = regression(args.pg_bin.resolve(), original)
    print(json.dumps(result, indent=2, sort_keys=True))
