"""Compare durable progress at equal elapsed time in the two native crossover series."""
import argparse
import hashlib
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
EDGES = ("entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")
SERIES = (
    ("short", "2026-11-23-bulk-enqueue-v2-short/evidence/bulk-enqueue-v2-short", "bulk", "river"),
    ("large", "2026-11-23-ordinary-large-workmem/evidence/ordinary-large-workmem-matched", "ordinary", "apalis-external"),
)


def read(directory, lane, sealed, profile):
    raw, = directory.glob(f"measured-{lane}/*/raw.tar.gz")
    assert hashlib.sha256(raw.read_bytes()).hexdigest() == sealed["raw_sha256"][str(raw.parent.relative_to(directory))]
    with tarfile.open(raw) as tape:
        book = json.load(tape.extractfile("adapter/ledger.json"))
        assert json.load(tape.extractfile("status.json"))["exit_code"] == 0
    count, slots = (128, 16) if profile == "short" else (64, 64)
    assert book["slots"] == slots and book["profile"] == profile
    tx, = book["transactions"]
    rows = book["jobs"]
    keys = {r["job_key"] for r in rows}
    assert len(rows) == len(keys) == len(tx["enqueued_ids"]) == count and set(tx["enqueued_ids"]) == keys
    assert all(j["attempts"] == j["effects"] == j["attempt"] == 1 for j in rows)
    commits = {int(r[k]) for r in rows for k in ("attempt_xid", "effect_xid")}
    assert len(commits) == 2 * count and int(tx["xid"]) not in commits
    assert {int(j["settled_xid"]) for j in rows}.isdisjoint(commits | {int(tx["xid"])})
    origin = tx["committed_us"]
    assert all([origin, *[r[k] for k in EDGES]] == sorted([origin, *[r[k] for k in EDGES]]) for r in rows)
    score = sealed["scores"][lane]
    assert score["completed"] == score["ordered_independent_writes"] == count and score["failed"] == 0
    assert score["affinity"] == [0, 1]
    ordered = {key: sorted(r[key] - origin for r in rows) for key in EDGES}
    assert round(count * 1e6 / score["commit_jobs_per_s"]) == ordered["settled_us"][-1]
    return origin, rows, ordered


def render():
    lines = ["# When handlers first enter and when the queue finishes", "",
             "The clocks start at each arm’s own PostgreSQL enqueue commit. Handler-entry ranks count distinct jobs. "
             "A last-stage timestamp is the maximum across the batch, so the last entry, last write, and last settlement can belong to different jobs. "
             "All times are elapsed milliseconds. Each arm ran on CPUs 0 and 1.", ""]
    for profile, source, mesh, peer in SERIES:
        label = "Short jobs with set-based enqueue and grouped successes" if profile == "short" else "Large jobs with EXTERNAL arguments and 16 MiB work_mem"
        peer_name = "River" if profile == "short" else "Apalis"
        lines += [f"## {label}", "", "| Rotation | Engine | First entry | Half entered | Last entry | Second half of entries | Last first write | Last second write | Last settlement |",
                  "|---|---|---:|---:|---:|---:|---:|---:|---:|"]
        backlogs = []
        for group in range(1, 6):
            file, = (HERE.parent / source / "blocks").glob(f"{group:02}-*/reconciled.json")
            sealed = json.loads(file.read_text())
            records = [read(file.parent, lane, sealed, profile) for lane in (mesh, peer)]
            for name, (_, rows, ordered) in zip(("Trusted Mesh", peer_name), records, strict=True):
                entries = ordered["entered_us"]
                half = entries[len(rows)//2 - 1]
                stamps = (entries[0], half, entries[-1], entries[-1] - half,
                          ordered["attempt_committed_us"][-1], ordered["effect_committed_us"][-1], ordered["settled_us"][-1])
                lines.append(f"| {group} | {name} | " + " | ".join(f"{s/1000:.1f}" for s in stamps) + " |")
            origin, jobs, ours = records[0]
            elapsed = records[1][2]["settled_us"][-1]
            cutoff = origin + elapsed
            assert all(j["claimed_us"] % 1000 == 0 and j["claimed_us"] <= j["entered_us"] for j in jobs)
            unclaimed = sum(j["claimed_us"] > cutoff for j in jobs)
            upper = sum(min(j["claimed_us"] + 999, j["entered_us"]) > cutoff for j in jobs)
            backlogs.append((group, elapsed, unclaimed, upper, *[sum(r[k] > cutoff for r in jobs) for k in EDGES]))
        lines += ["", f"### Mesh backlog when {peer_name} finished", "",
                  "The following counts overlap. Each shows how many Mesh jobs still had the indicated step ahead of them "
                  "when the peer reached its last settlement. The claim range allows for the millisecond resolution of the Mesh stamp and uses handler entry as its upper bound.", "",
                  "| Rotation | Peer finished, ms | Unclaimed | Awaiting entry | Awaiting first write | Awaiting second write | Awaiting settlement |",
                  "|---|---:|---:|---:|---:|---:|---:|"]
        for g, elapsed, low, high, *counts in backlogs:
            lines.append(f"| {g} | {elapsed/1000:.1f} | {low}–{high} | " + " | ".join(map(str, counts)) + " |")
        lines += ["", f"[Matched {profile} run ledgers](../{source})", ""]
    lines += ["These are observed arrival and commit times. The backlog at a fixed cutoff can change when an earlier stage changes. [Four-engine scores](four-engine-scorecard.md) · [Replay](handler-progress.py)", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument("--check", action="store_true")
    out = HERE / "handler-progress.md"
    if args.parse_args().check:
        assert out.read_text() == render()
    else:
        out.write_text(render())
