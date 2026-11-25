"""Untimed PG17.11 TID and WAL proof on two groups of 16 canonical dense rows."""
import argparse
import gzip
from hashlib import sha256
import json
from pathlib import Path
import re
import subprocess
import sys
from tempfile import TemporaryDirectory

from layout import GUARD, LANES, guard_sql, install_sql
from wal import reduce

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
REVISION = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
BATCH = HERE.parent / "2026-11-24-public-dense-set-claim/evidence/batch_000000.json.gz"
SNAPSHOT = """
SELECT json_object_agg(j.args->>'key',
    json_build_object('tid', j.ctid::text, 'tuple_bytes', p.lp_len,
        'args_bytes', octet_length(p.t_attrs[8]),
        'args_end', position(p.t_attrs[8] IN p.t_data) + octet_length(p.t_attrs[8]) - 1))
FROM mesh_jobs.jobs j CROSS JOIN LATERAL (
    SELECT h.*, tuple_data_split('mesh_jobs.jobs'::regclass, h.t_data, h.t_infomask,
                                 h.t_infomask2, h.t_bits, false) AS t_attrs
    FROM heap_page_items(get_raw_page('mesh_jobs.jobs', (j.ctid::text::point)[0]::int)) h
    WHERE h.lp = (j.ctid::text::point)[1]::int
) p WHERE p.lp = (j.ctid::text::point)[1]::int AND j.position BETWEEN {first} AND {last}
"""
WAL = """
SELECT json_agg(row_to_json(w) ORDER BY start_lsn, block_id) FROM (
    SELECT start_lsn, block_id, relblocknumber AS block, record_type, record_length,
        block_fpi_length AS fpi, block_data_length, description,
        CASE WHEN description ~ 'flags: 0x[2367][0-9A-F]'
             THEN encode(substring(block_data for 4), 'hex') END AS data_head
    FROM pg_get_wal_block_info('{start}', '{stop}', true)
    WHERE reldatabase = (SELECT oid FROM pg_database WHERE datname = current_database())
      AND relfilenode = pg_relation_filenode('mesh_jobs.jobs')
      AND relforknumber = 0 AND resource_manager = 'Heap'
      AND record_type IN ('UPDATE', 'UPDATE+INIT', 'HOT_UPDATE')
) w
"""


def run(command, **kwargs):
    completed = subprocess.run(command, text=True, capture_output=True, **kwargs)
    if completed.returncode:
        print(completed.stderr, file=sys.stderr, end="")
        raise subprocess.CalledProcessError(completed.returncode, command)
    return completed.stdout.strip()


def frozen_sql(name):
    return run(["git", "-C", ROOT, "show", f"{REVISION}:compiler/mesh-rt/src/jobs/{name}"]) + "\n"


def inspect_wal(query, start, stop, previous, current):
    blocks = {}
    for row in json.loads(query(WAL.format(start=start, stop=stop))):
        blocks.setdefault(row["start_lsn"], []).append(row)
    transitions = {}
    for records in blocks.values():
        new = records[0]
        old = records[-1]
        assert [r["block_id"] for r in records] in ([0], [0, 1])
        parsed = re.search(r"old_off: (\d+), .*flags: 0x([0-9A-F]+), .*new_off: (\d+)", new["description"])
        assert parsed, new
        old_off, flags, new_off = parsed.groups()
        flags = int(flags, 16)
        words = bytes.fromhex(new["data_head"] or "")
        prefix = int.from_bytes(words[:2], sys.byteorder) if flags & 0x20 else 0
        suffix_at = 2 if flags & 0x20 else 0
        suffix = int.from_bytes(words[suffix_at:suffix_at + 2], sys.byteorder) if flags & 0x40 else 0
        same = new["block"] == old["block"]
        assert (not flags & 0x20 or prefix >= 3) and (not flags & 0x40 or suffix >= 3)
        assert same or not (prefix or suffix)
        assert not (prefix or suffix) or (new["fpi"] == 0 and new["block_data_length"] > 0)
        tid = f"({new['block']},{new_off})"
        assert tid not in transitions
        transitions[tid] = {"old_tid": f"({old['block']},{old_off})", "same_page": same,
                            "prefix": prefix, "suffix": suffix, "wal": records}
    assert len(transitions) == len(previous) == len(current) == 16
    result = {}
    for key in sorted(previous):
        transition = transitions[current[key]["tid"]]
        assert transition["old_tid"] == previous[key]["tid"], (key, transition)
        assert previous[key]["args_bytes"] == current[key]["args_bytes"] == 1110
        result[key] = transition
    return result


def batch_sql(jobs):
    specs = [dict(worker="parity", version=1, queue="default", args=j, metadata={}, tags=[],
                  unique=None, replace={}, priority=0, max_attempts=3, timeout_ms=60000,
                  backoff_base_ms=1000, backoff_max_ms=1000) for j in jobs]
    batch = json.dumps([dict(spec=s, request_key=j["key"], fingerprint=sha256(
        json.dumps(s, sort_keys=True).encode()).hexdigest()) for j, s in zip(jobs, specs)])
    tag = "$batch$"
    assert tag not in batch
    return f"mesh_jobs.enqueue_batch_v1('parity', {tag}{batch}{tag}::jsonb)"


def negative_controls(query, jobs, lane):
    mutations = {
        "other_fillfactor": "ALTER TABLE mesh_jobs.jobs SET (fillfactor=51)",
        "jobs_other_option": "ALTER TABLE mesh_jobs.jobs SET (toast_tuple_target=512)",
        "toast_option": "ALTER TABLE mesh_jobs.jobs SET (toast.autovacuum_enabled=false)",
        "queues_fillfactor": "ALTER TABLE mesh_jobs.queues SET (fillfactor=50)",
        "column": "ALTER TABLE mesh_jobs.jobs ALTER COLUMN priority SET DEFAULT 0",
        "storage": "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL",
        "compression": "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET COMPRESSION pglz",
        "index": "CREATE INDEX unexpected_index ON mesh_jobs.jobs (attempt)",
        "index_option": "ALTER INDEX mesh_jobs.mesh_jobs_ready SET (fillfactor=50)",
        "trigger": "ALTER TABLE mesh_jobs.jobs DISABLE TRIGGER USER",
    }
    result = {}
    for label, ddl in mutations.items():
        observed = query(f"""
            BEGIN; SET LOCAL ROLE parity_migrator; {ddl};
            SET LOCAL ROLE parity_producer;
            SELECT {GUARD};
            SELECT ordinal,receipt FROM {batch_sql(jobs)};
            SELECT (SELECT count(*) FROM mesh_jobs.jobs),
                   (SELECT count(*) FROM mesh_jobs.receipts),
                   (SELECT count(*) FROM mesh_jobs.queues);
            ROLLBACK;
        """)
        assert observed == "f\n0|stock\n0|0|0", (lane, label, observed)
        result[label] = observed.splitlines()
    return result


def turnover(query, dump, jobs, first):
    observe = SNAPSHOT.format(first=first, last=first + 15)
    enqueue_start = query("SELECT pg_current_wal_insert_lsn()")
    enqueue = query(f"""BEGIN; SET LOCAL ROLE parity_producer;
        INSERT INTO mesh_jobs.admissions(namespace) VALUES ('parity') ON CONFLICT DO NOTHING;
        SELECT namespace FROM mesh_jobs.admissions WHERE namespace='parity' FOR NO KEY UPDATE;
        SELECT json_agg(ordinal::int ORDER BY ordinal::int) FROM {batch_sql(jobs)};
        COMMIT;""")
    enqueue_end = query("SELECT pg_current_wal_insert_lsn()")
    enqueue_wal = dump(enqueue_start, enqueue_end)
    assert reduce(enqueue_wal)["inserts"] == 16
    assert enqueue.splitlines()[0] == "parity" and json.loads(enqueue.splitlines()[1]) == list(range(1, 17))
    query("UPDATE mesh_jobs.queues SET concurrency=16 WHERE namespace='parity';")
    initial = json.loads(query(observe))
    available_heap_bytes = int(query("SELECT pg_relation_size('mesh_jobs.jobs')"))
    start = query("SELECT pg_current_wal_insert_lsn()")
    claimed = json.loads(query("""SET ROLE parity_subject;
        SELECT json_agg(json_build_array(status, id, token, context::jsonb))
        FROM mesh_jobs.claim_batch_set_v1('parity','default','[{"worker":"parity","version":1}]',60000,16,NULL);
    """))
    claim_end = query("SELECT pg_current_wal_insert_lsn()")
    assert [row[3]["args"] for row in claimed] == jobs
    assert all(row[0] == "claimed" and row[3]["attempt"] == 1 for row in claimed)
    executing = json.loads(query(observe))

    # Two success-cohort transactions as supported by b3557273.
    settle_start = query("SELECT pg_current_wal_insert_lsn()")
    wrong_token = query(f"""SET ROLE parity_subject;
        SELECT count(*) FROM mesh_jobs.complete_batch_v1('parity',
            ARRAY['{claimed[0][1]}']::uuid[], ARRAY['00000000-0000-0000-0000-000000000000']::uuid[])
    """)
    assert wrong_token == "0"
    for index in (0, 8):
        group = claimed[index:index + 8]
        ids = ",".join(f"'{r[1]}'" for r in group)
        tokens = ",".join(f"'{r[2]}'" for r in group)
        ordinals = json.loads(query(f"""SET ROLE parity_subject;
            SELECT json_agg(ordinal ORDER BY ordinal)
            FROM mesh_jobs.complete_batch_v1('parity', ARRAY[{ids}]::uuid[], ARRAY[{tokens}]::uuid[]);
        """))
        assert ordinals == list(range(1, 9))
    settle_end = query("SELECT pg_current_wal_insert_lsn()")
    completed = json.loads(query(observe))
    assert json.loads(query(f"""SELECT json_agg(args ORDER BY position) FROM mesh_jobs.jobs
        WHERE state='completed' AND lease_token IS NULL AND lease_expires_at_ms IS NULL
        AND attempt=1 AND position BETWEEN {first} AND {first + 15}""")) == jobs
    wal = {}
    for phase, lo, hi, before, after in (
        ("claim", start, claim_end, initial, executing),
        ("complete", settle_start, settle_end, executing, completed),
    ):
        records = inspect_wal(query, lo, hi, before, after)
        raw = dump(lo, hi)
        totals = reduce(raw)
        assert totals["updates"] == 16
        assert totals["same_page"] == sum(r["same_page"] for r in records.values())
        assert totals["prefix"] == sum(bool(r["prefix"]) for r in records.values())
        wal[phase] = records
        wal[phase + "_waldump"] = raw.splitlines()

    return {
        "wrong_token_settled": int(wrong_token),
        "snapshots": [initial, executing, completed],
        **wal,
        "available_heap_bytes": available_heap_bytes,
        "enqueue_waldump": enqueue_wal.splitlines(),
        "heap_bytes": int(query("SELECT pg_relation_size('mesh_jobs.jobs')")),
        "total_relation_bytes": int(query("SELECT pg_total_relation_size('mesh_jobs.jobs')")),
    }


def inspect(bindir):
    batch = gzip.decompress(BATCH.read_bytes())
    assert sha256(sha256(batch).hexdigest().encode()).hexdigest() == INPUT
    jobs = json.loads(batch)["jobs"][:32]
    assert all(len(j["payload"]) == 1024 for j in jobs)
    schema = frozen_sql("schema.sql")
    enqueue = frozen_sql("enqueue_schema.sql")
    assert sha256(schema.encode()).hexdigest() == "391b3bff16ef6894fe2030b2d2bbfcc767a2c0456e99e520dcb34e0431a56937"
    roles = ("producer", "subject", "operator", "inspector")
    grants = run(["git", "-C", ROOT, "show", f"{REVISION}:scripts/fixtures/jobs-role-grants.sql"]).split("-- Optional producer fast path.")[0]
    for role in roles:
        grants = grants.replace(':"worker"' if role == "subject" else f':"{role}"', f"parity_{role}")

    with TemporaryDirectory(dir=ROOT / "target", prefix="ff-") as tmp:
        scratch = Path(tmp)
        data = scratch / "pg"

        def tool(name, *args, **kwargs):
            return run([bindir / name, *args], **kwargs)

        def query(sql, db="mesh"):
            return tool("psql", "-XqAt", "-v", "ON_ERROR_STOP=1", "-h", scratch,
                        "-U", "owner", "-d", db, input=sql)

        tool("initdb", "-D", data, "-A", "trust", "--no-locale", "-E", "UTF8", "-U", "owner")
        tool("pg_ctl", "-D", data, "-l", scratch / "pg.log", "-w", "-o",
             f"-c listen_addresses='' -c unix_socket_directories={scratch}", "start")
        try:
            assert query("SHOW server_version_num", "postgres") == "170011"
            assert query("SHOW wal_level; SHOW full_page_writes", "postgres") == "replica\non"
            counters = query("""SELECT count(*) FROM pg_attribute
                WHERE attrelid='pg_stat_user_tables'::regclass
                AND attname IN ('n_tup_ins','n_tup_upd','n_tup_newpage_upd','n_tup_hot_upd')""", "postgres")
            assert counters == "4"
            query("".join(f"CREATE ROLE parity_{role};" for role in (*roles, "migrator")), "postgres")
            arms = {}
            for lane in LANES:
                query("CREATE DATABASE mesh OWNER parity_migrator", "postgres")
                query("SET ROLE parity_migrator;\n" + schema + enqueue + "".join(
                    frozen_sql(name) for name in ("drain_schema.sql", "cron_schema.sql", "effect_fence_schema.sql")))
                query("CREATE EXTENSION pageinspect; CREATE EXTENSION pg_walinspect;")
                query("SET ROLE parity_migrator;\n" + grants + f"""
                      GRANT EXECUTE ON FUNCTION {GUARD}, mesh_jobs.enqueue_batch_v1(text,jsonb) TO parity_producer;
                      GRANT EXECUTE ON FUNCTION mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint),
                           mesh_jobs.complete_batch_v1(text,uuid[],uuid[]) TO parity_subject;
                      """)
                old_acl = query(f"SELECT proacl FROM pg_proc WHERE oid='{GUARD}'::regprocedure")
                query(install_sql(enqueue, lane))
                assert query(f"SELECT proacl FROM pg_proc WHERE oid='{GUARD}'::regprocedure") == old_acl
                if lane == "fill50":
                    assert query(f"BEGIN; SET LOCAL ROLE parity_migrator; {enqueue} SELECT {GUARD}; ROLLBACK;") == "f"
                negatives = negative_controls(query, jobs[:16], lane)
                relation = query("""SELECT dattablespace || '/' || oid || '/' ||
                    pg_relation_filenode('mesh_jobs.jobs') FROM pg_database WHERE datname=current_database()""")
                def dump(start, stop):
                    return tool("pg_waldump", "-p", data / "pg_wal", "-r", "Heap", "-R", relation,
                                "-s", start, "-e", stop)

                cohorts = [turnover(query, dump, jobs[:16], 1), turnover(query, dump, jobs[16:], 17)]
                arms[lane] = {"rejections": negatives, "cohorts": cohorts}
                query("DROP DATABASE mesh", "postgres")
            return {"postgres": "17.11", "wal_level": "replica", "full_page_writes": True,
                    "revision": REVISION, "input_sha256": INPUT, "byteorder": sys.byteorder,
                    "guard_sha256": sha256(guard_sql(enqueue).encode()).hexdigest(), "arms": arms}
        finally:
            tool("pg_ctl", "-D", data, "-m", "fast", "-w", "stop")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bindir", type=Path)
    print(json.dumps(inspect(parser.parse_args().bindir.resolve()), indent=2))
