"""Check the archived SQL component, including raw CPU ticks and TOAST identities."""
import argparse
import hashlib
import json
import math
import re
import tarfile
from pathlib import Path
from statistics import median
from uuid import UUID

HERE = Path(__file__).resolve().parent
DIGEST = "86fbe1a01ef0db7d37b859fc362ed19ae808c8a60381de887f1a8faf4ced1248"
ARGS_WRITE_DIGEST = "82dfdc1f0c3e9b5bd14612f74512f8e899ccad301cd9af0db0d03e8ba7afa472"
STAGES = ("enqueue", "claim", "settle")
MIB = 1024**2


def metrics(receipt, scale):
    assert receipt["terminal"] == [["completed", "1", "64", "64"]]
    assert len(receipt["receipt_ids"]) == len(set(map(UUID, receipt["receipt_ids"]))) == 64
    digests = receipt["contexts_sha256"]
    assert len(digests) == len(set(digests)) == 64 and all(re.fullmatch("[0-9a-f]{64}", h) for h in digests)
    assert int(receipt["submitted_xid"]) > 0 and receipt["committed"]
    settings = receipt["settings"]
    assert settings["server_version_num"] == "170011" and settings["work_mem"] == "16384"
    assert all(settings[k] == "on" for k in ("fsync", "synchronous_commit", "track_commit_timestamp"))
    result = {"toast_heap_mib": receipt["toast_heap_bytes"] / MIB, "total_cpu_ms": 0}
    for stage in STAGES:
        observed = receipt["stages"][stage]
        ticks = observed["backend_cpu_ticks"]
        cpu = observed["backend_cpu_ms"]
        for raw, key in zip(ticks, ("user", "system"), strict=True):
            assert raw >= 0 and math.isclose(raw * scale, cpu[key])
        cpu_ms = sum(ticks) * scale
        result["total_cpu_ms"] += cpu_ms
        result[stage + "_cpu_ms"] = cpu_ms
        result[stage + "_wal_mib"] = observed["cluster_wal_bytes"] / MIB
        result[stage + "_wall_ms"] = observed["wall_ms"]
    return result


def check_toast(witness, on_write):
    stages = ("enqueue", "claim", "settle", "priority_self_assignment")
    ids = {}
    for field in ("jsonb", "rendered_text"):
        values = [witness[stage][field] for stage in stages]
        assert len({(v["sha256"], v["bytes"], v["chunks"]) for v in values}) == 1
        assert values[0]["bytes"] > 524288 and values[0]["chunks"] == 263
        ids[field] = [v["id"] for v in values]
    assert len(set(ids["jsonb"])) == 1 and len(set(ids["rendered_text"])) == (1 if on_write else 4)
    assert witness["enqueue"]["jsonb"]["sha256"] != witness["enqueue"]["rendered_text"]["sha256"]
    if on_write:
        for field in ids:
            edited = witness["explicit_args_edit"][field]
            assert edited["id"] != ids[field][-1] and edited["sha256"] != witness["enqueue"][field]["sha256"]
            ids[field].append(edited["id"])
    return ids


def series(filename, digest, run, witness, cache):
    archive = HERE / filename
    arms = ["stock_a", "live_text", cache, "stock_b"]
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
    with tarfile.open(archive) as records:
        def read(path):
            return json.load(records.extractfile(path))

        manifest = read(f"{run}/manifest.json")
        assert manifest["frozen"] == "542925005370032c71db1a2aa488c2d35d16f14f9fdc8f54ee0afbb81f80fc82"
        assert manifest["input_sha256"] == "30f91354336fab21dbe85f2933f3347ec53ab7634af3805169a892536747fa1c"
        assert manifest["arms"] == arms and manifest["orders"] == [[0,1,3,2],[1,2,0,3],[2,3,1,0],[3,0,2,1]]
        calibration = manifest["calibration"]
        numerator, denominator = calibration["mach_timebase"]
        assert (numerator, denominator) == (125, 3)
        assert abs(calibration["mach_ns"] / calibration["getrusage_self_ns"] - 1) < 0.01
        scale = numerator / denominator / 1_000_000
        if run == "pass-02":
            assert b"__round__" in records.extractfile("pass-01/exit.log").read()
            metrics(read("pass-01/1-stock_a.json"), scale)
        measured = {arm: [] for arm in arms}
        loads = []
        for group, order in enumerate(manifest["orders"], 1):
            for i in order:
                arm = arms[i]
                record = read(f"{run}/{group}-{arm}.json")
                loads.append(record["load"][0])
                measured[arm].append(metrics(record, scale))
        ids = check_toast(read(witness + "/toast.json"), cache == "on_args_write")

    medians = {arm: {key: round(median(row[key] for row in rows), 3) for key in rows[0]}
               for arm, rows in measured.items()}
    paired = {}
    for arm in ("live_text", cache):
        paired[arm] = {}
        for key in ("enqueue_cpu_ms", "claim_cpu_ms", "settle_cpu_ms", "total_cpu_ms"):
            deltas = [candidate[key] - min(a[key], b[key])
                      for candidate, a, b in zip(measured[arm], measured["stock_a"], measured["stock_b"], strict=True)]
            paired[arm][key] = {"candidate_minus_faster_stock_ms": [round(v, 3) for v in deltas],
                                "median_ms": round(median(deltas), 3), "wins": sum(v < 0 for v in deltas)}
    return dict(arm_medians=medians, paired=paired, load1_range=[min(loads), max(loads)],
                toast_ids_in_stage_order=ids, arms=16, completed_jobs=1024, archive_sha256=digest)


def reduce():
    return {
        "generated": series("receipts.tar.gz", DIGEST, "pass-02", "toast-01", "stored_text"),
        "explicit_args": series("args-write.tar.gz", ARGS_WRITE_DIGEST, "args-write-01", "toast-args-write-01", "on_args_write"),
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    summary = json.dumps(reduce(), indent=2, sort_keys=True) + "\n"
    path = HERE / "summary.json"
    if args.write:
        path.write_text(summary)
    elif path.read_text() != summary:
        raise SystemExit("summary differs from the archived receipts")
    print("Verified 32 arms, 2,048 jobs and both physical TOAST witnesses.")
