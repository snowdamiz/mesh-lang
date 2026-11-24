"""Replay first-write, second-write and settlement timestamps from native ledgers."""

from pathlib import Path
from statistics import median
import sys
import tarfile

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
sys.path.insert(0, str(SUITE))
import run


def stage_times(ledger, score):
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    n = len(jobs)
    if n != score["completed"] or n not in (128, 1024):
        raise ValueError("unexpected job count")
    fields = ("entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")
    for job in jobs:
        times = [tx["committed_us"], *(job[key] for key in fields)]
        if times != sorted(times):
            raise ValueError("unordered stage timestamps")
    if len({job[key] for job in jobs for key in ("attempt_xid", "effect_xid")}) != 2 * n:
        raise ValueError("application XIDs are not independent")

    last = [max(job[key] for job in jobs) for key in fields]
    for key, start in (("submit_jobs_per_s", tx["submitted_us"]), ("commit_jobs_per_s", tx["committed_us"])):
        if abs(n * 1_000_000 / score[key] - (last[-1] - start)) > 0.5:
            raise ValueError("recorded end-to-end clock differs")

    effects = sorted(job["effect_committed_us"] for job in jobs)
    trim = n // 10
    lo, hi = effects[trim], effects[n - 1 - trim]
    if hi <= lo:
        raise ValueError("empty middle interval")

    def mean_outstanding(start, end):
        return sum(max(0, min(hi, job[end]) - max(lo, job[start])) for job in jobs) / (hi - lo)

    return dict(enqueue=(tx["committed_us"] - tx["submitted_us"]) / 1000,
                entry=(last[0] - tx["committed_us"]) / 1000,
                write=(last[2] - tx["committed_us"]) / 1000,
                tail=(last[3] - last[2]) / 1000,
                submit=(last[3] - tx["submitted_us"]) / 1000,
                waiting_first=mean_outstanding(fields[0], fields[1]),
                between_writes=mean_outstanding(fields[1], fields[2]),
                waiting_terminal=mean_outstanding(fields[2], fields[3]))


def read_series(series, lanes):
    inputs = run.read_json(HERE / "series-list.json")
    root = SUITE / next(item["path"] for item in inputs if item["name"] == series)
    blocks = run.read_json(HERE / "scores.json")[series]["blocks"]
    if [block["group"] for block in blocks] != [1, 2, 3, 4, 5]:
        raise ValueError(f"incomplete {series}")
    groups = []
    for block in blocks:
        rows = {}
        for lane in lanes:
            score = block["scores"][lane]
            folder = f"measured-{lane}/measured-{score['engine']}"
            raw = root / "blocks" / block["attempt"] / folder / "raw.tar.gz"
            if run.digest(raw) != block["raw_sha256"][folder]:
                raise ValueError(f"raw archive digest differs for {raw}")
            with tarfile.open(raw) as archive:
                rows[lane] = stage_times(run.file_json(archive, "adapter/ledger.json"), score)
        groups.append(rows)
    return groups


def report():
    print("# Application writes and durable settlement on the Linode\n")
    print("## Dense jobs with two 32-slot runners per engine\n")
    print("The first three columns count jobs between timestamps. For each job, the replay clips "
          "the interval to the 103rd through 922nd second-write commits, sums the overlapping durations, "
          "and divides by elapsed time. The table shows the median across five rotations. "
          "Each arm committed 2,048 application writes for 1,024 jobs.\n")
    print("| Engine | Entered, awaiting first commit | Between application commits | Second commit to terminal | "
          "Last entry (ms) | Last write (ms) | Final settlement after last write (ms) |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    lanes = {"mesh-v1-2x32": "Mesh V1", "river-2x32": "River", "oban-2x32": "Oban", "apalis-2x32": "Apalis"}
    groups = read_series("dense-equal-topology", lanes)
    for lane, label in lanes.items():
        keys = ("waiting_first", "between_writes", "waiting_terminal", "entry", "write", "tail")
        values = [median(g[lane][key] for g in groups) for key in keys]
        print("| " + " | ".join([label, *(f"{v:,.2f}" for v in values)]) + " |")
    print("\nLast entry and last write are measured from each arm’s durable enqueue commit. The intervals "
          "include scheduling, application code, database work and waits. A peer can continue taking "
          "jobs while earlier terminal acknowledgements are outstanding.\n")
    print("Mesh V1 records entry before decoding the context and calling the application service "
          "for its pool handle. River and Apalis receive a pool with their worker state. "
          "The ledgers have no timestamp for the Mesh service reply. A native comparison that changes only "
          "how the handler receives its pool can price that step. "
          "[Mesh fixture](../2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz) · "
          "[River worker](../../go/worker.go) · [Apalis worker](../../rust/src/main.rs)\n")
    print("## Short jobs with the same bulk Mesh and River binaries\n")
    print("Each row compares 128-job runs. A positive gap means Mesh reached that boundary later. "
          "Last-write gaps use enqueue commit as the origin. The terminal tail measures elapsed time "
          "between the final application commit and the final durable settlement across the arm.\n")
    print("| Series | Rotation | Mesh enqueue (ms) | River enqueue (ms) | Last-write gap (ms) | "
          "Mesh terminal tail (ms) | River terminal tail (ms) | Submit-to-settlement gap (ms) |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    for series, label in (("bulk-hot-short", "Three lanes"), ("bulk-hot-short-peers", "Six lanes")):
        for number, group in enumerate(read_series(series, ("bulk-d2", "river")), 1):
            mesh, river = group["bulk-d2"], group["river"]
            print(f"| {label} | {number} | {mesh['enqueue']:.2f} | {river['enqueue']:.2f} | "
                  f"{mesh['write'] - river['write']:+.2f} | {mesh['tail']:.2f} | {river['tail']:.2f} | "
                  f"{mesh['submit'] - river['submit']:+.2f} |")
    print("\nThe two series ran at different times and in different orders. The observed tails alone "
          "cannot identify the cause of their variation. [Native rates and paired wins](four-engine-scorecard.md) · "
          "[Score inputs and raw archive digests](scores.json)")


if __name__ == "__main__":
    report()
