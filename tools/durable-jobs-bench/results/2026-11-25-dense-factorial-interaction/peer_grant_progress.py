"""Count released Mesh grants and application writes at matched peer deadlines."""
from hashlib import sha256
from io import BytesIO
import json
import tarfile

from grant_capacity_frontier import EVIDENCE, SEAL
from late_capacity_io import queue_timeout_records


def load(block, lane, score):
    raw, = block.glob(f"measured-{lane}/measured-*/raw.tar.gz")
    data = raw.read_bytes()
    assert sha256(data).hexdigest() == score["raw_sha256"]
    with tarfile.open(fileobj=BytesIO(data)) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        timeouts = []
        if lane == "B":
            lines = archive.extractfile("adapter/postgres.log").read().decode().splitlines()
            timeouts = queue_timeout_records(lines)
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    assert ledger["runners"] == 8 and ledger["slots"] == 64
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    assert tx["enqueued_ids"] == {j["job_key"]: str(j["id"]) for j in jobs}
    assert tx["committed_us"] == score["first_commit_us"] and tx["submitted_us"] == score["first_submit_us"]
    assert max(j["settled_us"] for j in jobs) == score["last_settlement_us"]
    for j in jobs:
        assert j["state"] == "completed" and j["attempt"] == j["attempts"] == j["effects"] == 1
        assert j["entered_us"] <= j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"]
        if lane == "B":
            assert j["claimed_us"] % 1000 == 0 and j["claimed_us"] <= j["entered_us"]
    return jobs, tx, timeouts


def mesh_at(jobs, cutoff):
    # First handler entry also proves that the (millisecond-truncated) claim occurred.
    earliest = [j["claimed_us"] for j in jobs]
    latest = [min(j["claimed_us"] + 999, j["entered_us"]) for j in jobs]
    return dict(unstamped_lower=sum(t > cutoff for t in earliest),
                unstamped_upper=sum(t > cutoff for t in latest),
                last_stamp_minus_cutoff_us=[max(earliest) - cutoff, max(latest) - cutoff],
                yet_to_enter=sum(j["entered_us"] > cutoff for j in jobs),
                yet_to_commit_first=sum(j["attempt_committed_us"] > cutoff for j in jobs),
                yet_to_commit_effect=sum(j["effect_committed_us"] > cutoff for j in jobs),
                yet_to_settle=sum(j["settled_us"] > cutoff for j in jobs))


def timeout_history(jobs, records, origin):
    rows = []
    for stamp, site, pid in records:
        # Include only observations inside the preceding 100 ms for every possible
        # instant in the error timestamp's millisecond. This does not time lock ownership.
        begin, end = stamp + 999 - 100_000, stamp
        grants = [j for j in jobs if begin <= j["claimed_us"]
                  and min(j["claimed_us"] + 999, j["entered_us"]) <= end]
        rows.append(dict(error_from_commit_us=stamp-origin, site=site, pid=pid,
                         granted_jobs=len(grants), granting_runners=len({j["worker_pid"] for j in grants}),
                         durable_effects=sum(begin <= j["effect_committed_us"] <= end for j in jobs)))
    return rows


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["request_sha256"] == SEAL
    series = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    rows = []
    for index in range(1, 6):
        block = EVIDENCE / "blocks" / f"{index:02}-noop"
        scores = json.loads((block / "reconciled.json").read_bytes())
        assert scores == series[str(index)]
        jobs, mesh_tx, records = load(block, "B", scores["measured-B"])
        inputs = {j["job_key"]: j["payload_sha256"] for j in jobs}
        peers = {}
        for peer in ("river", "oban", "apalis"):
            others, peer_tx, _ = load(block, peer, scores["measured-" + peer])
            assert inputs == {j["job_key"]: j["payload_sha256"] for j in others}
            effects = sorted(j["effect_committed_us"] for j in others)
            milestones = {f"effect_{n}": effects[n-1] for n in (64, 256, 512, 768, 1024)}
            milestones["settled"] = max(j["settled_us"] for j in others)
            peers[peer] = {clock: {name: mesh_at(jobs, mesh_tx[clock] + instant - peer_tx[clock])
                                   for name, instant in milestones.items()}
                           for clock in ("submitted_us", "committed_us")}
        rows.append(dict(group=index, raw_sha256={lane: scores["measured-" + lane]["raw_sha256"]
                         for lane in ("B", "river", "oban", "apalis")}, peers=peers,
                         timeout_windows=timeout_history(jobs, records, mesh_tx["committed_us"])))
    return rows


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
