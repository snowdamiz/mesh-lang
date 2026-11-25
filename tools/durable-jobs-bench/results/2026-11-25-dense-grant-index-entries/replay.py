"""Inspect target index nodes in native plans and count small grant stamps."""
from collections import Counter
import hashlib
import json
from pathlib import Path
import tarfile

RESULTS = Path(__file__).resolve().parents[1]
PLAN = RESULTS / "2026-11-24-public-dense-bounded-grant/evidence/native"
PLAN_SHA = "3ad3ee1645690c99dbfd18627cbc22a673c9213698c2004098259f492df9f37f"
SERIES = {
    "grouped": "2026-11-24-dense-product-d2-transfer/evidence/native-closure-retry/summary-1795560178774805000.json",
    "individual": "2026-11-24-dense-individual-hot-pool/logger-off-retry/evidence/native/summary-1795570643531732000.json",
}


def load_member(path, digest, name):
    assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as archive:
        return json.load(archive.extractfile(name))


def nodes(node):
    yield node
    for child in node.get("Plans", ()):
        yield from nodes(child)


def target_access():
    raw, = PLAN.glob("blocks/*/*/*/raw.tar.gz")
    plans = load_member(raw, PLAN_SHA, "adapter/grant-plans.json")["plans"]
    out = {}
    for shape, modes in plans.items():
        out[shape] = {}
        for mode, pair in modes.items():
            plan = pair["stock"]["plan"]["Plan"]
            target, = (n for n in nodes(plan) if n.get("Subplan Name") == "CTE granted")
            assert target["Node Type"] == "ModifyTable" and target["Operation"] == "Update"
            index, = (n for n in nodes(target) if "Index Name" in n)
            out[shape][mode] = {
                "index": index["Index Name"],
                "node_type": index["Node Type"],
                "rows_per_loop": index["Actual Rows"],
                "loops": index["Actual Loops"],
                "index_rows_times_loops": index["Actual Rows"] * index["Actual Loops"],
                "granted": target["Actual Rows"],
            }
    return out


def observed_grants(summary):
    info = json.loads(summary.read_bytes())
    assert info["qualified_rotations"] == len(info["blocks"]) == 6
    assert not info["missing"] and not info["failures"]
    out = {}
    for rotation, block in info["blocks"].items():
        score = block["scores"]["measured-hot"]
        raw = summary.parent / "blocks" / block["attempt"] / "measured-hot/measured-mesh/raw.tar.gz"
        ledger = load_member(raw, score["raw_sha256"], "adapter/ledger.json")
        jobs = ledger["jobs"]
        assert len({j["id"] for j in jobs}) == len(jobs) == score["completed"] == 1024
        assert score["failed"] == 0 and len({j["worker_pid"] for j in jobs}) == ledger["runners"] == 2
        counts = Counter((j["worker_pid"], j["claimed_us"]) for j in jobs)
        small = [width for width in counts.values() if width <= 8]
        out[rotation] = {
            "raw_sha256": score["raw_sha256"],
            "runner_timestamp_groups": len(counts),
            "groups_with_at_most_eight_jobs": len(small),
            "jobs_in_those_groups": sum(small),
        }
    return out


if __name__ == "__main__":
    print(json.dumps({
        "native_stock_plans": target_access(),
        "native_grant_stamps": {k: observed_grants(RESULTS / v) for k, v in SERIES.items()},
    }, indent=2, sort_keys=True))
