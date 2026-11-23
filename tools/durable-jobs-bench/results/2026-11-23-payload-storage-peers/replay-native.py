#!/usr/bin/env python3
"""Recheck all three native archives offline using the frozen inventory."""

import argparse
import json
from pathlib import Path
import sys
import tarfile

SUITE = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(SUITE / "native/payload-storage-peers"), str(SUITE)]
from replay import check
from run import digest, file_json, read_json, require, score_arm


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("attempt", type=Path)
    attempt = parser.parse_args().attempt
    config = read_json(attempt / "symmetric-input.json")
    require(config["profiles"] == ["large"] and config["cpus"] == [0, 1], "unexpected workload or CPU mask")
    results = {}
    for label, engine in (("oban", "oban"), ("river", "go"), ("apalis", "rust")):
        directory = attempt / f"diagnostic-{engine}"
        inventory = config["lanes"][f"{label}-external"]["inventory"]
        score = score_arm(directory, {"profile": "large"}, "diagnostic", engine, inventory)
        host = [score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]]
        require(host == config["host"], f"{label}: host differs from the Mesh qualification")
        raw = directory / "raw.tar.gz"
        with tarfile.open(raw, "r:gz") as archive:
            proof = file_json(archive, "adapter/payload-storage.json")
            ledger = file_json(archive, "adapter/ledger.json")
            if engine == "oban":
                masks = file_json(archive, "adapter/process-affinity.json")
                require(len(masks) == 9 and all(mask == [0, 1] for mask in masks.values()),
                        "Oban PostgreSQL or runner affinity differs")
        results[label] = dict(check(proof, ledger, engine), raw_sha256=digest(raw))
    print(json.dumps(results, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
