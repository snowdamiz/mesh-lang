"""Compare matched dense progress times and reconstruct slot occupancy from durable rows."""
import hashlib
import json
from pathlib import Path
from statistics import mean, median
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "2026-11-23-ordinary-runner-topology"
EDGES = ("claimed_us", "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")


def read_arm(path, digest):
    with path.open("rb") as stream:
        assert hashlib.file_digest(stream, "sha256").hexdigest() == digest
    with tarfile.open(path) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    tx, = ledger["transactions"]
    origin = tx["committed_us"]
    rows = {}
    xids = {tx["xid"]}
    for job in ledger["jobs"]:
        times = [max(0, job[k] - origin) for k in EDGES]
        assert times == sorted(times)
        assert job["attempts"] == job["effects"] == job["attempt"] == 1
        assert job["job_key"] not in rows
        rows[job["job_key"]] = times
        xids.update(job[k] for k in ("attempt_xid", "effect_xid", "settled_xid"))
    assert len(xids) == 3073 and len(rows) == 1024 and rows.keys() == tx["enqueued_ids"].keys()
    return rows


def timing(rows):
    last_claim = max(v[0] for v in rows.values())
    last_done = max(v[-1] for v in rows.values())
    means = [mean(v[i+1]-v[i] for v in rows.values())/1000 for i in range(4)]
    return {"last_claim_ms": last_claim / 1000, "drain_ms": last_done / 1000,
            "tail_ms": (last_done - last_claim) / 1000, "mean_stages_ms": means,
            "occupied": [s * len(rows)/(last_done/1000) for s in means],
            "postclaim_mean_ms": sum(means)}


def replay():
    seal = {line.split("  ", 1)[1]: line[:64] for line in (SOURCE / "SHA256SUMS").read_text().splitlines()}
    summary = json.loads((SOURCE / "evidence/native-01/summary.json").read_text())
    results = []
    for group in summary["groups"]:
        measured = {}
        for label, source in group["arms"].items():
            path = f"evidence/native-01/{group['number']:02}/{label}/diagnostic-mesh/raw.tar.gz"
            rows = read_arm(SOURCE/path, seal[path])
            times = timing(rows)
            assert abs(1024e3/times["drain_ms"] - source["commit_jobs_per_s"]) < 1e-6
            assert abs(sum(times["occupied"])-source["mean_occupied_slots"]) < 1e-6
            measured[label] = (rows, times)
        candidate, c = measured["tuned-2x32"]
        faster = min(("stock-a", "stock-b"), key=lambda k: measured[k][1]["drain_ms"])
        baseline, b = measured[faster]
        assert baseline.keys() == candidate.keys()
        diff = b["drain_ms"] - c["drain_ms"]
        claim_diff = b["last_claim_ms"] - c["last_claim_ms"]
        assert abs(diff - (claim_diff + b["tail_ms"] - c["tail_ms"])) < 1e-6
        cutoff = round(c["drain_ms"] * 1000)
        # Claim stamps are floored to a millisecond. Bound unclaimed jobs at this cutoff.
        pending = [sum(v[0] > cutoff for v in baseline.values()),
                   sum(min(v[0]+999,v[1]) > cutoff for v in baseline.values())]
        results.append(dict(group=group["number"], stock=faster, drain_gain_ms=diff,
                       last_claim_gain_ms=claim_diff, tail_gain_ms=b["tail_ms"]-c["tail_ms"],
                       stock_unclaimed_when_candidate_finished=pending,
                       arms={k: t for k, (_, t) in measured.items()}))
    return results


def report(results):
    lines = ["# Where the two-runner dense drain gained time", "",
        "Two native 1,024-job rotations compared one, two, and eight trusted Mesh V1 runners on the same two Linode CPUs. All deployments kept 64 slots and 32 warm application connections. Each of the eight arms recorded two application transactions and one separate settlement transaction per job. The replay verifies all eight archive digests, durable XIDs and occupancy integrals.", "",
        "| Rotation | Faster eight-runner control | Two-runner drain gain (ms) | Earlier last claim (ms) | Change in last-claim-to-finish tail (ms saved) | Control jobs still unclaimed when two runners finished |",
        "|---:|---|---:|---:|---:|---:|"]
    for g in results:
        lo, hi = g["stock_unclaimed_when_candidate_finished"]
        lines.append(f"| {g['group']} | {g['stock']} | {g['drain_gain_ms']:.1f} | {g['last_claim_gain_ms']:.1f} | {g['tail_gain_ms']:.1f} | {lo}–{hi} |")
    lines += ["", "The last-claim and tail differences sum to the total gain. This locates the elapsed difference. The time of a claim also depends on the release of earlier slots, scheduling and database service. Each cutoff is aligned to that arm’s own enqueue commit. The unclaimed interval allows for the millisecond resolution of the claim stamp.", "",
        "| Layout | Mean claimed to entry (ms/job) | Entry to first commit | First to second commit | Second commit to settlement | Mean occupied slots |",
        "|---|---:|---:|---:|---:|---:|"]
    for name, labels in [("Eight runners (both controls)", ("stock-a", "stock-b")), ("Two runners", ("tuned-2x32",)), ("One runner", ("tuned-1x64",))]:
        arms = [g["arms"][k] for g in results for k in labels]
        cells = [median(v) for v in zip(*(a["mean_stages_ms"] for a in arms), strict=True)]
        cells.append(median([sum(a["occupied"]) for a in arms]))
        lines.append("| " + name + " | " + " | ".join(f"{v:.2f}" for v in cells) + " |")
    lines += ["", "Each stage is the across-arm median of its mean per-job residence time. Two runners sustained more work in flight and moved the final claim forward despite a longer wait from claim to handler entry per job. Process count, scheduler supply and the maximum grant size differed together in this native diagnostic. [Original topology report](../2026-11-23-ordinary-runner-topology/README.md) · [Replay](topology.py)", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    (HERE/"native-progress.md").write_text(report(replay()))
