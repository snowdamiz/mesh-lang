"""Count runners with outstanding application work in matched public dense pilots."""
from collections import Counter, defaultdict
import hashlib
import io
import json
import tarfile

from eight_runner_cohorts import member
from public_bridge import blob


def account(jobs, begin, end, slots, round_claim_late):
    events = defaultdict(Counter)
    events[begin] = Counter()
    events[end] = Counter()
    for job in jobs:
        claim, entry, effect, terminal = (job[k] for k in (
            "claimed_us", "entered_us", "effect_committed_us", "settled_us"))
        assert claim <= entry <= job["attempt_committed_us"] <= effect <= terminal
        assert claim % 1000 == 0
        if round_claim_late:
            claim = min(entry, claim + 999)
        for name, a, b in (("before_entry", claim, entry), ("handler", entry, effect),
                           ("after_effect", effect, terminal)):
            left, right = max(begin, a), min(end, b)
            if left < right:
                events[left][name] += 1
                events[right][name] -= 1
    area = Counter()
    active = Counter()
    prev = begin
    for stamp, changes in sorted(events.items()):
        if active["handler"]:
            category = "entered_work"
        elif active["before_entry"]:
            category = "only_stamped_or_post_effect"
        elif active["after_effect"] >= slots:
            category = "full_runner_after_effect"
        elif active["after_effect"]:
            category = "only_post_effect_with_spare_seats"
        else:
            category = "no_stamped_job_awaiting_settlement"
        area[category] += stamp - prev
        active.update(changes)
        assert all(n >= 0 for n in active.values())
        prev = stamp
    assert not any(active.values()) and sum(area.values()) == end - begin
    return area


def capture(content, digest, runners):
    assert hashlib.sha256(content).hexdigest() == digest
    with tarfile.open(fileobj=io.BytesIO(content)) as raw:
        assert json.load(raw.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(raw.extractfile("adapter/ledger.json"))
    assert ledger["runners"] == runners and ledger["slots"] == 64
    tx, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert len(jobs) == len(set(tx["enqueued_ids"])) == len({j["id"] for j in jobs}) == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    by_runner = defaultdict(list)
    for job in jobs:
        assert job["state"] == "completed" and job["attempts"] == job["effects"] == 1
        by_runner[job["worker_pid"]].append(job)
    assert len(by_runner) == runners
    effects = sorted(j["effect_committed_us"] for j in jobs)
    begin, end = effects[255], effects[767]
    assert tx["committed_us"] < begin < end
    out = {}
    for edge, late in (("start_of_claim_ms", False), ("end_of_claim_ms", True)):
        total = Counter()
        for runner_jobs in by_runner.values():
            total.update(account(runner_jobs, begin, end, 64 // runners, late))
        assert sum(total.values()) == runners * (end - begin)
        out[edge] = dict(total)
    # Sweep handler intervals separately to distinguish system-wide gaps from
    # gaps confined to a subset of the runners.
    changes = defaultdict(Counter)
    changes[begin] = Counter()
    changes[end] = Counter()
    for pid, rows in by_runner.items():
        for job in rows:
            left = max(begin, job["entered_us"])
            right = min(end, job["effect_committed_us"])
            if left < right:
                changes[left][pid] += 1
                changes[right][pid] -= 1
    active = Counter()
    concurrency = Counter()
    previous = begin
    for stamp, deltas in sorted(changes.items()):
        if stamp > previous:
            concurrency[sum(n > 0 for n in active.values())] += stamp - previous
        active.update(deltas)
        assert all(n >= 0 for n in active.values())
        previous = stamp
    assert sum(concurrency.values()) == end - begin
    assert sum(n * us for n, us in concurrency.items()) == out["start_of_claim_ms"]["entered_work"]
    remaining = sum(j["claimed_us"] > end for j in jobs)
    assert remaining > 0
    return dict(raw_sha256=digest, middle_us=end - begin, later_claimed_jobs=remaining, runner_us=out, active_runner_elapsed_us=dict(concurrency))


def replay():
    eight = json.loads(member("replay.json"))
    two = json.loads(blob("replay.json"))
    assert json.loads(member("controller-release.json"))["exit_status"] == 0
    assert json.loads(blob("controller-release.json"))["exit_status"] == 0
    out = {"eight_runner": [], "two_runner": []}
    for n in (1, 2):
        arms = {}
        for lane in ("A1", "A2", "B", "C"):
            cell = f"measured-{lane}"
            data = member(f"blocks/{n:02}-noop/{cell}/measured-mesh/raw.tar.gz")
            arms[lane] = capture(data, eight["groups"][str(n)][cell]["raw_sha256"], 8)
        out["eight_runner"].append(arms)
        path, = [p for p in two["raw_sha256"] if f"/{n:02}-" in p and "/measured-B/" in p]
        out["two_runner"].append(capture(blob(path), two["raw_sha256"][path], 2))
    return out


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
