"""Capture the physical payload policy and persisted values outside the workload."""

import json
from pathlib import Path
import re
import time

HERE = Path(__file__).resolve().parent
TABLES = {
    "oban": ("oban_jobs", "args", "jsonb", "state", "completed"),
    "go": ("river_job", "args", "jsonb", "state", "completed"),
    "rust": ("jobs", "job", "bytea", "status", "Done"),
}


def write(path, document):
    path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n")


def qualified_table(engine, schema):
    expected = "apalis" if engine == "rust" else r"parity_[0-9a-f]{16}"
    if not re.fullmatch(expected, schema):
        raise ValueError(f"unexpected {engine} schema")
    return f"{schema}.{TABLES[engine][0]}"


def metadata(relation, column):
    return f"""
        SELECT jsonb_build_object(
            'relation', '{relation}', 'relation_oid', c.oid, 'toast_oid', c.reltoastrelid,
            'owner', pg_get_userbyid(c.relowner), 'persistence', c.relpersistence,
            'column', a.attname, 'type', a.atttypid::regtype::text,
            'attstorage', a.attstorage, 'attcompression', a.attcompression)
        FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
        WHERE c.oid = '{relation}'::regclass AND a.attname = '{column}' AND NOT a.attisdropped
    """


def snapshot(engine, relation):
    _, column, kind, state, _ = TABLES[engine]
    args = column if kind == "jsonb" else f"convert_from({column}, 'UTF8')::jsonb"
    return f"""
        SELECT jsonb_build_object(
            'metadata', ({metadata(relation, column)}),
            'toast_heap_bytes', pg_relation_size(c.reltoastrelid),
            'trigger_absent', NOT EXISTS (SELECT FROM pg_event_trigger
                WHERE evtname = 'bench_payload_storage_create'),
            'rows', coalesce((
                SELECT jsonb_agg(to_jsonb(w) ORDER BY w.job_key) FROM (
                    SELECT id::text, ({args})->>'key' AS job_key,
                           octet_length(({args})->>'payload') AS payload_bytes,
                           encode(sha256(convert_to(({args})->>'payload', 'UTF8')), 'hex') AS payload_sha256,
                           pg_column_size({column}) AS stored_column_bytes,
                           pg_column_compression({column}) AS compression,
                           {state}::text AS native_state, xmin::text AS settled_xid
                    FROM {relation}
                ) w
            ), '[]'::jsonb))
        FROM pg_class c WHERE c.oid = '{relation}'::regclass
    """


def apply(cluster, engine, database, schema):
    relation = qualified_table(engine, schema)
    column = TABLES[engine][1]
    query = metadata(relation, column)
    ddl = f"ALTER TABLE {relation} ALTER COLUMN {column} SET STORAGE EXTERNAL"
    rows = int(cluster.sql(database, f"SELECT count(*) FROM {relation}"))
    before = json.loads(cluster.sql(database, query))
    if rows != 0 or before["attstorage"] != "x" or before["type"] != TABLES[engine][2]:
        raise ValueError(f"{relation}: expected an empty EXTENDED payload column")
    started = time.time_ns() // 1000
    cluster.sql(database, f"SET ROLE parity_subject; {ddl}")
    finished = time.time_ns() // 1000
    after = json.loads(cluster.sql(database, query))
    if after != dict(before, attstorage="e"):
        raise ValueError(f"{relation}: STORAGE EXTERNAL did not take effect")
    return dict(before=before, after=after, rows_before=rows, ddl=ddl,
                ddl_start_us=started, ddl_end_us=finished)


def finish(cluster, engine, database, setup, out):
    started = time.time_ns() // 1000
    persisted = json.loads(cluster.sql(database, snapshot(engine, setup["after"]["relation"])))
    write(out / "payload-storage.json", dict(engine=engine, policy="EXTERNAL", setup=setup, persisted=persisted,
                                             verify_start_us=started, verify_end_us=time.time_ns() // 1000))


def install_river(cluster):
    # Pass one SQL literal to format(). The only interpolated identifier is the
    # validated schema, even when the snapshot SQL names it several times.
    query = snapshot("go", "river_schema.river_job").replace("river_schema", "%1$I")
    sql = (HERE / "river.sql").read_text().replace("SNAPSHOT_QUERY", query)
    started = time.time_ns() // 1000
    cluster.sql("river", sql)
    return dict(install_start_us=started, install_end_us=time.time_ns() // 1000)


def finish_river(cluster, install, out):
    receipt = json.loads(cluster.sql("river", """
        SELECT row_to_json(w) FROM (
            SELECT setup, persisted, verify_start_us, verify_end_us,
                   NOT EXISTS (SELECT FROM pg_event_trigger WHERE evtname IN
                       ('bench_payload_storage_create', 'bench_payload_storage_drop')) AS triggers_retired
            FROM bench_payload_storage.witness
        ) w
    """))
    write(out / "payload-storage.json", dict(engine="go", policy="EXTERNAL", **install, **receipt))
