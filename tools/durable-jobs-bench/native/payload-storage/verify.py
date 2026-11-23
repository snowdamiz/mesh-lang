"""Verify both native SETUP inventories, ledgers and catalog readbacks offline."""
import argparse
from pathlib import Path

import controller

def verify(directory):
    run = controller.run
    complete = run.read_json(directory / "stage-complete.json")
    for output in ("large", "crossed"):
        if complete[f"{output}_sha256"] != run.digest(directory / f"{output}.json"):
            raise ValueError(f"sealed {output} inventory changed")
    if complete["installer_sha256"] != run.digest(controller.HERE / "prepare.py"):
        raise ValueError("installer differs from the sealed stage")
    large = run.read_json(directory / "large.json")
    for label in controller.LABELS:
        arm = directory / label
        inventory = run.read_json(arm / "inventory.json")
        if inventory != large["lanes"][label]["inventory"]:
            raise ValueError(f"incorrect {label} inventory")
        folder = arm / "diagnostic-mesh"
        proof = controller.read_storage(folder / "raw.tar.gz")
        score = run.score_arm(folder, {"profile": "large"}, "diagnostic", "mesh", inventory)
        if {"storage": proof, "score": score} != run.read_json(arm / "reconciled.json"):
            raise ValueError(f"changed {label} storage or queue receipt")
        print(f"{label} {proof['jobs']} jobs, {proof['stored_bytes']} argument bytes, {proof['raw_sha256']}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, nargs="?", default=
                        controller.run.RESULTS / "2026-11-23-payload-storage-mesh-setup")
    verify(parser.parse_args().directory)
