"""Price pairing adjacent intact terminal requests from different dense runners."""
import hashlib
import json
import tarfile
from collections import defaultdict
from dataclasses import dataclass

from replay import EVIDENCE, record_commits


@dataclass(frozen=True)
class Terminal:
    owner: int
    xid: int
    stamp: int
    width: int
    release_after: int
    release_after_xid: int


def transactions(jobs):
    by_xid = defaultdict(list)
    for job in jobs:
        by_xid[int(job["settled_xid"])].append(job)
    prior = {}
    events = []
    stamps = record_commits(jobs)
    assert len(stamps) == 2048 + len(by_xid)
    for xid, members in sorted(by_xid.items()):
        owner, = {int(j["worker_pid"]) for j in members}
        previous_xid, previous_commit = prior.get(owner, (0, 0))
        assert previous_commit <= stamps[xid] and len(members) <= 8
        events.append(Terminal(owner, xid, stamps[xid], len(members),
                               max(previous_commit, max(j["effect_committed_us"] for j in members)),
                               max(previous_xid, max(int(j["effect_xid"]) for j in members))))
        prior[owner] = xid, stamps[xid]
    assert len(prior) == 8
    earliest = max(stamps.values())
    cutoff = {}
    for xid in sorted(stamps, reverse=True):
        earliest = min(earliest, stamps[xid])
        cutoff[xid] = earliest
    return events, cutoff


def adjacent_pairs(events, cutoff, span, lag):
    # Each stream is a path. First qualifying edge maximizes disjoint edges.
    ordered = sorted(events, key=lambda e: (e.stamp, e.xid))
    pairs = []
    i = 0
    while i + 1 < len(ordered):
        a, b = ordered[i:i + 2]
        first_xid = min(a.xid, b.xid)
        if (a.owner != b.owner and a.width + b.width <= 8 and b.stamp - a.stamp <= span
                and max(a.release_after_xid, b.release_after_xid) < first_xid
                and max(a.release_after, b.release_after) + lag <= cutoff[first_xid]):
            pairs.append((a, b))
            i += 2
        else:
            i += 1
    return pairs


def load_ledger(path, seal, *, with_queue=False):
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    assert digest == seal
    with tarfile.open(path) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        sql = json.load(archive.extractfile("adapter/queue-policy.json")) if with_queue else None
    jobs = ledger["jobs"]
    assert len(jobs) == len({j["id"] for j in jobs}) == 1024
    assert all(j["attempts"] == j["effects"] == 1 and j["state"] == "completed" for j in jobs)
    assert len({int(j[k]) for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    return ledger, sql


def clocks(ledger):
    tx, = ledger["transactions"]
    end = max(j["settled_us"] for j in ledger["jobs"])
    return {"submit": end - tx["submitted_us"], "drain": end - tx["committed_us"]}


def read_arm(path, seal):
    ledger, sql = load_ledger(path, seal, with_queue=True)
    events, cutoff = transactions(ledger["jobs"])
    assert len(events) == sql["terminal_xids"]
    scalar, = [r for r in sql["queue_statements"] if r["query"].startswith("SELECT coalesce(mesh_jobs.settle_state")]
    batch, = [r for r in sql["queue_statements"] if r["query"].startswith("SELECT ordinal::text FROM mesh_jobs.complete_batch_v1")]
    assert scalar["calls"] == sum(e.width == 1 for e in events)
    assert scalar["calls"] + batch["calls"] == len(events)
    assert scalar["calls"] + batch["rows"] == 1024
    owners = sorted({e.owner for e in events})
    paired = [[e for e in events if e.owner in owners[i:i + 2]] for i in range(0, 8, 2)]
    result = {}
    for label, streams in (("all_eight", [events]), ("four_pairs_by_pid", paired)):
        result[label] = {}
        for span in (250, 1000, 4000):
            result[label][span] = {}
            for lag in (0, 250, 1000):
                pairs = [p for s in streams for p in adjacent_pairs(s, cutoff, span, lag)]
                result[label][span][lag] = len(pairs)
    return {"raw_sha256": seal, "clocks_us": clocks(ledger), "terminal_transactions": len(events),
            "scalar_calls": scalar["calls"], "batch_calls": batch["calls"],
            "scalar_sql_elapsed_ms": scalar["execution_ms"], "batch_sql_elapsed_ms": batch["execution_ms"],
            "disjoint_pairs_by_stream_commit_spacing_and_readiness_delay_us": result}


def main():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0
    seals = json.loads((EVIDENCE / "summary.json").read_bytes())["raw_sha256_by_rotation"]
    groups = {}
    for group in range(1, 6):
        path, = EVIDENCE.glob(f"blocks/{group:02}-*/measured-B/*/raw.tar.gz")
        sources = seals[str(group)]
        mesh = read_arm(path, sources["measured-B"])
        peers = {}
        for lane in ("river", "oban"):
            raw, = path.parents[2].glob(f"measured-{lane}/*/raw.tar.gz")
            peer, _ = load_ledger(raw, sources[f"measured-{lane}"])
            peer_clocks = clocks(peer)
            peers[lane] = {
                "raw_sha256": sources[f"measured-{lane}"],
                "clocks_us": peer_clocks,
                "mesh_minus_peer_us": {clock: mesh["clocks_us"][clock] - elapsed
                                       for clock, elapsed in peer_clocks.items()},
                "saving_needed_for_20pct_throughput_lead_us": {
                    clock: max(0, mesh["clocks_us"][clock] - elapsed * 5 // 6)
                    for clock, elapsed in peer_clocks.items()
                },
            }
        mesh["matched_peers"] = peers
        groups[group] = mesh
    print(json.dumps(groups, indent=2))


if __name__ == "__main__":
    main()
