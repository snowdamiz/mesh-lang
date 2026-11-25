"""Install the same exact-match guard in all three disposable owner fixtures."""
from hashlib import sha256

GUARD_SOURCE_SHA = "6b7c5cef45efd0caabb5eabccda0ba9e19ae7ea586c2da7f61b8f2910ade89c6"
FILLFACTOR = 50
JOB = "mesh_jobs.jobs"
GUARD = "mesh_jobs.enqueue_batch_layout_v1()"
OWNER = "parity_migrator"
LANES = ("default-a", "fill50", "default-b")


def guard_sql(source):
    if sha256(source.encode()).hexdigest() != GUARD_SOURCE_SHA:
        raise ValueError("unexpected frozen enqueue schema")
    guard, _ = source.split("\nCREATE OR REPLACE FUNCTION mesh_jobs.enqueue_batch_v1", 1)
    anchor = "relhasrules, reloptions, toast_options) FROM tables"
    if guard.count(anchor) != 1:
        raise ValueError("unexpected layout guard anchor")
    # The frozen writer also supports EXTERNAL. This fixture holds args at the
    # default storage and fingerprints every column's actual storage mode.
    storage_start = guard.index("            -- Treat only")
    storage_end = guard.index("            a.attcompression,", storage_start)
    guard = guard[:storage_start] + "            a.attstorage,\n" + guard[storage_end:]
    return guard.replace(anchor, f"""relhasrules,
            CASE WHEN oid = '{JOB}'::regclass
                       AND reloptions = ARRAY['fillfactor={FILLFACTOR}']::text[]
                 THEN NULL::text[] ELSE reloptions END, toast_options) FROM tables""")


def install_sql(source, lane):
    if lane not in LANES:
        raise ValueError(f"unknown lane {lane}")
    options = f"ARRAY['fillfactor={FILLFACTOR}']::text[]" if lane == "fill50" else "NULL::text[]"
    ddl = f"ALTER TABLE {JOB} SET (fillfactor={FILLFACTOR});" if lane == "fill50" else ""
    return f"""BEGIN;
SET LOCAL ROLE {OWNER};
LOCK TABLE ONLY {JOB} IN ACCESS EXCLUSIVE MODE;
DO $preflight$
BEGIN
    IF current_database() <> 'mesh' OR current_setting('server_version_num') <> '170011'
       OR EXISTS (SELECT FROM {JOB})
       OR (SELECT reloptions IS NOT NULL FROM pg_class WHERE oid = '{JOB}'::regclass)
       OR NOT {GUARD}
    THEN RAISE EXCEPTION 'expected empty stock PostgreSQL 17.11 Mesh fixture'; END IF;
END
$preflight$;
{guard_sql(source)}
{ddl}
DO $verify$
BEGIN
    IF (SELECT reloptions FROM pg_class WHERE oid = '{JOB}'::regclass) IS DISTINCT FROM {options}
       OR NOT {GUARD}
    THEN RAISE EXCEPTION 'invalid same-page fixture'; END IF;
END
$verify$;
COMMIT;
"""
