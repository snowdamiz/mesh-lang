#!/usr/bin/env python3
"""Replay local provenance and representative stock durable ledgers."""
import json
from pathlib import Path
import subprocess
import sys

from budgets import build
from rotation import digest
from trial import durable

HERE = Path(__file__).resolve().parent
RAW = {
    "short": (
        "2026-11-23-ordinary-hot-checkout-bulk-v2",
        "evidence/ordinary-hot-checkout-bulk-v2-short/blocks/01-short.*/measured-bulk-d2/measured-mesh/raw.tar.gz",
        "bulk-d2", 34.147,
    ),
    "large": (
        "2026-11-23-ordinary-broker-once",
        "evidence/ordinary-broker-once-matched/blocks/01-large.*/measured-ordinary-a/measured-mesh/raw.tar.gz",
        "ordinary-a", 749.608,
    ),
}


def raw(profile):
    series, glob, _, _ = RAW[profile]
    path, = (HERE.parent / series).glob(glob)
    return path


if __name__ == "__main__":
    assert build() == json.loads((HERE / "budgets.json").read_text())
    for profile, (_, _, lane, enqueue_wall) in RAW.items():
        path = raw(profile)
        capture = json.loads((HERE / f"{profile}-capture.json").read_text())
        assert digest(path) == capture["raw_ledger_archive_sha256"]
        assert digest(HERE / f"{profile}-broker.mpl") == capture["broker_source_sha256"]
        cfg = json.loads((HERE / f"{profile}-reference.json").read_text())
        summary = durable(path.parent, cfg, "measured", lane, {"first_submit_to_commit_ms": enqueue_wall})
        assert summary["committed_jobs"] == capture["jobs"]
        subprocess.run([sys.executable, "-B", HERE / "trial.py", profile, "offline-example"], check=True)
        print(json.dumps(summary, sort_keys=True))
