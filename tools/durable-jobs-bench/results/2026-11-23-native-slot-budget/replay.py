"""Replay the sealed native backlog ledgers. Timings are in integer microseconds."""

from hashlib import sha256
import json
from pathlib import Path
from statistics import median
import tarfile


RESULTS = Path(__file__).resolve().parents[1]
ROOT = RESULTS.parents[2]
BASE = RESULTS / "2026-11-23-baseline"
ANALYSIS = RESULTS / "2026-11-23-baseline-stage-analysis" / "analysis.json"
ANALYSIS_HASH = "df5819943eed98ac7055038f2a3fbe1158a40704a641c53df9bc97d75228e1f9"
PROFILES = {"noop": 64, "slow": 64, "short": 16, "large": 64}


def mean_occupied(jobs, left, right):
    # Jobs stores floor(clock milliseconds). Place each grant as late as
    # its stamp permits, allowing for a handler that entered within that ms.
    total = 0
    for row in jobs:
        granted = min(row["claimed_us"] + 999, row["entered_us"])
        total += max(0, min(row["settled_us"], right) - max(granted, left))
    return total / (right - left)


def price(block, index):
    mesh = block["arms"]["mesh"]
    receipt = BASE / "blocks" / block["block"] / "measured-mesh" / "raw.tar.gz"
    digest = sha256(receipt.read_bytes()).hexdigest()
    if digest != index["sources"][receipt.relative_to(ROOT).as_posix()]:
        raise ValueError(f"{receipt} differs from the sealed raw arm")
    with tarfile.open(receipt, "r:gz") as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    slots = PROFILES[block["profile"]]
    if len(ledger["transactions"]) != 1 or len(jobs) != mesh["jobs"] or ledger["slots"] != slots:
        raise ValueError("expected one committed backlog and the scored slot and job counts")
    start = ledger["transactions"][0]["committed_us"]
    ordered = sorted(row["settled_us"] for row in jobs)
    end = ordered[-1]
    if start != mesh["first_commit_us"] or end != mesh["last_terminal_us"]:
        raise ValueError("commit window differs from the published score")
    span = sum(row["settled_us"] - row["claimed_us"] for row in jobs)
    if span != mesh["residence_us"]["claim_to_terminal"]:
        raise ValueError("residence sum differs from the stage replay")
    if any(row["claimed_us"] % 1000 for row in jobs):
        raise ValueError("unexpected Mesh claim timestamp precision")
    for row in jobs:
        if not (row["claimed_us"] <= row["entered_us"] <= row["settled_us"]
                and start <= row["entered_us"]):
            raise ValueError("invalid Mesh attempt timeline")
    low = sum(row["settled_us"] - min(row["entered_us"], row["claimed_us"] + 999)
              for row in jobs)
    a, b = mesh["middle80_ranks"]
    first, last = ordered[a - 1], ordered[b - 1]
    if last - first != mesh["window_us"]["middle80"]:
        raise ValueError("middle completion ranks disagree with the stage replay")
    output = {"group": block["group"], "profile": block["profile"], "jobs": len(jobs),
              "slots": slots, "full_mean_occupied_lower": mean_occupied(jobs, start, end),
              "middle_mean_occupied_lower": mean_occupied(jobs, first, last), "peers": {}}
    for engine in ("oban", "go", "rust"):
        peer_us = block["arms"][engine]["window_us"]["commit"]
        cutoff = start + peer_us
        output["peers"][engine] = {
            "peer_drain_us": peer_us,
            "unclaimed_at_peer_duration_lower": sum(row["claimed_us"] > cutoff for row in jobs),
            "entered_at_peer_duration": sum(row["entered_us"] <= cutoff for row in jobs),
            "minimum_mean_residence_cut_us": max(0, low - slots * peer_us) / len(jobs),
            "minimum_mean_residence_cut_fraction": max(0, low - slots * peer_us) / low,
        }
    return output


def main():
    data = ANALYSIS.read_bytes()
    if sha256(data).hexdigest() != ANALYSIS_HASH:
        raise ValueError("stage receipts differ from the reviewed baseline")
    index = json.loads(data)
    rows = [price(block, index) for block in index["blocks"] if block["profile"] in PROFILES]
    for profile, slots in PROFILES.items():
        arms = [row for row in rows if row["profile"] == profile]
        if {row["group"] for row in arms} != set(range(1, 6)) or len(arms) != 5:
            raise ValueError("incomplete five-group profile")
        full = median(row["full_mean_occupied_lower"] for row in arms)
        middle = median(row["middle_mean_occupied_lower"] for row in arms)
        print(f"{profile}  slots {slots}  mean occupied lower bound, full {full:.2f}, middle {middle:.2f}")
        for engine in ("oban", "go", "rust"):
            peers = [row["peers"][engine] for row in arms]
            unclaimed = [p["unclaimed_at_peer_duration_lower"] for p in peers]
            cut = [p["minimum_mean_residence_cut_us"] / 1000 for p in peers]
            fraction = median(p["minimum_mean_residence_cut_fraction"] for p in peers)
            print(f"  {engine:4s}  unclaimed {min(unclaimed)}–{max(unclaimed)}  "
                  f"required cut {min(cut):.2f}–{max(cut):.2f} ms/job  median {fraction:.1%}")


if __name__ == "__main__":
    main()
