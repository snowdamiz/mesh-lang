"""Reconcile released FILE comparisons and compare their byte scale with the DENSE submit gap."""
from hashlib import sha256
import io
import json
from pathlib import Path
import runpy
import tarfile

RESULTS = Path(__file__).resolve().parent.parent
ROOT = RESULTS.parents[2]
LARGE = RESULTS / "2026-11-24-public-large-file-session/resume/evidence/native"
SLOW = RESULTS / "2026-11-25-public-slow-file-topology/roster-retry/evidence/native/pilot"
DENSE = RESULTS / "2026-11-25-public-dense-late-capacity/peer-comparison/evidence"


def fixture(profile):
    settings = json.loads((ROOT / "scripts/jobs-parity-profiles.json").read_bytes())
    inputs = runpy.run_path(str(RESULTS / "2026-11-24-enqueue-spec-preparation/fixture-generator.py"))["inputs"]
    batch, = inputs(settings["workloads"][profile], 64, settings["seed"])
    hashes = {j["key"]: sha256(j["payload"].encode()).hexdigest() for j in batch["jobs"]}
    payload_bytes = sum(len(job["payload"].encode()) for job in batch["jobs"])
    return hashes, payload_bytes


def measured(block, lane, digest, payloads):
    raw, = (block / ("measured-" + lane)).glob("measured-*/raw.tar.gz")
    data = raw.read_bytes()
    assert sha256(data).hexdigest() == digest
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    rows = ledger["jobs"]
    assert len(rows) == len(payloads)
    assert {row["job_key"]: row["payload_sha256"] for row in rows} == payloads
    tx, = ledger["transactions"]
    assert tx["enqueued_ids"] == {row["job_key"]: str(row["id"]) for row in rows}
    assert len({row[k] for row in rows for k in ("attempt_xid", "effect_xid")}) == 2 * len(rows)
    assert all(row["state"] == "completed" and row["attempts"] == row["effects"] == 1 for row in rows)
    return dict(raw_sha256=digest, enqueue_us=tx["committed_us"] - tx["submitted_us"])


def price(profile, release, groups, blocks):
    assert json.loads(release.read_bytes())["exit_status"] == 0
    hashes, payload_bytes = fixture(profile)
    rotations = []
    for block, scores, comparisons in groups:
        lanes = {lane for treatment, controls in comparisons for lane in (treatment, *controls)}
        arms = {lane: measured(blocks / block, lane, scores[lane]["raw_sha256"], hashes) for lane in sorted(lanes)}
        savings = {t: min(arms[c]["enqueue_us"] for c in controls) - arms[t]["enqueue_us"]
                   for t, controls in comparisons}
        rotations.append(dict(block=block, measured=arms, control_minus_treatment_us=savings))
    return dict(jobs=len(hashes), submitted_payload_bytes=payload_bytes, rotations=rotations)


def replay():
    large = json.loads((LARGE / "score-replay.json").read_bytes())["groups"]
    slow = json.loads((SLOW / "replay.json").read_bytes())["groups"]
    dense = json.loads((DENSE / "scored-before-release.json").read_bytes())["groups"]
    assert (len(large), len(slow), len(dense)) == (5, 2, 5)
    one = ("file", ("inline-a", "inline-b"))
    split = ("split-file", ("split-inline",))
    return dict(
        large=price("large", LARGE / "timing-adapter-path-retry-release.json",
                    [(g["path"], g["scores"], [one]) for g in large], LARGE / "timing-adapter-path-retry/blocks"),
        slow=price("slow", SLOW / "controller-release.json",
                   [(g["path"], g["scores"]["measured"], [one, split]) for g in slow], SLOW / "blocks"),
        dense=price("noop", DENSE / "controller-release.json",
                    [(f"{i:02}-noop", {k: dense[str(i)]["measured-" + k] for k in ("B", "river")},
                      [("river", ("B",))]) for i in range(1, 6)], DENSE / "blocks"),
    )


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
