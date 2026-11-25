"""Reconcile queue traffic and application overlap from the released pool/supply pilot."""
import argparse
import hashlib
import json
from pathlib import Path

from inflight_xids import measure
from settlement_scans import archive, read


def replay(root):
    receipt = (root / "pilot-replay.json").read_bytes()
    assert hashlib.sha256(receipt).hexdigest() == "e6e7d9721a05d5865e1f212edb56ef350d2f4d001da193d812766eaa9722fd3b"
    scores = json.loads(receipt)
    rows = {}
    for group, arms in scores["measured"].items():
        rows[group] = {}
        for lane, arm in arms.items():
            args = root, int(group), lane, arm["raw_sha256"]
            with archive(*args) as raw:
                counters = read(raw, "adapter/raw.json")
                assert counters["raw_idle_sql_calls"] == 136
            windows = measure(*args)["windows"]
            rows[group][lane] = dict(
                raw_sha256=arm["raw_sha256"],
                claim_calls=counters["raw_claim_calls"],
                runner_sql_calls=counters["raw_runner_sql_calls"],
                terminal_xids=arm["terminal_xids"],
                runner_lifetime_cpu_ms=arm["runner_cpu_ms"],
                application_backends_with_cpu_ticks=arm["app_backends_with_cpu_ticks"],
                middle_512_xid_overlap=windows["middle_512"],
            )
    return rows


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("native_evidence", type=Path)
    print(json.dumps(replay(parser.parse_args().native_evidence), indent=2, sort_keys=True))
