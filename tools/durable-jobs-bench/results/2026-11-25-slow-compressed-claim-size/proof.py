"""Compare the public set-based and checked-cursor grant routes on PG 17.11."""
import gzip
import hashlib
import json
from pathlib import Path
import platform
import shlex
import subprocess
import tempfile
import uuid

import layout_race

HERE = Path(__file__).resolve().parent
SOURCE = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
FIXTURE = HERE.parent / "2026-11-23-slow-storage-transfer/frozen/canonical-batch.json.gz"
SET_CLAIM = "mesh_jobs.claim_batch_set_v1"
CURSOR = "mesh_jobs.claim_batch_v1"
RAW = "mesh_jobs.proof_claim_raw_size"
POST_LOCK = "mesh_jobs.proof_claim_postlock"
ARGS = """('n','q','[{"worker":"work","version":1}]',30000,16,1)"""


def output(command, **kwargs):
    return subprocess.check_output(command, text=True, **kwargs).strip()


def config(flag):
    return output(["pg_config", flag])


def candidate(schema):
    start = schema.index("CREATE OR REPLACE FUNCTION " + SET_CLAIM)
    end = schema.index("END\n$$;", start) + len("END\n$$;")
    body = schema[start:end].replace(SET_CLAIM, RAW, 1)
    first = body.index("pg_column_size(p.args)")
    last = body.index("AND octet_length(p.worker)", first)
    original = body[first:last].strip()
    size_check = f"""(({original})
                 OR ((pg_column_compression(p.args) IS NOT NULL
                    OR pg_column_compression(p.metadata) IS NOT NULL
                    OR pg_column_compression(p.tags) IS NOT NULL)
                    AND public.proof_raw_size(p.args) + public.proof_raw_size(p.metadata)
                        + public.proof_raw_size(p.tags) <= 65536))
                    """
    return body[:first] + size_check + body[last:]


def post_lock_candidate(schema):
    # The scalar subquery is shared by newly eligible compressed rows in
    # this render command. This command runs after the queue lock returns.
    sql = candidate(schema).replace(RAW, POST_LOCK, 1)
    anchor = "AND public.proof_raw_size(p.args)"
    assert sql.count(anchor) == 1
    guard = """AND (SELECT CASE
           WHEN has_function_privilege(current_user, 'mesh_jobs.effect_key(mesh_jobs.jobs)', 'execute')
           AND has_function_privilege(current_user, 'mesh_jobs.enqueue_batch_layout_v1()', 'execute')
           THEN mesh_jobs.enqueue_batch_layout_v1() ELSE false END)
           """
    return sql.replace(anchor, guard + anchor)


def check_cluster(sql, module, schema):
    sql(schema)
    sql(f"CREATE FUNCTION public.proof_raw_size(jsonb) RETURNS bigint "
        f"AS '{module}', 'mesh_jsonb_raw_size' LANGUAGE C IMMUTABLE STRICT;")
    sql(output(["git", "show", f"{SOURCE}:compiler/mesh-rt/src/jobs/enqueue_schema.sql"]))
    sql(candidate(schema))
    assert sql("SELECT mesh_jobs.enqueue_batch_layout_v1()") == "t"
    guarded = "mesh_jobs.proof_claim_known_layout"
    schema_check = """BEGIN
    IF NOT has_function_privilege(current_user, 'mesh_jobs.effect_key(mesh_jobs.jobs)', 'execute')
        OR NOT mesh_jobs.enqueue_batch_layout_v1() THEN
        RETURN QUERY SELECT * FROM mesh_jobs.claim_batch_set_v1(
            ns, queue_name, workers, lease_ms, slots, cutoff_ms);
        RETURN;
    END IF;"""
    guarded_sql = candidate(schema).replace(RAW, guarded, 1).replace("BEGIN", schema_check, 1)
    sql(guarded_sql)
    sql(post_lock_candidate(schema))
    sql("INSERT INTO mesh_jobs.queues(namespace,name,concurrency) VALUES ('n','q',64)")
    batch = json.loads(gzip.decompress(FIXTURE.read_bytes()))
    canonical = (json.dumps(batch, sort_keys=True, indent=2) + "\n").encode()
    digest = hashlib.sha256(canonical).hexdigest()
    assert digest == "d9c91f63e1962cbd1a6c92c0a5f9154d2bec171058e38b997a8987b73ea86f8d"
    assert len(batch["jobs"]) == 512
    rows = json.dumps(batch["jobs"], separators=(",", ":"))
    assert "$fixture$" not in rows
    data_sql = "$fixture$" + rows + "$fixture$"
    seed = f"""INSERT INTO mesh_jobs.jobs(namespace,id,worker,version,queue,args,metadata,tags,
            state,priority,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
        SELECT 'n',md5(spec->>'key')::uuid,'work',1,'q',spec,'{{}}','[]','available',0,2,30000,1,1,0,0
        FROM jsonb_array_elements({data_sql}::jsonb) WITH ORDINALITY j(spec,ord)
        ORDER BY ord"""
    layouts = {}
    for policy in ("pglz", "lz4"):
        sql(f"DELETE FROM mesh_jobs.jobs; SET default_toast_compression='{policy}';" + seed)
        sizes = json.loads(sql("""SELECT json_build_object('total',count(*),
             'compressed',count(pg_column_compression(args)), 'stored_bytes',sum(pg_column_size(args)),
             'min_raw',min(public.proof_raw_size(args)), 'max_raw',max(public.proof_raw_size(args)))
             FROM mesh_jobs.jobs"""))
        assert sizes["total"] == 512 and sizes["compressed"] == (512 if policy == "lz4" else 0)

        def collect(fn, check_fallback=False):
            # Retire each batch in the disposable transaction to read the next sixteen.
            read = f"""SELECT json_agg(json_build_array(status,id,context_bytes,
                    encode(sha256(convert_to(context,'UTF8')),'hex'))) FROM {fn}{ARGS};"""
            retire = """UPDATE mesh_jobs.jobs SET state='completed',finished_at_ms=2,
                            lease_token=NULL,lease_expires_at_ms=NULL WHERE state='executing';"""
            marker = f"SELECT status FROM {SET_CLAIM}{ARGS};" if check_fallback else ""
            lines = sql("SET track_functions=pl; BEGIN;" + (marker + read + retire)*32 + "ROLLBACK;").splitlines()
            if check_fallback:
                assert lines[::2] == ["fallback"] * 32
                lines = lines[1::2]
            return [json.loads(line) for line in lines]

        stock_status = sql(f"BEGIN; SELECT DISTINCT status FROM {SET_CLAIM}{ARGS}; ROLLBACK;")
        assert stock_status == ("fallback" if policy == "lz4" else "claimed")
        stock = collect(CURSOR if policy == "lz4" else SET_CLAIM, check_fallback=policy == "lz4")
        def guard_calls():
            return int(sql("SELECT coalesce(sum(calls),0) FROM pg_stat_user_functions "
                           "WHERE funcid='mesh_jobs.enqueue_batch_layout_v1()'::regprocedure"))

        before = guard_calls()
        fast = collect(POST_LOCK)
        gate_calls = guard_calls() - before
        assert gate_calls == (32 if policy == "lz4" else 0), gate_calls
        assert fast == stock and all(len(page) == 16 for page in fast)
        ids = [job[1] for page in fast for job in page if job[0] == "claimed"]
        assert ids == [str(uuid.UUID(hashlib.md5(j["key"].encode()).hexdigest())) for j in batch["jobs"]]
        manifest = hashlib.sha256(json.dumps(fast,separators=(",",":")).encode()).hexdigest()
        layouts[policy] = dict(sizes=sizes, stock_first_status=stock_status, matched_contexts=512, layout_calls_in_32_postlock_batches=gate_calls,
                               ordered_context_manifest=manifest)

    assert layouts["lz4"]["ordered_context_manifest"] == layouts["pglz"]["ordered_context_manifest"]
    large = json.loads(sql(f"""BEGIN; SET default_toast_compression='lz4';
        UPDATE mesh_jobs.jobs SET args=jsonb_build_object('payload',repeat('x',1048576))
        WHERE position=(SELECT min(position) FROM mesh_jobs.jobs);
        SELECT json_build_object('physical_bytes',pg_column_size(args),'raw_bytes',public.proof_raw_size(args),
          'status',(SELECT status FROM {RAW}{ARGS} LIMIT 1)) FROM mesh_jobs.jobs ORDER BY position LIMIT 1;
        ROLLBACK;"""))
    assert large["physical_bytes"] < 65536 < large["raw_bytes"] and large["status"] == "fallback"
    custom, fast_key, guarded_status = sql(f"""BEGIN;
        CREATE OR REPLACE FUNCTION mesh_jobs.effect_key(j mesh_jobs.jobs) RETURNS text
        LANGUAGE SQL IMMUTABLE STRICT AS $$SELECT j.id::text || '/2'$$;
        SAVEPOINT initial;
        SELECT context::jsonb->>'effect_key' FROM {CURSOR}{ARGS} LIMIT 1;
        ROLLBACK TO initial;
        SELECT context::jsonb->>'effect_key' FROM {RAW}{ARGS} LIMIT 1;
        ROLLBACK TO initial;
        SELECT status FROM {guarded}{ARGS};
        ROLLBACK;""").splitlines()
    assert guarded_status == "fallback"
    assert custom.endswith("/2") and custom[:-2] + "/1" == fast_key
    denied = sql(f"""BEGIN;
        CREATE ROLE proof_worker;
        GRANT USAGE ON SCHEMA mesh_jobs TO proof_worker;
        GRANT SELECT, UPDATE ON ALL TABLES IN SCHEMA mesh_jobs TO proof_worker;
        GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA mesh_jobs TO proof_worker;
        REVOKE EXECUTE ON FUNCTION mesh_jobs.effect_key(mesh_jobs.jobs) FROM PUBLIC, proof_worker;
        SET LOCAL ROLE proof_worker;
        SELECT mesh_jobs.enqueue_batch_layout_v1(), status FROM {POST_LOCK}{ARGS};
        DO $$ BEGIN
            PERFORM * FROM {CURSOR}{ARGS};
            RAISE EXCEPTION 'cursor unexpectedly accepted revoked helper';
        EXCEPTION WHEN insufficient_privilege THEN
            IF SQLERRM NOT LIKE '%effect_key%' THEN RAISE; END IF;
        END $$;
        ROLLBACK;""")
    assert denied == 't|fallback'
    return dict(batch_sha256=digest, source=SOURCE, layouts=layouts, oversized=large,
                revoked_helper=dict(layout_matches=True, guarded_status="fallback", cursor_error="42501"),
                custom_effect_key=dict(cursor=custom, experimental=fast_key, guarded_status=guarded_status))


def main():
    assert "17.11" in config("--version")
    bindir = Path(config("--bindir"))
    schema = output(["git","show",f"{SOURCE}:compiler/mesh-rt/src/jobs/schema.sql"])
    with tempfile.TemporaryDirectory(prefix="slow-claim-size-") as tmp:
        work = Path(tmp)
        module = work / "proof.so"
        flags = ["-bundle","-undefined","dynamic_lookup"] if platform.system() == "Darwin" else ["-shared"]
        output([*shlex.split(config("--cc")), *shlex.split(config("--cppflags")), "-fPIC", *flags,"-I"+config("--includedir-server"),
                str(HERE/"raw_size.c"),"-o",str(module)])
        data = work/"pg"
        output([bindir/"initdb","-D",data,"--no-locale","-E","UTF8","--auth=trust"])
        sock = work/"s"
        sock.mkdir(mode=0o700)
        output([bindir/"pg_ctl","-D",data,"-l",work/"pg.log","-o",
                f"-k {sock} -c listen_addresses=''","-w","start"])
        try:
            command = [bindir/"psql", "-XqAt", "-v", "ON_ERROR_STOP=1",
                       "-h", sock, "-d", "postgres"]
            def sql(statement):
                return output(command, input=statement)
            result = check_cluster(sql, module, schema)
            stock = layout_race.run(command, SET_CLAIM, ARGS, CURSOR)
            guarded = layout_race.run(command, "mesh_jobs.proof_claim_known_layout", ARGS, CURSOR)
            assert stock["pending_claim_reply"] == "fallback"
            assert stock["subsequent_cursor_key"] == guarded["subsequent_cursor_key"]
            post_lock = layout_race.run(command, POST_LOCK, ARGS, CURSOR)
            assert post_lock == stock
            key = guarded["pending_claim_reply"]
            assert key.endswith("/1") and key[:-2] + "/2" == stock["subsequent_cursor_key"]
            result["concurrent_replacement"] = {"stock": stock, "prelock_guard": guarded, "postlock_guard": post_lock}
            print(json.dumps(result, sort_keys=True, indent=2))
        finally:
            output([bindir/"pg_ctl","-D",data,"-m","fast","-w","stop"])


if __name__ == "__main__":
    main()
