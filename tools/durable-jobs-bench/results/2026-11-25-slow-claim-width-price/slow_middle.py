"""Account for capacity between the 64th and 448th independently committed effects."""
import hashlib
import json
import tarfile

from replay import STOCK


def read_arm(path, digest, engine):
    assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as raw:
        assert json.load(raw.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(raw.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    assert ledger["slots"] == ledger["peak_executing"] == 64 and ledger["runners"] == 1
    assert ledger["engine"] == engine and ledger["database"]["max_connections"] == 68
    assert len(jobs) == len({j["id"] for j in jobs}) == 512
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 1024
    effects = sorted(j["effect_committed_us"] for j in jobs)
    start, stop = effects[63], effects[447]
    assert start < effects[64] and stop < effects[448]
    span = stop - start

    def occupied(begin, end):
        return max(0, min(end, stop) - max(begin, start))

    entry_work = hold = hold_excess = claim_to_entry = effect_to_terminal = 0
    for j in jobs:
        entry, first, effect, terminal = (j[k] for k in (
            "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us"))
        assert tx["committed_us"] <= entry <= first <= effect <= terminal
        assert j["attempts"] == j["effects"] == 1 and j["state"] == "completed"
        assert j["hold_ms"] == 100 and first + 100000 <= effect
        entry_work += occupied(entry, first)
        hold += occupied(first, first + 100000)
        hold_excess += occupied(first + 100000, effect)
        if ledger["engine"] == "mesh":
            assert j["claimed_us"] % 1000 == 0 and j["claimed_us"] <= entry
            claim_to_entry += occupied(j["claimed_us"], entry)
            effect_to_terminal += occupied(effect, terminal)

    capacity = 64 * span
    outside = capacity - entry_work - hold - hold_excess
    assert outside >= 0
    last = max(j["settled_us"] for j in jobs)
    parts = dict(enqueue_us=tx["committed_us"] - tx["submitted_us"],
                 first_64_effects_us=start - tx["committed_us"], middle_384_effects_us=span,
                 last_64_effects_us=effects[-1] - stop, terminal_tail_us=last - effects[-1])
    assert sum(parts.values()) == last - tx["submitted_us"]
    seats = dict(entry_to_first=entry_work, required_hold=hold,
                 above_required_hold=hold_excess, outside_entry_to_effect=outside)
    if ledger["engine"] == "mesh":
        residual = outside - claim_to_entry - effect_to_terminal
        assert residual >= 0
        seats.update(claim_stamp_to_entry=claim_to_entry, effect_to_terminal=effect_to_terminal,
                     outside_claim_stamp_to_terminal=residual)
    return dict(raw_sha256=digest, elapsed=parts, middle_slot_us=seats)


def replay():
    summary = (STOCK / "summary.json").read_bytes()
    assert hashlib.sha256(summary).hexdigest() == "763b9cb39bec01492a9cf6ab6240e5758ef90573345c6571fd5e2fb851d3eaa8"
    groups = []
    for number, group in enumerate(json.loads(summary)["groups"], 1):
        arms = {}
        for lane in ("product", "river"):
            path, = STOCK.glob(f"blocks/{number:02}-*/measured-{lane}/*/raw.tar.gz")
            arms[lane] = read_arm(path, group[lane]["raw_sha256"], {"product": "mesh", "river": "go"}[lane])
        groups.append(arms)
    assert len(groups) == 5
    return groups


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
