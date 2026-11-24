"""The isolated SQL screen and its off-clock physical readback."""
from hashlib import sha256
import json
import re

SET = "mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)"
CURSOR = "mesh_jobs.claim_batch_v1(text,text,jsonb,bigint,integer,bigint)"
MARKER = "RETURN QUERY SELECT 'fallback'::text, NULL::text, NULL::text, NULL::text, NULL::text, NULL::text;"


def definition(pg, fn):
    # JSON framing preserves newlines through the adapter's unaligned psql.
    return json.loads(pg(f"SELECT to_json(pg_get_functiondef('{fn}'::regprocedure));"))


def install_thin(pg, namespace):
    assert re.fullmatch(r"parity_[0-9a-f]{16}", namespace)
    old = definition(pg, SET)
    cursor = definition(pg, CURSOR)
    anchor = "    -- Serialize with single claims, cursor batches and queue controls."
    assert old.count(anchor) == 1 and MARKER in old
    # Only a fresh diagnostic database gets this extra branch. Keep the exact
    # runtime exchanges, SQL signature, ACL and original body for other roles.
    early = f"""    IF ns = '{namespace}' AND queue_name = 'default' AND current_user = 'parity_subject' THEN
        {MARKER}
        RETURN;
    END IF;

"""
    pg(old.replace(anchor, early + anchor) + ";")
    new = definition(pg, SET)
    assert new == old.replace(anchor, early + anchor)
    assert definition(pg, CURSOR) == cursor
    return dict(set_before=sha256(old.encode()).hexdigest(), set_after=sha256(new.encode()).hexdigest(),
                cursor=sha256(cursor.encode()).hexdigest(), namespace=namespace)


def physical(pg, namespace):
    assert re.fullmatch(r"parity_[0-9a-f]{16}", namespace)
    return json.loads(pg(f"""
        SELECT json_agg(row_to_json(r) ORDER BY r.job_key) FROM (
          SELECT args->>'key' AS job_key,
              encode(sha256(convert_to(args->>'payload', 'UTF8')), 'hex') AS payload_sha256,
              octet_length(args->>'payload') AS payload_bytes,
              pg_column_size(args)::bigint + pg_column_size(metadata) + pg_column_size(tags) AS size_sum,
              pg_column_compression(args) AS args_compression,
              pg_column_compression(metadata) AS metadata_compression,
              pg_column_compression(tags) AS tags_compression
          FROM mesh_jobs.jobs WHERE namespace = '{namespace}'
        ) AS r;
    """))
