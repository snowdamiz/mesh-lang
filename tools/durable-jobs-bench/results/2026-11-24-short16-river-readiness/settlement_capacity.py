"""Count durable writes preceding final row commits in the six SHORT16 rotations."""
from collections import defaultdict
from hashlib import sha256
import json

from peer_effect_progress import EVIDENCE, read_arm


def count_arm(jobs):
    writes = sorted(row["effect_committed_us"] for row in jobs)
    start, end = writes[15], writes[-1]
    seats = 16
    phases = dict(before_first=0, required_hold=0, excess_between=0, effect_to_terminal=0)

    def overlap(left, right):
        return max(0, min(end, right) - max(start, left))

    terminal = defaultdict(list)
    peaks = []
    later_first_writes = 0
    for row in jobs:
        entered, first, effect, settled = (
            row[key] for key in ("entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")
        )
        assert row["hold_ms"] == 1 and first + 1000 <= effect
        phases["before_first"] += overlap(entered, first)
        phases["required_hold"] += overlap(first, first + 1000)
        phases["excess_between"] += overlap(first + 1000, effect)
        phases["effect_to_terminal"] += overlap(effect, settled)
        terminal[row["settled_xid"]].append((effect, settled))
        earlier = sum(j["attempt_committed_us"] < first < j["settled_us"] for j in jobs)
        later_first_writes += earlier >= seats
        peaks.append(sum(j["attempt_committed_us"] <= first < j["settled_us"] for j in jobs))
    groups = []
    for members in terminal.values():
        stamps = {settled for _, settled in members}
        assert len(stamps) == 1
        groups.append((stamps.pop(), max(effect for effect, _ in members)))
    groups.sort()
    # Final row xmin is a DB commit. The worker may observe the receipt later.
    preceding = [a[0] - b[1] for a, b in zip(groups, groups[1:]) if a[0] < b[0]]
    phases["outside_entry_to_effect"] = seats * (end - start) - sum(
        phases[key] for key in ("before_first", "required_hold", "excess_between")
    )
    return dict(middle_us=end - start, middle_job_us=phases, terminal_xids=len(groups),
                peak_first_committed_awaiting_terminal=max(peaks),
                first_commits_with_16_strictly_earlier_pending=later_first_writes,
                effects_before_first_terminal=sum(effect < groups[0][0] for effect in writes),
                adjacent_terminal_pairs_with_strict_commit_order=len(preceding),
                pairs_with_all_next_effects_before_previous_commit=sum(gap > 0 for gap in preceding))


def replay():
    source = (EVIDENCE / "summary.json").read_bytes()
    assert sha256(source).hexdigest() == "306ca0b0b97d50a653cd4fdb8255cb7e7deb1b3b051f189d3a124af0ee33fea4"
    result = []
    for group in json.loads(source)["rotations"]:
        lanes = {}
        expected_payloads = None
        for label in sorted(group["lanes"]):
            jobs, _, digest = read_arm(group, label)
            payloads = {j["job_key"]: j["payload_sha256"] for j in jobs}
            if expected_payloads is None:
                expected_payloads = payloads
            assert payloads == expected_payloads
            lanes[label] = dict(raw_sha256=digest, **count_arm(jobs))
        assert len(lanes) == 7
        result.append(dict(group=group["group"], lanes=lanes))
    assert [g["group"] for g in result] == list(range(1, 7))
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
