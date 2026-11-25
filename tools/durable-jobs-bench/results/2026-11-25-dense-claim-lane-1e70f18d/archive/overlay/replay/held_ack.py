"""Untimed, same-binary causal test of early launch during individual settlement."""

import json
import os
from pathlib import Path
import signal
import sys

from semantic import NS, arm, cluster, rows, submit, wait
from wire import Relay


def snapshot(status_path):
    status_path.unlink(missing_ok=True)
    status_path.with_suffix(".request").touch(exist_ok=False)
    wait("runner status response", status_path.exists)
    return json.loads(status_path.read_text())


def local_control(status_path, action, **args):
    request = status_path.with_suffix(".control")
    result = status_path.with_suffix(".control-result")
    result.unlink(missing_ok=True)
    request.write_text(json.dumps(dict(action=action, queue="default", **args)))
    request.rename(status_path.with_suffix(".control-request"))
    wait(f"local queue {action}", result.exists)
    return json.loads(result.read_text())


def held_ack(out, executable, mode):
    enabled = mode != "off"
    with (cluster(out, executable, "dense") as (db, subject, env),
          Relay(db) as queue, Relay(db) as app):
        effect = app.arm(b"INSERT INTO public.mesh_jobs_parity_effects")
        status_path = out / "status"
        log = out / "runner.log"
        runner = subject.run(dict(env, JOBS_PARITY_POOL_URL=db.url("mesh", port=queue.port),
                                 MESH_EFFECT_APP_URL=db.url("mesh", "compiled_caller", port=app.port)
                                 + "&mesh_pool_hot_checkout=1",
                                 JOBS_PARITY_STATUS_PATH=str(status_path),
                                 MESH_JOBS_GRANT_DIAGNOSTICS=str(out / "grant-ages.json"),
                                 MESH_JOBS_CLAIM_LANE_LAUNCH=str(int(enabled))), 0, log)
        try:
            wait("runner ready", lambda: arm.h.runner_ready([runner], [log], track=False))
            terminal = queue.arm(b"SELECT coalesce(mesh_jobs.settle_state(", drop=mode == "lost-ack")
            first = submit(out, subject, env, "first", [("first", {})])
            assert effect.hit.wait(4), "first handler did not reach its second commit"
            grant = queue.arm(b"COMMIT")
            peer = submit(out, subject, env, "peer", [("peer", {})])
            assert grant.hit.wait(4), "independent peer grant did not reach ReadyForQuery"
            before = rows(db)
            assert before["first"]["state"] == before["peer"]["state"] == "executing", before
            assert not before["peer"]["attempts"] and before["peer"]["effect"] is None, before
            assert before["first"]["effect"], before
            effect.release.set()
            assert terminal.hit.wait(4), "first terminal statement did not reach ReadyForQuery"
            blocked = snapshot(status_path)
            first_id = first["ids"]["first"]
            peer_id = peer["ids"]["peer"]
            assert blocked["undrained_jobs"] == [first_id] and blocked["pending_claims"] > 0, blocked
            assert blocked["finished"] == 0, blocked
            assert rows(db)["first"]["state"] == "completed"
            if mode == "stop":
                runner.send_signal(signal.SIGTERM)
                assert runner.poll() is None
            if mode in ("pause", "scale"):
                control = local_control(status_path, "pause") if mode == "pause" else (
                    local_control(status_path, "scale", limit=1))
                queue_state = control["queues"]["default"]
                assert queue_state["state"] == "paused" if mode == "pause" else queue_state["limit"] == 1
                assert control["undrained_jobs"] == [first_id] and control["pending_claims"] > 0
            else:
                control = None
            grant.release.set()
            assert grant.forwarded.wait(2)
            if mode in ("pause", "scale"):
                def registered():
                    status = snapshot(status_path)
                    return status if status["pending_claims"] == 0 else None

                overlap = wait("unstarted peer is registered for refund", registered)
                assert set(overlap["undrained_jobs"]) == {first_id, peer_id}
                assert overlap["finished"] == 0
                if mode == "scale":
                    local_control(status_path, "pause")
            elif enabled and mode != "stop":
                wait("peer's first application commit while terminal receipt is held",
                     lambda: len(rows(db)["peer"]["attempts"]) == 1)
                overlap = snapshot(status_path)
                assert set(overlap["undrained_jobs"]) == {first_id, peer_id}, overlap
                assert overlap["finished"] == overlap["pending_claims"] == 0, overlap
            elif mode == "off":
                overlap = snapshot(status_path)
                assert overlap["undrained_jobs"] == [first_id] and overlap["pending_claims"] > 0, overlap
            else:
                overlap = None
            during = rows(db)
            assert terminal.hit.is_set() and not terminal.release.is_set()
            assert during["peer"]["state"] == "executing", during
            assert bool(during["peer"]["attempts"]) is (mode in ("on", "lost-ack")), during
            terminal.release.set()
            if mode == "stop":
                assert runner.wait(timeout=12) == 0
                final = rows(db)
                assert not final["peer"]["attempts"] and final["peer"]["token"] is None, final
            elif mode in ("pause", "scale"):
                wait("unstarted grant returned", lambda: rows(db)["peer"]["token"] is None)
                final = rows(db)
                assert not final["peer"]["attempts"] and final["peer"]["state"] == "available", final
                arm.h.stop_runners([runner], require_clean=True)
            else:
                wait("both individual terminal commits", lambda: all(
                    row["state"] == "completed" for row in rows(db).values()))
                final = rows(db)
                xids = [first["xid"], peer["xid"], before["first"]["xid"], before["peer"]["xid"]]
                for row in final.values():
                    assert row["attempt"] == 1 and len(row["attempts"]) == 1 and row["token"] is None, row
                    xids.extend([*row["attempts"], row["effect"], row["xid"]])
                assert len({int(xid) % 2**32 for xid in xids}) == 10, xids
                expected_stale = int(mode == "lost-ack")

                def reconciled():
                    status = snapshot(status_path)
                    if status["drained"] and status["stale_acks"] == expected_stale:
                        return status
                    return None

                drained = wait("settlement reconciliation and slot release", reconciled)
                assert not drained["undrained_jobs"] and drained["pending_claims"] == 0, drained
                arm.h.stop_runners([runner], require_clean=True)
            report = json.loads(log.read_text().splitlines()[-1])
            assert report["state"] == "stopped" and report["drained"], report
            assert report["undrained_jobs"] == [] and report["pending_claims"] == 0, report
            arm.h.write_json(out / "result.json", dict(
                mode=mode, executable_sha256=arm.h.digest(executable), before=before, during=during,
                blocked=blocked, overlap=overlap, control=control, final=final, shutdown=report, grant=grant.witness,
                terminal=terminal.witness, requests=queue.requests, app_requests=app.requests))
        finally:
            for gate in queue.gates + app.gates:
                gate.release.set()
            if runner.poll() is None:
                arm.h.stop_runners([runner], require_clean=False)


if __name__ == "__main__":
    output, executable = (Path(arg).resolve() for arg in sys.argv[1:3])
    output.mkdir(mode=0o700)
    for mode in sys.argv[3:] or ("off", "on", "lost-ack", "stop"):
        held_ack(output / mode, executable, mode)
        print(f"PASS {mode}", flush=True)
