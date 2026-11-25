"""Check the recovery SQL on disposable PostgreSQL databases without running a benchmark."""
import argparse
from contextlib import contextmanager
from hashlib import sha256
import json
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
REV = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
STOCK = "c5e348c3f324c1e0a7b7d38715be47ae557b1bcfd46eb63e4edeffc9c5046763"
CANDIDATE = "86a17110290c835a983cbdae9feb0a5007d397d8aab3300b2d018fda52cecbc9"
DUE = "SELECT mesh_jobs.recover('n', 1, '[\"a\"]')"
SEED = """
INSERT INTO mesh_jobs.queues(namespace,name) VALUES ('n','a'),('n','b'),('z','a');
INSERT INTO mesh_jobs.jobs(namespace,queue,id,worker,version,args,metadata,tags,state,
    priority,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
SELECT ns,q,md5(k::text)::uuid,'fixture',1,jsonb_build_object('k',k),'{}','[]',state,
       0,2,60000,1000,1000,0,s
FROM (VALUES ('n','a',1,'scheduled',1::bigint), ('n','a',2,'retryable',2),
             ('n','b',3,'scheduled',3), ('z','a',4,'scheduled',4),
             ('n','a',5,'scheduled',4102444800000), ('n','a',6,'suspended',0)) v(ns,q,k,state,s);
"""
READ = """SELECT json_object_agg(args->>'k',state ORDER BY args->>'k') FROM mesh_jobs.jobs"""


@contextmanager
def lock_first(command):
    with subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as session:
        try:
            session.stdin.write("BEGIN; SELECT args->>'k' FROM mesh_jobs.jobs WHERE id=md5('1')::uuid FOR UPDATE;\n")
            session.stdin.flush()
            assert session.stdout.readline().strip() == "1"
            yield
        finally:
            session.stdin.write("ROLLBACK;\n")
            session.stdin.close()
            assert session.wait(timeout=10) == 0


def guard_plan(psql, candidate):
    start = candidate.index("WITH due AS")
    end = candidate.index(";", start)
    statement = candidate[start:end].replace("= ns", "= $1").replace("<= cutoff", "<= $2")
    statement = statement.replace("queue_filter", "$3").replace("LIMIT page_size", "LIMIT $4")
    plan, = json.loads(psql("SET plan_cache_mode=force_generic_plan; "
                            "PREPARE recovery(text,bigint,jsonb,integer) AS " + statement +
                            "; EXPLAIN (COSTS OFF, FORMAT JSON) EXECUTE recovery('n', 1, NULL, 1);"))

    def walk(node, filters):
        if node.get("Parent Relationship") in ("InitPlan", "SubPlan"):
            filters = []
        if "One-Time Filter" in node:
            filters = [*filters, node["One-Time Filter"]]
        if node["Node Type"].endswith("Scan") and node.get("Relation Name") == "jobs" and node.get("Alias") == "j":
            yield filters
        for child in node.get("Plans", []):
            yield from walk(child, filters)

    paths = list(walk(plan["Plan"], []))
    assert paths and all(any("InitPlan" in f for f in filters) for filters in paths), paths
    return paths


def prove(bindir):
    def source(name):
        return subprocess.check_output(["git", "show", f"{REV}:compiler/mesh-rt/src/jobs/{name}"],
                                       cwd=ROOT, text=True)

    schema = source("schema.sql")
    bulk = source("enqueue_schema.sql")
    start = schema.index("CREATE OR REPLACE FUNCTION mesh_jobs.recover(")
    stock = schema[start:schema.index("\n$$;", start) + 4] + "\n"
    candidate = (HERE / "candidate.sql").read_text()
    assert sha256(stock.encode()).hexdigest() == STOCK
    assert sha256(candidate.encode()).hexdigest() == CANDIDATE
    assert candidate.replace("\n        AND EXISTS (SELECT 1 FROM due);", ";") == stock
    result = {}
    (ROOT / "target").mkdir(exist_ok=True)
    with TemporaryDirectory(dir=ROOT / "target", prefix="er-") as tmp:
        scratch = Path(tmp)
        data = scratch / "db"
        def run(args, **kwargs):
            return subprocess.run(args, text=True, capture_output=True, check=True, **kwargs).stdout.strip()

        run([bindir / "initdb", "-D", data, "-U", "bench_owner", "-A", "trust", "--no-locale", "-E", "UTF8"])
        ctl = [bindir / "pg_ctl", "-D", data, "-w"]
        run(ctl + ["-l", scratch / "pg.log", "-o", f"-c listen_addresses='' -k {scratch}", "start"])
        try:
            base = [bindir / "psql", "-XqAt", "-v", "ON_ERROR_STOP=1", "-U", "bench_owner", "-h", scratch, "-d"]
            for name in ("stock", "candidate"):
                run(base + ["postgres"], input=f"CREATE DATABASE {name}")
                command = base + [name]
                def psql(statement):
                    return run(command, input=statement)
                assert psql("SHOW server_version_num") == "170011"
                psql(schema + bulk)
                if name == "candidate":
                    psql(candidate)
                assert psql("SELECT mesh_jobs.enqueue_batch_layout_v1()") == "t"
                psql(SEED)
                before = json.loads(psql(READ))
                if name == "candidate":
                    filters = guard_plan(psql, candidate)
                with lock_first(command):
                    assert psql("SET lock_timeout='2s'; " + DUE) == "0"
                    after_skip = json.loads(psql(READ))
                    assert after_skip == {**before, "2": "available"}
                rolled = psql("BEGIN; " + DUE + "; " + READ + "; ROLLBACK").splitlines()
                assert rolled[0] == "0" and json.loads(rolled[1]) == {**after_skip, "1": "available"}
                assert json.loads(psql(READ)) == after_skip
                assert psql(DUE) == "0"
                # A statement-level trigger must fire even when recovery promotes zero rows.
                psql("""CREATE TABLE public.observed(n bigint);
                    CREATE FUNCTION public.observe_due() RETURNS trigger LANGUAGE plpgsql AS $$
                    BEGIN INSERT INTO public.observed VALUES ((SELECT count(*) FROM promoted)); RETURN NULL; END $$;
                    CREATE TRIGGER observe_due AFTER UPDATE ON mesh_jobs.jobs
                    REFERENCING NEW TABLE AS promoted FOR EACH STATEMENT EXECUTE FUNCTION public.observe_due();""")
                assert psql(DUE) == "0" and psql("SELECT n FROM observed") == "0"
                assert psql("SELECT mesh_jobs.recover('n',1,NULL)") == "0"
                assert psql("SELECT sum(n) FROM observed") == "1"
                final = json.loads(psql(READ))
                assert final == {**after_skip, "1": "available", "3": "available"}
                psql("""DROP TRIGGER observe_due ON mesh_jobs.jobs;
                    UPDATE mesh_jobs.jobs SET state='executing',attempt=2,lease_token=md5('token')::uuid,
                        lease_expires_at_ms=1 WHERE namespace='n' AND id=md5('1')::uuid;""")
                assert psql(DUE) == "1" and json.loads(psql(READ)) == {**final, "1":"discarded"}
                assert psql("SELECT count(*) FROM mesh_jobs.jobs WHERE lease_token IS NOT NULL") == "0"
                result[name] = dict(after_locked_head=after_skip, after_unfiltered=final,
                                    recovered_expired=1, empty_statement_trigger_rows=0,
                                    bulk_layout=True, outer_rollback=True)
            assert result["stock"] == result["candidate"]
        finally:
            run(ctl + ["-m", "fast", "stop"])
    return dict(postgres="17.11", stock_sha256=STOCK, candidate_sha256=CANDIDATE,
                generic_target_filters=filters, checks=result)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bindir", type=Path)
    print(json.dumps(prove(parser.parse_args().bindir.resolve()), indent=2, sort_keys=True))
