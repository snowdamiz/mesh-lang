"""Replay stock nested clock calls and check PG17 inlining on a disposable database."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SOURCE = "b3557273d72be3802bf189b284e7caaf5f3caa0f"


def replay_calls():
    evidence = HERE.parent / "2026-11-24-r2-d2-producer-boundary/evidence"
    release = json.loads((evidence / "recovery/controller-timing-5f921d94efd8.json").read_bytes())
    assert release["exit_status"] == 0 and release["groups"] == [1, 2, 3, 4, 5]
    rows = []
    for group in release["groups"]:
        block, = (evidence / "native-timing").glob(f"*/blocks/{group:02d}-short.*")
        lane = "measured-nested/measured-mesh"
        digest = json.loads((block / "reconciled.json").read_bytes())["raw_sha256"][lane]
        raw = block / lane / "raw.tar.gz"
        assert sha256(raw.read_bytes()).hexdigest() == digest
        with tarfile.open(raw) as archive:
            stats = json.load(archive.extractfile("adapter/producer-sql.json"))
            ledger = json.load(archive.extractfile("adapter/ledger.json"))
        clock, = [s for s in stats["statements"] if s["queryid"] == -3928656517956893112]
        assert clock["query"].strip() == "SELECT floor(extract($1 FROM clock_timestamp()) * $2)::bigint"
        assert clock["toplevel"] is False and clock["calls"] == clock["rows"] == len(ledger["jobs"]) == 128
        rows.append(dict(group=group, calls=clock["calls"], inner_exec_ms=clock["total_exec_time"], raw_sha256=digest))
    return rows


def verify(pg_bin):
    with tempfile.TemporaryDirectory(dir=ROOT / "target") as tmp:
        scratch = Path(tmp)
        data = scratch / "pg"
        def command(binary, *args):
            return subprocess.check_output([pg_bin / binary, *args], text=True)

        command("initdb", "-A", "trust", "-U", "clock_owner", "--no-locale", "-E", "UTF8", "-D", data)
        command("pg_ctl", "-w", "-D", data, "-l", scratch / "server.log", "-o",
                f"-c listen_addresses='' -k {scratch}", "start")
        try:
            def sql(query):
                return subprocess.check_output([pg_bin / "psql", "-XqAt", "-v", "ON_ERROR_STOP=1",
                                                "-h", scratch, "-U", "clock_owner", "-d", "postgres"],
                                               input=query, text=True).strip()

            assert sql("SHOW server_version_num") == "170011"
            for file in ("schema.sql", "enqueue_schema.sql"):
                sql(subprocess.check_output(["git", "show", f"{SOURCE}:compiler/mesh-rt/src/jobs/{file}"],
                                            cwd=ROOT, text=True))
            stock_definition = sql("SELECT pg_get_functiondef('mesh_jobs.now_ms()'::regprocedure)")
            batch = json.dumps([dict(request_key=f"job_{i}", fingerprint=f"fp_{i}", spec=dict(
                worker="worker", version=1, queue="default", args={"n": i}, metadata={}, tags=[],
                unique=None, replace={}, suspended=False, priority=0, max_attempts=1,
                timeout_ms=1000, backoff_base_ms=0, backoff_max_ms=0)) for i in range(128)])
            def write(namespace):
                return sql(f"""SET track_functions = 'all';
                    BEGIN;
                    INSERT INTO mesh_jobs.admissions(namespace) VALUES ('{namespace}');
                    SELECT string_agg(ordinal, ',') FROM mesh_jobs.enqueue_batch_v1('{namespace}', '{batch}'::jsonb);
                    COMMIT;""")
            def counted_write(namespace):
                sql("SELECT pg_stat_reset_single_function_counters('mesh_jobs.now_ms()'::regprocedure)")
                assert write(namespace) == ",".join(map(str, range(1, 129)))
                rows = sql(f"""SELECT count(*) FROM mesh_jobs.jobs j JOIN mesh_jobs.receipts r
                    ON r.namespace=j.namespace AND r.receipt->>'id'=j.id::text
                    WHERE j.namespace='{namespace}' AND r.request_key='job_' || (j.args->>'n')
                    AND r.fingerprint='fp_' || (j.args->>'n') AND j.inserted_at_ms=j.scheduled_at_ms""")
                assert rows == "128"
                return int(sql("SELECT coalesce(pg_stat_get_function_calls('mesh_jobs.now_ms()'::regprocedure), 0)"))
            explain = "EXPLAIN (VERBOSE, FORMAT JSON, COSTS OFF) SELECT mesh_jobs.now_ms() FROM generate_series(1,128)"
            def snapshot():
                expression, = json.loads(sql(explain))[0]["Plan"]["Output"]
                return dict(expression=expression, bulk_layout=sql("SELECT mesh_jobs.enqueue_batch_layout_v1()"))

            stock = snapshot()
            assert stock["bulk_layout"] == "t" and "mesh_jobs.now_ms()" in stock["expression"]
            stock["calls"] = counted_write("original")
            assert stock["calls"] == 128
            sql("""
                SET search_path = pg_catalog, pg_temp;
                CREATE OR REPLACE FUNCTION mesh_jobs.now_ms() RETURNS bigint
                LANGUAGE sql VOLATILE RETURN floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint;
            """)
            alternative = snapshot()
            assert alternative["bulk_layout"] == "f" and "clock_timestamp()" in alternative["expression"]
            assert "mesh_jobs.now_ms()" not in alternative["expression"]
            marker = write("whole-helper")
            assert marker == "0"
            sql(stock_definition)
            writer = sql("SELECT pg_get_functiondef('mesh_jobs.enqueue_batch_v1(text,jsonb)'::regprocedure)")
            assert writer.count("mesh_jobs.now_ms() AS ts") == 1
            sql(writer.replace("mesh_jobs.now_ms() AS ts", "floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint AS ts"))
            assert sql("SELECT mesh_jobs.enqueue_batch_layout_v1()") == "t"
            calls = counted_write("writer-only")
            assert calls == 0
            return dict(stock=stock, inlined=alternative, fallback_ordinal=marker, writer_only_calls=calls)
        finally:
            command("pg_ctl", "-w", "-D", data, "-m", "fast", "stop")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pg_bin", type=Path)
    args = parser.parse_args()
    print(json.dumps(dict(source=SOURCE, nested=replay_calls(), postgres=verify(args.pg_bin)), indent=2))
