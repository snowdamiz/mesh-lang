"""Bracket confirmed work awaiting handler entry on each dense runner."""
from bisect import bisect_right
from collections import Counter, defaultdict
import json

from peer_grant_progress import EVIDENCE, SEAL, load


def account(jobs, positive_grants):
    turns = defaultdict(list)
    for job in jobs:
        turns[job["worker_pid"], job["claimed_us"]].append(job)
    assert len(turns) == positive_grants and max(map(len, turns.values())) <= 8
    owners = sorted({owner for owner, _ in turns})
    assert len(owners) == 8
    effects = sorted(j["effect_committed_us"] for j in jobs)
    begin, end = effects[255], effects[767]
    events = defaultdict(Counter)
    events[end] = Counter()

    def record(owner, name, start, finish):
        start, finish = max(start, begin), min(finish, end)
        if start < finish:
            events[start][owner, name] += 1
            events[finish][owner, name] -= 1

    all_stamps = sorted({stamp for _, stamp in turns})
    local_stamps = {owner: sorted(s for pid, s in turns if pid == owner) for owner in owners}

    def earlier_witness(stamps, stamp, first_entry):
        index = bisect_right(stamps, stamp)
        return min(first_entry, stamps[index] + 999) if index < len(stamps) else first_entry

    for (owner, stamp), cohort in turns.items():
        first_entry = min(j["entered_us"] for j in cohort)
        assert stamp <= first_entry
        # The next claim from this owner requires receipt of the previous COMMIT.
        # Any later claim on the queue requires its lock to have been released.
        client = earlier_witness(local_stamps[owner], stamp, first_entry)
        visible = earlier_witness(all_stamps, stamp, first_entry)
        for job in cohort:
            entry = job["entered_us"]
            record(owner, "possible", stamp, entry)
            record(owner, "visible", visible, entry)
            record(owner, "client", client, entry)
            record(owner, "confirmed", first_entry, entry)
            record(owner, "entered", entry, job["effect_committed_us"])

    occupancy = Counter()
    duration = Counter()
    previous = begin
    active = Counter()
    for now, delta in sorted(events.items()):
        elapsed = now - previous
        active_owners = sum(active[owner, "entered"] > 0 for owner in owners)
        duration[f"{active_owners}_entered_owners"] += elapsed
        for key in ("confirmed", "client", "visible", "possible"):
            for max_entered in (0, 1):
                eligible = [owner for owner in owners if active[owner, key] > 0
                            and active[owner, "entered"] <= max_entered]
                prefix = f"{key}_waiting_with_at_most_{max_entered}_entered"
                occupancy[prefix + "_runner_us"] += len(eligible) * elapsed
                occupancy[prefix + "_job_us"] += sum(active[owner, key] for owner in eligible) * elapsed
                occupancy[prefix + "_any_runner_wall_us"] += bool(eligible) * elapsed
            occupancy[key + "_waiting_job_us"] += sum(active[owner, key] for owner in owners) * elapsed
        for owner in owners:
            assert (0 <= active[owner, "confirmed"] <= active[owner, "client"]
                    <= active[owner, "visible"] <= active[owner, "possible"])
        previous = now
        active.update(delta)
        assert all(value >= 0 for value in active.values())
    assert sum(duration.values()) == end - begin and not any(active.values())
    return dict(middle_us=end-begin, occupancy=dict(occupancy), entered_owners_us=dict(duration))


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["request_sha256"] == SEAL
    scores = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    result = {}
    for i, arms in scores.items():
        block = EVIDENCE / "blocks" / f"{int(i):02}-noop"
        assert json.loads((block / "reconciled.json").read_bytes()) == arms
        lanes = {}
        for lane in ("B", "A1", "A2"):
            score = arms[f"measured-{lane}"]
            jobs, _, _ = load(block, lane, score)
            lanes[lane] = dict(raw_sha256=score["raw_sha256"], **account(jobs, score["positive_grants"]))
        result[i] = lanes
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
