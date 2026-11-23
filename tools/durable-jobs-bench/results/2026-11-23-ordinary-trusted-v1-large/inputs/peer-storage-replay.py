#!/usr/bin/env python3
"""Verify the storage receipt against the frozen native ledger."""

import argparse
import json
from pathlib import Path
import sys
import tarfile

from storage import TABLES, qualified_table

INPUT_SHA256 = "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"
JOBS = 64
PAYLOAD_BYTES = 512 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def check(proof, ledger, engine):
    require((proof["engine"], proof["policy"]) == (engine, "EXTERNAL")
            and (ledger["engine"], ledger["profile"], ledger["input_sha256"]) == (engine, "large", INPUT_SHA256),
            "wrong engine, policy, or frozen input")
    setup, persisted = proof["setup"], proof["persisted"]
    before, after = setup["before"], setup["after"]
    relation = after["relation"]
    schema = relation.split(".")[0]
    _, column, kind, _, terminal = TABLES[engine]
    require(relation == qualified_table(engine, schema)
            and before == dict(after, attstorage="x")
            and after == persisted["metadata"]
            and (after["attstorage"], after["column"], after["type"], after["persistence"], after["owner"])
            == ("e", column, kind, "p", "parity_subject")
            and int(after["toast_oid"]) > 0 and int(after["relation_oid"]) > 0
            and setup["rows_before"] == 0,
            "wrong payload column or storage change")
    require(setup["ddl"] == f"ALTER TABLE {relation} ALTER COLUMN {column} SET STORAGE EXTERNAL",
            "unexpected treatment DDL")

    transactions, jobs, rows = ledger["transactions"], ledger["jobs"], persisted["rows"]
    by_key = {row["job_key"]: row for row in rows}
    require(len(transactions) == 1 and len(jobs) == len(rows) == len(by_key) == JOBS,
            "expected one producer transaction and 64 unique jobs")
    tx = transactions[0]
    returned = tx.get("returned_us", tx.get("acknowledged_us"))
    require(0 < setup["ddl_start_us"] <= setup["ddl_end_us"] < tx["submitted_us"]
            <= tx["committed_us"] <= returned
            and max(job["settled_us"] for job in jobs) <= proof["verify_start_us"] <= proof["verify_end_us"],
            "DDL or storage proof overlapped the measured work")
    require(persisted["trigger_absent"] is True, "River tune trigger was still installed")
    if engine == "go":
        require(proof["triggers_retired"] is True
                and 0 < proof["install_start_us"] <= proof["install_end_us"] <= setup["ddl_start_us"],
                "River trigger installation or retirement is unproven")

    committed_xids = {int(tx["xid"]) % 2**32}
    if engine == "go":
        migration_xid = int(setup["migration_xid"]) % 2**32
        require(migration_xid not in committed_xids, "River DDL shared the enqueue transaction")
        committed_xids.add(migration_xid)
    settlement_xids = set()
    stored_bytes = 0
    for job in jobs:
        key = job["job_key"]
        row = by_key.pop(key)
        require((job["id"], job["payload_sha256"], job["settled_xid"])
                == (row["id"], row["payload_sha256"], row["settled_xid"])
                and job["id"] == tx["enqueued_ids"][key]
                and (job["state"], job["attempt"], job["attempts"], job["effects"]) == ("completed", 1, 1, 1)
                and (row["native_state"], row["payload_bytes"], row["compression"]) == (terminal, PAYLOAD_BYTES, None)
                and row["stored_column_bytes"] >= PAYLOAD_BYTES,
                f"{key}: persisted payload, native ID, or terminal state differs")
        first, second = (int(job[name]) for name in ("attempt_xid", "effect_xid"))
        require(first != second and committed_xids.isdisjoint((first, second))
                and tx["committed_us"] < job["attempt_committed_us"]
                < job["effect_committed_us"] <= job["settled_us"],
                f"{key}: expected two independent committed application writes")
        committed_xids.update((first, second))
        settlement_xids.add(int(row["settled_xid"]))
        stored_bytes += row["stored_column_bytes"]
    require(committed_xids.isdisjoint(settlement_xids), "settlement reused an enqueue, DDL, or application XID")
    require(persisted["toast_heap_bytes"] >= stored_bytes, "TOAST heap is smaller than persisted payload columns")
    return dict(engine=engine, policy="EXTERNAL", payloads=JOBS, payload_bytes=PAYLOAD_BYTES * JOBS,
                stored_column_bytes=stored_bytes, toast_heap_bytes=persisted["toast_heap_bytes"],
                committed_application_writes=2 * JOBS, terminal_transactions=len(settlement_xids),
                ddl_us=setup["ddl_end_us"] - setup["ddl_start_us"],
                storage_verification_us=proof["verify_end_us"] - proof["verify_start_us"])


def verify_finished(directory, engine, ledger=None):
    proof = json.loads((directory / "payload-storage.json").read_text())
    if ledger is None:
        ledger = json.loads((directory / "ledger.json").read_text())
    return check(proof, ledger, engine)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine", choices=TABLES)
    parser.add_argument("evidence", type=Path, help="an adapter directory or native raw.tar.gz")
    parser.add_argument("--river-stock-source", type=Path,
                        help="directory containing the pinned River run.py for macOS raw replay")
    args = parser.parse_args()
    engine, evidence = args.engine, args.evidence
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

    if args.river_stock_source:
        if engine != "go" or not evidence.is_dir():
            parser.error("use --river-stock-source with a local River evidence directory")
        raw = evidence / "river"
        config = json.loads((raw / "config.json").read_text())
        require(config["os"] == "darwin", "macOS raw replay requires a Darwin River receipt")
        sys.path.insert(0, str(args.river_stock_source))
        from raw_evidence import verify_run

        ledger, _ = verify_run(raw, "large", config["binary_sha256"], smoke=False)
        require(ledger["peak_runner_rss_bytes"] == 0, "unexpected macOS River RSS")
        summary = verify_finished(evidence, engine, ledger)
        print(json.dumps(dict(summary, mode="darwin-raw", scoring=False), indent=2, sort_keys=True))
        return

    from verify import reconcile

    if evidence.is_dir():
        ledger = json.loads((evidence / "ledger.json").read_text())
        proof = json.loads((evidence / "payload-storage.json").read_text())
    else:
        with tarfile.open(evidence, "r:gz") as archive:
            with archive.extractfile("adapter/ledger.json") as stream:
                ledger = json.load(stream)
            with archive.extractfile("adapter/payload-storage.json") as stream:
                proof = json.load(stream)

    artifact = {key: ledger[key] for key in ("artifact_sha256", "version")}
    reconcile(ledger, engine, "large", artifact)
    print(json.dumps(check(proof, ledger, engine), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
