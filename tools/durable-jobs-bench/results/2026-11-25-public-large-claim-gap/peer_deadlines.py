"""Durable-clock budgets from five matched default-storage FILE rotations."""
from hashlib import sha256
import json
from statistics import median

from replay import LANES, SERIES, read_lane


def budgets():
    release = (SERIES.parent / "timing-adapter-path-retry-release.json").read_bytes()
    score = (SERIES.parent / "timing-adapter-path-retry-scores.json").read_bytes()
    assert sha256(release).hexdigest() == "6f0f87de9172b30857001e40ba355e91430b80ed94a62821b56ee3deafa9ea5b"
    assert sha256(score).hexdigest() == "2e2fcf74bd587fa15fa5fa5ebbd1b5c09bf1b86648bb700b52addbf427bb7f26"
    assert json.loads(release)["exit_status"] == 0
    released_groups = json.loads(score)["groups"]
    plan = json.loads((SERIES / "series.json").read_bytes())["plan"]
    assert [block["group"] for block in plan] == list(range(1, 6))
    blocks = []
    for block in plan:
        group = block["group"]
        path, = SERIES.glob(f"blocks/{group:02d}-large.*/reconciled.json")
        scores = json.loads(path.read_bytes())["scores"]
        released = released_groups[group - 1]
        assert released["group"] == group and scores == released["scores"]
        assert set(block["order"]) == set(scores) == LANES
        assert json.loads((path.parent / "order.json").read_bytes()) == block
        lanes = {}
        for label in ("file", "river", "oban", "apalis"):
            score = scores[label]
            jobs, commit, drain, boot = read_lane(path.parent, label, score)
            lanes[label] = dict(boot=boot, submit_us=score["last_settlement_us"] - score["first_submit_us"],
                                drain_us=drain, last_effect_us=max(j["effect_committed_us"] for j in jobs) - commit,
                                raw_sha256=score["raw_sha256"])
        assert len({lane["boot"] for lane in lanes.values()}) == 1
        mesh = lanes["file"]
        deadlines = {}
        for peer in ("river", "oban", "apalis"):
            clocks = {}
            for clock in ("submit_us", "drain_us"):
                elapsed = lanes[peer][clock]
                # Microsecond ledgers and equal job counts give an exact 6:5 throughput test.
                deadline = elapsed * 5 // 6
                clocks[clock] = dict(peer_us=elapsed, file_us=mesh[clock], deadline_us=deadline,
                                     advance_needed_us=max(0, mesh[clock] - deadline),
                                     file_at_target=mesh[clock] <= deadline)
            deadline = clocks["drain_us"]["deadline_us"]
            deadlines[peer] = dict(clocks=clocks, effect_advance_even_with_zero_tail_us=max(0, mesh["last_effect_us"] - deadline))
        blocks.append(dict(group=group, lanes=lanes, deadlines=deadlines))
    summary = {}
    for peer in ("river", "oban", "apalis"):
        entries = [row["deadlines"][peer] for row in blocks]
        summary[peer] = dict(both_clocks=sum(all(c["file_at_target"] for c in e["clocks"].values()) for e in entries),
                             minimum_effect_advance_median_us=median(e["effect_advance_even_with_zero_tail_us"] for e in entries))
    return dict(blocks=blocks, summary=summary)


if __name__ == "__main__":
    print(json.dumps(budgets(), indent=2, sort_keys=True))
