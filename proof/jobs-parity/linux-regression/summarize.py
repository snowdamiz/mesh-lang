"""Reconcile regression guest receipts and emit a compact, shareable record."""

import argparse
import hashlib
import json
from pathlib import Path
from statistics import median


def load(path):
    return json.loads(path.read_text())


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def summarize(group):
    evidence = group / "evidence"
    config = load(evidence / "config.json")
    profile = config["profile"]
    phases = [json.loads(line) for line in (group / "host-load.jsonl").read_text().splitlines()]
    measured = [point["event"].split()[-1] for point in phases if " measured " in point["event"]]
    assert measured == config["arms"], phases
    expected_jobs = {"sparse": 24, "short": 128, "large": 64}[profile]
    result = {
        "name": group.name,
        "profile": profile,
        "mesh_elf_sha256": config["hashes"]["proof/mesh-current"],
        "oban_revision": config["oban_revision"],
        "guest_base_sha256": config["base_cpio_sha256"],
        "kernel_sha256": config["kernel_sha256"],
        "manifest_sha256": digest(evidence / "config.json"),
        "shared_component_sha256": {
            path: checksum for path, checksum in config["hashes"].items()
            if path.startswith("proof/") or path.endswith(("/beam.smp", ".JobsParityOban.beam", ".JobsParityOban.Worker.beam"))
        },
        "full_input_warmup": config["full_input_warmup"],
        "host_load": phases,
        "arms": [],
    }
    phases = (("warmup-", "warmup"), ("", "measured")) if config["full_input_warmup"] else (("", "measured"),)
    digests = set()
    for prefix, phase in phases:
        for engine in config["arms"]:
            arm = evidence / (prefix + engine)
            writes = load(arm / "ordered-writes.json")
            raw = load(arm / "raw.json")
            metrics = load(arm / "metrics.json")
            digests.add(metrics["input_sha256"])
            assert expected_jobs == writes["ordered"] == writes["jobs"] == metrics["completed"] == len(raw["attempts"])
            assert all(len(raw["namespace_writes"][role]) == expected_jobs for role in ("jobs", "attempts", "effects"))
            assert all(row["state"] == "completed" and row["attempts"] == row["effects"] == 1
                       for row in raw["attempts"])
            assert raw["raw_sql_calls"] == (
                raw["raw_runner_sql_calls"] + raw["raw_application_sql_calls"] + raw["raw_enqueue_sql_calls"]
            )
            claimed = [row["claimed_ms"] for row in raw["attempts"]]
            result["arms"].append({
                "phase": phase,
                "metrics": metrics,
                "ordered_write_pairs": writes,
                "claim_span_ms": max(claimed) - min(claimed),
                "median_claim_to_handler_ms": median(row["entered_ms"] - row["claimed_ms"]
                                                     for row in raw["attempts"]),
                "durable_enqueue_transactions": len(raw["transactions"]),
                "database_and_broker": load(arm / "database.json"),
                "raw_ledger_and_samples_sha256": digest(arm / "raw.json"),
                "runner_shutdown_sha256": [digest(path) for path in sorted(arm.glob("runner-*.log"))],
            })
    assert len(digests) == 1
    result["input_sha256"] = digests.pop()
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("groups", type=Path, nargs="+")
    args = parser.parse_args()
    print(json.dumps({
        "status": "paired regression screens",
        "connection_and_sql_roles": ["parity_subject", "parity_producer", "compiled_caller"],
        "enqueue_sql_scope": "shared producer and observer on parity_producer",
        "groups": [summarize(group) for group in args.groups],
    }, indent=2, sort_keys=True))
