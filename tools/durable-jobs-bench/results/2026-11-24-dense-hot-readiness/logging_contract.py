"""Check the actual public runner transcript with logging disabled."""
import json

WAIT_LINES = {"postgres socket wait read", "postgres socket wait write"}


def verify_runner_log(contents, *, trace=False, killed=False, lost_receipt=False):
    ready, *lines = contents.splitlines()
    if ready != "READY":
        raise ValueError("runner never became ready")
    if trace:
        lines = [line for line in lines if line not in WAIT_LINES]
    if lost_receipt:
        failures = [line for line in lines if line.startswith("trusted application write failed: ")]
        if len(failures) != 1:
            raise ValueError("expected one ambiguous write report")
        lines.remove(failures[0])
    if killed:
        if lines:
            raise ValueError("killed runner emitted unexpected output")
        return
    if len(lines) != 1:
        raise ValueError("runner emitted per-job events or unexpected output before shutdown")
    report = json.loads(lines[0])
    if (report.get("state") != "stopped" or report.get("drained") is not True
            or "event_buffer_bytes" in report or "event" in report
            or report.get("event_logs_dropped", 0) != 0):
        raise ValueError("runner emitted an active logger or an invalid shutdown receipt")
