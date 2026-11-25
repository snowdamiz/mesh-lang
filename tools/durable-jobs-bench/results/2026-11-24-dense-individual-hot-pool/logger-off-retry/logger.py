"""Reject live runner telemetry on canonical arms without changing the harness."""
import json


def verify_runner_log(text):
    lines = text.splitlines()
    if lines.count("READY") != 1:
        raise ValueError("runner did not reach readiness")
    records = []
    for line in lines:
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            if '"event":' in line or '"event_buffer_bytes":' in line:
                raise ValueError("runner emitted malformed event telemetry") from None
            continue
        if isinstance(record, dict):
            records.append(record)
    if any({"event", "event_buffer_bytes", "event_logs_dropped", "events_dropped"} & record.keys()
           for record in records):
        raise ValueError("runner had active event telemetry during a canonical arm")
    if not records or records[-1].get("state") != "stopped" or records[-1].get("drained") is not True:
        raise ValueError("runner did not report stopped and drained")


def verify_dense_logs(get, log):
    for i in range(2):
        verify_runner_log(log(f"runner-{i}.log"))
    raw = get("raw")
    if (raw["runner_event_buffer_bytes"] != [None, None]
            or raw["event_drops"] != [{"logger": None, "reader": None}] * 2):
        raise ValueError("frozen harness observed a telemetry sink")
