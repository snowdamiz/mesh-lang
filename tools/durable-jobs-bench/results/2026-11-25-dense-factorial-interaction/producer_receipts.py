"""Replay producer and drain clocks, plus the earlier instrumented receipt windows."""
import hashlib
import json
from pathlib import Path
import tarfile


RESULTS = Path(__file__).resolve().parent.parent
DENSE = RESULTS / "2026-11-25-dense-fixed8-river-readiness/evidence"
SHORT = RESULTS / "2026-11-24-r2-d2-producer-boundary/evidence/native-timing"


def dense_arm(receipt):
    path = DENSE / "native" / receipt["group_path"]
    assert hashlib.sha256(path.read_bytes()).hexdigest() == receipt["raw_sha256"]
    with tarfile.open(path) as archive:
        raw = archive.extractfile("adapter/ledger.json").read()
        assert hashlib.sha256(raw).hexdigest() == receipt["ledger_sha256"]
        ledger = json.loads(raw)
        (tx,) = ledger["transactions"]
        jobs = ledger["jobs"]
        assert len(jobs) == len(tx["enqueued_ids"]) == 1024
        assert len({j[key] for j in jobs for key in ("attempt_xid", "effect_xid")}) == 2048
        assert all(j["attempts"] == j["effects"] == 1 for j in jobs)
        if ledger["engine"] == "mesh":
            writer = json.load(archive.extractfile("adapter/producer-writer.json"))
            assert writer["bulk"] == 8 and writer["scalar"] == 0
        submit, commit = tx["submitted_us"], tx["committed_us"]
        terminal = max(j["settled_us"] for j in jobs)
        assert submit <= commit <= terminal
        if ledger["engine"] == "go":
            assert submit <= tx["read_finished_us"] <= tx["decoded_us"] <= tx["enqueue_called_us"] <= commit
            extra = {"file_and_decode_us": tx["decoded_us"] - submit,
                     "prepare_to_enqueue_call_us": tx["enqueue_called_us"] - tx["decoded_us"],
                     "enqueue_call_to_commit_us": commit - tx["enqueue_called_us"],
                     "return_after_commit_us": tx["returned_us"] - commit}
        else:
            extra = {"ack_after_commit_us": tx["acknowledged_us"] - commit}
        return {"enqueue_us": commit - submit, "drain_us": terminal - commit,
                "submit_us": terminal - submit, **extra}


def dense_pairs():
    bindings = json.loads((DENSE / "release-audit.json").read_text())
    pairs = []
    for group in range(1, 7):
        arms = {}
        for lane in ("combined", "river-warm"):
            (receipt,) = [r for r in bindings["raw_by_group"][str(group)]
                          if f"/measured-{lane}/" in r["group_path"]]
            arms[lane] = dense_arm(receipt)
        mesh, river = arms.values()
        pairs.append({"group": group, **arms,
                      "enqueue_difference_us": mesh["enqueue_us"] - river["enqueue_us"],
                      "mesh_free_enqueue_minus_river_full_submit_us": mesh["drain_us"] - river["submit_us"]})
    return pairs


def short_windows():
    (series,) = SHORT.iterdir()
    summary = json.loads((series / "summary.json").read_text())
    assert summary["complete"] and len(summary["groups"]) == 5
    for name in ("client_observer_usable", "nested_observer_usable"):
        assert not summary["attribution"][name]
    rows = []
    for group in summary["groups"]:
        for mode in ("client", "nested"):
            path = series / "blocks" / group["block"] / f"measured-{mode}/measured-mesh/raw.tar.gz"
            with tarfile.open(path) as archive:
                marks = {}
                for line in archive.extractfile("adapter/broker-errors.log"):
                    if line.startswith(b"r2_producer_stage "):
                        _, event, timestamp = line.decode().split()
                        assert event not in marks
                        marks[event] = int(timestamp)
                writer = json.load(archive.extractfile("adapter/producer-writer.json"))
                assert writer["bulk"] == 1 and writer["scalar"] == 0
                ledger = json.load(archive.extractfile("adapter/ledger.json"))
                assert len(ledger["jobs"]) == 128 and len(ledger["transactions"]) == 1
            stages = group["observations"][mode]["wire"]["stages_ms"]
            windows = {}
            for start, end, name in (
                ("bulk_first_row", "bulk_ready", "bulk_remaining_result"),
                ("bulk_ready", "release_send", "bulk_ready_to_release_send"),
                ("release_ready", "xid_send", "release_ready_to_xid_send"),
                ("xid_ready", "commit_send", "xid_ready_to_commit_send"),
            ):
                ns = marks[end] - marks[start]
                assert ns >= 0 and abs(ns - stages[name] * 1e6) < 0.01
                windows[name] = ns
            rows.append({"group": group["group"], "mode": mode, "windows_ns": windows,
                         "sum_ns": sum(windows.values()), "raw_sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    return rows


if __name__ == "__main__":
    print(json.dumps({"dense": dense_pairs(), "short_observed": short_windows()}, indent=2))
