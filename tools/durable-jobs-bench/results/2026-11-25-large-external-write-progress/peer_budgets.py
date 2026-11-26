"""Place each stock peer's 1.20x throughput deadlines on the public FILE ledger."""
from hashlib import sha256
import json
import tarfile

from replay import NATIVE, replay


def budgets():
    blocks = replay()["blocks"]
    result = {}
    for index, arms in blocks.items():
        mesh = arms["file"]
        raw, = NATIVE.glob(f"blocks/{index:02}-*/measured-file/*/raw.tar.gz")
        assert sha256(raw.read_bytes()).hexdigest() == mesh["raw_sha256"]
        with tarfile.open(raw) as archive:
            ledger = json.load(archive.extractfile("adapter/ledger.json"))
        transaction, = ledger["transactions"]
        jobs = ledger["jobs"]
        assert len(jobs) == len({j["job_key"] for j in jobs}) == 64
        assert all(j["state"] == "completed" and j["attempts"] == j["effects"] == 1 for j in jobs)
        assert all(j["claimed_us"] % 1000 == 0 and
                   j["claimed_us"] <= j["entered_us"] <= j["attempt_committed_us"] <=
                   j["effect_committed_us"] <= j["settled_us"] for j in jobs)
        drain = max(j["settled_us"] for j in jobs) - transaction["committed_us"]
        assert drain == mesh["submit_us"] - mesh["stages_us"]["enqueue"]
        peers = {}
        for name in ("river", "oban", "apalis"):
            peer = arms[name]
            target_submit = peer["submit_us"] * 5 // 6
            target_drain = (peer["submit_us"] - peer["stages_us"]["enqueue"]) * 5 // 6
            cutoff = transaction["committed_us"] + target_drain
            late = [j for j in jobs if j["claimed_us"] > cutoff]
            possible = [j for j in jobs if min(j["claimed_us"] + 999, j["entered_us"]) > cutoff]
            peers[name] = dict(
                peer_raw_sha256=peer["raw_sha256"], target_submit_us=target_submit,
                target_drain_us=target_drain, submit_saving_needed_us=mesh["submit_us"]-target_submit,
                drain_saving_needed_us=drain-target_drain,
                last_stamp_minus_cutoff_us=max(j["claimed_us"] for j in jobs)-cutoff,
                late_claim_jobs_min=len(late), late_claim_jobs_max=len(possible),
                late_claim_owners_min=len({j["worker_pid"] for j in late}),
                entered_by_cutoff=sum(j["entered_us"] <= cutoff for j in jobs),
                effects_by_cutoff=sum(j["effect_committed_us"] <= cutoff for j in jobs),
                settled_by_cutoff=sum(j["settled_us"] <= cutoff for j in jobs),
            )
        result[index] = dict(mesh_raw_sha256=mesh["raw_sha256"],
                             productive_runners=len({j["worker_pid"] for j in jobs}), peers=peers)
    return result


if __name__ == "__main__":
    print(json.dumps(budgets(), sort_keys=True, indent=2))
