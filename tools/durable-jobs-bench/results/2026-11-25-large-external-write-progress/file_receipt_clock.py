"""Compare durable enqueue and client acknowledgement in the same FILE submissions."""
import hashlib
from io import BytesIO
import json
from pathlib import PurePosixPath
from statistics import median
import subprocess
import tarfile


REV = "9d357b9b6f65b6e74d762527c98e3e3625f4e20b"
EVIDENCE = ("tools/durable-jobs-bench/results/2026-11-25-public-large-canonical-string-linux"
            "/evidence/file-91548db6e0ce")
MANIFEST = "9d1877412c8efcdf0aec815dcaa3a9fc1881dfb8bd5771ffcc70498667acae2e"


def blob(path):
    return subprocess.check_output(["git", "show", f"{REV}:{EVIDENCE}/{path}"])


def run():
    manifest = blob("RAW-SHA256SUMS")
    assert hashlib.sha256(manifest).hexdigest() == MANIFEST
    clocks = {}
    padding = []
    for line in manifest.decode().splitlines():
        digest, path = line.split("  ")
        group, phase_lane = PurePosixPath(path).parts[2:4]
        key = (int(group[:2]), phase_lane)
        assert key not in clocks
        raw = blob(path)
        assert hashlib.sha256(raw).hexdigest() == digest
        with tarfile.open(fileobj=BytesIO(raw)) as archive:
            ledger = json.load(archive.extractfile("adapter/ledger.json"))
        txn, = ledger["transactions"]
        assert txn["enqueued_ids"] == {job["job_key"]: job["id"] for job in ledger["jobs"]}
        assert len(txn["enqueued_ids"]) == 64
        submit, commit, ack = (txn[k] for k in ("submitted_us", "committed_us", "acknowledged_us"))
        assert submit < commit <= ack
        clocks[key] = (commit - submit, ack - submit)
        padding.append(ack - commit)
    assert len(clocks) == 40
    print("Commit-to-ack padding across 40 arms, min / median / max (us):",
          min(padding), int(median(padding)), max(padding))
    for control in ("control-a", "control-b", "exact-file"):
        pairs = [(clocks[g, "measured-" + control], clocks[g, "measured-candidate"])
                 for g in range(1, 6)]
        for i, clock in enumerate(("durable enqueue", "acknowledged enqueue")):
            gains = [ref[i] - cand[i] for ref, cand in pairs]
            print(control, clock, "paired gains (us):", gains, "median:", median(gains))


if __name__ == "__main__":
    run()
