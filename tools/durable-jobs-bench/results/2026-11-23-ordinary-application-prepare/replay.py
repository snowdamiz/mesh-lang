"""Reconcile the application-write pairs and display the local CPU comparison."""

import hashlib
from itertools import permutations
import json
from pathlib import Path
from statistics import median
import tarfile
from uuid import UUID

HERE = Path(__file__).resolve().parent
archive = HERE / "measurements.tar.gz"
assert hashlib.sha256(archive.read_bytes()).hexdigest() == (HERE / "SHA256SUMS").read_text().split()[0]
with tarfile.open(archive) as raw:
    environment = json.load(raw.extractfile("environment.json"))
    assert environment["version"] == "17.11" and environment["plan_cache_mode"] == "auto"
    assert environment["settings"] == dict.fromkeys(
        ("ssl", "fsync", "synchronous_commit", "full_page_writes", "track_commit_timestamp"), "on")
    results = []
    for group, order in enumerate(permutations(("stock-a", "named", "stock-b")), 1):
        rows = {}
        for variant in order:
            arm = f"{group:02}-{variant}"
            doc = json.load(raw.extractfile(arm + ".json"))
            jobs = doc["jobs"]
            assert doc["arm"] == arm and len(jobs) == 1024
            assert len({xid for row in jobs for xid in row[2:4]}) == 2048
            for i, (key, identity, x1, x2, t1, t2) in enumerate(jobs):
                assert key == f"job_{i:06d}" and UUID(identity).int == i+1
                assert 0 < int(x1) < int(x2) and 0 < t1 <= t2
            assert len(set(doc["pids"])) == len(doc["plans"]) == 8
            for plans in doc["plans"]:
                if variant == "named":
                    assert [row[0] for row in plans] == ["app_0", "app_1"]
                    assert all(int(g) + int(c) == 128 for _, g, c in plans)
                else:
                    assert plans == []
            deltas = [high-low for before, after in zip(doc["before"], doc["after"], strict=True)
                      for low, high in zip(before, after, strict=True)]
            assert len(deltas) == 16 and min(deltas) >= 0
            num, denom = doc["timebase"]
            rows[variant] = [sum(deltas)*num/denom/1_000_000/len(jobs), doc["wall_ns"]/1_000_000]
        results.append(rows)

for index, metric in enumerate(("Backend CPU-ms per write pair", "Elapsed ms per 1024 write pairs")):
    savings = [min(row["stock-a"][index], row["stock-b"][index]) - row["named"][index] for row in results]
    spread = [abs(row["stock-a"][index] - row["stock-b"][index]) for row in results]
    print(metric)
    print("Medians", {k: round(median(row[k][index] for row in results), 4) for k in results[0]})
    print("Savings against better stock", [round(s, 4) for s in savings])
    print("Median saving / median duplicate spread", round(median(savings), 4), round(median(spread), 4))
