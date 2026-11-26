"""Replay retained slow peer arms and test the scopes on one actual Mesh arm."""

import argparse
import copy
import json
from pathlib import Path

import native
from replay import directory_arm, verify_arm
from slow_arm import FROZEN
from arm import read


def peers():
    config = native.freeze("two")
    scores = {}
    for rotation in range(1, 6):
        block, = native.V1.parent.glob(f"blocks/{rotation:02}-*")
        reconciled = read(block / "reconciled.json")
        for label, peer in native.PEERS.items():
            lane = config["lanes"][label]
            for phase in ("warmup", "measured"):
                folder, = (block / f"{phase}-{peer}").glob(f"{phase}-*")
                score = native.score(folder, config, label, phase, lane["inventory"])
                native.require(score["raw_sha256"] == reconciled["raw_sha256"][str(folder.relative_to(block))],
                               "retained peer archive hash changed")
                scores[f"{block.name}/{phase}-{label}"] = {key: score[key] for key in (
                    "completed", "layout", "peak_connections", "peak_executing", "submit_ms", "drain_ms",
                    "raw_sha256", "application_xids", "terminal_xids")}
    return scores


def negative_checks(path):
    pins = read(FROZEN / "local-pins.json")
    score = directory_arm(path, "two")
    ledger = read(path / "ledger.json")
    artifact = {key: ledger[key] for key in ("artifact_sha256", "version")}
    artifact["executable_sha256"] = pins["executable_sha256"]

    def replace(name, transform):
        data = read(path / name)
        transform(data)
        return {name: data}

    def change_backend(rows):
        next(row for row in rows if row["role"] == "compiled_caller")["backend"] = 1

    def share_terminal(ledger):
        ledger["jobs"][1]["settled_xid"] = ledger["jobs"][0]["settled_xid"]

    def scalar_fallback(statements):
        row = next(row for row in statements if row["query"].startswith(
            "SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1("))
        row["rows"] = 8
        row["wal_bytes"] = 0

    def physical_overflow(raw):
        raw["samples"][0]["physical_clients"] = 69

    def invented_owner(ledger):
        ledger["jobs"][0]["worker_pid"] = 1

    edits = {
        "wrong allocation": replace("topology.json", lambda d: d.update(slots_per_runner=64)),
        "lost warm backend": replace("sessions-end.json", change_backend),
        "shared Mesh terminal XID": replace("ledger.json", share_terminal),
        "unknown worker PID": replace("ledger.json", invented_owner),
        "bulk-shaped scalar fallback": replace("statements-end.json", scalar_fallback),
        "physical cap overflow": replace("raw.json", physical_overflow),
    }
    rejected = {}
    for label, edit in edits.items():
        def files(name):
            return copy.deepcopy(edit[name]) if name in edit else read(path / name)
        try:
            verify_arm(files, lambda name: (path / name).read_text(), "two", artifact)
        except ValueError as error:
            rejected[label] = str(error)
        else:
            raise ValueError(f"validator accepted {label}")
    return dict(real_jobs=score["completed"], rejected=rejected)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("two_runner_arm", type=Path)
    args = parser.parse_args()
    print(json.dumps(dict(historical_stock_peers=peers(), validator=negative_checks(args.two_runner_arm)),
                     indent=2, sort_keys=True))
