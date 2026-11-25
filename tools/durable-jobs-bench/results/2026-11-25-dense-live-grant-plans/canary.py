"""Untimed role, nested-plan, and query-ID proof against a private PG17.11."""
import json
from pathlib import Path
import subprocess
import sys

from plans import extract, nodes
from settings import PREFIX, role_sql

pgbin, module, scratch = (Path(p).resolve(strict=True) for p in sys.argv[1:])
data = scratch / "pg"
log = scratch / "postgres.log"


def call(binary, *argv):
    return subprocess.check_output([pgbin / binary, *argv], text=True, timeout=25).strip()


def psql(sql, db="mesh", role="owner"):
    return call("psql", "-XAtq", "-v", "ON_ERROR_STOP=1", "-h", str(scratch),
                "-d", db, "-U", role, "-c", sql)


call("initdb", "-D", data, "-U", "owner", "-A", "trust", "--no-locale")
with (data / "postgresql.conf").open("a") as config:
    config.write(f"\nlisten_addresses = ''\nunix_socket_directories = '{scratch}'\n"
                 f"shared_preload_libraries = 'pg_stat_statements'\n"
                 f"log_line_prefix = '{PREFIX}'\ncompute_query_id = on\n")
call("pg_ctl", "-D", data, "-l", log, "-w", "start")
try:
    call("createdb", "-h", scratch, "-U", "owner", "mesh")
    psql("""
        CREATE ROLE parity_subject LOGIN;
        CREATE ROLE unrelated LOGIN;
        CREATE EXTENSION pg_stat_statements;
        CREATE TABLE jobs(id integer PRIMARY KEY, state integer);
        INSERT INTO jobs VALUES (1,0),(2,0);
        CREATE FUNCTION grant_one(wanted integer) RETURNS integer AS $$
        DECLARE changed integer;
        BEGIN
            UPDATE public.jobs SET state = state + 1 WHERE id = wanted RETURNING id INTO changed;
            RETURN changed;
        END $$ LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog;
    """)
    psql(role_sql(module, "1"))
    assert psql("SELECT grant_one(1)", role="parity_subject") == "1"
    assert psql("SELECT grant_one(2)", role="unrelated") == "2"
    assert psql("SELECT array_agg(state ORDER BY id) FROM jobs") == "{1,1}"
    db_path = psql("SHOW dynamic_library_path")
    assert psql("SHOW server_version_num") == "170011"
    rows = json.loads(psql("""
        SELECT json_agg(row_to_json(q)) FROM (
            SELECT queryid, toplevel, query, userid::regrole::text AS role, calls
            FROM pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname='mesh')
        ) q
    """))
finally:
    call("pg_ctl", "-D", data, "-m", "fast", "-w", "stop")

captured = list(extract(log.read_text()))
assert len(captured) == 2 and {r["session_role"] for r in captured} == {"parity_subject"}, captured
mapping = []
for sample in captured:
    matches = [r for r in rows if r["queryid"] == sample["query_id"]]
    if sample["query"].startswith("UPDATE"):
        assert len(matches) == 1 and matches[0]["role"] == "owner" and not matches[0]["toplevel"]
        assert sample["plan"]["WAL Records"] > 0
    else:
        matches = [r for r in matches if r["role"] == "parity_subject"]
        assert len(matches) == 1 and matches[0]["toplevel"]
    assert any(n["Actual Loops"] > 0 and n["Shared Hit Blocks"] > 0 for n in nodes(sample["plan"]))
    mapping.append(matches[0])
print(json.dumps(dict(postgres="17.11", dynamic_library_path=db_path, scoped_role="parity_subject",
                      nested_query_keys=mapping, unrelated_sql_ran_without_plan=True), indent=2))
