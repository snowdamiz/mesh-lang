"""Replay handler occupancy between the 64th and final committed first writes on Linode."""
import json
from pathlib import Path
from runpy import run_path
from statistics import median
import tarfile


HERE = Path(__file__).resolve().parent
PACKET = HERE.parent / "2026-11-24-ordinary-slow-equal-topology-cap71"
SERIES = PACKET / "evidence/native/sql-retry-timing"
RANKS = (1, 64, 128, 192, 256, 320, 384, 448, 512)
SLOTS = 64


def window(jobs, commit):
    firsts = sorted(job["attempt_committed_us"] for job in jobs)
    start, end = firsts[63], firsts[-1]
    areas = [0, 0, 0]
    events = []
    for job in jobs:
        entry, first, second = (job[key] for key in ("entered_us", "attempt_committed_us", "effect_committed_us"))
        hold_end = first + 100_000
        if not (job["hold_ms"] == 100 and commit <= entry <= first <= hold_end <= second <= job["settled_us"]):
            raise ValueError("unexpected hold duration or timestamp order")
        events.extend(((entry, 1), (second, -1)))
        for index, (left, right) in enumerate(((entry, first), (first, hold_end), (hold_end, second))):
            areas[index] += max(0, min(right, end) - max(left, start))
    active = 0
    for _, change in sorted(events):
        active += change
        if not 0 <= active <= SLOTS:
            raise ValueError("handler occupancy outside available slots")
    width = end - start
    outside = SLOTS * width - sum(areas)
    if active or outside < 0:
        raise ValueError("unbalanced handler intervals")
    return ([firsts[rank - 1] - commit for rank in RANKS], [width] + [x / SLOTS for x in (*areas, outside)])


def measured():
    replay = run_path(str(PACKET / "native-report.py"))["replay_native"](SERIES)
    blocks = []
    for name, details in sorted(replay["arm_audit"].items(), key=lambda item: item[1]["group"]):
        arms = {}
        for engine in ("mesh", "rust", "go", "oban"):
            label = f"measured-{engine}"
            with tarfile.open(SERIES / "blocks" / name / label / label / "raw.tar.gz") as archive:
                ledger = json.load(archive.extractfile("adapter/ledger.json"))
            txn, = ledger["transactions"]
            arms[engine] = window(ledger["jobs"], txn["committed_us"])
        blocks.append((details["group"], arms))
    if [group for group, _ in blocks] != list(range(1, 9)) or replay["excluded_attempts"]:
        raise ValueError("unexpected series coverage")
    return blocks


def render(blocks):
    lines = ["# Slow-job refill on two 32-slot runners", "",
             "The eight Linode blocks each ran 512 jobs with a 100 ms hold and two independently committed application writes per job. "
             "Each rank uses a different job’s first committed write in each engine’s execution order. "
             "Times start at the engine’s own enqueue commit. Positive differences mean Mesh reached that rank later.", "",
             "## Mesh minus Apalis at each first-write rank (ms)", "",
             "| Block | " + " | ".join(map(str, RANKS)) + " |", "|---:|" + "---:|" * len(RANKS)]
    for group, arms in blocks:
        delta = [a - b for a, b in zip(arms["mesh"][0], arms["rust"][0])]
        lines.append(f"| {group} | " + " | ".join(f"{x / 1000:+.1f}" for x in delta) + " |")
    lines.extend(["", "## Slot occupancy during refill", "",
                  "The window begins with the 64th first write and ends with the 512th. "
                  "Each job contributes only its time inside that window. Summed durations are divided by 64, so the four occupancy columns add up to the window width in each arm. "
                  "The minimum hold column counts exactly 100 ms after each first write, clipped at either window edge. "
                  "The last column counts slot time outside the recorded interval from handler entry through the second write. "
                  "It includes work before entry and after the second write, as well as time without a reservation.", "",
                  "| Paired median, Mesh minus peer (ms) | Window width | Entry to first write | Minimum hold | End of minimum hold to second write | Outside entry to second write | Mesh had longer window |",
                  "|---|---:|---:|---:|---:|---:|---:|"])
    for engine, label in (("rust", "Apalis"), ("go", "River"), ("oban", "Oban")):
        differences = [[a - b for a, b in zip(arms["mesh"][1], arms[engine][1])] for _, arms in blocks]
        medians = [median(column) / 1000 for column in zip(*differences)]
        later = sum(row[0] > 0 for row in differences)
        lines.append(f"| {label} | " + " | ".join(f"{x:+.1f}" for x in medians) + f" | {later}/{len(blocks)} |")
    lines.extend(["", "Paired medians can come from different blocks. Engines can have different numbers of holds crossing the window edges. "
                  "The occupancy differences measure wall time and cannot isolate SQL, dispatcher CPU, or the time a runner took to release a slot.",
                  "", "[Matched results and raw ledgers](../2026-11-24-ordinary-slow-equal-topology-cap71/NATIVE-RESULTS.md) · "
                  "[Replay](slow-equal-refill.py)"])
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    print(render(measured()), end="")
