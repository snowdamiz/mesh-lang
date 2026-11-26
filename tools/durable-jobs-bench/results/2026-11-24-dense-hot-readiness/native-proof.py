"""Untimed TLS and lifecycle witnesses using the unchanged public handler."""
from contextlib import contextmanager
from pathlib import Path
import subprocess
import sys
import time
from unittest.mock import patch

import semantic

from backend import arm
from proof import check_grant
from semantic import NS, cluster, handler_cut, rows, submit, wait
from wire import Relay

HOT_READY = "&mesh_pool_hot_checkout=1&mesh_cooperative_pool=1"
WRITES = "INSERT INTO public.mesh_jobs_parity_"


@contextmanager
def runner(subject, env, out, **options):
    url = options.pop("app_url", env["MESH_EFFECT_APP_URL"])
    log = out / "runner.log"
    proc = subject.run(dict(env, MESH_EFFECT_APP_URL=url + HOT_READY,
                            MESH_PG_TRACE_WAITS="1", **options), 0, log)
    try:
        wait("runner ready", lambda: arm.h.runner_ready([proc], [log], track=False))
        yield proc
    finally:
        if proc.poll() is None:
            arm.h.stop_runners([proc], require_clean=True)
    assert "postgres socket wait read" in log.read_text()


def stale(out, executable):
    with cluster(out, executable, "dense") as (db, subject, env), Relay(db) as relay:
        with runner(subject, env, out, app_url=db.url("mesh", "compiled_caller", port=relay.port)):
            def complete(key):
                submit(out, subject, env, key, [(key, {})])
                return wait(key, lambda: (row if (row := rows(db).get(key)) and row["state"] == "completed" else None))

            first = complete("first")
            mark = len(relay.requests)
            time.sleep(.55)
            second = complete("healthy-stale")
            healthy = relay.requests[mark:]
            assert len(healthy) == 3 and [query == "SELECT 1" for _, query in healthy] == [True, False, False]
            assert len({port for port, _ in healthy}) == 1
            port = healthy[0][0]
            assert db.sql("mesh", f"""SELECT pg_terminate_backend(pid) FROM pg_stat_activity
                WHERE usename='compiled_caller' AND client_port={port}""") == "t"
            mark = len(relay.requests)
            time.sleep(.55)
            third = complete("dead-stale")
            dead = relay.requests[mark:]
            assert len(dead) == 3 and dead[0] == (port, "SELECT 1"), dead
            assert dead[1][0] == dead[2][0] != port and all(q.startswith(WRITES) for _, q in dead[1:])
        xids = [x for row in (first, second, third) for x in (*row["attempts"], row["effect"], row["xid"])]
        assert len(xids) == len(set(xids)) == 9
        arm.h.write_json(out / "result.json", dict(healthy_stale=healthy, dead_stale=dead,
                                                  retired_port=port, xids=xids))


@contextmanager
def hold_effect_table(db):
    command, env = db.psql("mesh")
    child = subprocess.Popen(command, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True)
    try:
        child.stdin.write("BEGIN; LOCK TABLE public.mesh_jobs_parity_effects IN SHARE MODE;\n"
                          "SELECT 'LOCK_READY';\n")
        child.stdin.flush()
        assert child.stdout.readline().strip() == "LOCK_READY"
        yield
        child.communicate("ROLLBACK;\n", timeout=5)
        assert child.returncode == 0
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()


def abandoned(out, executable, kill):
    with cluster(out, executable, "dense") as (db, subject, env):
        def blocked():
            return db.sql("mesh", """SELECT pid FROM pg_stat_activity
                WHERE datname='mesh' AND usename='compiled_caller' AND state='active'
                AND wait_event_type='Lock' AND query LIKE '%mesh_jobs_parity_effects%'""")

        with runner(subject, env, out) as child:
            with hold_effect_table(db):
                receipt = submit(out, subject, env, "abandoned", [("abandoned", {})])
                pid = int(wait("effect waiting on real TLS backend", blocked))
                held = rows(db)["abandoned"]
                assert held["state"] == "executing" and len(held["attempts"]) == 1 and held["effect"] is None
                if kill:
                    child.kill()
                    assert child.wait(timeout=5) == -9
                    assert rows(db)["abandoned"] == held
                    terminal = None
                else:
                    db.sql("mesh", f"SELECT mesh_jobs.job_control('{NS}', '{receipt['ids']['abandoned']}', 'cancel', NULL, NULL)")
                    wait("actor cancelled before the PostgreSQL write retired",
                         lambda: rows(db)["abandoned"]["state"] == "cancelled")
                    terminal = rows(db)["abandoned"]
                    assert terminal["effect"] is None and terminal["attempts"] == held["attempts"]
                assert blocked() == str(pid)
            wait("abandoned server statement retired", lambda: not blocked())
            effect = db.sql("mesh", f"""SELECT coalesce(max(pg_xact_commit_timestamp(xmin))::text, '')
                FROM public.mesh_jobs_parity_effects WHERE namespace='{NS}' AND job_key='abandoned'""")
            final = rows(db)["abandoned"]
            assert final["attempts"] == held["attempts"]
            if kill:
                assert final["state"] == "executing" and final["xid"] == held["xid"]
            else:
                assert final["state"] == "cancelled" and final["xid"] == terminal["xid"]
                if effect:
                    assert db.sql("mesh", f"""SELECT pg_xact_commit_timestamp(e.xmin) > pg_xact_commit_timestamp(j.xmin)
                        FROM public.mesh_jobs_parity_effects e JOIN mesh_jobs.jobs j
                        ON e.namespace=j.namespace AND e.job_key=j.args->>'key'
                        WHERE e.namespace='{NS}' AND e.job_key='abandoned'""") == "t"
                submit(out, subject, env, "after-cancel", [("next", {})])
                wait("replacement TLS connection served the next handler",
                     lambda: rows(db).get("next", {}).get("state") == "completed")
            assert bool(effect) == bool(final["effect"])
            arm.h.write_json(out / "result.json", dict(killed=kill, backend=pid, held=held,
                terminal_while_held=terminal, final=final, effect_timestamp=effect))


def lost_receipt(out, elf):
    class ReadinessSubject(semantic.Subject):
        def run(self, env, index, log):
            return super().run(dict(env, MESH_EFFECT_APP_URL=env["MESH_EFFECT_APP_URL"] +
                                    "&mesh_cooperative_pool=1", MESH_PG_TRACE_WAITS="1"), index, log)

    with patch.object(semantic, "Subject", ReadinessSubject):
        handler_cut(out, elf, fault=True, hot=True)
    assert "postgres socket wait read" in (out / "runner.log").read_text()


def main(out, elf):
    out.mkdir()
    cases = (
        ("hot-sync", lambda dest: check_grant(dest, elf, mixed=False, hot=True)),
        ("hot-ready-stale", lambda dest: stale(dest, elf)),
        ("hot-ready-lost-receipt", lambda dest: lost_receipt(dest, elf)),
        ("hot-ready-cancel", lambda dest: abandoned(dest, elf, kill=False)),
        ("hot-ready-kill", lambda dest: abandoned(dest, elf, kill=True)),
    )
    for name, case in cases:
        print(f"START {name}", flush=True)
        case(out / name)
        print(f"PASS {name}", flush=True)
    (out / "PASS").touch()


if __name__ == "__main__":
    main(*(Path(value).resolve() for value in sys.argv[1:]))
