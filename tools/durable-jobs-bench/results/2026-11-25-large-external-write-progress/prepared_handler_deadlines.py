"""Reprice the historical Oban drain gate on the two measured prepared-span duplicates."""
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

REV = "ae33e9e6ca738675189906930848e88c05e935cb"
PILOT = "tools/durable-jobs-bench/results/2026-11-25-public-large-prepared-render-spans/evidence/pilot"


def blob(path):
    return subprocess.check_output(["git", "show", f"{REV}:{PILOT}/{path}"])


def arm_intervals(group, label, scored):
    block, = (b for b in subprocess.check_output(
        ["git", "ls-tree", "--name-only", f"{REV}:{PILOT}/blocks"], text=True
    ).splitlines() if b.startswith(f"{group:02d}-"))
    raw = blob(f"blocks/{block}/measured-{label}/measured-mesh/raw.tar.gz")
    assert hashlib.sha256(raw).hexdigest() == scored["raw_sha256"]
    with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        statements = json.load(archive.extractfile("adapter/trusted-application-statements.json"))
    tx, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert len(jobs) == len(set(tx["enqueued_ids"])) == 64
    assert tx["enqueued_ids"] == {j["job_key"]: j["id"] for j in jobs}
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 192
    probe, = (s for s in statements if s["query"] == "SELECT $1")
    assert probe["calls"] == probe["rows"] == 128
    for table in ("parity_attempts", "parity_effects"):
        write, = (s for s in statements if s["query"].startswith("INSERT INTO public.mesh_jobs_" + table))
        assert write["calls"] == write["rows"] == 64

    intervals = []
    for j in jobs:
        assert j["state"] == "completed" and j["attempts"] == j["attempt"] == j["effects"] == j["hold_ms"] == 1
        entry = j["entered_us"] - tx["committed_us"]
        first = j["attempt_committed_us"] - j["entered_us"]
        second = j["effect_committed_us"] - j["attempt_committed_us"] - 1000
        terminal = j["settled_us"] - j["effect_committed_us"]
        assert min(entry, first, second, terminal) >= 0
        intervals.append((entry + 1000 + terminal, first, second))
    assert max(base + x + y for base, x, y in intervals) == round(scored["drain_ms"] * 1000)
    return intervals, probe["execution_ms"]


def drain_envelope(rows, phase):
    # At discount k, a job keeps max(0, first-k) and/or max(0, second-k).
    # The slowest job is the maximum of at most three straight lines.
    unchanged = max(base + x + y for base, x, y in rows)
    if phase == "first":
        return [max(base + y for base, x, y in rows), unchanged]
    if phase == "post_hold":
        return [max(base + x for base, x, y in rows), unchanged]
    return [max(base for base, x, y in rows),
            max(base + max(x, y) for base, x, y in rows), unchanged]


def price(duplicates, phase, deadline):
    envelopes = [drain_envelope(rows, phase) for rows in duplicates]
    saturated = max(max(x, y) for rows in duplicates for _, x, y in rows)
    best = -10**9
    changes = []
    for discount in range(saturated + 1):
        a, b = [max(value - slope * discount for slope, value in enumerate(lines)) for lines in envelopes]
        margin = deadline - max(a, b) - abs(a - b)
        best = max(best, margin)
        if not changes or changes[-1]["passes"] != (margin >= 0):
            changes.append(dict(discount_us=discount, passes=margin >= 0))
    return dict(best_margin_us=best, transitions=changes, saturation_us=saturated)


def main():
    score = json.loads(blob("scored-before-release.json"))
    release = json.loads(blob("controller-release.json"))
    assert release["exit_status"] == 0 and release["request_sha256"] == score["request_sha256"]
    for group in score["groups"]:
        number = group["group"]
        history = group["historical"]["clocks"]["oban_drain_ms"]
        deadline = round(1000 * max(history["deadlines"]))
        results = [arm_intervals(number, label, group["arms"]["measured"][label]) for label in ("span-a", "span-b")]
        intervals = [r[0] for r in results]
        print(json.dumps(dict(group=number, deadline_us=deadline, probe_sql_ms=[r[1] for r in results],
                              phases={p: price(intervals, p, deadline) for p in ("first", "post_hold", "both")}),
                         sort_keys=True))


if __name__ == "__main__":
    main()
