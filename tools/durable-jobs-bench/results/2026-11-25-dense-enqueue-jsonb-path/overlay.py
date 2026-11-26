"""Construct and verify the private PostgreSQL writer from the pinned source."""

from hashlib import sha256
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
WRITER = "mesh_jobs.enqueue_batch_v1(text,jsonb)"
STOCK_SHA = "6b7c5cef45efd0caabb5eabccda0ba9e19ae7ea586c2da7f61b8f2910ade89c6"
RELOCATIONS = (
    ("x->'spec'->'unique'", "x #> '{spec,unique}'", 1),
    ("x->'spec'->'replace'", "x #> '{spec,replace}'", 1),
    ("x.value->'spec'->>'queue'", "x.value #>> '{spec,queue}'", 2),
)


def digest(data):
    return sha256(data.encode()).hexdigest()


def source():
    source = (HERE / "enqueue-stock.sql").read_text()
    assert digest(source) == STOCK_SHA
    writer = source[source.index("CREATE OR REPLACE FUNCTION mesh_jobs.enqueue_batch_v1"):]
    return writer[:writer.index("\n\nREVOKE")].rstrip() + "\n"


def candidate():
    writer = source()
    for stock, projected, count in RELOCATIONS:
        assert writer.count(stock) == count
        writer = writer.replace(stock, projected)
    return writer


def body(sql):
    return sql.split(" AS $$", 1)[1].split("$$;", 1)[0]


def state(cluster):
    return json.loads(cluster.sql("mesh", f"""
        SELECT json_build_object(
            'body', encode(sha256(convert_to(p.prosrc,'UTF8')),'hex'),
            'layout', encode(sha256(convert_to(l.prosrc,'UTF8')),'hex'),
            'acl', p.proacl, 'owner', pg_get_userbyid(p.proowner), 'invoker', NOT p.prosecdef,
            'volatile', p.provolatile = 'v', 'configuration', p.proconfig,
            'producer', has_function_privilege('parity_producer', p.oid, 'EXECUTE'),
            'runner', has_function_privilege('parity_subject', p.oid, 'EXECUTE'),
            'application', has_function_privilege('compiled_caller', p.oid, 'EXECUTE'),
            'guard_acl', l.proacl)
        FROM pg_proc p, pg_proc l WHERE p.oid = '{WRITER}'::regprocedure
           AND l.oid = 'mesh_jobs.enqueue_batch_layout_v1()'::regprocedure
    """))


def apply_writer(cluster, projected):
    before = state(cluster)
    assert before["body"] == digest(body(source()))
    assert before["owner"] == "parity_migrator" and before["invoker"] and before["volatile"]
    assert before["producer"] and not (before["runner"] or before["application"])
    assert before["configuration"] == ["search_path=pg_catalog, pg_temp"]
    sql = candidate() if projected else source()
    if projected:
        # A fresh migrated clone has no producers during this installation.
        cluster.sql("mesh", "BEGIN; SET LOCAL ROLE parity_migrator;\n" + sql + "COMMIT;")
    after = state(cluster)
    assert after == dict(before, body=digest(body(sql))), (before, after)
    return dict(before=before, after=after)


def counts(db):
    return json.loads(db.sql("mesh", """
        SELECT coalesce(jsonb_object_agg(key, stats), '{}') FROM (
            SELECT CASE WHEN query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
                        THEN 'bulk' ELSE 'scalar' END AS key,
                   jsonb_build_array(sum(calls), sum(rows)) AS stats
            FROM pg_stat_statements WHERE toplevel
              AND dbid=(SELECT oid FROM pg_database WHERE datname=current_database())
              AND userid='parity_producer'::regrole
              AND (query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
                   OR query LIKE 'SELECT mesh_jobs.enqueue(%')
            GROUP BY 1
        ) c
    """))
