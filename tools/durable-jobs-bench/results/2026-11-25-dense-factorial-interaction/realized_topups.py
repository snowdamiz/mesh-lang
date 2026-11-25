"""Bound jobs that used confirmed additional seats on the actual B grant schedule."""
from collections import Counter
from hashlib import sha256
from io import BytesIO
import json
import tarfile

from grant_capacity_frontier import EVIDENCE, SEAL
from late_capacity_io import queue_timeout_records


def census(block, score):
    raw_path, = block.glob("measured-B/measured-*/raw.tar.gz")
    raw = raw_path.read_bytes()
    assert sha256(raw).hexdigest() == score["raw_sha256"]
    with tarfile.open(fileobj=BytesIO(raw)) as archive:
        def read(name):
            return json.load(archive.extractfile("adapter/" + name + ".json"))

        ledger = read("ledger")
        assert (ledger["runners"], ledger["slots"]) == (8, 64)
        jobs = ledger["jobs"]
        assert len({j["id"] for j in jobs}) == len(jobs) == 1024
        assert all(j["state"] == "completed" and j["attempt"] == 1 for j in jobs)
        queries = read("queue-policy")["queue_statements"]
        claims, = [q for q in queries if "FROM mesh_jobs.claim" in q["query"]]
        assert "FROM mesh_jobs.claim_batch_bounded_v1(" in claims["query"]
        prelock, = [q for q in queries if q["query"].endswith("name = $2 FOR NO KEY UPDATE")]
        rollbacks = sum(q["calls"] for q in queries if q["query"] == "ROLLBACK")
        errors = queue_timeout_records(archive.extractfile("adapter/postgres.log").read().decode().splitlines())
        assert rollbacks == len(errors)
        after_lock_errors = sum(site == "bounded_claim" for _, site, _ in errors)
        final = []
        for index in range(8):
            lines = archive.extractfile(f"adapter/runner-{index}.log").read().splitlines()
            state = json.loads(lines[-1])
            assert state["drained"] and state["queues"]["default"]["limit"] == 8
            final.append(state)
        snapshots = []
        for line in archive.extractfile("adapter/broker-output.log"):
            if line.startswith(b"{"):
                obj = json.loads(line)
                if "claims" in obj:
                    snapshots.append(obj)
        assert len(snapshots) == 3 and all(s["dealloc"] == 0 for s in snapshots)
        assert snapshots[-1]["claims"] == claims["calls"]
        post_idle_calls = snapshots[-1]["claims"] - snapshots[-2]["claims"]
        assert post_idle_calls == read("raw")["raw_claim_calls"]

    widths = Counter((j["claimed_us"], j["worker_pid"]) for j in jobs)
    assert all(1 <= w <= 8 for w in widths.values())
    assert len(widths) == score["positive_grants"]
    assert claims["rows"] - 1024 == claims["calls"] - len(widths)
    first_by_pid = {}
    for (stamp, pid), width in sorted(widths.items()):
        first_by_pid.setdefault(pid, (stamp, width))
    assert len(first_by_pid) == 8
    effects = sorted(j["effect_committed_us"] for j in jobs)
    assert all(stamp + 999 < effects[255] for stamp, _ in first_by_pid.values())
    samples, topups, credits = (sum(r["late_capacity"][k] for r in final)
                              for k in ("samples", "topups", "added"))
    assert samples == prelock["rows"] == prelock["calls"]
    assert sum(r["claimed"] for r in final) == 1024

    # Before any committed job is visible the owner can only request its full eight.
    # Credit all post-idle empty SELECTs and post-lock errors with seven wasted seats.
    other_turns = post_idle_calls - len(widths) + after_lock_errors
    positive_slack = sum(8 - w for w in widths.values())
    positive_slack -= sum(8 - w for _, w in first_by_pid.values())
    rows_lower = max(0, credits - 7 * other_turns - positive_slack)
    middle_full = sum(w == 8 and effects[255] <= stamp and stamp + 999 <= effects[767]
                      for (stamp, _), w in widths.items())
    # First positive grants could never top up. Every full grant that topped up
    # used every seat it added, even if subsequent COMMIT receipt was delayed.
    outside_or_partial = len(widths) - len(first_by_pid) - middle_full
    middle_turns_lower = max(0, topups - other_turns - outside_or_partial)
    return dict(raw_sha256=score["raw_sha256"], positive_grants=len(widths),
                post_idle_empty=post_idle_calls - len(widths), errors_after_prelock=after_lock_errors,
                topups=topups, added_seats=credits, potentially_unused_positive_seats=positive_slack,
                consumed_added_seats_lower=rows_lower, middle_full_grants=middle_full,
                middle_full_grants_using_added_seats_lower=middle_turns_lower)


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["request_sha256"] == SEAL
    scores = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    return {i: census(EVIDENCE / "blocks" / f"{int(i):02}-noop", arms["measured-B"])
            for i, arms in scores.items()}


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
