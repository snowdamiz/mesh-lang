"""Compare per-runner grant widths with the durable finish of previously claimed jobs."""
from bisect import bisect_right
from collections import Counter, defaultdict
from hashlib import sha256
import io
import json
from pathlib import Path
from statistics import median
import tarfile

from late_capacity_peer_phases import SEAL

EVIDENCE = Path(__file__).resolve().parent.parent / "2026-11-25-public-dense-late-capacity/peer-comparison/evidence"


def capacity(rows):
    by_runner = defaultdict(lambda: defaultdict(list))
    for row in rows:
        by_runner[row["worker_pid"]][row["claimed_us"]].append(row)
    assert len(by_runner) == 8
    all_stamps = sorted(row["claimed_us"] for row in rows)
    widths = Counter()
    near_end = 0
    partial = 0
    occupied = Counter()
    seat_deficit = Counter()
    refill_gaps = []
    final_cohort_sizes = Counter()
    middle_empty_us = 0
    effects = sorted(job["effect_committed_us"] for job in rows)
    middle_begin, middle_end = effects[255], effects[767]
    for groups in by_runner.values():
        earlier_terminals = []
        terminal_xids = {}
        for stamp, jobs in sorted(groups.items()):
            width = len(jobs)
            assert 1 <= width <= 8
            first_entry = min(job["entered_us"] for job in jobs)
            assert stamp <= first_entry
            widths[width] += 1
            if earlier_terminals:
                latest_commit = max(earlier_terminals)
                # Leave a millisecond for the grant clock's resolution.
                middle_empty_us += max(0, min(stamp - 1000, middle_end)
                                       - max(latest_commit, middle_begin))
                if width == 8:
                    refill_gaps.append(stamp - latest_commit)
                    last = [size for time, size in terminal_xids.values() if time == latest_commit]
                    if len(last) == 1:
                        final_cohort_sizes[last[0]] += 1
                    else:
                        final_cohort_sizes["tied"] += 1
            # Exclude other runners' jobs in this same millisecond from the count of later jobs.
            this_and_later = width + len(rows) - bisect_right(all_stamps, stamp)
            if width < 8:
                if this_and_later < 8:
                    near_end += 1
                else:
                    partial += 1
                    for margin in (0, 1000, 5000):
                        still_live = sum(end > first_entry + margin for end in earlier_terminals)
                        assert still_live + width <= 8
                        occupied[margin] += int(still_live > 0)
                        seat_deficit[margin] += still_live
            earlier_terminals.extend(job["settled_us"] for job in jobs)
            for job in jobs:
                xid = job["settled_xid"]
                time, size = terminal_xids.get(xid, (job["settled_us"], 0))
                assert time == job["settled_us"]
                terminal_xids[xid] = (time, size + 1)
    assert 0 <= middle_empty_us <= 8 * (middle_end - middle_begin)
    return dict(width_counts=dict(widths), partial_with_eight_historical_jobs=partial,
                partial_near_end=near_end, groups_with_older_unsettled_job=dict(occupied),
                older_unsettled_seats=dict(seat_deficit), full_refills=len(refill_gaps),
                median_prior_terminal_to_full_grant_us=median(refill_gaps) if refill_gaps else None,
                last_cohort_before_full_refill={str(k): v for k, v in final_cohort_sizes.items()},
                middle_span_us=middle_end - middle_begin, middle_between_batches_runner_us=middle_empty_us)


def read_arm(arm, score):
    data = (arm / "measured-mesh/raw.tar.gz").read_bytes()
    assert sha256(data).hexdigest() == score["raw_sha256"]
    with tarfile.open(fileobj=io.BytesIO(data)) as raw:
        assert json.load(raw.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(raw.extractfile("adapter/ledger.json"))
        sql = json.load(raw.extractfile("adapter/queue-policy.json"))["queue_statements"]
    jobs = ledger["jobs"]
    assert ledger["slots"] == 64 and ledger["runners"] == 8
    assert len(jobs) == len({j["id"] for j in jobs}) == 1024
    assert len({row[k] for row in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    assert all(j["state"] == "completed" and j["attempt"] == j["attempts"] == j["effects"] == 1
               and j["entered_us"] <= j["effect_committed_us"] <= j["settled_us"] for j in jobs)
    claim, = [q for q in sql if "claim_batch_bounded_v1" in q["query"]]
    assert not any("claim_batch_cursor" in q["query"] for q in sql)
    counts = capacity(jobs)
    # Each successful nonempty call has one runner and one millisecond grant stamp.
    # Each excess returned row is one nonproductive call's marker.
    assert sum(counts["width_counts"].values()) == claim["calls"] - (claim["rows"] - len(jobs))
    assert sum(w * n for w, n in counts["width_counts"].items()) == len(jobs)
    result = dict(raw_sha256=score["raw_sha256"], productive_calls=score["positive_grants"],
                          checked_calls=claim["calls"], **counts)
    assert score["positive_grants"] == sum(counts["width_counts"].values())
    return result


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["request_sha256"] == SEAL and release["exit_status"] == 0
    scored = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    result = {}
    for number in range(1, 6):
        block = EVIDENCE / "blocks" / f"{number:02}-noop"
        block_scores = scored[str(number)]
        assert block_scores == json.loads((block / "reconciled.json").read_bytes())
        result[number] = {}
        for lane in ("A1", "A2", "reference", "B"):
            name = "measured-" + lane
            result[number][lane] = read_arm(block / name, block_scores[name])
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
