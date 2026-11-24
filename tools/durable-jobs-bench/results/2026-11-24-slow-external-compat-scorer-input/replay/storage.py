"""Observe physical PostgreSQL payload storage outside the timed workload."""
import hashlib
import json
import re

TABLES = {
    "mesh": ("mesh_jobs.jobs", "args", "jsonb", "state", "completed"),
    "oban": ("{schema}.oban_jobs", "args", "jsonb", "state", "completed"),
    "go": ("{schema}.river_job", "args", "jsonb", "state", "completed"),
    "rust": ("apalis.jobs", "job", "bytea", "status", "Done"),
}


def write(path, obj):
    temporary = path.with_suffix(path.suffix + ".partial")
    temporary.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def metadata(relation, column):
    return f"""
        SELECT jsonb_build_object(
            'relation', '{relation}', 'relation_oid', c.oid, 'toast_oid', c.reltoastrelid,
            'persistence', c.relpersistence, 'column', a.attname,
            'type', a.atttypid::regtype::text, 'attstorage', a.attstorage,
            'attcompression', CASE a.attcompression WHEN 'p' THEN 'pglz'
                              WHEN 'l' THEN 'lz4' ELSE 'default' END,
            'default_toast_compression', current_setting('default_toast_compression'))
        FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
        WHERE c.oid = '{relation}'::regclass AND a.attname = '{column}' AND NOT a.attisdropped
    """


def snapshot(engine, schema=""):
    if engine in ("go", "oban") and not re.fullmatch(r"parity_[0-9a-f]{16}|river_schema", schema):
        raise ValueError(f"unexpected {engine} schema {schema!r}")
    relation, column, kind, state, _ = TABLES[engine]
    relation = relation.format(schema=schema)
    args = column if kind == "jsonb" else f"convert_from({column}, 'UTF8')::jsonb"
    return f"""
        SELECT jsonb_build_object(
            'metadata', ({metadata(relation, column)}),
            'heap_bytes', pg_relation_size(c.oid),
            'toast_heap_bytes', pg_relation_size(c.reltoastrelid),
            'total_bytes', pg_total_relation_size(c.oid),
            'rows', (SELECT coalesce(jsonb_agg(to_jsonb(w) ORDER BY w.job_key), '[]'::jsonb)
                FROM (SELECT id::text, ({args})->>'key' AS job_key,
                    octet_length(({args})->>'payload') AS payload_bytes,
                    encode(sha256(convert_to(({args})->>'payload','UTF8')), 'hex') AS payload_sha256,
                    ({args}) - 'payload' AS other_arguments,
                    pg_column_size({column}) AS stored_column_bytes,
                    pg_column_compression({column}) AS compression,
                    {state}::text AS native_state, xmin::text AS settled_xid
                FROM {relation}) w))
        FROM pg_class c WHERE c.oid = '{relation}'::regclass
    """


def verify_snapshot(engine, persisted, jobs, policy, column_policy="default"):
    _, _, kind, _, terminal = TABLES[engine]
    meta = persisted["metadata"]
    if (meta["default_toast_compression"], meta["attstorage"], meta["attcompression"], meta["type"]
            ) != (policy, "e" if column_policy == "external" else "x",
                  "lz4" if column_policy == "column-lz4" else "default", kind):
        raise ValueError(f"{engine}: wrong payload metadata {meta}")
    rows = persisted["rows"]
    jobs_by_key = {row["job_key"]: row for row in jobs}
    if len(rows) != 512 or len(jobs_by_key) != 512:
        raise ValueError(f"{engine}: incomplete payload and effect ledger")
    if {row["job_key"] for row in rows} != jobs_by_key.keys():
        raise ValueError(f"{engine}: payload and effect ledger keys differ")
    permitted_compression = None if column_policy == "external" else (
        "lz4" if column_policy == "column-lz4" else policy
    )
    for row in rows:
        job = jobs_by_key[row["job_key"]]
        expected_args = dict(key=job["job_key"], hold_ms=job["hold_ms"], fail_first=job["fail_first"])
        want = (job["id"], 16384, job["payload_sha256"], terminal, job["settled_xid"], expected_args)
        got = tuple(row[name] for name in ("id", "payload_bytes", "payload_sha256",
                    "native_state", "settled_xid", "other_arguments"))
        if got != want or row["compression"] not in (None, permitted_compression):
            raise ValueError(f"{engine}: payload or terminal receipt differs for {row['job_key']}")
