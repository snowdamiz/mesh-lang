"""Replay progress on the SHORT16 Mesh ledger at matched peer effect ranks."""
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import tarfile

EVIDENCE = Path(__file__).resolve().parent / "evidence"
LANES = ("r2-d2", "river-warm", "river", "oban")
RANKS = (16, 32, 64, 96, 112, 128)


def read_arm(group, label):
    key, = [k for k in group["raw_sha256"] if k.startswith(f"measured-{label}/")]
    raw = (EVIDENCE / "timing/blocks" / group["attempt"] / key / "raw.tar.gz").read_bytes()
    digest = sha256(raw).hexdigest()
    assert digest == group["raw_sha256"][key]
    with tarfile.open(fileobj=BytesIO(raw)) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    assert ledger["slots"] == 16 and ledger["runners"] == 1
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 128
    assert tx["enqueued_ids"] == {j["job_key"]: str(j["id"]) for j in jobs}
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 256
    for j in jobs:
        assert j["state"] == "completed" and j["attempt"] == j["attempts"] == j["effects"] == 1
        assert j["entered_us"] <= j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"]
        if label == "r2-d2":
            assert j["claimed_us"] % 1000 == 0 and j["claimed_us"] <= j["entered_us"]
    end = max(j["settled_us"] for j in jobs)
    for clock, score in (("submitted_us", "submit_to_settlement_ms"), ("committed_us", "commit_to_settlement_ms")):
        assert end - tx[clock] == round(1000 * group["lanes"][label][score])
    return jobs, tx, digest


def progress(jobs, cutoff):
    # The millisecond SQL claim stamp precedes confirmation. Entry provides an upper bound.
    return dict(stamped_min=sum(min(j["claimed_us"] + 999, j["entered_us"]) <= cutoff for j in jobs),
                stamped_max=sum(j["claimed_us"] <= cutoff for j in jobs),
                entered=sum(j["entered_us"] <= cutoff for j in jobs),
                first_committed=sum(j["attempt_committed_us"] <= cutoff for j in jobs),
                effects=sum(j["effect_committed_us"] <= cutoff for j in jobs),
                settled=sum(j["settled_us"] <= cutoff for j in jobs))


def terminal_backlog(jobs):
    effects = sorted(j["effect_committed_us"] for j in jobs)
    # Count durable second writes still awaiting their job's terminal transaction.
    backlog = [sum(j["effect_committed_us"] <= t < j["settled_us"] for j in jobs) for t in effects]
    return dict(peak=max(backlog), at_rank={rank: backlog[rank - 1] for rank in RANKS},
                terminal_xids=len({j["settled_xid"] for j in jobs}))


def replay():
    summary = json.loads((EVIDENCE / "summary.json").read_bytes())
    assert not summary["missing"] and [g["group"] for g in summary["rotations"]] == list(range(1, 7))
    result = []
    for group in summary["rotations"]:
        mesh, tx, digest = read_arm(group, LANES[0])
        hashes = {LANES[0]: digest}
        peers = {}
        backlogs = {LANES[0]: terminal_backlog(mesh)}
        assert backlogs[LANES[0]]["peak"] <= 16
        free_enqueue = {}
        inputs = {j["job_key"]: j["payload_sha256"] for j in mesh}
        for peer in LANES[1:]:
            jobs, peer_tx, hashes[peer] = read_arm(group, peer)
            assert inputs == {j["job_key"]: j["payload_sha256"] for j in jobs}
            effects = sorted(j["effect_committed_us"] for j in jobs)
            peers[peer] = {}
            backlogs[peer] = terminal_backlog(jobs)
            mesh_drain = max(j["settled_us"] for j in mesh) - tx["committed_us"]
            peer_submit = max(j["settled_us"] for j in jobs) - peer_tx["submitted_us"]
            free_enqueue[peer] = mesh_drain - peer_submit
            for clock in ("committed_us", "submitted_us"):
                peers[peer][clock] = {rank: progress(mesh, tx[clock] + effects[rank - 1] - peer_tx[clock])
                                      for rank in RANKS}
        result.append(dict(group=group["group"], peers=peers, terminal_backlog=backlogs,
                           mesh_drain_minus_peer_full_submit_us=free_enqueue, raw_sha256=hashes))
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
