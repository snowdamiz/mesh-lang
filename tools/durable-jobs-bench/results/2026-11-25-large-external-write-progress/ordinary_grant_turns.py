"""Reconstruct ordinary cursor turns from the 48-arm renderer pilot."""
import argparse
from functools import lru_cache
from hashlib import sha256
import json
from pathlib import Path
import tarfile

REQUEST = "25282314b8a9909a912010689d6ce9eb35c7ea43679feeaa2dbe62fed3a3e890"


def partitions(jobs, maximum):
    """All partitions consistent with ownership, checked entry, width, and SQL calls."""
    # A tied pair on the same PID could be ordered either way. Using its latest
    # entry for both rows keeps every feasible partition in the search.
    latest_entry = {}
    for row in jobs:
        stamp = row["claimed_us"]
        latest_entry[stamp] = max(latest_entry.get(stamp, 0), row["entered_us"])

    @lru_cache(None)
    def suffix(index, left):
        if index == len(jobs):
            return [()]
        if len(jobs) - index > 8 * left:
            return []
        result = []
        earliest_entry = latest_entry[jobs[index]["claimed_us"]]
        for end in range(index, min(index + 8, len(jobs))):
            row = jobs[end]
            earliest_entry = min(earliest_entry, latest_entry[row["claimed_us"]])
            if row["worker_pid"] != jobs[index]["worker_pid"] or earliest_entry < row["claimed_us"]:
                break
            for tail in suffix(end + 1, left - 1):
                result.append((end + 1,) + tail)
        return result
    return suffix(0, maximum)


def arm(raw, score):
    digest = sha256(raw.read_bytes()).hexdigest()
    assert digest == score["raw_sha256"]
    with tarfile.open(raw) as archive:
        def read(name):
            return json.load(archive.extractfile("adapter/" + name + ".json"))
        ledger = read("ledger")
        sql = read("queue-policy")["queue_statements"]
        cursor, = [s for s in sql if "FROM mesh_jobs.claim_batch_v1(" in s["query"]]
        hints = sum(json.loads(archive.extractfile(f"adapter/runner-{i}.log").read().splitlines()[-1])
                    ["batch_contention_hints"] for i in range(8))
    jobs = sorted(ledger["jobs"], key=lambda j: (j["claimed_us"], j["job_key"]))
    stamps = [j["claimed_us"] for j in jobs]
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 64
    assert all(a % 1000 == 0 for a in stamps)
    assert all(a["worker_pid"] == b["worker_pid"] and min(a["entered_us"], b["entered_us"]) >= b["claimed_us"] + 1000
               for a, b in zip(jobs, jobs[1:]) if a["claimed_us"] == b["claimed_us"])
    tx, = ledger["transactions"]
    commit = tx["committed_us"]
    settled = max(j["settled_us"] for j in jobs)
    assert round(score["drain_ms"] * 1000) == settled - commit
    assert cursor["rows"] - len(jobs) == hints - score["set_timeouts"]
    # Each extra row is a one-row hint. Successful zero-row calls still fit this upper bound.
    cap = cursor["calls"] - (cursor["rows"] - len(jobs))
    options = partitions(jobs, cap)
    assert options
    breakdowns = []
    for ends in options:
        starts = (0,) + ends[:-1]
        interior = sum(stamps[e-1] - stamps[s] for s, e in zip(starts, ends))
        between = sum(stamps[e] - stamps[e-1] for e in ends[:-1])
        assert stamps[0] - commit + interior + between + settled - stamps[-1] == settled - commit
        breakdowns.append((interior, between))
    handoffs = None
    if len(options) == 1:
        handoffs = []
        start = 0
        for end in options[0][:-1]:
            assert stamps[end-1] < stamps[end]
            entry = min(j["entered_us"] for j in jobs[start:end])
            next_stamp = stamps[end]
            handoffs.append(dict(
                after_prior_entry_us=max(0, next_stamp-entry),
                first_writes=sum(entry <= j["attempt_committed_us"] < next_stamp for j in jobs),
                effect_writes=sum(entry <= j["effect_committed_us"] < next_stamp for j in jobs),
                settlements=sum(entry <= j["settled_us"] < next_stamp for j in jobs)))
            start = end
    # The positive windows are disjoint because each ends at the next turn's
    # first stamp and that turn's own first handler enters only after COMMIT.
    after_entry = {key: sum(row[key] for row in handoffs) for key in handoffs[0]} if handoffs else None
    return dict(raw_sha256=digest, owners=len({j["worker_pid"] for j in jobs}), partitions=len(options),
                after_previous_handler_entry=after_entry,
                max_productive_calls=cap, min_productive_calls=min(map(len, options)),
                widths=[b-a for a, b in zip((0,) + options[0][:-1], options[0])] if len(options) == 1 else None,
                first_stamp_us=stamps[0]-commit, inside_us=[min(x[0] for x in breakdowns), max(x[0] for x in breakdowns)],
                between_us=[min(x[1] for x in breakdowns), max(x[1] for x in breakdowns)],
                after_last_stamp_us=settled-stamps[-1], drain_us=settled-commit)


def replay(evidence, canonical):
    release_bytes = canonical.read_bytes()
    assert (evidence / "controller-release.json").read_bytes() == release_bytes
    assert sha256(release_bytes).hexdigest() == "9db80b688c9b1728a0588385e71fe4a49597ecddf24ec895722f495b64653f78"
    scores = (evidence / "scored-before-release.json").read_bytes()
    assert sha256(scores).hexdigest() == "87179bd17fb8219c807843c770daf501b2d3e8b825fa93915a99847841d72102"
    official = json.loads(scores)
    release = json.loads(release_bytes)
    assert release["request_sha256"] == REQUEST and release["exit_status"] == 0
    result = {}
    for policy in ("external-4m", "default"):
        result[policy] = {}
        reports = sorted((evidence / policy).glob("blocks/*/reconciled.json"))
        assert len(reports) == 2
        for report in reports:
            block = json.loads(report.read_bytes())
            assert block == official["policies"][policy]["groups"][block["block"] - 1]
            phases = {}
            for phase in ("warmup", "measured"):
                lanes = {}
                for name, score in block["arms"][phase].items():
                    raw, = report.parent.glob(f"{phase}-{name}/*/raw.tar.gz")
                    lanes[name] = arm(raw, score)
                assert set(lanes) == {"bridge", "stock-a", "stock-b", "eager", "span-a", "span-b"}
                phases[phase] = lanes
            result[policy][block["block"]] = phases
    return dict(release_sha256=sha256(release_bytes).hexdigest(), policies=result)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("release", type=Path)
    args = parser.parse_args()
    print(json.dumps(replay(args.evidence, args.release), indent=2, sort_keys=True))
