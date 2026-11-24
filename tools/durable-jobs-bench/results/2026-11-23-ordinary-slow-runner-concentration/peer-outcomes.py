"""Report all six native slow outcomes while keeping the Mesh layouts separate."""

import json
from pathlib import Path
from statistics import median
import subprocess
import sys


HERE = Path(__file__).resolve().parent
LANES = {
    "two": "Mesh bulk V1 2 × 32",
    "one-a": "Mesh bulk V1 1 × 64 A",
    "one-b": "Mesh bulk V1 1 × 64 B",
    "river-stock-1x64": "River 1 × 64",
    "oban-stock-1x64": "Oban 1 × 64",
    "apalis-stock-1x64": "Apalis 1 × 64",
}
CLOCKS = ("submit_ms", "drain_ms")


def scores(arms):
    return {lane: tuple(512000 / arms[f"measured-{lane}"][clock] for clock in CLOCKS)
            for lane in LANES}


def main():
    raw = subprocess.check_output([sys.executable, "-B", HERE / "native-import-retry.py", "replay"])
    replay = json.loads(raw)
    attempts = sorted(replay["attempts"])
    if replay["missing"] or [name[:2] for name in attempts] != [f"{i:02}" for i in range(1, 7)]:
        sys.exit("All six original slow orders must reconcile")
    groups = [scores(replay["attempts"][attempt]) for attempt in attempts]
    print("| Deployment | " + " | ".join(f"Group {i} submit / drain" for i in range(1, 7))
          + " | Median submit / drain |")
    print("| --- |" + " ---: |" * 7)
    for lane, label in LANES.items():
        rates = [group[lane] for group in groups]
        mids = tuple(median(column) for column in zip(*rates, strict=True))
        print("| " + label + " | " + " | ".join(f"{submit:.2f} / {drain:.2f}"
                                                for submit, drain in [*rates, mids]) + " |")
    print("\nEach clock is reported in jobs/s. Peer wins below count matched groups.\n")
    print("| Mesh deployment | River submit / drain wins | Oban submit / drain wins | Apalis submit / drain wins |")
    print("| --- | ---: | ---: | ---: |")
    for lane in ("two", "one-a", "one-b"):
        outcomes = []
        for peer in ("river-stock-1x64", "oban-stock-1x64", "apalis-stock-1x64"):
            submit, drain = (sum(group[lane][i] > group[peer][i] for group in groups) for i in range(2))
            outcomes.append(f"{submit}/6 · {drain}/6")
        print("| " + " | ".join([LANES[lane], *outcomes]) + " |")


if __name__ == "__main__":
    main()
