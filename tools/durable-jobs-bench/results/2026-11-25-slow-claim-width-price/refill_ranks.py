"""Account for observed first-to-last progress across eight ranks of 64 jobs."""
import argparse
import hashlib
import json
from pathlib import Path
import tarfile

from replay import STOCK


def ranks(jobs, field):
    times = sorted(j[field] for j in jobs)
    return times[63::64]


def read(path, digest):
    assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as raw:
        assert json.load(raw.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(raw.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    assert ledger["slots"] == 64 and ledger["runners"] == 1
    assert len(jobs) == len({j["id"] for j in jobs}) == 512
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 1024
    assert all(j["attempts"] == j["effects"] == 1 and j["state"] == "completed"
               and j["hold_ms"] == 100 and j["entered_us"] <= j["attempt_committed_us"]
               and j["attempt_committed_us"] + 100_000 <= j["effect_committed_us"]
               <= j["settled_us"] for j in jobs)
    entry, first, effect, terminal = (ranks(jobs, field) for field in (
        "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us"))
    transitions = []
    for i in range(7):
        transitions.append({
            "settled_rank": 64 * (i + 1),
            "new_entry_rank": 64 * (i + 2),
            "settlement_to_next_entry_us": entry[i + 1] - terminal[i],
            "next_entry_to_first_commit_us": first[i + 1] - entry[i + 1],
        })
    if ledger["engine"] == "mesh":
        assert len({j["settled_xid"] for j in jobs}) == 512
        assert all(row["settlement_to_next_entry_us"] >= 0 for row in transitions)
    parts = {
        "enqueue": tx["committed_us"] - tx["submitted_us"],
        "initial_64_first_commits": first[0] - tx["committed_us"],
        "required_holds": 800_000,
        "excess_between_commits": sum(b - a - 100_000 for a, b in zip(first, effect)),
        "effect_to_terminal": sum(b - a for a, b in zip(effect, terminal)),
        "settlement_to_next_entry": sum(r["settlement_to_next_entry_us"] for r in transitions),
        "next_entry_to_first_commit": sum(r["next_entry_to_first_commit_us"] for r in transitions),
    }
    assert sum(parts.values()) == terminal[-1] - tx["submitted_us"]
    return {"raw_sha256": digest, "components_us": parts, "transitions": transitions}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", type=Path)
    parser.add_argument("--sha256")
    args = parser.parse_args()
    if args.raw is not None or args.sha256 is not None:
        if args.raw is None or args.sha256 is None:
            parser.error("--raw and --sha256 are required together")
        result = read(args.raw, args.sha256)
    else:
        summary = (STOCK / "summary.json").read_bytes()
        assert hashlib.sha256(summary).hexdigest() == "763b9cb39bec01492a9cf6ab6240e5758ef90573345c6571fd5e2fb851d3eaa8"
        groups = json.loads(summary)["groups"]
        assert len(groups) == 5
        result = {}
        for number, group in enumerate(groups, 1):
            result[number] = {}
            for lane in ("product", "river"):
                path, = STOCK.glob(f"blocks/{number:02}-*/measured-{lane}/*/raw.tar.gz")
                result[number][lane] = read(path, group[lane]["raw_sha256"])
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
