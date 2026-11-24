"""Exercise the replay parser with synthetic PostgreSQL records only."""

import json

from trace import analyze

prefix = "2026-11-24 05:13:"
origin = 1795497180000000
pid = "12345"


def record(time, n, text):
    return f"{prefix}{time} UTC [{pid}] {n} parity_subject mesh | {text}"


def plan(query, n, elapsed, stamp, node):
    data = json.dumps({"Query Text": query, "Plan": node}, indent=2)
    return record(stamp, n, f"LOG: duration: {elapsed} ms  plan:\n" +
                  "\n".join("\t" + item for item in data.splitlines()))


def test():
    claim = "SELECT status, id FROM mesh_jobs.claim_batch_set_v1($1)"
    log = "\n".join([
        record("01.500", 1, "LOG: duration: 0.090 ms  execute <unnamed>: " + claim),
        record("02.005", 2, "LOG: duration: 0.025 ms  bind <unnamed>: " + claim),
        record("02.015", 3, "LOG: process 12345 acquired ShareLock on transaction 515 after 3.400 ms"),
        plan("SELECT * FROM mesh_jobs.queues FOR NO KEY UPDATE", 4, 3.6, "02.016",
             {"Node Type": "LockRows", "Actual Rows": 1, "Actual Loops": 1}),
        plan("WITH picked AS MATERIALIZED (SELECT 1) SELECT 1", 5, 3.3, "02.020",
             {"Node Type": "Update", "Actual Rows": 16, "Actual Loops": 1,
              "Plans": [{"Node Type": "Seq Scan", "Relation Name": "jobs",
                         "Actual Rows": 800, "Actual Loops": 2, "Rows Removed by Filter": 224}]}),
        record("02.021", 6, "LOG: duration: 7.200 ms  execute <unnamed>: " + claim),
        record("02.025", 7, "LOG: duration: 0.700 ms  execute <unnamed>: COMMIT"),
        record("02.550", 8, "ERROR: canceling statement due to lock timeout"),
    ])
    result = analyze(log, (origin+1_000_000, origin+1_800_000),
                     origin+1_805_000, origin+2_000_000, origin+3_000_000)
    idle, active = result["claims"]
    assert idle["window"] == "idle" and active["window"] == "committed_drain", result
    assert active["acquired_lock_wait_lower_ms"] == 3.4
    assert active["finish_kind"] == "COMMIT" and active["select_end_to_finish_start_ms"] == 3.3
    assert active["queue_lock_statement_end_us"] == origin+2_016_000
    grant = next(p for p in result["plans"] if p["kind"] == "grant")
    assert grant["nodes"][1]["emitted"] == 1600
    assert grant["nodes"][1]["removed"] == {"Rows Removed by Filter": 448}
    assert result["observed_worker_role_lock_timeouts"] == 1
    assert result["incomplete_plan_records"] == 0


if __name__ == "__main__":
    test()
    print("Offline parser checks passed for idle, lock, nested plan rows, claim Execute and COMMIT")
