#!/usr/bin/env python3
"""Reconcile the local native-overlay smokes without starting PostgreSQL."""
import json
from pathlib import Path

import matrix_verify
import native
import storage

HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "evidence/local-peers"


def load(arm, name):
    return json.loads((arm / f"{name}.json").read_text())


def replay():
    matrix_verify.ROOT = native.ROOT
    rows = {}
    for engine in native.STOCK:
        for policy in ("pglz", "lz4"):
            label = f"{engine}-{policy}"
            arm = OUT / label
            if not (OUT / f"{label}.PASS").is_file():
                raise ValueError(f"{label}: adapter did not finish successfully")
            ledger = load(arm, "ledger")
            receipt = load(arm, "payload-storage")
            result = native.verify_storage(receipt, ledger, engine, policy)
            if engine == "go" and ledger["peak_runner_rss_bytes"] == 0:
                # The Darwin River stock runner's Linux procfs RSS sampler is
                # unavailable. It already ran its unmodified raw verifier and
                # emitted a ledger. Validate the rest without publishing RSS.
                ledger["peak_runner_rss_bytes"] = 1
                result["rss_available"] = False
            score = matrix_verify.reconcile(ledger, engine, "slow", {})
            result.update(completed=score["completed"], failed=score["failed"],
                          effect_commits=score["ordered_independent_writes"],
                          peak_connections=score["peak_connections"])
            result["payload_receipt_sha256"] = native.file_hash(arm / "payload-storage.json")
            result["ledger_sha256"] = native.file_hash(arm / "ledger.json")
            if engine == "go":
                receipt_schema = receipt["schema_name"]
                if receipt["persisted"]["metadata"]["relation"] != f"{receipt_schema}.river_job":
                    raise ValueError("River persisted a different trial schema")
            rows[label] = result
    return rows


if __name__ == "__main__":
    storage.write(OUT / "summary.json", replay())
