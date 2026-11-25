"""Price an args shape check against the stock FILE and Oban durable clocks."""
import hashlib
import json
from pathlib import Path
import tarfile


HERE = Path(__file__).resolve().parent
RUN = HERE.parent / "2026-11-25-public-large-file-external/grant-parser-retry/evidence/native"


def ledger(block, lane, score):
    raw, = block.glob(f"measured-{lane}/*/raw.tar.gz")
    digest = hashlib.sha256(raw.read_bytes()).hexdigest()
    assert digest == score["receipts"]["measured"][lane]["raw_sha256"]
    with tarfile.open(raw) as archive:
        result = json.load(archive.extractfile("adapter/ledger.json"))
        if lane == "file":
            storage = json.load(archive.extractfile("adapter/storage-topology.json"))
            assert storage["args_storage"] == "e" and storage["storage"] == "external-4m"
    jobs = result["jobs"]
    assert len(jobs) == 64 and len({j["job_key"] for j in jobs}) == 64
    assert all(j["state"] == "completed" and j["attempt"] == 1 for j in jobs)
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 128
    tx, = result["transactions"]
    assert all(tx["committed_us"] <= j["attempt_committed_us"] <= j["effect_committed_us"]
               <= j["settled_us"] for j in jobs)
    return jobs, tx["committed_us"], digest


def remaining(jobs, commit, deadline):
    late = [j for j in jobs if j["effect_committed_us"] - commit > deadline]
    return {
        "drain_deadline_us": deadline,
        "effects_still_outstanding": len(late),
        "workers_with_late_effects": len({j["worker_pid"] for j in late}),
        "advance_needed_at_last_effect_if_terminal_cost_zero_us":
            max(0, max(j["effect_committed_us"] for j in jobs) - commit - deadline),
    }


def main():
    release = json.loads((RUN / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0
    assert release["id"] == "2026-11-25-public-large-file-external-grant-parser-retry"
    groups = {}
    for path in sorted(RUN.glob("blocks/*/reconciled.json")):
        group = int(path.parent.name.split("-")[0])
        assert group not in groups
        score = json.loads(path.read_bytes())
        jobs, commit, file_sha = ledger(path.parent, "file", score)
        oban, peer_commit, oban_sha = ledger(path.parent, "oban", score)
        assert {j["job_key"]: j["payload_sha256"] for j in jobs} == {
            j["job_key"]: j["payload_sha256"] for j in oban
        }
        peer = max(j["settled_us"] for j in oban) - peer_commit
        last_effect = max(j["effect_committed_us"] for j in jobs)
        last_settlement = max(j["settled_us"] for j in jobs)
        groups[group] = {
            "file_sha256": file_sha, "oban_sha256": oban_sha,
            "file_drain_us": last_settlement - commit,
            "last_effect_to_last_settlement_us": last_settlement - last_effect,
            "tie_oban": remaining(jobs, commit, peer),
            "twenty_percent_more_jobs_per_second": remaining(jobs, commit, peer * 5 // 6),
        }
    assert sorted(groups) == [1, 2, 3, 4, 5]
    print(json.dumps(groups, indent=2))


if __name__ == "__main__":
    main()
