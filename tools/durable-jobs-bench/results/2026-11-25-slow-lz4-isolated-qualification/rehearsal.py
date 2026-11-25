#!/usr/bin/env python3
"""Replay Darwin functional ledgers and challenge physical codec readback."""
from copy import deepcopy
import json
from pathlib import Path
import sys

import packet
from strict import verify_codec
from verify import database_check



def rejected(label, persisted, ledger, engine, policy):
    try:
        verify_codec(packet.storage, engine, persisted, ledger["jobs"], policy)
    except ValueError as error:
        return str(error)
    raise ValueError(f"accepted deliberately corrupted {label}")


def negative_controls(arms, engine):
    pglz, pglz_ledger = arms["pglz"]
    lz4, lz4_ledger = arms["lz4"]
    relabeled = deepcopy(pglz)
    relabeled["metadata"]["default_toast_compression"] = "lz4"
    # The historical permissive validator accepts this policy-only relabel.
    packet.storage.verify_snapshot(engine, relabeled, pglz_ledger["jobs"], "lz4")
    mixed = deepcopy(lz4)
    mixed["rows"][0]["compression"] = None
    packet.storage.verify_snapshot(engine, mixed, lz4_ledger["jobs"], "lz4")
    corrupted = deepcopy(lz4)
    corrupted["rows"][0]["payload_sha256"] = "0" * 64
    return {
        "uncompressed_arm_relabeled_as_lz4": rejected("arm", relabeled, pglz_ledger, engine, "lz4"),
        "one_lz4_row_relabeled_as_uncompressed": rejected("row", mixed, lz4_ledger, engine, "lz4"),
        "lz4_payload_digest_corrupted": rejected("payload", corrupted, lz4_ledger, engine, "lz4"),
    }


def check_arm(path, engine, policy, expected):
    raw = path / "ledger.json"
    ledger = packet.read(raw)
    storage = path / "payload-storage.json"
    receipt = packet.read(storage)
    server = packet.read(path / "codec-postgres.json")
    packet.verify_security(engine, receipt["security"])
    database_check(ledger["database"], 68)
    packet.require(ledger["engine"] == engine and ledger["profile"] == "slow"
                   and ledger["input_sha256"] == "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
                   and (ledger["runners"], ledger["slots"]) == (1, 64)
                   and 0 < ledger["peak_connections"] <= 68, "local workload or topology changed")
    transaction, = ledger["transactions"]
    key = transaction["request_id"]
    jobs = ledger["jobs"]
    truths = {job["job_key"]: (job["payload_sha256"], job["hold_ms"], job["fail_first"]) for job in jobs}
    packet.require(truths == expected[key] and len(jobs) == 512
                   and transaction["enqueued_ids"] == {job["job_key"]: job["id"] for job in jobs},
                   "local canonical payload or producer receipt differs")
    writes = {job[field] for job in jobs for field in ("attempt_xid", "effect_xid")}
    packet.require(len(writes) == 1024 and transaction["xid"] not in writes
                   and all(job["state"] == "completed" and (job["attempts"], job["effects"]) == (1, 1)
                           and job["settled_xid"] not in (job["attempt_xid"], job["effect_xid"])
                           and transaction["committed_us"] <= job["attempt_committed_us"]
                           < job["effect_committed_us"] <= job["settled_us"] for job in jobs),
                   "local ordered independent writes differ")
    persisted = receipt["persisted"]
    counts = verify_codec(packet.storage, engine, persisted, jobs, policy)
    packet.original.verify_storage(receipt, ledger, engine, policy)
    output = {
        "compression_counts": counts,
        "physical_column_type": persisted["metadata"]["type"],
        "stored_column_bytes": sum(row["stored_column_bytes"] for row in persisted["rows"]),
        "engine_artifact_sha256": ledger["artifact_sha256"], "postgres_sha256": server["postgres_sha256"],
        "runner_rss_sampler": "unavailable on Darwin" if engine == "go" else "available",
        "ledger_sha256": packet.digest(raw), "storage_receipt_sha256": packet.digest(storage),
        "canonical_jobs": len(jobs), "independent_application_transactions": len(writes),
        "role_and_tls_readback": receipt["security"],
    }
    return output, (persisted, ledger)


def main():
    packet.original_config()
    expected = packet.matrix.expected_inputs("slow")
    historical = packet.OLD / "evidence/local-peers"
    negative = {}
    for engine in ("mesh", "go", "oban", "rust"):
        controls = {}
        for policy in ("pglz", "lz4"):
            path = historical / f"{engine}-{policy}"
            controls[policy] = (packet.read(path / "payload-storage.json")["persisted"],
                                packet.read(path / "ledger.json"))
        negative[engine] = negative_controls(controls, engine)
    if len(sys.argv) == 1:
        print(json.dumps(negative, indent=2, sort_keys=True))
        return

    primary, apalis = map(Path, sys.argv[1:])
    arms = {}
    builds = {"stock": set(), "isolated": set()}
    for engine in ("mesh", "go", "oban", "rust"):
        for build, policy in (("stock", "pglz"), ("isolated", "pglz"), ("isolated", "lz4")):
            label = f"{build}-{engine}-{policy}"
            directory = (apalis if engine == "rust" else primary) / label
            output, _ = check_arm(directory, engine, policy, expected)
            builds[build].add(output["postgres_sha256"])
            arms[label] = output
    packet.require(all(len(pins) == 1 for pins in builds.values()) and builds["stock"] != builds["isolated"],
                   "arms did not use the two expected PostgreSQL builds")
    print(json.dumps({"scope": "Darwin functional only", "arms": arms, "negative_controls": negative},
                     indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
