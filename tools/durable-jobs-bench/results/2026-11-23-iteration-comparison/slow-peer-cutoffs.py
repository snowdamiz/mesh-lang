"""Locate unfinished Mesh work when each peer reaches durable settlement on the Linode."""
import argparse
import hashlib
import json
import tarfile
from pathlib import Path
from statistics import median

HERE = Path(__file__).resolve().parent
EVIDENCE = HERE.parent / "2026-11-24-slow-external-compat-scorer-input/evidence/native"
INPUT = "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
ENGINES = {"bulk": "mesh", "oban": "oban", "river": "go", "apalis": "rust"}
EDGES = ("entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")


def read_arm(block, name, score):
    raw, = (block / name).glob("*/raw.tar.gz")
    assert hashlib.sha256(raw.read_bytes()).hexdigest() == score["raw_sha256"], raw
    with tarfile.open(raw) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        assert json.load(archive.extractfile("invocation.json"))["phase"] == "measured"
        book = json.load(archive.extractfile("adapter/ledger.json"))
    assert book["input_sha256"] == INPUT and book["profile"] == "slow"
    assert book["engine"] == ENGINES[name.split("-")[1]] and book["slots"] == 64
    assert score["affinity"] == [0, 1] and book["peak_connections"] <= 68
    tx, = book["transactions"]
    jobs = book["jobs"]
    assert len(jobs) == len(tx["enqueued_ids"]) == score["completed"] == 512
    assert {j["job_key"]: j["id"] for j in jobs} == tx["enqueued_ids"]
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 1024
    for job in jobs:
        assert job["attempts"] == job["effects"] == 1 and job["state"] == "completed"
        assert job["hold_ms"] == 100
        times = [tx["committed_us"], *(job[k] for k in EDGES)]
        assert times == sorted(times)
    finish = max(job["settled_us"] for job in jobs)
    assert (tx["submitted_us"], tx["committed_us"], finish) == (
        score["first_submit_us"], score["first_commit_us"], score["last_settlement_us"])
    return tx, jobs, finish


def phase_times(run):
    tx, jobs, finish = run
    starts = sorted(j["attempt_committed_us"] for j in jobs)
    edges = (tx["submitted_us"], tx["committed_us"], starts[63], starts[-1], finish)
    return tuple(b - a for a, b in zip(edges, edges[1:]))


def render(groups):
    lines = ["# Slow Mesh jobs outstanding at peer completion", "",
             f"Verified {len(groups)} of five Linode rotations of the 512-job storage comparison. "
             "Each job held for 100 ms between two independent application writes. "
             "Each row compares two arms from the same rotation and storage policy on CPUs 0 and 1.", "",
             "Gap is Mesh finish minus peer finish in milliseconds, relative to each arm’s own submit or enqueue commit. "
             "Positive values mean Mesh finished later. The four counts partition its outstanding jobs at the peer’s finish. "
             "Waiting for the second write includes the required hold. These counts describe progress and do not assign CPU cost.", "",
             "| Group | Storage | Peer | Clock | Gap (ms) | Awaiting entry | Awaiting first write | Awaiting second write | Awaiting settlement |",
             "|---:|---|---|---|---:|---:|---:|---:|---:|"]
    differences = {}
    for group in groups:
        path, = EVIDENCE.glob(f"timing-*/{group:02d}-slow/reconciled.json")
        scores = json.loads(path.read_text())
        for policy in ("default", "external"):
            runs = {}
            for arm in ENGINES:
                name = f"measured-{arm}-{policy}"
                runs[arm] = read_arm(path.parent, name, scores[name])
            mesh_tx, jobs, finish = runs["bulk"]
            payloads = {j["job_key"]: j["payload_sha256"] for j in jobs}
            for peer in ("oban", "river", "apalis"):
                peer_tx, peer_jobs, peer_finish = runs[peer]
                assert payloads == {j["job_key"]: j["payload_sha256"] for j in peer_jobs}
                delta = [a - b for a, b in zip(phase_times(runs["bulk"]), phase_times(runs[peer]))]
                assert sum(delta) == (finish - mesh_tx["submitted_us"]) - (peer_finish - peer_tx["submitted_us"])
                differences.setdefault((policy, peer), []).append(delta)
                for clock, start in (("submit", "submitted_us"), ("commit", "committed_us")):
                    cutoff = mesh_tx[start] + peer_finish - peer_tx[start]
                    reached = [sum(j[k] <= cutoff for j in jobs) for k in EDGES]
                    counts = [a - b for a, b in zip([len(jobs), *reached], reached)]
                    assert min(counts) >= 0 and sum(counts) + reached[-1] == len(jobs)
                    row = [group, policy, peer, clock, f"{(finish - cutoff) / 1000:+.3f}", *counts]
                    lines.append("| " + " | ".join(map(str, row)) + " |")
    lines.extend(["", "## Where the elapsed time differs", "",
                  "Each column gives the median paired Mesh minus peer difference in milliseconds. "
                  "The middle interval covers the next 448 jobs reaching their first durable write. "
                  "The four intervals sum to the submit-to-settlement difference in each individual pair. "
                  "Column medians can come from different pairs.", "",
                  "| Storage | Peer | Submit to enqueue commit | Enqueue commit to 64th first write | 64th to 512th first write | Last first write to final settlement | Mesh faster during middle interval |",
                  "|---|---|---:|---:|---:|---:|---:|"])
    for (policy, peer), pairs in differences.items():
        cells = [f"{median(values) / 1000:+.1f}" for values in zip(*pairs)]
        wins = sum(row[2] < 0 for row in pairs)
        lines.append("| " + " | ".join([policy, peer, *cells, f"{wins}/{len(pairs)}"]) + " |")
    lines.extend(["", "The ranks use committed application writes from each engine’s own ledger. "
                  "They leave CPU attribution and individual claim timing open. "
                  "[Native storage receipts](../2026-11-24-slow-external-compat-scorer-input/evidence/native) · "
                  "[Ledger replay](slow-peer-cutoffs.py)"])
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("groups", type=int, nargs="+", choices=range(1, 6))
    args = parser.parse_args()
    if args.groups != sorted(set(args.groups)):
        parser.error("specify distinct group numbers in ascending order")
    print(render(args.groups), end="")
