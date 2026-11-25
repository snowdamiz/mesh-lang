"""Replay protected LARGE milestones from the released Linode archives."""
import hashlib
import json
from pathlib import Path
from statistics import median
import tarfile

SOURCE = Path(__file__).resolve().parent.parent / "2026-11-23-large-v3-timing-955b356b"
STOCK = "mesh-stock"
PREPARED = "mesh-pg17-exact"
MILESTONES = (
    ("entered_us", 8),
    ("effect_committed_us", 8),
    ("effect_committed_us", 32),
    ("effect_committed_us", 56),
    ("effect_committed_us", 64),
    ("settled_us", 64),
)


def read_milestones(archive):
    with tarfile.open(archive) as stream:
        ledger = json.load(stream.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    assert len({job["id"] for job in jobs}) == len(jobs) == 64
    assert all(job["attempts"] == job["effects"] == 1 for job in jobs)
    start = min(row["committed_us"] for row in ledger["transactions"])
    return {f"{key}_{rank}": sorted(job[key] for job in jobs)[rank - 1] - start
            for key, rank in MILESTONES}


def replay():
    release = json.loads((SOURCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["groups"] == list(range(1, 6))
    groups = {}
    scores = []
    for path in sorted(SOURCE.glob("blocks/*/reconciled.json")):
        group = int(path.parent.name.split("-")[0])
        assert group not in groups
        result = json.loads(path.read_bytes())
        assert len(result["raw_sha256"]) == 8
        for name, digest in result["raw_sha256"].items():
            raw = path.parent / name / "raw.tar.gz"
            assert hashlib.sha256(raw.read_bytes()).hexdigest() == digest
        milestones = {}
        for lane in (STOCK, PREPARED):
            folder = f"measured-{lane}/measured-mesh"
            assert folder in result["raw_sha256"]
            milestones[lane] = read_milestones(path.parent / folder / "raw.tar.gz")
        groups[group] = {key: value - milestones[PREPARED][key]
                         for key, value in milestones[STOCK].items()}
        scores.append(result["scores"])
    assert sorted(groups) == release["groups"]
    comparisons = {}
    for lane in (STOCK, "river", "oban"):
        comparisons[lane] = {}
        for clock in ("submit_jobs_per_s", "commit_jobs_per_s"):
            ratios = [g[PREPARED][clock] / g[lane][clock] for g in scores]
            comparisons[lane][clock] = {
                "wins": sum(x > 1 for x in ratios),
                "median_gain_percent": round((median(ratios) - 1) * 100, 2),
            }
    return {"stock_minus_prepared_from_commit_us": groups, "throughput": comparisons}


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
