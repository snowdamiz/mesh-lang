"""Bound same-runner exit proximity in the checked individual dense 8×8 controls."""
import hashlib
import json
from collections import Counter, defaultdict
from itertools import combinations

from settlement_scans import RESULTS, archive, drain


def proximity(rows, window):
    possible = set()
    certain = set()
    pairs = []
    for left, right in combinations(rows, 2):
        a, b, first = left
        c, d, second = right
        if max(a - d, c - b) <= window:
            possible.update((first, second))
        if max(b - c, d - a) <= window:
            certain.update((first, second))
            pairs.append((max(b, d), min(b, d), first, second))
    disjoint = set()
    for _, _, first, second in sorted(pairs):
        if first not in disjoint and second not in disjoint:
            disjoint.update((first, second))
    assert disjoint <= certain <= possible
    return Counter(with_certain_neighbor=len(certain), disjoint_certain_pairs=len(disjoint) // 2,
                   without_possible_neighbor=len(rows) - len(possible))


def replay():
    root = RESULTS / "2026-11-25-public-dense-stock-logical-key/continuation/evidence/native"
    data = (root / "replay.json").read_bytes()
    assert hashlib.sha256(data).hexdigest() == "42243e26b4b590a46eda3b597aab945d9944d44b54846c346801b1b8c23612d6"
    scores = json.loads(data)
    assert not scores["fair_8x8_peers_eligible"]
    blocks = {}
    for group in (1, 2):
        blocks[group] = {}
        for lane, score in scores["groups"][str(group)]["arms"].items():
            with archive(root, group, lane, score["raw_sha256"]) as raw:
                _, _, jobs = drain(raw)
            assert len({j["settled_xid"] for j in jobs}) == len(jobs)
            runners = defaultdict(list)
            for j in jobs:
                assert j["state"] == "completed" and j["attempt"] == 1
                assert j["claimed_us"] <= j["entered_us"] <= j["effect_committed_us"] <= j["settled_us"]
                runners[j["worker_pid"]].append((j["effect_committed_us"], j["settled_us"], j["id"]))
            assert len(runners) == 8
            assert {p: len(rows) for p, rows in runners.items()} == {r["pid"]: r["jobs"] for r in score["runners"]}
            counts = Counter()
            for rows in runners.values():
                counts.update(proximity(rows, 8000))
            blocks[group][lane] = dict(raw_sha256=score["raw_sha256"], **counts)
    return blocks


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
