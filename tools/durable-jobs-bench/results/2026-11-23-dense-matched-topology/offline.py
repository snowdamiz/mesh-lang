"""Replay prior raw receipts against the prospective scoring checks."""

from collections import Counter
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile

from matrix import (PACKET, TOPOLOGY, diagnostic_reconciler, score_arm, run, require, text,
                    mesh_topology, peer_topology, validate, METRICS, WORK)
from verify import PROFILES, reconcile

ARCHIVE = PACKET.parent / "2026-11-23-ordinary-trusted-v1/evidence/ordinary-trusted-v1-dense"
HISTORY = TOPOLOGY / "evidence/native-01"


def reject(call, message):
    try:
        call()
    except ValueError as error:
        require(message in str(error), f"wrong rejection: {error}")
    else:
        raise ValueError(f"accepted invalid evidence ({message})")


def fixture_path(directory, group, lane, engine, phase):
    return next(directory.glob(f"blocks/{group:02}-*/{phase}-{lane}/{phase}-{engine}"))


def replay():
    config = run.read_json(PACKET / "stock.json")
    orders = validate(config, "stock")
    require(len(orders) == 5 and all(block["phases"] == ["warmup", "measured"] for block in orders),
            "must preserve all five groups and warmup")
    require(all(Counter(order["order"][position] for order in orders) == Counter(config["lanes"].keys())
                for position in range(5)), "unbalanced lane positions")
    checked = []
    for group in range(1, 6):
        for phase in ("warmup", "measured"):
            for engine, lane in (("go", "river"), ("rust", "apalis"), ("oban", "oban")):
                folder = fixture_path(ARCHIVE, group, lane, engine, phase)
                row = score_arm(folder, engine, 8, phase, config["inventory"], config)
                require(all(metric in row for metric in METRICS), "missing stock scoring fields")
                checked.append(dict(group=group, phase=phase, lane=lane, raw_sha256=row["raw_sha256"],
                                    topology=row["topology"]))
    pins = run.read_json(TOPOLOGY / "native-pins.json")
    for group in (1, 2):
        for label, runners in (("stock-a", 8), ("tuned-2x32", 2)):
            folder = HISTORY / f"{group:02}" / label / "diagnostic-mesh"
            inv = run.read_json(HISTORY / "staging" / label / "inventory.json")
            row = score_arm(folder, "mesh", runners, "diagnostic", inv, pins, historical=True)
            require(all(metric in row for metric in METRICS), "missing Mesh scoring fields")
            checked.append(dict(group=group, phase="diagnostic", lane=label, raw_sha256=row["raw_sha256"],
                                topology=row["topology"]))
    with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
        def files(name):
            return run.file_json(archive, "adapter/" + name)

        def logs(name):
            return text(archive, "adapter/" + name)

        ledger = files("ledger.json")
        artifact = {key: ledger[key] for key in ("version", "artifact_sha256")}
        reject(lambda: reconcile(ledger, "mesh", "noop", artifact), "unexpected runners")
        reject(lambda: diagnostic_reconciler(8)(ledger, "mesh", "noop", artifact), "unexpected runners")
        reject(lambda: mesh_topology(lambda n: dict(files(n), slots_per_runner=8) if n == "topology.json" else files(n),
                                     logs, ledger, 2, "diagnostic"), "topology receipt differs")
        require(PROFILES["noop"][0] == 8 and ledger["runners"] == 2,
                "canonical contract or real ledger was modified")
    return dict(qualified_historical_arms=len(checked), receipts=checked,
                negative_checks=["canonical rejects the two-runner ledger", "incorrect diagnostic count rejected",
                                 "incorrect observed runner slot count rejected"])


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
