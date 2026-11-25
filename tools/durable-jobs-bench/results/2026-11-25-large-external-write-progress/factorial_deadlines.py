"""Replay the FILE bridge and price earlier writes against historical peer deadlines."""
from hashlib import sha256
import json
from pathlib import Path
import sys
import tarfile

from producer_window import window
from replay import replay as replay_baseline


def checked_json(path, digest):
    content = path.read_bytes()
    assert sha256(content).hexdigest() == digest, path
    return json.loads(content)


def run(pilot):
    release = checked_json(pilot / "controller-release.json",
                           "89252dc00d74985d5362dfee823041f39f3079e6a7378d57d83dae314683d5b8")
    score = checked_json(pilot / "scored-before-release.json",
                         "ff2f7f59835cd8767309d2f8c54636647c21e6898675fb84d40f10691be25a13")
    assert release["exit_status"] == 0 and score["advanced"] is False
    assert [g["gates"]["passed"] for g in score["groups"]] == [True, False]
    historical = replay_baseline()["blocks"]
    deadlines = {}
    for peer in ("river", "oban", "apalis"):
        lanes = [block[peer] for block in historical.values()]
        deadlines[peer] = [
            (lane["submit_us"] - lane["stages_us"]["enqueue"]) * 5 // 6 for lane in lanes
        ]
    ticks = json.loads((pilot / "series.json").read_bytes())["config"]["clock_ticks"]
    controls, combined = [], []
    for group in score["groups"]:
        n = group["group"]
        for phase in ("warmup", "measured"):
            for lane in ("bridge", "off-a", "off-b", "both"):
                raw, = pilot.glob(f"blocks/{n:02d}-*/{phase}-{lane}/*/raw.tar.gz")
                digest = group["raw_sha256"][phase][lane]
                sample = window(raw, digest, ticks)
                with tarfile.open(raw) as archive:
                    ledger = json.load(archive.extractfile("adapter/ledger.json"))
                    writer = json.load(archive.extractfile("adapter/producer-writer.json"))
                assert writer["scalar"] == 64 and writer["bulk"] == 0
                txn, = ledger["transactions"]
                commit = txn["committed_us"]
                jobs = ledger["jobs"]
                assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 128
                drain = max(j["settled_us"] for j in jobs) - commit
                if phase == "measured":
                    arm = group["arms"][lane]
                    assert drain == round(arm["drain_ms"] * 1000)
                    assert sample["enqueue_us"] == round(arm["enqueue_ms"] * 1000)
                if lane != "both":
                    controls.append(dict(group=n, phase=phase, lane=lane, producer_window=sample,
                                         drain_us=drain, commit_to_ack_us=txn["acknowledged_us"]-commit,
                                         runner_lifetime_cpu_ms=round(ledger["runner_cpu_seconds"] * 1000)))
                elif phase == "measured":
                    milestones = ("claimed_us", "entered_us", "attempt_committed_us",
                                  "effect_committed_us", "settled_us")
                    last = {key: max(j[key] for j in jobs) - commit for key in milestones}
                    comparisons = {}
                    for peer, limits in deadlines.items():
                        comparisons[peer] = []
                        for limit in limits:
                            after = {key: sum(j[key] - commit > limit for j in jobs)
                                     for key in milestones}
                            comparisons[peer].append(dict(deadline_us=limit, after=after,
                                min_effect_advance_us=max(0, last["effect_committed_us"] - limit)))
                    combined.append(dict(group=n, raw_sha256=digest, drain_us=drain,
                                         last_from_commit_us=last, comparisons=comparisons))
    return dict(historical_drain_deadlines_us=deadlines, controls=controls, combined=combined)


if __name__ == "__main__":
    print(json.dumps(run(Path(sys.argv[1])), indent=2, sort_keys=True))
