"""Audit adjacent pairs of complete terminal transactions from one runner."""
import hashlib
import json
import tarfile
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path

EVIDENCE = (Path(__file__).resolve().parents[1] /
            "2026-11-25-public-dense-late-capacity/peer-comparison/evidence")


@dataclass(frozen=True)
class Group:
    xid: int
    committed: int
    latest_effect: int
    latest_effect_xid: int
    width: int


def record_commits(jobs):
    stamps = {}
    for row in jobs:
        for xid_key, stamp_key in (("attempt_xid", "attempt_committed_us"),
                                   ("effect_xid", "effect_committed_us"),
                                   ("settled_xid", "settled_us")):
            xid = int(row[xid_key])
            assert stamps.setdefault(xid, row[stamp_key]) == row[stamp_key]
    assert max(stamps) - min(stamps) < (1 << 31)
    return stamps


def match(groups, cutoff, span, lag):
    # Earliest qualifying adjacent edge gives maximum cardinality on a path.
    pairs = 0
    i = 0
    while i + 1 < len(groups):
        a, b = groups[i:i + 2]
        fits = (
            a.width + b.width <= 8
            and max(a.latest_effect_xid, b.latest_effect_xid) < a.xid
            and max(a.latest_effect, b.latest_effect) + lag <= cutoff[a.xid]
            and (span is None or b.committed - a.committed <= span)
        )
        if fits:
            pairs += 1
            i += 2
        else:
            i += 1
    return pairs


def score(path, expected):
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    assert digest == expected
    with tarfile.open(path) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        policy = json.load(archive.extractfile("adapter/queue-policy.json"))
    jobs = ledger["jobs"]
    assert len(jobs) == len({row["id"] for row in jobs}) == 1024
    assert ledger["runners"] == 8 and ledger["slots"] == 64
    assert all(row["attempts"] == row["effects"] == 1 and row["state"] == "completed"
               for row in jobs)
    members = defaultdict(list)
    for row in jobs:
        members[int(row["settled_xid"])].append(row)
    stamps = record_commits(jobs)
    assert len(stamps) == 2048 + len(members)
    # First SQL submission precedes its own XID assignment. A higher XID's
    # commit is another upper bound on the assignment time in this fixture.
    cutoff = {}
    earliest = max(stamps.values())
    for xid in sorted(stamps, reverse=True):
        earliest = min(earliest, stamps[xid])
        cutoff[xid] = earliest
    runners = defaultdict(list)
    for xid, rows in members.items():
        owners = {row["worker_pid"] for row in rows}
        assert len(owners) == 1 and 1 <= len(rows) <= 8
        runners[owners.pop()].append(Group(xid, stamps[xid],
            max(row["effect_committed_us"] for row in rows),
            max(int(row["effect_xid"]) for row in rows), len(rows)))
    assert len(runners) == 8
    for groups in runners.values():
        groups.sort(key=lambda g: g.xid)
        assert all(a.committed <= b.committed for a, b in zip(groups, groups[1:]))
    widths = Counter(len(rows) for rows in members.values())
    batch, = (q for q in policy["queue_statements"]
              if q["query"].startswith("SELECT ordinal::text FROM mesh_jobs.complete_batch_v1"))
    assert batch["calls"] + widths[1] == policy["terminal_xids"] == len(members)
    assert batch["rows"] + widths[1] == 1024
    counts = {}
    for lag in [0, 250, 1000]:
        counts[lag] = {}
        for span in [1000, 4000, 8000, None]:
            counts[lag][str(span) if span else "unrestricted"] = sum(
                match(groups, cutoff, span, lag) for groups in runners.values())
    return {"raw_sha256": digest, "terminal_xids": len(members),
            "width_histogram": dict(sorted(widths.items())), "pairs_by_lag_and_span_us": counts}


def main():
    seals = json.loads((EVIDENCE / "summary.json").read_text())["raw_sha256_by_rotation"]
    results = {}
    for rotation in range(1, 6):
        raw, = EVIDENCE.glob(f"blocks/{rotation:02}-*/measured-B/*/raw.tar.gz")
        results[rotation] = score(raw, seals[str(rotation)]["measured-B"])
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
