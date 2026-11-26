"""Price earlier cursor handoffs against the released pilot's historical peer screens."""

from fractions import Fraction
import hashlib
import io
import json
from pathlib import Path
from statistics import median
import subprocess
import tarfile


OWNER = "91e8270e3e9d4f96869d94ad879610a8fd862301"
EVIDENCE = "tools/durable-jobs-bench/results/2026-11-25-public-large-cursor-render/pilot/evidence"
TURNS = Path(__file__).with_name("ordinary-grant-turns.json")


def git_blob(name):
    return subprocess.check_output(["git", "show", f"{OWNER}:{EVIDENCE}/{name}"])


def timeline(raw, widths, score):
    assert hashlib.sha256(raw).hexdigest() == score["raw_sha256"]
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    tx, = ledger["transactions"]
    jobs = sorted(ledger["jobs"], key=lambda j: (j["claimed_us"], j["job_key"]))
    assert len({job["id"] for job in jobs}) == sum(widths) == len(jobs) == 64
    commit = tx["committed_us"]
    events = []
    start = 0
    for width in widths:
        turn = jobs[start:start + width]
        assert len({job["worker_pid"] for job in turn}) == 1
        events.append(dict(first=turn[0]["claimed_us"], entry=min(job["entered_us"] for job in turn),
                           done=max(job["settled_us"] for job in turn) - commit))
        start += width
    enqueue = commit - tx["submitted_us"]
    done = max(t["done"] for t in events)
    assert enqueue == round(score["enqueue_ms"] * 1000)
    assert done == round(score["drain_ms"] * 1000)
    assert enqueue + done == round(score["submit_ms"] * 1000)
    return enqueue, events


def budget(events, deadline):
    windows = [max(0, b["first"] - a["entry"]) for a, b in zip(events, events[1:])]
    available = 0
    remaining = []
    fraction = Fraction(0)
    feasible = True
    for index, event in enumerate(events):
        if index:
            available += windows[index - 1]
        need = max(0, event["done"] - deadline)
        remaining.append(event["done"] - available)
        if need and not available:
            feasible = False
        if available:
            fraction = max(fraction, Fraction(need, available))
    return dict(observed_late_us=max(t["done"] for t in events) - deadline,
                full_credit_late_us=max(remaining) - deadline,
                uniform_share_needed_percent=round(100 * float(fraction), 2) if feasible else None,
                positive_window_us=sum(windows), windows_us=windows)


def main():
    release_bytes = git_blob("controller-release.json")
    scored = git_blob("scored-before-release.json")
    turns = json.loads(TURNS.read_bytes())
    assert hashlib.sha256(release_bytes).hexdigest() == turns["release_sha256"]
    assert hashlib.sha256(scored).hexdigest() == "87179bd17fb8219c807843c770daf501b2d3e8b825fa93915a99847841d72102"
    official = json.loads(scored)
    root = subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip()
    paths = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", OWNER, "--", EVIDENCE],
                                    cwd=root, text=True).splitlines()
    result = {}
    for policy, details in official["policies"].items():
        for group in details["groups"]:
            block = group["block"]
            gate = group["gate"]
            noise = gate["duplicate_noise_ms"]
            for lane in ("span-a", "span-b"):
                name, = [path.removeprefix(EVIDENCE + "/") for path in paths
                         if f"/{policy}/blocks/{block:02}-" in path
                         and path.endswith(f"/measured-{lane}/measured-mesh/raw.tar.gz")]
                score = group["arms"]["measured"][lane]
                turn = turns["policies"][policy][str(block)]["measured"][lane]
                assert turn["partitions"] == 1 and turn["raw_sha256"] == score["raw_sha256"]
                enqueue, events = timeline(git_blob(name), turn["widths"], score)
                out = {}
                for peer in ("river", "oban"):
                    clocks = gate["historical_screen"][peer]
                    # Reconstruct integer peer clocks from their released 5/6 deadlines.
                    limits = {}
                    for clock, offset in (("drain_ms", 0), ("submit_ms", enqueue)):
                        peer_us = median([round(t * 1200) for t in clocks[clock]["deadlines_ms"]])
                        limits[clock] = peer_us * 5 // 6 - round(1000 * noise[clock]) - offset
                    out[peer] = dict(drain_limit_us=limits["drain_ms"], submit_limit_after_enqueue_us=limits["submit_ms"],
                                     **budget(events, min(limits.values())))
                    assert out[peer]["positive_window_us"] == turn["after_previous_handler_entry"]["after_prior_entry_us"]
                result[f"{policy}/{block}/{lane}"] = out
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
