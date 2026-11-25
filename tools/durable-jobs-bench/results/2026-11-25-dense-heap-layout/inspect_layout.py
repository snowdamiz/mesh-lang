"""Inspect PostgreSQL heap tuple and physical args sizes for 16 real dense input rows."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import runpy
import subprocess
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SQL = ROOT / "compiler/mesh-rt/src/jobs"
INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"

HEAP = """
WITH current_rows AS (
    SELECT j.ctid AS loc, (j.ctid::text::point)[0]::integer AS block,
        (j.ctid::text::point)[1]::integer AS item, pg_column_compression(j.args) AS codec
    FROM mesh_jobs.jobs j
), sizes AS (
    SELECT p.lp_len, octet_length(p.t_attrs[8]) AS arg_bytes, r.codec
    FROM current_rows r JOIN LATERAL heap_page_item_attrs(
        get_raw_page('mesh_jobs.jobs',r.block), 'mesh_jobs.jobs'::regclass, false
    ) p ON p.lp = r.item
)
SELECT json_build_object('rows', count(*), 'tuple_min', min(lp_len), 'tuple_max', max(lp_len),
    'args_min', min(arg_bytes), 'args_max', max(arg_bytes), 'compressed', count(codec)) FROM sizes
"""


def inspect(bindir):
    def tool(name, *args, **kwargs):
        return subprocess.run([bindir / name, *map(str, args)], check=True, text=True,
                              stdout=subprocess.PIPE, **kwargs).stdout.strip()

    work = ROOT / "target"
    work.mkdir(exist_ok=True)
    with TemporaryDirectory(dir=work, prefix="dh-") as tmp:
        scratch = Path(tmp)
        harness = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))
        config = json.loads(harness["CONFIG"].read_text())
        batches = harness["inputs"](config["workloads"]["noop"], 64, config["seed"])
        files = harness["batch_files"](scratch, batches)
        assert sha256("".join(harness["digest"](p) for p in files).encode()).hexdigest() == INPUT
        jobs = batches[0]["jobs"][:16]
        assert all(len(j["payload"]) == 1024 for j in jobs)
        data = scratch / "pg"
        tool("initdb", "-D", data, "-A", "trust", "--no-locale", "-E", "UTF8", "-U", "owner")
        tool("pg_ctl", "-D", data, "-l", scratch / "pg.log", "-w", "-o",
             f"-c listen_addresses='' -c unix_socket_directories={scratch}", "start")
        try:
            def query(db, text):
                return tool("psql", "-XqAt", "-v", "ON_ERROR_STOP=1", "-h", scratch,
                            "-U", "owner", "-d", db, input=text)
            assert query("postgres", "SHOW server_version_num") == "170011"
            out = {}
            for db, ddl in (
                ("stock", ""),
                ("external", "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL"),
                ("external512", "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL;"
                                "ALTER TABLE mesh_jobs.jobs SET (toast_tuple_target=512)"),
            ):
                query("postgres", f"CREATE DATABASE {db}")
                query(db, (SQL / "schema.sql").read_text())
                query(db, (SQL / "enqueue_schema.sql").read_text())
                query(db, "CREATE EXTENSION pageinspect")
                if ddl:
                    query(db, ddl)
                guard = query(db, "SELECT mesh_jobs.enqueue_batch_layout_v1()")
                assert guard == ("f" if db == "external512" else "t")
                # Use the same scalar product writer in each geometry proof.
                specs = [dict(worker="parity", version=1, queue="default", args=j, metadata={},
                              tags=[], unique=None, priority=0, max_attempts=3, timeout_ms=60000,
                              backoff_base_ms=1000, backoff_max_ms=1000) for j in jobs]
                specs_json = json.dumps(specs)
                assert "$specs$" not in specs_json
                inserted = query(db, f"""SELECT count(*) FROM jsonb_array_elements(
                  $specs${specs_json}$specs$::jsonb) s
                  WHERE (mesh_jobs.enqueue('parity',s,s->'args'->>'key',md5(s::text))->>'inserted')::boolean""")
                assert inserted == "16"
                sample = dict(guard=guard, available=json.loads(query(db, HEAP)))
                query(db, "UPDATE mesh_jobs.queues SET concurrency=16 WHERE namespace='parity'")
                claimed = json.loads(query(db, """SELECT json_agg(json_build_array(status, context::jsonb->'args'))
                    FROM mesh_jobs.claim_batch_set_v1('parity','default','[{"worker":"parity","version":1}]',
                    60000,16,NULL)"""))
                assert claimed == [["claimed", j] for j in jobs]
                sample["executing"] = json.loads(query(db, HEAP))
                settled = query(db, """SELECT count(*) FROM mesh_jobs.jobs j WHERE mesh_jobs.settle_state(
                    j.namespace,j.id,j.lease_token,'{"action":"complete"}',false)='completed'""")
                assert settled == "16"
                sample["completed"] = json.loads(query(db, HEAP))
                terminal = json.loads(query(db, """SELECT json_agg(args ORDER BY position) FROM mesh_jobs.jobs
                                                     WHERE state='completed'"""))
                assert terminal == jobs
                if db != "stock":
                    pointer = query(db, """BEGIN;
                        UPDATE mesh_jobs.jobs SET args=jsonb_set(args, '{payload}',
                            to_jsonb(repeat(args->>'payload', 5))) WHERE position=1;
                        SELECT p.lp_len, octet_length(p.t_attrs[8])
                        FROM mesh_jobs.jobs j CROSS JOIN LATERAL heap_page_item_attrs(
                            get_raw_page('mesh_jobs.jobs',(j.ctid::text::point)[0]::int),
                            'mesh_jobs.jobs'::regclass, false) p
                        WHERE j.position=1 AND p.lp=(j.ctid::text::point)[1]::int;
                        ROLLBACK;""")
                    size, raw_arg = map(int, pointer.split("|"))
                    assert raw_arg == 18 and size < sample["completed"]["tuple_min"]
                    sample["oversized_control"] = dict(tuple_bytes=size, args_bytes=raw_arg)
                out[db] = sample
            sources = {p.name: sha256(p.read_bytes()).hexdigest()
                       for p in (SQL / "schema.sql", SQL / "enqueue_schema.sql")}
            return {"postgres": "17.11", "profile": "noop", "input_sha256": INPUT,
                    "schema_sha256": sources, "layouts": out}
        finally:
            tool("pg_ctl", "-D", data, "-m", "fast", "-w", "stop")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bindir", type=Path)
    args = parser.parse_args()
    print(json.dumps(inspect(args.bindir.resolve()), indent=2))
