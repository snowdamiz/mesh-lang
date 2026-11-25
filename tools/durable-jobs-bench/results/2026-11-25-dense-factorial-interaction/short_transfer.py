"""Replay the released short grouped and individual ledgers at the 16-slot cap."""
from bisect import bisect_right
from collections import Counter
import hashlib
import json
from pathlib import Path
import tarfile


HERE = Path(__file__).resolve().parent
SERIES = (HERE.parent / "2026-11-24-trusted-jobs-product-composition"
          / "evidence/native/timing-closure-retry/short-smaller")
HOLD_US = 8_000
WIDTH = 8
SLOTS = 16


def model(jobs, flush_at_pressure):
    # The marker was truncated to milliseconds. Give each original grant its
    # latest possible time, also bounded by that handler's observed entry.
    claims = Counter(min(job["claimed_us"] + 999, job["entered_us"]) for job in jobs)
    confirmed = Counter(job["settled_us"] for job in jobs)
    pending = []
    releases = []
    seen = released = excess_claims = peak_excess = pressure_flushes = 0
    for stamp in sorted(claims.keys() | confirmed.keys()):
        if pending and pending[0] + HOLD_US <= stamp:
            releases.append((pending[0] + HOLD_US, len(pending)))
            released += len(pending)
            pending.clear()
        for _ in range(confirmed[stamp]):
            pending.append(stamp)
            if len(pending) == WIDTH:
                releases.append((stamp, WIDTH))
                released += WIDTH
                pending.clear()
        if claims[stamp]:
            seen += claims[stamp]
            if flush_at_pressure and seen - released > SLOTS and pending:
                releases.append((stamp, len(pending)))
                released += len(pending)
                pressure_flushes += 1
                pending.clear()
            excess = max(0, seen - released - SLOTS)
            excess_claims += min(claims[stamp], excess)
            peak_excess = max(peak_excess, excess)
    if pending:
        releases.append((pending[0] + HOLD_US, len(pending)))
    assert sum(width for _, width in releases) == seen == 128
    return dict(groups=len(releases), pressure_flushes=pressure_flushes,
                new_seats_over_cap_at_original_claim=excess_claims,
                peak_excess_seats=peak_excess,
                last_release_after_original_settlement_us=releases[-1][0] - max(confirmed))


def replay():
    result = {}
    groups = json.loads((SERIES / "summary.json").read_bytes())["groups"]
    assert len(groups) == 5
    for group, scores in enumerate(groups, 1):
        arms = {}
        for lane in ("product-group", "product-bulk"):
            score = scores[lane]
            digest = score["raw_sha256"]
            candidates = SERIES.glob(f"blocks/{group:02}-*/measured-{lane}/*/raw.tar.gz")
            archive, = (f for f in candidates if hashlib.sha256(f.read_bytes()).hexdigest() == digest)
            with tarfile.open(archive) as raw:
                ledger = json.load(raw.extractfile("adapter/ledger.json"))
            jobs = ledger["jobs"]
            assert ledger["slots"] == SLOTS and ledger["runners"] == 1
            assert len(jobs) == score["completed"] == len({j["id"] for j in jobs}) == 128
            assert score["failed"] == 0 and len({j["worker_pid"] for j in jobs}) == 1
            assert all(j["state"] == "completed" and j["attempts"] == j["effects"] == 1
                       and j["claimed_us"] <= j["entered_us"] <= j["attempt_committed_us"]
                       <= j["effect_committed_us"] <= j["settled_us"] for j in jobs)
            assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 256
            claims = sorted(min(j["claimed_us"] + 999, j["entered_us"]) for j in jobs)
            terminal = sorted(j["settled_us"] for j in jobs)
            assert all(n - bisect_right(terminal, stamp) <= SLOTS
                       for n, stamp in enumerate(claims, 1))
            fixed = model(jobs, False)
            pressure = model(jobs, True)
            assert pressure["peak_excess_seats"] == 0
            arms[lane] = dict(raw_sha256=digest, terminal_xids=len({j["settled_xid"] for j in jobs}),
                              fixed=fixed, flush_under_pressure=pressure)
        result[group] = arms
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
