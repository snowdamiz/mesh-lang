"""Bound dense grant, dispatcher and actor delays from the released native arms."""
from bisect import bisect_right
from collections import defaultdict
from hashlib import sha256
import json
from pathlib import Path
from statistics import median
import tarfile


HERE = Path(__file__).resolve().parent
NATIVE = HERE.parent / "2026-11-24-dense-individual-hot-pool/logger-off-retry/evidence/native"
SUMMARY = NATIVE / "summary-1795570643531732000.json"


def certify_batches(jobs, calls, rows):
    buckets = defaultdict(list)
    for j in jobs:
        key = j["worker_pid"], j["claimed_us"]
        assert key[1] % 1000 == 0 and j["state"] == "completed" and j["attempts"] == 1
        buckets[key].append(j)
    markers = rows - len(jobs)
    assert markers >= 0 and calls - markers == len(buckets), "timestamp alias or unpriced empty call"
    assert all(1 <= len(batch) <= 16 for batch in buckets.values())
    return buckets


def upper_bounds(buckets):
    # One shared queue lock for every successful set grant, one dispatcher per PID.
    all_stamps = sorted({stamp for _, stamp in buckets})
    local_stamps = defaultdict(list)
    terminals = defaultdict(list)
    for (pid, stamp), batch in buckets.items():
        local_stamps[pid].append(stamp)
        terminals[pid].extend(j["settled_us"] for j in batch)
    for values in (*local_stamps.values(), *terminals.values()):
        values.sort()

    def next_stamp(stamps, stamp):
        i = bisect_right(stamps, stamp)
        # now_ms() floors the real clock. Use the late edge and skip clock ties.
        return stamps[i] + 1000 if i < len(stamps) else float("inf")

    for (pid, stamp), batch in buckets.items():
        first_entry = min(j["entered_us"] for j in batch)
        assert stamp <= first_entry
        same_runner = next_stamp(local_stamps[pid], stamp)
        durable_by = min(first_entry, next_stamp(all_stamps, stamp))
        consumed_by = min(first_entry, same_runner)
        # A synchronous settlement recorded strictly after a handler entered
        # cannot predate dispatch of the batch containing that handler.
        terminal_stamps = terminals[pid]
        i = bisect_right(terminal_stamps, first_entry)
        member_terminal = min(j["settled_us"] for j in batch)
        next_terminal = terminal_stamps[i] if i < len(terminal_stamps) else member_terminal
        spawned_by = min(same_runner, member_terminal, next_terminal)
        assert durable_by <= consumed_by <= spawned_by
        yield pid, stamp, batch, first_entry, durable_by, consumed_by, spawned_by


def reduce_mesh(raw, score):
    def read(name):
        return json.load(raw.extractfile("adapter/" + name + ".json"))
    ledger = read("ledger")
    jobs = ledger["jobs"]
    assert ledger["phase"] == "measured" and (ledger["runners"], ledger["slots"]) == (2, 64)
    assert ledger["database"]["synchronous_commit"] and ledger["database"]["fsync"]
    assert len(jobs) == len({j["id"] for j in jobs}) == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 3072
    assert all(j["entered_us"] <= j["attempt_committed_us"] < j["effect_committed_us"] <= j["settled_us"] for j in jobs)
    policy = read("queue-policy")
    assert policy["queue_policy"] == "set_based" and policy["terminal_xids"] == 1024
    assert policy["completion_cohort_queries"] == 0
    statements = policy["queue_statements"]
    assert not any("claim_batch_v1(" in row["query"] for row in statements)
    grant, = (row for row in statements if "claim_batch_set_v1(" in row["query"])
    assert grant["calls"] == score["admitted_claim_queries"] == policy["admitted_queries"]
    for n in (0, 1):
        end = json.loads(raw.extractfile(f"adapter/runner-{n}.log").read().splitlines()[-1])
        assert end["drained"] and end["last_error"] is None and end["recovered"] == end["stale_acks"] == 0
        assert end["finished"] == end["claimed"] == end["outcomes"]["completed"]

    buckets = certify_batches(jobs, grant["calls"], grant["rows"])
    assert len({pid for pid, _ in buckets}) == 2
    durations = []
    groups = []
    settlements_during_gap = set()
    for pid, stamp, batch, entry, durable, consumed, spawned in upper_bounds(buckets):
        groups.append((entry - durable, len(batch)))
        for j in batch:
            e = j["entered_us"]
            durations.append((e - stamp, e - durable, e - consumed, max(0, e - spawned)))
        settlements_during_gap.update(j["id"] for j in jobs if j["worker_pid"] == pid and durable < j["settled_us"] < entry)
    total, db, dispatch, actor = (list(column) for column in zip(*durations))
    return dict(sql_calls=grant["calls"], returned_rows=grant["rows"], batches=len(groups),
                total_stamp_to_entry_job_us=sum(total),
                after_durable_lower_job_us=sum(db), after_consumption_started_lower_job_us=sum(dispatch),
                after_spawn_lower_job_us=sum(actor), jobs_witnessed_after_spawn=sum(x > 0 for x in actor),
                batches_durable_before_first_entry=sum(x > 0 for x, _ in groups),
                median_durable_to_first_entry_lower_us=median(x for x, _ in groups),
                distinct_same_runner_terminals_during_pre_entry_durable_windows=len(settlements_during_gap))


def replay():
    official = json.loads(SUMMARY.read_text())
    assert official["qualified_rotations"] == 6 and official["missing"] == [] and official["failures"] == {}
    result = {}
    for group, block in sorted(official["blocks"].items()):
        lanes = {}
        for label in ("hot", "checked-a", "checked-b"):
            key = "measured-" + label
            score = block["scores"][key]
            path = NATIVE / "blocks" / block["attempt"] / key / "measured-mesh/raw.tar.gz"
            digest = sha256(path.read_bytes()).hexdigest()
            assert digest == score["raw_sha256"]
            with tarfile.open(path) as raw:
                lanes[label] = dict(raw_sha256=digest, **reduce_mesh(raw, score))
        result[group] = lanes
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
