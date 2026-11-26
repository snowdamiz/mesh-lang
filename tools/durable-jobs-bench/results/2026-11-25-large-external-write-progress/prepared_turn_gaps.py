"""Replay the prepared pilot's client-confirmed gaps from immutable Git blobs."""
import argparse
from hashlib import sha256
from io import BytesIO
import json
import subprocess
import tarfile

REV = "dc5458b2c09d23588a91e8501901dc65b49651f8"
ROOT = "tools/durable-jobs-bench/results/2026-11-25-public-large-prepared-render-spans/evidence/pilot"
SCORE = "2125f771fec5ef5c0e26cb6ec688555f52a48c532b7eee2546d69577e7220e6e"
EVENTS = ("attempt_committed_us", "effect_committed_us", "settled_us")


def blob(path):
    return subprocess.check_output(["git", "show", f"{REV}:{ROOT}/{path}"])


def activation_budget(jobs, turns, zero, score, group):
    # Eliminate all elapsed time from the first stamp through checked receipt,
    # even when one receipt overlaps the following turn. This overcredits UPDATE.
    credit = 0
    owners = {}
    for ordinal, (first, _, receipt, pid) in enumerate(turns, 1):
        credit += receipt - first
        owners[pid] = (8 * ordinal, credit)
    enqueue = round(score["enqueue_ms"] * 1000)
    drain = max(job["settled_us"] for job in jobs) - zero
    assert drain == round(score["drain_ms"] * 1000)
    assert drain + enqueue == round(score["submit_ms"] * 1000)
    assert all(prior[1] <= following[0] for prior, following in zip(turns, turns[1:]))
    assert max(t[2] for t in turns) - zero == round(score["confirmed_ms"] * 1000)
    margins = group["gate"]["margins"]
    deadlines = {}
    for clock, event in (("submit_ms", "settled_us"), ("drain_ms", "settled_us"),
                         ("effect_committed_us/commit/64", "effect_committed_us")):
        margin = margins["material_" + clock]
        end = round(1000 * (margin["reference"] - margin["threshold"]))
        deadlines[clock] = (event, end - enqueue if clock == "submit_ms" else end)
    peer = group["historical"]["clocks"]["oban_drain_ms"]
    for label, cutoff in (("loose", max(peer["deadlines"])), ("tight", min(peer["deadlines"]))):
        deadlines["historical_oban_" + label] = ("settled_us", round(1000 * (cutoff - peer["noise"])))
    result = {}
    for name, (event, deadline) in deadlines.items():
        required_per_update = 0
        residuals = []
        for job in jobs:
            prefix_rows, gross_credit = owners[job["worker_pid"]]
            deficit = job[event] - zero - deadline
            required_per_update = max(required_per_update, deficit / prefix_rows)
            residuals.append((deficit - gross_credit, prefix_rows // 8, gross_credit))
        residual, ordinal, gross_credit = max(residuals)
        result[name] = dict(deadline_from_commit_us=deadline,
                            uniform_saving_to_reach_cutoff_us_per_update=round(required_per_update, 3),
                            residual_after_full_credit_us=residual, limiting_turn=ordinal,
                            gross_prefix_credit_us=gross_credit)
    return result


def reduce_arm(raw, score, group=None):
    assert sha256(raw).hexdigest() == score["raw_sha256"]
    with tarfile.open(fileobj=BytesIO(raw)) as archive:
        def member(path):
            return archive.extractfile(f"adapter/{path}").read()

        ledger = json.loads(member("ledger.json"))
        txn, = ledger["transactions"]
        jobs = ledger["jobs"]
        assert len(jobs) == len(txn["enqueued_ids"]) == 64
        assert len({job[k] for job in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 192
        zero = txn["committed_us"]
        turns = []
        _, *runners = json.loads(member("process-identity.json"))
        assert len({p["pid"] for p in runners}) == len(runners) == 8
        for i, runner in enumerate(runners):
            ready, final = member(f"runner-{i}.log").splitlines()
            assert ready == b"READY"
            report = json.loads(final)
            owned = [j for j in jobs if j["worker_pid"] == runner["pid"]]
            assert len(owned) == report["claimed"] == report["finished"] == 8
            assert report["batch_protocol"] == "locked_prepare_pg17_v1"
            receipt = report["last_batch_confirmed_us"]
            stamps = [j["claimed_us"] for j in owned]
            assert all(zero <= s <= receipt for s in stamps)
            assert all(receipt <= j["entered_us"] <= j[EVENTS[0]] <= j[EVENTS[1]] <= j[EVENTS[2]] for j in owned)
            turns.append((min(stamps), max(stamps), receipt, runner["pid"]))

    turns.sort()
    if group is not None:
        return activation_budget(jobs, turns, zero, score, group)
    gaps = []
    for prior, following in zip(turns, turns[1:]):
        start, end = prior[2], following[0]
        assert prior[1] <= end
        gaps.append(dict(previous_pid=prior[3], next_pid=following[3],
                         stamp_gap_us=end - prior[1], after_receipt_us=max(0, end - start),
                         commits_inside={k: sum(start < j[k] < end for j in jobs) for k in EVENTS}))
    last_receipt = max(t[2] for t in turns)
    assert last_receipt - zero == round(score["confirmed_ms"] * 1000)
    parts = dict(first_stamp_us=turns[0][0] - zero, inside_turns_us=sum(t[1] - t[0] for t in turns),
                 between_turns_us=sum(g["stamp_gap_us"] for g in gaps),
                 final_receipt_tail_us=last_receipt - turns[-1][1])
    assert sum(parts.values()) == last_receipt - zero
    receipts = {pid: checked for _, _, checked, pid in turns}
    # Shift each job separately, then find the latest across all 64 shifted jobs.
    # Keep later grants and all downstream per-job durations at their observed values.
    ceilings = {}
    for phase, start, stop in (
        ("receipt_to_entry", [receipts[j["worker_pid"]] for j in jobs], "entered_us"),
        ("entry_to_first_write", [j["entered_us"] for j in jobs], "attempt_committed_us"),
        ("receipt_to_first_write", [receipts[j["worker_pid"]] for j in jobs], "attempt_committed_us"),
    ):
        elapsed = [j[stop] - begin for j, begin in zip(jobs, start)]
        ceilings[phase] = {
            k: max(j[k] for j in jobs) - max(j[k] - dt for j, dt in zip(jobs, elapsed))
            for k in ("effect_committed_us", "settled_us")
        }
    return dict(raw_sha256=score["raw_sha256"], fixed_schedule_gain_ceiling_us=ceilings, parts=parts, confirmed_us=last_receipt - zero,
                after_receipt_us=sum(g["after_receipt_us"] for g in gaps),
                commits_inside={k: sum(g["commits_inside"][k] for g in gaps) for k in EVENTS}, gaps=gaps)


def replay(activation=False):
    raw_score = blob("scored-before-release.json")
    assert sha256(raw_score).hexdigest() == SCORE
    groups = json.loads(raw_score)["groups"]
    dirs = subprocess.check_output(["git", "ls-tree", "--full-tree", "--name-only", f"{REV}:{ROOT}/blocks"], text=True).splitlines()
    result = {}
    for group in groups:
        directory, = (d for d in dirs if d.startswith(f'{group["group"]:02d}-'))
        for phase, arms in group["arms"].items():
            assert set(arms) == {"bridge", "eager", "span-a", "span-b", "stock-a", "stock-b"}
            for label, score in arms.items():
                if label == "bridge":
                    continue
                if activation and (phase != "measured" or label not in {"span-a", "span-b"}):
                    continue
                path = f"blocks/{directory}/{phase}-{label}/{phase}-mesh/raw.tar.gz"
                result[f'{group["group"]}/{phase}/{label}'] = reduce_arm(blob(path), score, group if activation else None)
    assert len(result) == (4 if activation else 20)
    return dict(custody=REV, score_sha256=SCORE, arms=result)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--activation", action="store_true", help="price changes after the first row stamp of each turn")
    print(json.dumps(replay(parser.parse_args().activation), indent=2, sort_keys=True))
