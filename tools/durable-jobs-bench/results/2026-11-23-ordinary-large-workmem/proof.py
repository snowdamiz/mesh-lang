#!/usr/bin/env python3
"""Replay original strict ledgers and verify the work_mem and resource sidecar."""

import argparse
import json
from pathlib import Path
import sys
import tarfile

from freeze import ASSEMBLY, INPUTS, LABELS, LANE, SUITE, digest, read

sys.path[1:1] = [str(ASSEMBLY / "replay-retry"), str(SUITE), str(LANE / "overlay")]
import interleave as stock_matrix
import receipt as stock_receipt
import run
from verify import reconcile
from workmem import DATABASES, SETTINGS, expected_setting

DURABLE = {"max_connections": "96", "fsync": "on", "full_page_writes": "on",
           "synchronous_commit": "on", "track_commit_timestamp": "on", "ssl": "on",
           "reserved_connections": "0", "superuser_reserved_connections": "0",
           "pg_stat_statements.track": "top", "log_temp_files": "-1"}


def check_sidecar(data, engine, mb, score, native):
    before, after, stats, memory = (data[key] for key in ("before", "after", "io", "memory"))
    if data["expected_mb"] != mb or before["database"] != "postgres" or after["database"] != DATABASES[engine]:
        raise ValueError("wrong attempted treatment or observed database")
    if before["settings"] != after["settings"] or set(after["settings"]) != set(SETTINGS):
        raise ValueError("missing or changed PostgreSQL settings during trial")
    settings = after["settings"]
    if settings["work_mem"] != expected_setting(mb):
        raise ValueError("wrong effective work_mem or assignment source")
    if (after["database_cap"] != 96 or score["peak_connections"] > 96
            or any(not window["tls"] or window["overrides"] != 0 for window in (before, after))
            or any(settings[name]["value"] != value for name, value in DURABLE.items())):
        raise ValueError("cluster durability, TLS, role overrides, or physical cap differs")
    claims = stats["mesh_claim"]
    if stats["statements_evicted"] != 0 or not stats["roles"]:
        raise ValueError("missing or evicted pg_stat_statements")
    if engine == "mesh" and (len(claims) != 1 or claims[0]["calls"] <= 0):
        raise ValueError("ordinary claim statement counters missing")
    if engine != "mesh" and claims:
        raise ValueError("peer unexpectedly ran the Mesh claim")
    points = memory["points"]
    if any(p["processes"] <= 0 or p["rss_sum_kib"] <= 0 or p["capture_us"] < 0 for p in points):
        raise ValueError("invalid PostgreSQL process RSS sample")
    window = [p for p in points if score["first_submit_us"] <= p["epoch_us"]
              and p["epoch_us"] + p["capture_us"] <= score["last_settlement_us"]]
    pss = [p["pss_sum_kib"] for p in window if p["pss_sum_kib"] is not None]
    if any(n <= 0 for n in pss) or (not native and pss):
        raise ValueError("wrong proportional resident memory observation")
    db = stats["database"]
    return {
        "work_mem_mib": mb,
        "database_temp_files": db["temp_files"], "database_temp_bytes": db["temp_bytes"],
        "temp_blocks_by_role": stats["roles"], "mesh_claim": claims,
        "peak_postgres_pss_kib": max(pss, default=None),
        "peak_postgres_rss_sum_kib": max((p["rss_sum_kib"] for p in window), default=None),
        "postgres_memory_samples_in_window": len(window),
        "postgres_memory_samples_total": len(points),
        "postgres_memory_sample_peak_us": max((p["capture_us"] for p in window), default=None),
    }


def matched_settings(points):
    reference = points[0]["settings"]
    for point in points[1:]:
        if any(value != point["settings"][name] for name, value in reference.items() if name != "work_mem"):
            raise ValueError("ordinary control/candidate changed PostgreSQL settings besides work_mem")


def native(root, config, phase, label, score=None):
    lane = config["lanes"][label]
    engine, inventory = (lane[key] for key in ("engine", "inventory"))
    score = score or run.score_arm(root, {"profile": "large"}, phase, engine, inventory)
    if label.startswith("ordinary"):
        config = dict(config, lanes=dict(ordinary=lane))
        base_label = "ordinary"
    else:
        base_label = label
    storage = stock_receipt.native(root, "external", config, phase, base_label)
    raw = root / "raw.tar.gz"
    with tarfile.open(raw, "r:gz") as archive:
        read_raw = stock_receipt.reads_archive(archive)
        sidecar = read_raw("workmem.json")
        if engine in {"mesh", "oban"}:
            affinity = read_raw("process-affinity.json")
            pid = str(sidecar["memory"]["postmaster_pid"])
            if affinity.get(pid) != config["cpus"]:
                raise ValueError("PostgreSQL memory sampler reported a different postmaster")
        return dict(raw_sha256=digest(raw), settings=sidecar["after"]["settings"],
                    **{k: v for k, v in storage.items() if k != "raw_sha256"},
                    **check_sidecar(sidecar, engine, LABELS[label][1], score, True))


def local(root, mb):
    fetch = stock_receipt.reads_local(root)
    ledger = fetch("ledger.json")
    if ledger["local_smoke"] is not True:
        raise ValueError("expected the local diagnostic")
    pins = read(INPUTS / "local-pins.json")
    artifact = {"version": fetch("build.json")["version"], "artifact_sha256": pins["runner_manifest_sha256"]}
    score = reconcile(ledger, "mesh", "large", artifact)
    storage = stock_receipt.ordinary(fetch, score, "external", pins, [])
    sidecar = fetch("workmem.json")
    return dict(settings=sidecar["after"]["settings"], **stock_receipt.summarize(score, storage),
                **check_sidecar(sidecar, "mesh", mb, score, False))


def local_campaign(root):
    groups = {}
    for group in sorted(root.glob("group-*")):
        rows = {}
        for phase in ("warmup", "measured"):
            labels = read(group / "order.json")
            if set(labels) != {"4mb-a", "4mb-b", "16mb"}:
                raise ValueError("missing a local control")
            rows[phase] = {label: local(group / f"{phase}-{label}", int(label.split("mb")[0]))
                           for label in labels}
            matched_settings(list(rows[phase].values()))
        groups[group.name] = rows
    return groups


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("local",))
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    print(json.dumps(local_campaign(args.evidence), indent=2, allow_nan=False))
