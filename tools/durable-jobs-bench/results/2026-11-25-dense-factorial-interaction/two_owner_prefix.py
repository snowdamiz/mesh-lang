"""Count adjacent grants from two eight-seat owners that could fit one checked SELECT."""
from collections import Counter, defaultdict
import json

from peer_grant_progress import EVIDENCE, SEAL, load


def screen(jobs, score):
    groups = defaultdict(list)
    by_owner = defaultdict(list)
    for job in jobs:
        groups[job["claimed_us"], job["worker_pid"]].append(job)
        by_owner[job["worker_pid"]].append(job)
    assert len(by_owner) == 8
    turns = sorted(groups.items())
    assert len(turns) == score["positive_grants"]
    assert all(1 <= len(group) <= 8 for group in groups.values())
    stamps = Counter(stamp for stamp, _ in groups)
    effects = sorted(job["effect_committed_us"] for job in jobs)
    middle = effects[255], effects[767]
    eligible = {}
    for width in (8, 16):
        for lag in (0, 4000, 8000):
            for phase in ("all", "middle"):
                # A path matching maximizes disjoint adjacent pairs, preserving order.
                two_back = previous = 0
                for ((left, owner), first), ((right, peer), second) in zip(turns, turns[1:]):
                    current = previous
                    unique_order = stamps[left] == stamps[right] == 1 and left < right
                    inside = phase == "all" or middle[0] <= left and right + 999 <= middle[1]
                    if unique_order and inside and owner != peer and len(first) + len(second) <= width:
                        # Charge prior seats until terminal commit plus the assumed
                        # receipt lag. Check at the start of the earlier stamp's ms.
                        occupied = sum(j["claimed_us"] < left < j["settled_us"] + lag
                                       for j in by_owner[peer])
                        if occupied + len(second) <= 8:
                            current = max(previous, two_back + 1)
                    two_back, previous = previous, current
                eligible[f"{width}-{lag}-{phase}"] = previous

    refills = []
    latest_terminal = {}
    for (stamp, owner), group in turns:
        free_at = latest_terminal.get(owner)
        if len(group) == 8 and free_at is not None and middle[0] <= free_at <= stamp and stamp + 999 <= middle[1]:
            intervening = [len(other) for (other_stamp, pid), other in turns
                           if pid != owner and free_at < other_stamp and other_stamp + 999 < stamp]
            refills.append(dict(gap_us=stamp - free_at, other_turns=len(intervening),
                                other_granted=sum(intervening)))
        latest_terminal[owner] = max(free_at or 0, max(j["settled_us"] for j in group))
    return dict(raw_sha256=score["raw_sha256"], productive_turns=len(turns),
                turns_at_tied_stamps=sum(n for n in stamps.values() if n > 1),
                eligible_disjoint_pairs=eligible, middle_full_refills=refills)


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["request_sha256"] == SEAL
    series = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    result = {}
    for index in range(1, 6):
        block = EVIDENCE / "blocks" / f"{index:02}-noop"
        scores = json.loads((block / "reconciled.json").read_bytes())
        assert scores == series[str(index)]
        score = scores["measured-B"]
        jobs, _, _ = load(block, "B", score)
        result[index] = screen(jobs, score)
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
