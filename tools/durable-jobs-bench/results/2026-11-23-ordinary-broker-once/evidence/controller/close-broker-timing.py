"""Publish the completed broker timing release as soon as the fences are idle."""

from pathlib import Path
import runpy
from subprocess import SubprocessError
import time


STATE = Path(__file__).resolve().parent
ops = runpy.run_path(str(STATE / "hot-checkout-slots.py"))
run, probe, note = (ops[name] for name in ("run", "probe", "note"))
ID = "ordinary-broker-once-matched"
NONCE = "9a71cf5c63cc"
OUTPUT = STATE.parent / "ordinary-broker-once" / ID
pending = run.read_json(OUTPUT / f"dispatch-{NONCE}.json")
summary_path = OUTPUT / f"summary-{NONCE}.json"
summary = run.read_json(summary_path)
run.require(pending["id"] == ID and pending["exit_status"] == pending["summary_exit_status"] == 0
            and pending.get("lock_probe_error") and len(summary["rows"]["large"]) == 5
            and not summary["missing"] and not summary["issues"]
            and all(codes == [0] for codes in pending["groups"].values()),
            "broker timing and all five raw rotations must complete before release")

note(f"Release pending for {ID}. All five rotations and scorer exited 0. "
     "The initial release fence probe returned EAGAIN on the local timing lock. "
     "Wait for the local replay to release before probing the original three fences.")
for _ in range(240):
    try:
        probe()
    except (OSError, SubprocessError):
        time.sleep(5)
    else:
        break
else:
    raise RuntimeError("The broker timing release is still pending fence verification")

report = dict(pending, released_at=run.utc_now(), release_probe_recovered=True,
              controller_process_exit_status=1, summary_sha256=run.digest(summary_path))
run.write_json(OUTPUT / f"recovered-release-{NONCE}.json", report)
run.write_json(STATE / "requests" / f"{ID}-{NONCE}.released.json", report)
note(f"RELEASED {ID} {report['released_at']}, exit 0. "
     f"All timing and replay commands qualified. Initial controller exited 1 pending the fences. Evidence: {OUTPUT}")
