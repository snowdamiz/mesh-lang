"""Price consecutive individual settlements from the five released public SLOW blocks."""
import json
import tarfile
from hashlib import sha256

from replay import STOCK


def read_arm(number, lane, score):
    raw, = STOCK.glob(f"blocks/{number:02}-*/measured-{lane}/*/raw.tar.gz")
    digest = sha256(raw.read_bytes()).hexdigest()
    assert digest == score["raw_sha256"]
    with tarfile.open(raw) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        if lane == "product":
            policy = json.load(archive.extractfile("adapter/queue-policy.json"))
            statement, = [s for s in policy["queue_statements"]
                          if s["query"].startswith("SELECT coalesce(mesh_jobs.settle_state(")]
            assert statement["calls"] == policy["terminal_xids"] == 512
    jobs = sorted(ledger["jobs"], key=lambda job: job["settled_us"])
    assert len(jobs) == len({j["id"] for j in jobs}) == 512
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 1024
    for job in jobs:
        assert job["state"] == "completed" and job["attempts"] == job["effects"] == 1
        assert job["attempt_committed_us"] + 100_000 <= job["effect_committed_us"] <= job["settled_us"]
    first_submit = min(tx["submitted_us"] for tx in ledger["transactions"])
    last_settlement = jobs[-1]["settled_us"]
    assert (first_submit, last_settlement) == (score["first_submit_us"], score["last_settlement_us"])
    return jobs, last_settlement - first_submit


def replay():
    source = (STOCK / "summary.json").read_bytes()
    assert sha256(source).hexdigest() == "763b9cb39bec01492a9cf6ab6240e5758ef90573345c6571fd5e2fb851d3eaa8"
    rows = []
    for number, group in enumerate(json.loads(source)["groups"], 1):
        jobs, mesh_us = read_arm(number, "product", group["product"])
        _, river_us = read_arm(number, "river", group["river"])
        assert len({j["settled_xid"] for j in jobs}) == len({j["settled_us"] for j in jobs}) == 512
        leads = [previous["settled_us"] - following["effect_committed_us"]
                 for previous, following in zip(jobs, jobs[1:])]
        counts = {str(delay): sum(lead > delay for lead in leads) for delay in (0, 1000, 4000)}
        gap = mesh_us - river_us
        rows.append(dict(block=number, mesh_raw_sha256=group["product"]["raw_sha256"],
                         river_raw_sha256=group["river"]["raw_sha256"], boundaries=len(leads),
                         next_effect_lead_exceeds_us=counts, mesh_submit_us=mesh_us,
                         river_submit_us=river_us, mesh_minus_river_us=gap,
                         needed_us_per_leading_effect=round(gap / counts["0"], 3) if gap > 0 else None))
    assert len(rows) == 5
    return rows


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
