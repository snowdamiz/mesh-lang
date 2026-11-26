"""Compare prepared FILE receipts with five unscaled, historical Oban drain clocks."""
from hashlib import sha256
from itertools import groupby
import json
from pathlib import Path
import sys
import tarfile


def measure(raw, expected, deadlines):
    assert sha256(raw.read_bytes()).hexdigest() == expected
    with tarfile.open(raw) as archive:
        def read(name):
            return json.load(archive.extractfile("adapter/" + name + ".json"))

        ledger = read("ledger")
        producer, *runners = read("process-identity")
        receipts = {}
        for i, runner in enumerate(runners):
            ready, report = archive.extractfile(f"adapter/runner-{i}.log").read().splitlines()
            report = json.loads(report)
            assert ready == b"READY" and report["batch_protocol"] == "locked_prepare_pg17_v1"
            assert report["drained"] and not report["last_error"]
            receipts[runner["pid"]] = report["last_batch_confirmed_us"]
        activation, = (s for s in read("queue-policy")["queue_statements"] if "activate_batch_v1(" in s["query"])
    transaction, = ledger["transactions"]
    zero = transaction["committed_us"]
    jobs = ledger["jobs"]
    assert len(jobs) == len({j["id"] for j in jobs}) == 64 and len(receipts) == 8
    assert transaction["enqueued_ids"] == {j["job_key"]: j["id"] for j in jobs}
    ordered = sorted(jobs, key=lambda j: j["claimed_us"])
    turns = [(pid, list(rows)) for pid, rows in groupby(ordered, key=lambda j: j["worker_pid"])]
    assert activation["calls"] == len(turns) and activation["rows"] == len(jobs) + len(turns)
    final_turn = {pid: i for i, (pid, _) in enumerate(turns)}
    assert final_turn.keys() == receipts.keys()
    last_pid = max(receipts, key=receipts.get)
    last_receipt = receipts[last_pid]
    assert sum(value == last_receipt for value in receipts.values()) == 1
    effects = [j["effect_committed_us"] for j in jobs]
    settlements = [j["settled_us"] for j in jobs]
    post_credit = []
    for i, (pid, rows) in enumerate(turns):
        last_stamp = rows[-1]["claimed_us"]
        first_entry = min(j["entered_us"] for j in rows)
        # If earlier receipt is censored, allow it as late as first handler entry.
        receipt = receipts[pid] if final_turn[pid] == i else first_entry
        assert zero <= last_stamp <= receipt <= first_entry
        for job in rows:
            assert job["state"] == "completed" and job["attempts"] == job["effects"] == 1
            assert job["entered_us"] <= job["attempt_committed_us"] <= job["effect_committed_us"] <= job["settled_us"]
            post_credit.append(job["settled_us"] - (receipt - last_stamp))

    def backlog(t):
        return dict(awaiting_first=sum(j["attempt_committed_us"] > t for j in jobs),
                    first_done_awaiting_effect=sum(j["attempt_committed_us"] <= t < j["effect_committed_us"] for j in jobs),
                    effect_done_awaiting_settlement=sum(j["effect_committed_us"] <= t < j["settled_us"] for j in jobs),
                    settled=sum(j["settled_us"] <= t for j in jobs))

    other = [j for j in jobs if j["worker_pid"] != last_pid]
    last_terminal = max(settlements)
    return dict(raw_sha256=expected, turns=len(turns), censored_turns=len(turns)-len(final_turn),
                last_receipt_us=last_receipt-zero, last_effect_us=max(effects)-zero, drain_us=last_terminal-zero,
                jobs_at_last_receipt=backlog(last_receipt),
                earlier_owners_with_unsettled_jobs=len({j["worker_pid"] for j in other if j["settled_us"] > last_receipt}),
                last_other_owner_settled_us=max(j["settled_us"] for j in other)-zero,
                max_gain_if_last_owner_finished_at_receipt_us=last_terminal-max(last_receipt, max(j["settled_us"] for j in other)),
                max_gain_if_all_last_stamp_to_receipt_intervals_removed_us=last_terminal-max(post_credit),
                jobs_at_oban_settlement=[backlog(zero+t) for t in deadlines])


def replay(pilot, recovery):
    root = Path(__file__).parent
    history = json.loads((root / "native.json").read_bytes())["blocks"]
    deadlines = [b["oban"]["submit_us"] - b["oban"]["stages_us"]["enqueue"] for b in history.values()]
    arms = {}
    for record in json.loads((root / "confirmed-handler.json").read_bytes())["measured"]:
        group, lane = record["group"], record["lane"]
        raw, = pilot.glob(f"blocks/{group:02d}-*/measured-{lane}/*/raw.tar.gz")
        arms[f"pilot-{group}-{lane}"] = measure(raw, record["raw_sha256"], deadlines)
    for lane, record in json.loads((root / "prepared-grant-frontier.json").read_bytes())["arms"].items():
        raw, = recovery.glob(f"blocks/*/diagnostic-{lane}/*/raw.tar.gz")
        arms[lane] = measure(raw, record["raw_sha256"], deadlines)
    assert all(sum(b.values()) == 64 for r in arms.values() for b in [r["jobs_at_last_receipt"], *r["jobs_at_oban_settlement"]])
    return dict(historical_oban_drain_us=deadlines, arms=arms)


if __name__ == "__main__":
    print(json.dumps(replay(*map(Path, sys.argv[1:])), indent=2))
