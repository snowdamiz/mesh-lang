"""Price the post-confirmation part of LARGE EXTERNAL grants on the fixed grant schedule."""
from hashlib import sha256
from itertools import groupby
import json
from pathlib import Path
import sys
import tarfile

from factorial_deadlines import checked_json
from replay import replay as baseline


def read_arm(raw, digest, score):
    assert sha256(raw.read_bytes()).hexdigest() == digest
    with tarfile.open(raw) as archive:
        def read(name):
            return json.load(archive.extractfile(f"adapter/{name}.json"))

        ledger = read("ledger")
        _, *identities = read("process-identity")
        policy = read("queue-policy")
        runners = {}
        for index, identity in enumerate(identities):
            ready, report = archive.extractfile(f"adapter/runner-{index}.log").read().splitlines()
            outcome = json.loads(report)
            assert ready == b"READY" and outcome["drained"] and outcome["last_error"] is None
            assert outcome["batch_protocol"] == "locked_prepare_pg17_v1"
            runners[identity["pid"]] = outcome
    txn, = ledger["transactions"]
    zero = txn["committed_us"]
    jobs = ledger["jobs"]
    assert len(runners) == 8 and len(jobs) == len({j["id"] for j in jobs}) == 64
    assert txn["enqueued_ids"] == {j["job_key"]: j["id"] for j in jobs}
    assert all(j["attempts"] == j["effects"] == j["hold_ms"] == 1 and j["state"] == "completed" for j in jobs)
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 192
    ordered = sorted(jobs, key=lambda j: j["claimed_us"])
    turns = [(pid, list(rows)) for pid, rows in groupby(ordered, lambda j: j["worker_pid"])]
    activation, = (s for s in policy["queue_statements"] if "activate_batch_v1" in s["query"])
    assert activation["calls"] == policy["admitted_queries"] == len(turns)
    assert activation["rows"] == 64 + len(turns)
    for (_, left), (_, right) in zip(turns, turns[1:]):
        assert left[-1]["claimed_us"] < right[0]["claimed_us"]
    for pid, runner in runners.items():
        assert runner["claimed"] == sum(len(group) for owner, group in turns if owner == pid)

    # The runner retains only its final positive COMMIT acknowledgement.
    last_turn = {pid: i for i, (pid, _) in enumerate(turns)}
    receipt_to_entry_removed = [[], []]
    before_first_write_removed = [[], []]
    entry_to_first_removed = []
    censored = []
    for i, (pid, group) in enumerate(turns):
        assert 1 <= len(group) <= 8
        earliest = max(j["claimed_us"] for j in group)
        latest = min(j["entered_us"] for j in group)
        assert zero <= earliest <= latest
        if last_turn[pid] == i:
            confirmed = runners[pid]["last_batch_confirmed_us"]
            assert earliest <= confirmed <= latest
            bounds = (confirmed, confirmed)
        else:
            bounds = (earliest, latest)
            censored.append(len(group))
        for j in group:
            entry, first, effect = (j[k] for k in ("entered_us", "attempt_committed_us", "effect_committed_us"))
            assert entry <= first <= effect <= j["settled_us"]
            entry_to_first_removed.append(effect - (first - entry) - zero)
            for end, receipt in enumerate(bounds):
                receipt_to_entry_removed[end].append(effect - (entry - receipt) - zero)
                before_first_write_removed[end].append(effect - (first - receipt) - zero)
    last_receipt = max(r["last_batch_confirmed_us"] for r in runners.values()) - zero
    last_effect = max(j["effect_committed_us"] for j in jobs) - zero
    assert last_receipt == round(score["confirmed_ms"] * 1000)
    assert last_effect == round(score["landmarks"]["effect64_from_commit_ms"] * 1000)
    return dict(raw_sha256=digest, earlier_turn_sizes_with_bounded_receipt=censored,
                last_confirmation_us=last_receipt, last_effect_us=last_effect,
                last_effect_without_entry_to_first_us=max(entry_to_first_removed),
                last_effect_without_receipt_to_entry_us=list(map(max, receipt_to_entry_removed)),
                last_effect_without_receipt_to_first_us=list(map(max, before_first_write_removed)))


def run(pilot):
    score = checked_json(pilot / "scored-before-release.json",
                         "ff2f7f59835cd8767309d2f8c54636647c21e6898675fb84d40f10691be25a13")
    assert score["advanced"] is False and [g["gates"]["passed"] for g in score["groups"]] == [True, False]
    deadlines = [(b["oban"]["submit_us"] - b["oban"]["stages_us"]["enqueue"]) * 5 // 6
                 for b in baseline()["blocks"].values()]
    arms = []
    for group in score["groups"]:
        for lane in ("consumer", "both"):
            raw, = pilot.glob(f"blocks/{group['group']:02d}-*/measured-{lane}/*/raw.tar.gz")
            record = read_arm(raw, group["raw_sha256"]["measured"][lane], group["arms"][lane])
            arms.append(dict(group=group["group"], lane=lane, **record,
                             minimum_confirmation_advance_us=[
                                 max(0, record["last_confirmation_us"] - deadline) for deadline in deadlines]))
    return dict(historical_oban_deadlines_us=deadlines, measured=arms)


if __name__ == "__main__":
    print(json.dumps(run(Path(sys.argv[1])), indent=2))
