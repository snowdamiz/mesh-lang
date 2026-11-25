"""Stop the real reduced-UID launcher at arm.main and exercise its SQL against local PG."""

from contextlib import ExitStack
import ctypes
import json
import os
from pathlib import Path
import sys

# Loaded as sitecustomize by the local container only. Leave the root staging
# interpreter alone and intercept the worker before it can begin a workload.
if os.geteuid() == 65534:
    pq = ctypes.CDLL("libpq.so.5")
    for name, args, result in (
        ("PQconnectdb", [ctypes.c_char_p], ctypes.c_void_p),
        ("PQstatus", [ctypes.c_void_p], ctypes.c_int),
        ("PQerrorMessage", [ctypes.c_void_p], ctypes.c_char_p),
        ("PQfinish", [ctypes.c_void_p], None),
        ("PQexec", [ctypes.c_void_p, ctypes.c_char_p], ctypes.c_void_p),
        ("PQresultStatus", [ctypes.c_void_p], ctypes.c_int),
        ("PQgetvalue", [ctypes.c_void_p, ctypes.c_int, ctypes.c_int], ctypes.c_char_p),
        ("PQclear", [ctypes.c_void_p], None),
    ):
        function = getattr(pq, name)
        function.argtypes, function.restype = args, result


def connect(stack, role, database="mesh"):
    connection = pq.PQconnectdb(
        f"host=127.0.0.1 port=5432 dbname={database} user={role} sslmode=require connect_timeout=3".encode())
    stack.callback(pq.PQfinish, connection)
    assert pq.PQstatus(connection) == 0, pq.PQerrorMessage(connection)
    return connection


def query(connection, sql):
    result = pq.PQexec(connection, sql.encode())
    try:
        status = pq.PQresultStatus(result)
        assert status in (1, 2), pq.PQerrorMessage(connection)
        return pq.PQgetvalue(result, 0, 0).decode() if status == 2 else None
    finally:
        pq.PQclear(result)


def check(frame):
    import backend
    import contract
    from roster import ROSTER
    from roster8 import bind

    lane = Path(sys.argv[-1]).name
    probe = lane.startswith("plan-")
    candidate = lane in {"key", "plan-key"}
    assert sys.dont_write_bytecode and os.geteuid() == 65534
    assert os.environ["TRUSTED_ADMISSION_POLICY"] == "set_based"
    assert os.environ["MESH_JOBS_SET_BASED_ENQUEUE"] == "1"
    assert backend.MODES[os.environ["PRODUCT_COMPOSITION"]] == ("set_based", False)
    subject = frame.f_globals["Subject"]
    cluster = frame.f_globals["Cluster"]
    assert (subject.__name__, cluster.__name__) == (
        ("ObservedSubject", "ObservedCluster") if probe else ("Subject", "Cluster"))
    assert backend.arm.PROFILES["noop"][:4] == (8, 64, 4, 96)
    stage = Path(backend.__file__).parent
    assert stage.stat().st_mode & 0o777 == 0o755
    assert all(p.stat().st_uid == 0 and p.stat().st_mode & 0o777 == 0o644 for p in stage.iterdir())

    with ExitStack() as stack:
        admin = connect(stack, "postgres", "postgres")
        assert query(admin, "SHOW server_version_num") == "170011"
        query(admin, "DROP DATABASE IF EXISTS mesh")
        if query(admin, "SELECT count(*) FROM pg_roles WHERE rolname='parity_subject'") == "0":
            query(admin, """
                CREATE ROLE parity_subject LOGIN CONNECTION LIMIT 32;
                CREATE ROLE compiled_caller LOGIN CONNECTION LIMIT 32;
                CREATE ROLE parity_producer LOGIN CONNECTION LIMIT 1;
                CREATE ROLE parity_migrator NOLOGIN;
                GRANT pg_read_all_stats TO parity_producer;
            """)
        query(admin, "CREATE DATABASE mesh CONNECTION LIMIT 96")
        owner = connect(stack, "postgres")
        query(owner, Path("/fixture/schema.sql").read_text())
        query(owner, f"""
            CREATE EXTENSION pg_stat_statements;
            ALTER FUNCTION {contract.SIGNATURE} OWNER TO parity_migrator;
            REVOKE ALL ON FUNCTION {contract.SIGNATURE} FROM PUBLIC;
            GRANT EXECUTE ON FUNCTION {contract.SIGNATURE} TO parity_subject;
        """)

        class LocalSQL:
            data = evidence = Path(sys.argv[-1])

            def sql(self, database, sql):
                assert database == "mesh"
                return query(owner, sql)

        adapter = subject.__new__(subject)
        adapter.cluster = LocalSQL()
        stock_subject = subject.__bases__[0] if probe else subject
        stock = stock_subject.call.__globals__
        assert stock["LANE"] == lane and stock["PROBE"] is probe
        assert (stock["CANDIDATE"] is not None) is candidate
        overlay = contract.apply(adapter.cluster, "mesh", stock["CANDIDATE"])
        expected = contract.body(stock["CANDIDATE"] if candidate else contract.STOCK)
        assert overlay["body_sha256"] == contract.digest(expected)
        if probe:
            # Isolate the observer's real post-migration hook. The exact
            # migrated schema and SQL replacement were applied immediately above.
            stock_subject.call = lambda *_: None
            adapter.call("migrate", {}, None)
            observer = subject.call.__globals__
            capture = json.loads(query(owner, observer["STATS"]))
            assert capture["version_num"] == "170011"
            assert "backend_start_us" in observer["STATS"]

        with ExitStack() as clients:
            producer = connect(clients, "parity_producer")
            adapter.cluster.sql = lambda db, sql: query(producer, sql)
            witness = dict(snapshots=[adapter.roster(0)], runners_by_index=list(range(801, 809)))
            for index in range(1, 9):
                for role in ("compiled_caller", "parity_subject"):
                    for _ in range(4):
                        connection = connect(clients, role)
                    setting = query(connection, "SELECT coalesce(current_setting('auto_explain.sample_rate', true), '')")
                    assert bool(setting) is (probe and role == "parity_subject")
                    if setting:
                        assert query(connection, "SHOW auto_explain.sample_rate") == "1"
                witness["snapshots"].append(adapter.roster(index))
            roster, ports = bind(witness)
            assert len(roster) == 65 and len(ports) == 8
            assert json.loads(query(producer, ROSTER))["caps"] == witness["snapshots"][0]["caps"]
        print(f"{lane} Linux uid={os.geteuid()} PostgreSQL 17.11: exact overlay, scoped observer={probe}, "
              "8 TLS rosters, 65 sessions, role caps 32/32/1, database and server cap 96", flush=True)


def stop_at_main(frame, event, arg):
    if (event == "call" and frame.f_code.co_name == "main"
            and frame.f_code.co_filename.endswith("/tools/durable-jobs-bench/native/arm.py")):
        sys.settrace(None)
        check(frame.f_back)
        raise SystemExit(0)


if os.geteuid() == 65534:
    sys.settrace(stop_at_main)
