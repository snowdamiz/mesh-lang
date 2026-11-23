"""Verify original receipts for the large enqueue and slow EXTERNAL writer experiments."""
from datetime import datetime
import hashlib
import json
from pathlib import Path
from statistics import median
import tarfile

HERE = Path(__file__).resolve().parent
SLOW = HERE.parent / "2026-11-23-external-bulk-sql"
LARGE_ARMS = ("scalar_a", "group16", "ordered16", "merge16", "scalar_b")
SLOW_ARMS = ("default_a", "external_scalar", "external_batch", "default_b")


def read_capture(path, checksum):
    archive = path / "receipts.tar.gz"
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == checksum
    with tarfile.open(archive) as tar:
        names = tar.getnames()
        assert len(names) == len(set(names))
        docs = {name: json.load(tar.extractfile(name)) for name in names}
    manifest = docs.pop("manifest.json")
    calibrate = manifest["calibration"]
    assert calibrate["mach_timebase"] == [125, 3]
    assert abs(calibrate["mach_ns"] - calibrate["process_cpu_ns"]) < 0.01 * calibrate["process_cpu_ns"]
    return manifest, docs


def check_transaction(row, count, xid, stamp, cpu, storage_bytes):
    assert int(row[xid]) > 0 and datetime.fromisoformat(row[stamp]).utcoffset() is not None
    assert row["jobs"] == count and row["storage"] == {"none": [count, storage_bytes]}
    expected = {"server_version_num": "170011", "fsync": "on", "synchronous_commit": "on",
                "track_commit_timestamp": "on", "default_toast_compression": "pglz"}
    assert all(row["settings"][k] == v for k, v in expected.items())
    assert 0 < row[cpu] and abs(row[cpu] - row["backend_ticks"] * 125 / 3 / 1e6) < 1e-8
    assert row["wall_ms"] > 0


def paired(groups, arms, controls, cpu):
    summary = {}
    for arm in arms:
        metrics = {}
        for key in (cpu, "wall_ms"):
            values = [g[arm][key] for g in groups]
            diffs = [g[arm][key] - min(g[c][key] for c in controls) for g in groups]
            metrics[key] = dict(median=round(median(values), 3), faster_than_both=sum(d < 0 for d in diffs),
                                difference_to_faster_ms=[round(d, 3) for d in diffs])
        summary[arm] = metrics
    loads = [r["load"][0] for g in groups for r in g.values()]
    return dict(arms=summary, one_minute_load_range=[min(loads), max(loads)])


def large():
    manifest, docs = read_capture(HERE, "3db86b27452d94f00825b5a69c763c69b650cab8c90aa68769d85d4def8039eb")
    assert manifest["batch_sha256"] == "30f91354336fab21dbe85f2933f3347ec53ab7634af3805169a892536747fa1c"
    assert tuple(manifest["arms"]) == LARGE_ARMS and manifest["policies"] == ["external", "default"]
    assert manifest["orders"] == [[0,1,4,2,3], [1,2,0,3,4], [2,3,1,4,0], [3,4,2,0,1], [4,0,3,1,2]]
    results = {}
    for policy in manifest["policies"]:
        groups = []
        for group, order in enumerate(manifest["orders"], 1):
            rows = {}
            for i in order:
                arm = LARGE_ARMS[i]
                row = docs.pop(f"{group}-{policy}-{arm}.json")
                check_transaction(row, 64, "xid", "committed", "cpu_ms", 33559808)
                merging = arm == "merge16" and policy == "default"
                ordered = arm == "ordered16"
                assert row["layout"] == (policy == "default")
                assert row["calls"] == dict(merge=4 if merging else 0, ordered=4 if ordered else 0,
                                            merge_declines=4 if arm == "merge16" and not merging else 0,
                                            scalar=0 if merging or ordered else 64,
                                            savepoints=64 if arm.startswith("scalar_") else 4)
                rows[arm] = row
            groups.append(rows)
        results[policy] = paired(groups, LARGE_ARMS, ("scalar_a", "scalar_b"), "cpu_ms")
    assert not docs
    return results


def slow():
    manifest, docs = read_capture(SLOW, "728f0d5a0cd4878054c6995b49fd2f41afd83d4818ac6a6c826a678d2187ea85")
    assert manifest["batch_sha256"] == "d9c91f63e1962cbd1a6c92c0a5f9154d2bec171058e38b997a8987b73ea86f8d"
    assert tuple(manifest["arms"]) == SLOW_ARMS
    assert manifest["orders"] == [[0,1,3,2], [1,2,0,3], [2,3,1,0], [3,0,2,1]]
    assert manifest["checks"] == dict.fromkeys(("additional_index", "changed_constraint", "different_argument_policy",
                                                "row_security", "unrelated_column_storage"), True)
    groups = []
    for group, order in enumerate(manifest["orders"], 1):
        rows = {}
        for i in order:
            arm = SLOW_ARMS[i]
            row = docs.pop(f"{group}-{arm}.json")
            check_transaction(row, 512, "top_xid", "commit", "backend_cpu_ms", 8431616)
            fallback = arm == "external_scalar"
            assert row["receipt_rows"] == 512 and row["layout"] == (not fallback)
            assert row["argument_attstorage"] == ("e" if arm.startswith("external_") else "x")
            assert row["calls"] == dict(bulk_calls=8, bulk_grants=0 if fallback else 504,
                                        scalar_grants=512 if fallback else 8, layout_fallbacks=8 if fallback else 0)
            rows[arm] = row
        groups.append(rows)
    assert not docs
    return paired(groups, SLOW_ARMS, ("default_a", "default_b"), "backend_cpu_ms")


if __name__ == "__main__":
    for path, reduction in ((HERE, large()), (SLOW, slow())):
        output = json.dumps(reduction, indent=2, sort_keys=True) + "\n"
        (path / "summary.json").write_text(output)
        print(f"{path.name} verified")
