"""Test whether the stock layout fingerprint pins a helper across one volatile call."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from time import monotonic, sleep

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "2026-11-25-dense-same-page"))
from prove import ROOT, frozen_sql, run

KEY = "00000000-0000-4000-8000-000000000001"
EFFECT = f"""mesh_jobs.effect_key(jsonb_populate_record(NULL::mesh_jobs.jobs,
            '{{"id":"{KEY}","run":1}}'::jsonb))"""


def inspect(bindir):
    with TemporaryDirectory(dir=ROOT / "target", prefix="lgs-") as tmp:
        base = Path(tmp)
        data = base / "pg"
        ctl = [bindir / "pg_ctl", "-D", data, "-w"]
        run([bindir / "initdb", "-D", data, "--no-locale", "-E", "UTF8", "-A", "trust", "-U", "bootstrap"])
        run([*ctl, "-l", base / "pg.log", "-o", f"-c listen_addresses='' -k {base}", "start"])
        psql = [bindir / "psql", "-XqAt", "-v", "ON_ERROR_STOP=1", "-h", base, "-d", "postgres"]

        def query(sql, role="bootstrap"):
            return run([*psql, "-U", role], input=sql)

        try:
            assert query("SHOW server_version_num") == "170011"
            query("CREATE ROLE schema_owner LOGIN; CREATE ROLE worker LOGIN; "
                  "GRANT CREATE ON DATABASE postgres TO schema_owner")
            setup = "\n".join(frozen_sql(name) for name in ("schema.sql", "enqueue_schema.sql",
                              "drain_schema.sql", "cron_schema.sql", "effect_fence_schema.sql"))
            query(setup + f"""
                GRANT USAGE ON SCHEMA mesh_jobs TO worker;
                GRANT EXECUTE ON FUNCTION mesh_jobs.enqueue_batch_layout_v1() TO worker;
                CREATE SCHEMA witness;
                GRANT USAGE ON SCHEMA witness TO worker;
                CREATE FUNCTION witness.guarded_read() RETURNS text LANGUAGE plpgsql VOLATILE AS $f$
                BEGIN
                    IF current_setting('transaction_isolation') <> 'read committed' OR
                       NOT pg_try_advisory_xact_lock_shared(1782495102,1) OR
                       NOT mesh_jobs.enqueue_batch_layout_v1() THEN
                        RAISE EXCEPTION 'stock guard failed before barrier';
                    END IF;
                    PERFORM pg_advisory_xact_lock(72865, 7);
                    RETURN {EFFECT};
                END $f$;
            """, "schema_owner")
            assert query("SELECT mesh_jobs.enqueue_batch_layout_v1(), " + EFFECT, "worker") == f"t|{KEY}/1"
            holder = subprocess.Popen([*psql, "-U", "schema_owner"], text=True,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE)
            reader = None
            try:
                holder.stdin.write("SELECT 'locked' FROM pg_advisory_lock(72865,7);\n")
                holder.stdin.flush()
                assert holder.stdout.readline().strip() == "locked"
                reader = subprocess.Popen([*psql, "-U", "worker", "-c",
                    "SET statement_timeout='15s'; SELECT witness.guarded_read()"],
                    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                until = monotonic() + 10
                while query("""SELECT count(*) FROM pg_locks WHERE locktype='advisory'
                            AND classid=72865 AND objid=7 AND NOT granted""") != "1":
                    if reader.poll() is not None or monotonic() >= until:
                        raise RuntimeError("worker failed to reach the post-guard barrier")
                    sleep(0.02)
                cooperative_lock = query("SELECT pg_try_advisory_xact_lock(1782495102,1)", "schema_owner")
                assert cooperative_lock == "f"
                query("""SET lock_timeout='2s'; CREATE OR REPLACE FUNCTION mesh_jobs.effect_key(j mesh_jobs.jobs)
                      RETURNS text LANGUAGE sql IMMUTABLE STRICT SET search_path=pg_catalog,pg_temp
                      AS $$ SELECT 'replacement/' || j.run::text $$""", "schema_owner")
                holder.stdin.write("SELECT pg_advisory_unlock(72865,7);\n")
                holder.stdin.flush()
                observed, err = reader.communicate(timeout=10)
                assert reader.returncode == 0 and observed.strip() == "replacement/1", (err, observed)
                after = query("SELECT mesh_jobs.enqueue_batch_layout_v1()", "worker")
                assert after == "f"
                return dict(postgres="17.11", isolation="read committed", stock_guard_before=True,
                            stock_effect_key=KEY+"/1", cooperative_exclusive_lock_acquired=False,
                            effect_key_inside_guarded_call=observed.strip(), stock_guard_after=False)
            finally:
                if reader is not None and reader.poll() is None:
                    reader.kill()
                    reader.wait()
                holder.communicate(timeout=5)
        finally:
            run([*ctl, "-m", "fast", "stop"])


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--pg-bin", required=True, type=Path)
    print(json.dumps(inspect(p.parse_args().pg_bin), indent=2, sort_keys=True))
