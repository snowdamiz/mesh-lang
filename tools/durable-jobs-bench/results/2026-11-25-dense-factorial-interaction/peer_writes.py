"""Decompose the five independent Linode peer blocks using per-job commit records."""
from collections import Counter
from hashlib import sha256
import json
from pathlib import Path
from statistics import median
import tarfile

from replay import TRIAL

COUNTS = {"peak_effects_awaiting_terminal_commit", "terminal_transactions"}
PEERS = ("river-2x32", "oban-2x32", "apalis-2x32")


def intervals(folder, score):
    raw, = folder.glob("measured-*/raw.tar.gz")
    assert sha256(raw.read_bytes()).hexdigest() == score["raw_sha256"]
    with tarfile.open(raw) as tar:
        assert json.load(tar.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(tar.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    origin, = ledger["transactions"]
    assert len(jobs) == 1024 and len({r["job_key"] for r in jobs}) == 1024
    assert all(r["attempt_committed_us"] <= r["effect_committed_us"] <= r["settled_us"] for r in jobs)
    assert len({r[k] for r in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    effects = sorted(r["effect_committed_us"] for r in jobs)
    end = max(r["settled_us"] for r in jobs)
    submit, commit = origin["submitted_us"], origin["committed_us"]
    assert end - submit == score["submit_us"] and end - commit == score["drain_us"]
    times = [submit, commit, *(effects[n-1] for n in (64, 256, 512, 922, 1024)), end]
    names = ("enqueue", "first_64_effects", "effects_65_to_256", "effects_257_to_512",
             "effects_513_to_922", "last_102_effects", "final_tail")
    spans = {k: (b-a)/1000 for k,a,b in zip(names, times, times[1:])}
    assert round(sum(spans.values())*1000) == end - submit
    pending = Counter()
    for job in jobs:
        pending[job["effect_committed_us"]] += 1
        pending[job["settled_us"]] -= 1
    live = peak = 0
    for stamp in sorted(pending):
        live += pending[stamp]
        peak = max(peak, live)
        assert live >= 0
    assert live == 0
    return dict(spans, submit=(end-submit)/1000, drain=(end-commit)/1000,
                peak_effects_awaiting_terminal_commit=peak,
                terminal_transactions=len({r["settled_xid"] for r in jobs}))


def replay():
    releases = [json.loads(f.read_text()) for f in (TRIAL / "controller").glob("controller-timing-*.json")]
    assert any(r["exit_status"] == 0 and r["groups"] == list(range(3, 8)) for r in releases)
    comparisons = []
    for group in range(3, 8):
        block, = (TRIAL / "blocks").glob(f"{group:02}-noop.*")
        rows = json.loads((block / "reconciled.json").read_text())["measured"]
        assert set(rows) == {"combined", *PEERS}
        arms = {k: intervals(block / f"measured-{k}", v) for k,v in rows.items()}
        deltas = {peer: {k: round(arms[peer][k] - v, 3) for k,v in arms["combined"].items() if k not in COUNTS}
                  for peer in PEERS}
        comparisons.append(dict(group=group, arms=arms, paired_peer_minus_mesh_ms=deltas))
    medians = {p: {k: median(g["paired_peer_minus_mesh_ms"][p][k] for g in comparisons)
                   for k in comparisons[0]["paired_peer_minus_mesh_ms"][p]} for p in PEERS}
    return dict(blocks=comparisons, paired_medians_ms=medians)


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True, allow_nan=False))
