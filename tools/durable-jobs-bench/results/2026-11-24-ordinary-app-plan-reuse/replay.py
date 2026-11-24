#!/usr/bin/env python3
"""Verify all 18 arm receipts and derive the decision from the frozen rule."""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
from statistics import median
import tarfile

from measure import (
    CPUS, HOST, JOBS, MINIMUM_MS, NAMES, ORDERS, PG_SHA, ROOT, WORKERS,
    frozen_inputs, historical_gap, verify_rows,
)


def read_receipts(path):
    if path.is_dir():
        return {p.name: json.loads(p.read_bytes()) for p in path.glob("*.json")}
    with tarfile.open(path) as archive:
        return {p.name: json.load(archive.extractfile(p)) for p in archive.getmembers()
                if p.isfile() and p.name.endswith(".json")}


def stats_entry(receipt, relation):
    entry, = [row for row in receipt["counters"]
              if row["query"].lstrip().startswith(f"INSERT INTO public.mesh_jobs_parity_{relation}")]
    assert entry["calls"] == entry["rows"] == JOBS
    assert entry["wal_records"] >= JOBS and entry["wal_bytes"] > 0
    assert 0 < entry["total_plan_time"] and 0 < entry["total_exec_time"]
    return entry


def arm_metrics(r, seen_xids, bindings):
    assert r["sslmode"] == "verify-full"
    assert WORKERS == len(r["pids"]) == len(r["tls"]) == len(set(r["pids"]))
    assert set(r["pids"]) == {pid for pid, ssl, version, cipher in r["tls"] if ssl and version and cipher}
    assert len(r["writers"]) == len(r["prepared"]) == WORKERS
    assert all(w["errors"] == 0 and 0 < w["elapsed_ns"] <= r["wall_ns"] for w in r["writers"])
    assert sum(w["pairs"] for w in r["writers"]) == JOBS
    xids = verify_rows(r["rows"], bindings, r["start_us"])
    assert not seen_xids.intersection(xids)
    seen_xids.update(xids)
    entries = [stats_entry(r, relation) for relation in ("attempts", "effects")]
    health, = [row for row in r["counters"] if row["query"] == "SELECT $1"]
    assert health["calls"] == health["rows"] == JOBS * 2
    if r["arm"] == "named":
        assert r["prepare_wall_ns"] > 0
        for w, plans in zip(r["writers"], r["prepared"], strict=True):
            assert [p[0].encode() for p in plans] == list(NAMES)
            assert all(int(g) + int(c) == w["pairs"] and int(g) > 0 and int(c) == 5 and source == "f"
                       for _, g, c, source in plans)
        for entry, name in zip(entries, NAMES, strict=True):
            # Reused generic executions count once per backend, not once per call.
            assert entry["plans"] == sum(1 + int(c) for plans in r["prepared"]
                                         for n, _, c, _ in plans if n.encode() == name)
    else:
        assert r["prepare_wall_ns"] == 0 and not any(r["prepared"])
        assert all(entry["plans"] == JOBS for entry in entries)
    cpu = [hi - lo for lo, hi in zip(r["backend_before"], r["backend_after"], strict=True)]
    assert len(cpu) == WORKERS and min(cpu) > 0 and r["client_cpu_ns"] > 0
    last_commit_us = max(effect[-1] for effect in r["rows"]["effects"]) - r["start_us"]
    assert 0 < last_commit_us <= r["wall_ns"] / 1000 + 1000
    before, after = r["host_before"], r["host_after"]
    assert before["epoch_us"] <= r["start_us"] < after["epoch_us"]
    assert after["monotonic_ns"] - before["monotonic_ns"] >= r["wall_ns"]
    cpu_ticks = [[b - a for a, b in zip(before["cpu"][f"cpu{cpu}"], after["cpu"][f"cpu{cpu}"], strict=True)]
                 for cpu in sorted(CPUS)]
    assert all(min(ticks) >= 0 and sum(ticks) > 0 for ticks in cpu_ticks)
    steal = 100 * sum(ticks[7] for ticks in cpu_ticks) / sum(map(sum, cpu_ticks))
    wal = r["wal"]
    assert wal["wal_write"] > 0 and wal["wal_write_time"] > 0
    return {
        "wall_ms": r["wall_ns"] / 1e6,
        "last_effect_commit_ms": last_commit_us / 1000,
        "backend_cpu_ms": sum(cpu) / 1e6,
        "client_cpu_ms": r["client_cpu_ns"] / 1e6,
        "combined_cpu_ms": (sum(cpu) + r["client_cpu_ns"]) / 1e6,
        "prepare_ms": r["prepare_wall_ns"] / 1e6,
        "attempt_plans": entries[0]["plans"],
        "effect_plans": entries[1]["plans"],
        "insert_plan_ms": sum(row["total_plan_time"] for row in entries),
        "insert_exec_ms": sum(row["total_exec_time"] for row in entries),
        "wal_syncs": wal["wal_sync"],
        "wal_sync_ms": wal["wal_sync_time"],
        "wal_writes": wal["wal_write"],
        "wal_write_ms": wal["wal_write_time"],
        "load_1m_before": r["load_before"][0],
        "load_1m_after": r["load_after"][0],
        "cpu_steal_percent": steal,
        "builders_detected": len(before["builders"]) + len(after["builders"]),
    }


def summarize(source):
    receipts = read_receipts(source)
    freeze = receipts.pop("freeze.json")
    env = receipts.pop("environment.json")
    assert freeze["orders"] == list(map(list, ORDERS))
    assert freeze["jobs"] == JOBS and freeze["workers"] == WORKERS and freeze["minimum_ms"] == MINIMUM_MS
    assert all(hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == expected
               for name, expected in freeze["files"].items())
    statements, policy, bindings = frozen_inputs()
    assert freeze["statements"] == [s.decode() for s in statements]
    assert freeze["policy_sha256"] == hashlib.sha256(policy.encode()).hexdigest()
    historical = freeze["historical_gap"]
    assert historical == historical_gap()
    assert env["system"] == "Linux" and env["machine"] == "x86_64" and env["logical_cores"] == 4
    host = env["native_host"]
    assert (host["boot_id"], host["kernel"], host["mount"]["maj:min"]) == HOST
    assert host["affinity"] == sorted(CPUS) and host["postgres_sha256"] == PG_SHA
    settings = env["settings"]
    assert settings["server_version_num"] == "170011" and settings["plan_cache_mode"] == "auto"
    assert settings["max_connections"] == "96"
    assert settings["reserved_connections"] == settings["superuser_reserved_connections"] == "0"
    assert all(settings[key] == "on" for key in (
        "ssl", "synchronous_commit", "fsync", "full_page_writes", "track_commit_timestamp",
        "pg_stat_statements.track_planning", "track_wal_io_timing"))
    seen = set()
    rotations = []
    by_arm = defaultdict(list)
    for i, order in enumerate(ORDERS, 1):
        arms = {}
        for arm in order:
            label = f"{i:02}-{arm}"
            receipt = receipts.pop(f"{label}.json")
            assert receipt["label"] == label and receipt["arm"] == arm
            arms[arm] = arm_metrics(receipt, seen, bindings)
            by_arm[arm].append(arms[arm])
        deltas = {}
        for metric in ("wall_ms", "last_effect_commit_ms", "backend_cpu_ms", "client_cpu_ms",
                       "combined_cpu_ms", "insert_plan_ms"):
            a, b, named = (arms[arm][metric] for arm in ("stock-a", "stock-b", "named"))
            deltas[metric] = dict(gain_vs_faster_stock=min(a, b) - named,
                                  gain_vs_mean_stock=(a + b) / 2 - named,
                                  control_spread=abs(a - b))
        rotations.append(dict(group=i, order=order, arms=arms, deltas=deltas))
    assert not receipts and len(seen) == len(ORDERS) * 3 * JOBS * 2
    distribution = {arm: {metric: dict(median=median(values), min=min(values), max=max(values))
                          for metric in arms[0]
                          if (values := [a[metric] for a in arms])}
                    for arm, arms in by_arm.items()}
    paired = {}
    for metric in rotations[0]["deltas"]:
        samples = [r["deltas"][metric] for r in rotations]
        paired[metric] = {
            "median_gain_vs_faster_stock": median(s["gain_vs_faster_stock"] for s in samples),
            "median_gain_vs_mean_stock": median(s["gain_vs_mean_stock"] for s in samples),
            "median_duplicate_spread": median(s["control_spread"] for s in samples),
            "faster_than_both": sum(s["gain_vs_faster_stock"] > 0 for s in samples),
            "gain_beyond_spread": sum(s["gain_vs_faster_stock"] > s["control_spread"] for s in samples),
        }
    wall = paired["wall_ms"]
    advance = wall["median_gain_vs_faster_stock"] >= MINIMUM_MS and wall["gain_beyond_spread"] >= 4
    noisy_arms = [f"{r['group']:02}-{arm}" for r in rotations for arm, m in r["arms"].items()
                  if m["cpu_steal_percent"] > 1 or m["builders_detected"]]
    verdict = "unqualified" if noisy_arms else "advance" if advance else "stop"
    return dict(verdict=verdict, unqualified_arms=noisy_arms, committed_rows=len(seen), errors=0,
                environment=env, historical=historical, distributions=distribution,
                paired=paired, rotations=rotations)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="native receipt directory or controller export")
    args = parser.parse_args()
    result = summarize(args.source)
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    if result["verdict"] == "unqualified":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
