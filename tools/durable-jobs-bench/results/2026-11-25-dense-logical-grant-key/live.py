"""Reduce the target and join counters from the first native drain."""
from hashlib import sha256
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "2026-11-25-dense-live-grant-plans"
BLOCK = SOURCE / "procfd-recovery/evidence/native/blocks/01-noop"


def plan_key(sample):
    return (sample["log_line"], sample["end_us"], sample["pid"], sample["query_id"])


def walk(node):
    yield node
    for child in node.get("Plans", []):
        yield from walk(child)


def reduce_capture():
    receipt = json.loads((SOURCE / "evidence/stock-capture-replay.json").read_bytes())
    block = (BLOCK / "reconciled.json").read_bytes()
    assert sha256(block).hexdigest() == receipt["group_one"]["reduction_sha256"]
    arm = json.loads(block)["arms"]["sampled"]
    raw = BLOCK / "sampled/measured-mesh/raw.tar.gz"
    assert sha256(raw.read_bytes()).hexdigest() == arm["raw_sha256"] == receipt["group_one"]["raw_sha256"]["sampled"]
    with tarfile.open(raw) as archive:
        plans = {plan_key(p): p for p in json.load(archive.extractfile("adapter/grant-plans.json"))}
    totals = dict(samples=0, grants=0, target_rows=0, join_rejections=0, target_buffer_hits=0)
    for sample in arm["observation"]["samples"]:
        if (sample["category"], sample["window"]) != ("grant", "drain"):
            continue
        actual = plans[plan_key(sample)]
        assert len(sample["statement_keys"]) == 1
        assert sample["statement_keys"][0]["queryid"] == 6827047345048828466
        assert actual["plan"] == sample["plan"] and actual["pid"] == sample["pid"]
        end = actual["end_us"]
        assert arm["first_commit_us"] <= end - 1000 - 1000 * actual["duration_ms"]
        assert end + 1000 <= arm["last_settlement_us"]
        update, = (n for n in walk(actual["plan"]) if n.get("Subplan Name") == "CTE granted")
        target, = (n for n in walk(update) if n.get("Node Type") == "Index Scan")
        assert target.get("Relation Name") == "jobs" and target.get("Alias") == "j_1"
        join, = (n for n in walk(update) if n.get("Join Filter") == "(s_1.id = j_1.id)")
        assert target["Index Cond"] == "(j_1.namespace = $1)" and target["Index Name"] == "jobs_pkey"
        assert target["Actual Loops"] == join["Actual Loops"] == actual["plan"]["Actual Loops"] == 1
        totals["samples"] += 1
        totals["grants"] += actual["plan"]["Actual Rows"]
        totals["target_rows"] += target["Actual Rows"]
        totals["join_rejections"] += join["Rows Removed by Join Filter"]
        totals["target_buffer_hits"] += target["Shared Hit Blocks"]
    assert totals["samples"] == receipt["group_one"]["drain_plan_samples"]
    return dict(raw_sha256=arm["raw_sha256"], **totals)


if __name__ == "__main__":
    print(json.dumps(reduce_capture(), indent=2))
