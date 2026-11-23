#!/usr/bin/env python3
"""Regenerate the comparison from compiled-component and TLS producer receipts."""

import json
from statistics import median
import tarfile

from build import HERE
from component import ORDER


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def compare(rows):
    assert set(rows) == set(range(1, len(ORDER) + 1))
    for order, group in zip(ORDER, rows.values(), strict=True):
        assert list(group) == list(order)
    comparisons = {arm: distribution([row[arm] for row in rows.values()]) for arm in ("A", "once", "B")}
    for arm in ("A", "B"):
        deltas = [row[arm] - row["once"] for row in rows.values()]
        comparisons[f"savings_vs_{arm}"] = dict(distribution(deltas), wins=sum(v > 0 for v in deltas))
    comparisons["beats_both"] = sum(row["once"] < min(row["A"], row["B"]) for row in rows.values())
    comparisons["control_abs_gap"] = distribution([abs(row["A"] - row["B"]) for row in rows.values()])
    return comparisons


def collect(items, field):
    rows = {}
    for item in items:
        group = rows.setdefault(item["rotation"], {})
        assert item["arm"] not in group
        group[item["arm"]] = field(item)
    return compare(rows)


def cpu(item, process):
    observation = item["cpu_counters"]
    pid, before, after = observation[process]
    assert pid > 0 and all(y >= x for x, y in zip(before, after, strict=True))
    return (sum(after) - sum(before)) * observation["ns_per_unit"] / 1e6


def replay():
    with tarfile.open(HERE / "local-raw.tar.gz", "r:gz") as raw:
        def read(name):
            with raw.extractfile(name) as source:
                return json.load(source)

        report = {"component": {}, "local_tls": {}}
        for build in ("01", "02"):
            counts = read(f"component-{build}/counts-and-membership.json")
            timing = read(f"component-{build}/timing.json")
            report["component"][build] = profiles = {}
            for label, fixtures in counts.items():
                for trial in fixtures:
                    assert trial["A"]["chunks"] == trial["once"]["chunks"]
                    assert trial["once"]["encode_count"] == trial["jobs"]
                    for chunk in trial["once"]["chunks"]:
                        assert 1 <= len(chunk["keys"]) <= 128 and chunk["bytes"] <= 1_048_576
                profiles[label] = dict(
                    jobs=sum(fixture["jobs"] for fixture in fixtures),
                    chunks=sum(len(fixture["A"]["chunks"]) for fixture in fixtures),
                    **{arm: {key: sum(fixture[arm][key] for fixture in fixtures)
                             for key in ("encode_count", "encode_bytes")} for arm in ("A", "once")})
                rows = [item for item in timing if item["profile"] == label]
                if rows:
                    profiles[label]["elapsed_ms"] = collect(
                        rows, lambda item: sum(row["elapsed_ns"] for row in item["measurements"]) / 1e6)
            assert counts["exact"][0]["A"]["chunks"][0]["bytes"] == 1_048_576
            assert profiles["large"]["A"] == dict(encode_count=127, encode_bytes=66_607_436)
            assert profiles["large"]["once"] == dict(encode_count=64, encode_bytes=33_565_952)
            assert profiles["large"]["chunks"] == 64

        proof_a = read("proof-04/A/proof.json")
        proof_once = read("proof-04/once/proof.json")
        assert proof_a["settings"] == proof_once["settings"]
        assert proof_a["failures"] == proof_once["failures"]
        for label in proof_a["successes"]:
            for a, b in zip(proof_a["successes"][label], proof_once["successes"][label], strict=True):
                assert a["chunks"] == b["chunks"] and a["input_sha256"] == b["input_sha256"]
                assert a["receipt"]["count"] == b["receipt"]["count"]
                for aj, bj in zip(a["jobs"], b["jobs"], strict=True):
                    fields = ("key", "payload_sha256", "stored_bytes", "hold_ms", "fail_first", "compression")
                    assert all(aj[k] == bj[k] for k in fields)
        report["local_tls"]["expected_failures"] = proof_a["failures"]

        rows = read("timing-01/measurements.json")
        pins = read("build-02/pins.json")
        assert pins["A"] == pins["B"] and pins["A"] != pins["once"]
        canon, = {row["input_sha256"] for row in rows}
        assert canon == counts["large"][0]["input_sha256"]
        for run in rows:
            arm = run["arm"]
            name = f"timing-01/{run['rotation']:02d}-{arm}"
            assert read(f"{name}/result.json") == run
            identity = read(f"{name}/database.json")
            assert identity["tls"] and identity["durable"] and identity["commit_timestamps"]
            assert identity["max_connections"] == identity["database_connection_limit"] == 96
            assert pins[arm] == run["binary_sha256"]
            assert run["settings"] == proof_a["settings"]
            assert sorted(run["sql_counts"].values()) == [64, 64]
            stamp = run["database_product"]
            transaction = run["adapter_timing"]
            assert run["producer"]["xid"] == stamp["xid"] == transaction["xid"]
            assert run["submit_to_commit_ms"] == (stamp["commit_us"] - transaction["submitted_us"]) / 1000
            assert transaction["submitted_us"] <= stamp["commit_us"] <= transaction["acknowledged_us"]
            jobs = run["jobs"]
            assert len(jobs) == len({job["xid"] for job in jobs} - {stamp["xid"]}) == 64
            assert len({job["id"] for job in jobs}) == 64
            assert [job["key"] for job in jobs] == [f"job_{i:06d}" for i in range(64)]
            assert all(job["commit_us"] == stamp["commit_us"] and job["compression"] == "none" for job in jobs)
            ids = {job["key"]: job["id"] for job in jobs}
            assert ids == run["producer"]["ids"] == run["committed_receipts"]

        report["local_tls"].update(
            input_sha256=canon, rotations=len(ORDER),
            load_1m=distribution([r["load_before"][0] for r in rows]),
            submit_to_commit_ms=collect(rows, lambda item: item["submit_to_commit_ms"]),
            adapter_round_trip_ms=collect(rows, lambda item: item["adapter_round_trip_ms"]),
            broker_cpu_ms=collect(rows, lambda item: cpu(item, "producer")),
            backend_cpu_ms=collect(rows, lambda item: cpu(item, "backend")))
        return report


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True, allow_nan=False))
