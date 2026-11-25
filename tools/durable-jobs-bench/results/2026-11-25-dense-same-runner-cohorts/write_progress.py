"""Compare rank-to-rank progress without assuming jobs finish in grant order."""
import hashlib
import json
import statistics
import tarfile
from pathlib import Path

EVIDENCE = Path(__file__).resolve().parents[1] / (
    "2026-11-25-public-dense-late-capacity/peer-comparison/evidence"
)
STAGES = ("claimed_us", "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")


def lane(path, digest, mesh):
    assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as raw:
        ledger = json.load(raw.extractfile("adapter/ledger.json"))
        if mesh:
            statements = json.load(raw.extractfile("adapter/trusted-application-statements.json"))
            assert sorted((q["calls"], q["rows"]) for q in statements
                          if q["query"].startswith("INSERT INTO")) == [(1024, 1024)] * 2
            assert sum(q["calls"] for q in statements if q["query"] == "SELECT $1") == 16
    jobs = ledger["jobs"]
    origin, = ledger["transactions"]
    assert len({j["id"] for j in jobs}) == len(jobs) == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    assert all(j["attempts"] == j["effects"] == 1 and j["state"] == "completed"
               and j["entered_us"] <= j["attempt_committed_us"] <= j["effect_committed_us"]
               <= j["settled_us"] for j in jobs)
    progress = {}
    for stage in STAGES:
        stamps = sorted(j[stage] for j in jobs)
        progress[stage] = {"middle512_us": stamps[767] - stamps[255],
                           "last_from_enqueue_us": stamps[-1] - origin["committed_us"]}
    return {"sha256": digest, "progress": progress,
            "median_entry_to_attempt_us": statistics.median(
                j["attempt_committed_us"] - j["entered_us"] for j in jobs),
            "median_attempt_to_effect_us": statistics.median(
                j["effect_committed_us"] - j["attempt_committed_us"] for j in jobs)}


def main():
    seals = json.loads((EVIDENCE / "summary.json").read_text())["raw_sha256_by_rotation"]
    results = {}
    for block in range(1, 6):
        lanes = {}
        for name in ("B", "river"):
            path, = EVIDENCE.glob(f"blocks/{block:02}-*/measured-{name}/*/raw.tar.gz")
            lanes[name] = lane(path, seals[str(block)][f"measured-{name}"], name == "B")
        gap = {}
        for stage in STAGES:
            a, b = (lanes[name]["progress"][stage] for name in ("B", "river"))
            gap[stage] = {k: a[k] - b[k] for k in a}
        results[block] = dict(lanes=lanes, mesh_minus_river_us=gap)
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
