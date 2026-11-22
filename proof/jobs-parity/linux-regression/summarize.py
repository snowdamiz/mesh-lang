"""Reconcile regression guest receipts and emit a compact, shareable record."""

import argparse
import hashlib
import json
from pathlib import Path
from statistics import median

from oban_runtime import verify_report


def load(path):
    return json.loads(path.read_text())


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_live(report, config, role):
    verify_report(report, config, role)
    expected_env = {"ERL_FLAGS": config["oban_erl_flags"]}
    if inetrc := config["oban_erl_inetrc"]:
        expected_env["ERL_INETRC"] = inetrc
    assert report["live_environment"] == expected_env
    tree = {row["pid"]: row for row in report["processes"]}
    assert len(tree) == len(report["processes"])
    root = tree[report["pid"]]
    assert root["comm"] == "beam.smp" and root["exe"] == "/opt/otp/erts-17.1/bin/beam.smp"
    reachable = {root["pid"]}
    while children := {pid for pid, row in tree.items() if row["ppid"] in reachable} - reachable:
        reachable.update(children)
    assert reachable == tree.keys()


def summarize(group):
    evidence = group / "evidence"
    config = load(evidence / "config.json")
    profile = config["profile"]
    phases = [json.loads(line) for line in (group / "host-load.jsonl").read_text().splitlines()]
    measured = [point["event"].split()[-1] for point in phases if " measured " in point["event"]]
    assert measured == config["arms"], phases
    expected_jobs = {"noop": 1024, "slow": 512, "sparse": 24, "short": 128, "large": 64}[profile]
    result = {
        "name": group.name,
        "profile": profile,
        "mesh_elf_sha256": config["hashes"]["proof/mesh-current"],
        "oban_revision": config["oban_revision"],
        "oban_erl_flags": config.get("oban_erl_flags"),
        "series": config.get("series", "reconstructed-explicit-schedulers-only"),
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
    resolver_evidence = "oban_erl_inetrc" in config
    if resolver_evidence:
        preflight = load(evidence / "oban-preflight.json")
        verify_report(preflight, config, "preflight")
        result.update(oban_erl_inetrc=config["oban_erl_inetrc"], oban_preflight=preflight)
        for executable in ("erl_child_setup", "inet_gethost"):
            path = f"opt/otp/erts-17.1/bin/{executable}"
            result["shared_component_sha256"][path] = config["hashes"][path]
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
            emulator = {}
            if engine == "oban" and config.get("oban_erl_flags"):
                reports = load(arm / "oban-emulator.json")
                emulator["oban_emulator"] = reports
                if resolver_evidence:
                    broker = load(arm / "oban-broker-emulator.json")
                    logs = list(arm.glob("runner-*.log"))
                    assert len(reports) == len(logs) == (8 if profile in {"noop", "large"} else 1)
                    for report in reports:
                        check_live(report, config, "run")
                    check_live(broker, config, "session")
                    assert len({report["pid"] for report in [*reports, broker]}) == len(reports) + 1
                    emulator["oban_broker_emulator"] = broker
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
                **emulator,
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
