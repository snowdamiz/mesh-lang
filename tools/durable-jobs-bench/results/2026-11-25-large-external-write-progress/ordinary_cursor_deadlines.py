"""Locate ordinary FILE job progress at each paired Oban and River deadline."""
import hashlib
import json
from pathlib import Path
import tarfile

RESULTS = Path(__file__).resolve().parent.parent
SERIES = {
    "DEFAULT": RESULTS / "2026-11-24-public-large-file-session/resume/evidence/native/timing-adapter-path-retry",
    "EXTERNAL": RESULTS / "2026-11-25-public-large-file-external/grant-parser-retry/evidence/native",
}


def read_ledger(block, lane, clock):
    raw, = block.glob(f"measured-{lane}/*/raw.tar.gz")
    assert hashlib.sha256(raw.read_bytes()).hexdigest() == clock["raw_sha256"]
    with tarfile.open(raw) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    txn, = ledger["transactions"]
    assert txn["committed_us"] == clock["first_commit_us"]
    assert max(j["settled_us"] for j in jobs) == clock["last_settlement_us"]
    assert clock["completed"] == len({j["job_key"] for j in jobs}) == len(jobs) == 64
    assert all(j["attempts"] == j["effects"] == 1 for j in jobs)
    return jobs


def progress(jobs, cutoff):
    """Count mutually exclusive stages. Keep sub-millisecond stamp ambiguity explicit."""
    stages = {}
    for j in jobs:
        stamp, entry = j["claimed_us"], j["entered_us"]
        first, effect, settled = j["attempt_committed_us"], j["effect_committed_us"], j["settled_us"]
        assert stamp % 1000 == 0 and stamp <= entry <= first <= effect <= settled
        if cutoff < stamp:
            stage = "before_stamp"
        elif cutoff < min(entry, stamp + 999):
            stage = "stamp_uncertain"
        elif cutoff < entry:
            stage = "stamped_before_entry"
        elif cutoff < first:
            stage = "entered_before_first_write"
        elif cutoff < effect:
            stage = "first_write_before_effect"
        elif cutoff < settled:
            stage = "effect_before_settlement"
        else:
            stage = "settled"
        stages.setdefault(stage, []).append(j["worker_pid"])
    return {stage: {"jobs": len(pids), "owners": len(set(pids))} for stage, pids in stages.items()}


def replay():
    rows = []
    for policy, series in SERIES.items():
        blocks = sorted(series.glob("blocks/*/reconciled.json"))
        assert len(blocks) == 5
        for group, block in enumerate(blocks, 1):
            assert int(block.parent.name.split("-")[0]) == group
            clocks = json.loads(block.read_bytes())["scores"]
            file = clocks["file"]
            jobs = read_ledger(block.parent, "file", file)
            for peer in ("oban", "river"):
                clock = clocks[peer]
                read_ledger(block.parent, peer, clock)
                submit = clock["last_settlement_us"] - clock["first_submit_us"]
                drain = clock["last_settlement_us"] - clock["first_commit_us"]
                for label, num, den in (("tie", 1, 1), ("1.20x", 5, 6)):
                    limits = {
                        "submit": file["first_submit_us"] + submit * num // den,
                        "drain": file["first_commit_us"] + drain * num // den,
                    }
                    cutoff = min(limits.values())
                    rows.append(dict(policy=policy, group=group, peer=peer, target=label,
                                     file_raw=file["raw_sha256"], peer_raw=clock["raw_sha256"],
                                     limiting_clock=min(limits, key=limits.get),
                                     budget_from_commit_us=cutoff-file["first_commit_us"],
                                     finish_minus_deadline_us=file["last_settlement_us"]-cutoff,
                                     at_deadline=progress(jobs, cutoff),
                                     after_94459us_credit=progress(jobs, cutoff+94459)))
    return rows


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
