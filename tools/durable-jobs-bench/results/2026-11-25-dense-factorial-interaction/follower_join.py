"""Replay a deterministic follower offer before its first wire send; archive arithmetic only."""
import json
import sys
from collections import Counter
from pathlib import Path

from claim_phase_budget import VERIFIED, checked


def ident(turn):
    return turn["pid"], turn["seq"]


def run(arm):
    turns = sorted((t for t in arm["turns"] if t["first_send_ns"] is not None),
                   key=lambda t: (t["first_send_ns"], ident(t)))
    used = set()
    pending = []
    rank = {ident(t): i for i, t in enumerate(sorted(
        (t for t in turns if t["rows"]), key=lambda t: t["grant_stamp_us"]))}
    counts = Counter()
    interior = []
    start, end = arm["write_edges_us"]
    low, high = arm["clock_offsets_ns"]["utc"]
    for current in turns:
        edge = current["first_send_ns"]
        # A receipt closes registration; its future status is invisible at edge.
        pending = [p for p in pending if p["prelock_receipt"]["ns"] > edge]
        candidates = [p for p in pending if ident(p) not in used and p["pid"] != current["pid"]
                      and p["first_send_ns"] < edge and not any(c["ns"] <= edge for c in p["cut"])]
        if candidates and not any(c["ns"] <= edge for c in current["cut"]):
            leader = candidates[0]  # Earliest outstanding prelock send.
            used.update([ident(current), ident(leader)])
            counts["joined"] += 1
            if not leader["rows"] or not current["rows"]:
                counts[f"empty_or_failed_leader_{int(not leader['rows'])}_follower_{int(not current['rows'])}"] += 1
                continue
            if current["cut"] or leader["cut"] or current["fallback"] or leader["fallback"]:
                counts["cut_or_fallback"] += 1
                continue
            if rank[ident(current)] != rank[ident(leader)] + 1:
                counts["nonconsecutive_durable_grants"] += 1
                continue
            decision = leader["select"]["ns"]
            credits = sum(c["value"] for c in current["credits"] if c["ns"] < decision)
            displaced = max(0, current["rows"] - min(8, current["orig"] + credits))
            if displaced:
                counts["follower_loses_rows"] += 1
                continue
            counts["consecutive_and_rows_fit"] += 1
            if edge + low >= start * 1000 and current["received_ns"] + high <= end * 1000:
                counts["consecutive_and_rows_fit_interior"] += 1
                assert current["first_send_ns"] < current["confirmed_ns"] <= current["received_ns"]
                assert current["select_receipts"] and leader["select_receipts"]
                marks = {e["mark"]: e["ns"] for e in current["events"]}
                interior.append({
                    "leader": ident(leader), "follower": ident(current),
                    "leader_rows": leader["rows"], "follower_rows": current["rows"],
                    "follower_select_ns": marks["select_receipt"] - marks["select_send"],
                    "follower_commit_ns": marks["commit_receipt"] - marks["commit_send"],
                    "follower_prelock_ns": (marks["prelock_receipt"] - marks["prelock_send"]
                                            if "prelock_receipt" in marks else None),
                    "bytes_both_selects": sum(t["select_receipts"][0]["value"]
                                              for t in (leader, current)),
                })
        elif current["prelock_receipt"] is not None:
            pending.append(current)
    return {
        "counts": counts, "interior_pairs": interior,
        "summed_follower_commit_ms": sum(p["follower_commit_ns"] for p in interior) / 1e6,
        "summed_follower_select_ms": sum(p["follower_select_ns"] for p in interior) / 1e6,
    }


if __name__ == "__main__":
    report = json.loads(checked((Path(sys.argv[1]) / "verified.json").read_bytes(), VERIFIED))
    print(json.dumps([run(g["arms"]["observe/measured"]) for g in report["result"]["groups"]],
                     indent=2, sort_keys=True))
