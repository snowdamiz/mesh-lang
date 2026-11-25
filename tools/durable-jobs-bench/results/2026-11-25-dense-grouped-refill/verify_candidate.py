"""Untimed faults against the patched public b3557273 fixture and TLS relay."""

import os
from pathlib import Path
import sys
import time

replay = Path(os.environ["PRODUCT_SOURCE"]) / (
    "tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay"
)
sys.path.insert(0, str(replay))
from semantic import (Relay, arm, cluster, outcomes, rows, sql_cut, submit,
                      teardown, wait)

SCALAR = b"SELECT coalesce(mesh_jobs.settle_state($1,"
BULK = b"SELECT ordinal::text FROM mesh_jobs.complete_batch_v1("


def held_terminal(out, executable, drop):
    with cluster(out, executable, "fault") as (db, subject, env), Relay(db) as relay:
        gate = relay.arm(SCALAR, drop=drop)
        log = out / "runner.log"
        env = dict(env, JOBS_PARITY_POOL_URL=db.url("mesh", port=relay.port))
        process = subject.run(env, 0, log)
        first = ["first"]
        other = [f"other{i}" for i in range(4)]
        try:
            wait("runner ready", lambda: arm.h.runner_ready([process], [log], track=False))
            initial = submit(out, subject, env, "first", [(key, {}) for key in first])
            assert gate.hit.wait(5), "terminal receipt did not reach ReadyForQuery"
            held = rows(db)
            assert all(held[key]["state"] == "completed" for key in first)
            assert len({held[key]["xid"] for key in first}) == 1
            assert gate.witness["rows"] == len(first) and gate.witness["idle"]
            follow = submit(out, subject, env, "follow",
                            [(key, {"hold_ms": 2200}) for key in other])
            free = 4 - len(first)

            def filled():
                current = rows(db)
                started = [key for key in other if current[key]["attempts"]]
                return (current, started) if len(started) >= free else None

            running, started = wait("claim through independent free capacity", filled)
            assert len(started) == free
            key = started[0]
            before = running[key]["expiry"]
            renewed = wait("renew through held terminal receipt",
                           lambda: (current := rows(db))[key]["expiry"] > before and current)
            assert sum(bool(renewed[key]["attempts"]) for key in other) == free
            assert all(renewed[key] == held[key] for key in first)
            assert not gate.release.is_set()
            gate.release.set()
            if drop:
                wait("lost terminal receipt reported", lambda:
                     log.read_text().count('"event":"ack_uncertain"') >= len(first))
            def completed():
                current = rows(db)
                return current if all(row["state"] == "completed" for row in current.values()) else None

            final = wait("all visits committed", completed)
            arm.h.stop_runners([process], require_clean=True)
            app = [xid for row in final.values() for xid in [*row["attempts"], row["effect"]]]
            terminal = {row["xid"] for row in final.values()}
            assert len(set(app)) == len(app) == 2 * len(final)
            assert not set(app) & terminal
            assert not {initial["xid"], follow["xid"]} & (set(app) | terminal)
            assert all(final[key] == held[key] for key in first)
            assert len(final) == len(first) + len(other)
            requests = [query for _, query in relay.requests]
            arm.h.write_json(out / "result.json", dict(wire=gate.witness, drop=drop,
                             capacity=4, available_while_held=free, started_while_held=started,
                             renewal_before=before, renewal_while_held=renewed[key]["expiry"],
                             attempted_bulk=sum(query.startswith(BULK.decode()) for query in requests),
                             terminal_xids=len(terminal), independent_app_xids=len(set(app)),
                             settled=final))
        finally:
            gate.release.set()
            if process.poll() is None:
                arm.h.stop_runners([process], require_clean=False)


def lost_bulk(out, executable):
    with cluster(out, executable, "fault") as (db, subject, env), Relay(db) as relay:
        renewal = relay.arm(b"SELECT mesh_jobs.try_renew(")
        terminal = relay.arm(BULK, drop=True)
        log = out / "runner.log"
        child = subject.run(dict(env, JOBS_PARITY_POOL_URL=db.url("mesh", port=relay.port)), 0, log)
        try:
            wait("runner ready", lambda: arm.h.runner_ready([child], [log], track=False))
            submit(out, subject, env, "bulk",
                   [("long", {"hold_ms": 2600}), ("a", {"hold_ms": 850}), ("b", {"hold_ms": 850})])
            assert renewal.hit.wait(4), "renewal did not reach ReadyForQuery"
            wait("two independent effects", lambda: all(rows(db)[key]["effect"] for key in ("a", "b")))
            time.sleep(.04)
            assert rows(db)["a"]["state"] == rows(db)["b"]["state"] == "executing"
            renewal.release.set()
            assert terminal.hit.wait(4), "grouped success did not reach ReadyForQuery"
            held = rows(db)
            assert held["a"]["state"] == held["b"]["state"] == "completed"
            assert held["a"]["xid"] == held["b"]["xid"] and terminal.witness["rows"] == 2
            terminal.release.set()
            wait("two ambiguous receipts", lambda: log.read_text().count('"event":"ack_uncertain"') == 2)
            wait("two token-stale confirmations", lambda: log.read_text().count('"event":"ack_stale"') == 2)
            wait("long completes", lambda: rows(db)["long"]["state"] == "completed")
            arm.h.stop_runners([child], require_clean=True)
            final = rows(db)
            assert final["a"] == held["a"] and final["b"] == held["b"]
            assert sum(sql.startswith(BULK.decode()) for _, sql in relay.requests) == 1
            arm.h.write_json(out / "result.json", dict(held=held, final=final, wire=terminal.witness,
                             bulk_submissions=1, token_stale_confirmations=2))
        finally:
            renewal.release.set()
            terminal.release.set()
            if child.poll() is None:
                arm.h.stop_runners([child], require_clean=False)


def stop_partial(out, executable, witness):
    with cluster(out, executable, "dense") as (db, subject, env):
        gate = out / "gate"
        gate.mkdir()
        runner = type(subject)("mesh", [str(witness)], witness.parent, db, db.env)
        log = out / "runner.log"
        child = runner.run(dict(env, COMBINED_TEARDOWN_DIR=str(gate)), 0, log)
        try:
            wait("runner ready", lambda: arm.h.runner_ready([child], [log], track=False))
            submit(out, subject, env, "partial", [("owned", {})])
            wait("termination callback", (gate / "entered").exists)
            held = rows(db)["owned"]
            assert held["state"] == "executing" and held["effect"]
            child.terminate()
            time.sleep(.05)
            assert rows(db)["owned"] == held
            (gate / "release").touch()
            assert child.wait(timeout=8) == 0
            result = rows(db)["owned"]
            assert result["state"] == "completed" and result["effect"] == held["effect"]
            assert result["attempts"] == held["attempts"] and result["xid"] != held["xid"]
            arm.h.write_json(out / "result.json", dict(held=held, drained=result))
        finally:
            if child.poll() is None:
                (gate / "release").touch()
                arm.h.stop_runners([child], require_clean=False)


if __name__ == "__main__":
    out, executable, witness = (Path(arg).resolve() for arg in sys.argv[1:])
    out.mkdir()
    os.environ["PRODUCT_COMPOSITION"] = "product-d2"
    os.environ["JOBS_GROUPED_WRITER"] = "refill8ms"
    sql_cut(out / "sql-cut", executable)
    held_terminal(out / "held-terminal", executable, False)
    held_terminal(out / "lost-terminal", executable, True)
    lost_bulk(out / "lost-bulk", executable)
    outcomes(out / "cancellation", executable)
    teardown(out / "held-teardown", executable, witness, forced=False)
    teardown(out / "forced-teardown", executable, witness, forced=True)
    stop_partial(out / "stop-partial", executable, witness)
