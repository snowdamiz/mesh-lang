"""Untimed pre-entry grant rejection with disposable PostgreSQL and the public handler."""
import json
from pathlib import Path
import sys

from semantic import NS, arm, cluster, rows, submit, wait
from wire import Relay


def probe(out, executable, mode):
    with cluster(out, executable, "fault") as (db, subject, env), Relay(db) as queue:
        log = out / "runner.log"
        case = submit(out, subject, env, mode, [(mode, {})])
        gate = None
        if mode == "cancelled":
            job_id = case["ids"][mode]
            row = db.sql("mesh", f"SELECT mesh_jobs.job_control('{NS}', '{job_id}', 'cancel', NULL, NULL)")
            assert json.loads(row)["state"] == "cancelled"
        else:
            gate = queue.arm(b"COMMIT", corrupt=mode == "malformed")
        process = subject.run(dict(env, JOBS_PARITY_POOL_URL=db.url("mesh", port=queue.port),
                                   MESH_JOBS_CLAIM_LANE_LAUNCH="1"), 0, log)
        try:
            wait("runner ready", lambda: arm.h.runner_ready([process], [log], track=False))
            if gate:
                assert gate.hit.wait(4)
                held = rows(db)[mode]
                assert held["state"] == "executing" and held["token"] and held["attempt"] == 1, held
                assert not held["attempts"] and held["effect"] is None
                if mode == "expired":
                    wait("original grant expires before its idle receipt is sent", lambda: int(
                        db.sql("mesh", "SELECT mesh_jobs.now_ms()")) >= held["expiry"])
                    assert not rows(db)[mode]["attempts"]
                gate.release.set()
                assert gate.forwarded.wait(2)
                wait("subsequent, independently confirmed attempt settled", lambda:
                     rows(db)[mode]["state"] == "completed")
                final = rows(db)[mode]
                assert final["attempt"] == 2 and len(final["attempts"]) == 1, final
                enter = int(db.sql("mesh", f"""SELECT (extract(epoch FROM entered_at)*1000)::bigint
                    FROM public.mesh_jobs_parity_attempts WHERE namespace='{NS}' AND job_key='{mode}'
                    AND attempt=2"""))
                assert enter >= held["expiry"]
            else:
                held = None
                final = rows(db)[mode]
                assert final["state"] == "cancelled" and final["attempt"] == 0, final
                assert not final["attempts"] and final["effect"] is None
            arm.h.stop_runners([process], require_clean=True)
            shutdown = json.loads(log.read_text().splitlines()[-1])
            assert shutdown["drained"] and not shutdown["undrained_jobs"] and not shutdown["pending_claims"]
            arm.h.write_json(out / "result.json", dict(mode=mode, held=held, final=final, shutdown=shutdown,
                                                       wire=gate.witness if gate else None,
                                                       malformed_ready=mode == "malformed"))
        finally:
            if gate:
                gate.release.set()
            if process.poll() is None:
                arm.h.stop_runners([process], require_clean=False)


if __name__ == "__main__":
    out, executable = map(Path, sys.argv[1:3])
    out.mkdir(mode=0o700)
    for mode in ("cancelled", "expired", "malformed"):
        probe(out / mode, executable, mode)
        print(f"PASS {mode}", flush=True)
