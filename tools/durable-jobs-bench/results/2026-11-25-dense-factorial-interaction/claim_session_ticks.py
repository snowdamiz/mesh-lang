"""Identify dedicated claim backends from the statements in the retained server log."""
from collections import Counter
from hashlib import sha256
import io
import json
import tarfile
from argparse import ArgumentParser

from grant_capacity_frontier import EVIDENCE, SEAL
from late_capacity_io import queue_timeout_records


def measure(block, score, lane="measured-B", before_submit=False):
    raw = (block / lane / "measured-mesh/raw.tar.gz").read_bytes()
    assert sha256(raw).hexdigest() == score["raw_sha256"]
    with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
        def read(name):
            return json.load(archive.extractfile("adapter/" + name + ".json"))

        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        cpu = read("pg-cpu-bracket")
        ready = read("canonical-ready")["sessions"]
        roster = read("postgres-tls-sessions")
        records = queue_timeout_records(archive.extractfile("adapter/postgres.log").read().decode().splitlines())
    roles = {s["backend"]: s["role"] for s in ready}
    assert len(roles) == len(ready) == 65 and all(s["tls"] for s in roster + ready)
    assert all(roles[s["backend"]] == s["role"] for s in roster)
    assert Counter(roles.values()) == {"compiled_caller": 32, "parity_subject": 32, "parity_producer": 1}
    first, last = cpu["before"], cpu["after"]
    assert cpu["clock_ticks"] == 100
    submit, commit, settled = score["first_submit_us"], score["first_commit_us"], score["last_settlement_us"]
    assert first["end_us"] <= submit <= commit < settled <= last["begin_us"]
    ticks = Counter()
    by_pid = {}
    lifetime = {}
    for pid, role in roles.items():
        a, b = (frame["rows"][str(pid)] for frame in (first, last))
        assert a[:2] == b[:2] and a[0] == cpu["postmaster"] and b[2] >= a[2]
        lifetime[pid] = a[2]
        by_pid[pid] = b[2] - a[2]
        ticks[role] += by_pid[pid]
    assert sum(ticks.values()) * 10 == score["pg_cpu_ms"]
    claims = Counter()
    for stamp, _, pid in records:
        assert roles[pid] == "parity_subject" and commit <= stamp <= stamp + 999 <= settled
        claims[pid] += 1
    assert len(claims) <= 8
    if before_submit:
        histograms = {}
        for role in sorted(set(roles.values())):
            histograms[role] = dict(sorted(Counter(
                lifetime[pid] for pid in roles if roles[pid] == role
            ).items()))
        return dict(raw_sha256=score["raw_sha256"], clock_ticks_per_second=cpu["clock_ticks"],
                    census_end_before_submit_us=submit - first["end_us"],
                    lifetime_ticks_histograms=histograms,
                    later_identified_claim_sessions={pid: lifetime[pid] for pid in sorted(claims)})
    result = dict(raw_sha256=score["raw_sha256"], early_census=len(roster), ready_census=len(ready),
                  role_cpu_ticks=dict(ticks),
                  identified_claim_backends=len(claims), timeouts=dict(Counter(site for _, site, _ in records)))
    if claims:
        values = [by_pid[pid] for pid in claims]
        outside_us = submit - first["begin_us"] + last["end_us"] - settled
        result.update(
            per_backend={pid: dict(timeouts=n, ticks=by_pid[pid]) for pid, n in claims.items()},
            claim_session_ticks=sum(values),
            other_queue_ticks=ticks["parity_subject"] - sum(values),
            outside_submit_to_settle_us=outside_us,
            claim_cpu_within_submit_to_settle_lower_us=max(
                0, 10000 * sum(max(0, t - 2) for t in values) - 2 * outside_us),
            claim_cpu_within_submit_to_settle_upper_us=10000 * sum(t + 2 for t in values),
        )
    return result


def replay(controls=False, before_submit=False):
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["request_sha256"] == SEAL and release["exit_status"] == 0
    scores = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    lanes = ("B", "A1", "A2", "reference") if controls else ("B",)
    results = {}
    for n in range(1, 6):
        block = EVIDENCE / "blocks" / f"{n:02}-noop"
        reconciled = json.loads((block / "reconciled.json").read_bytes())
        rows = {}
        for lane in lanes:
            arm = "measured-" + lane
            score = scores[str(n)][arm]
            assert score == reconciled[arm]
            rows[lane] = measure(block, score, arm, before_submit)
        results[n] = rows if controls else rows["B"]
    return results


if __name__ == "__main__":
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("--controls", action="store_true", help="Include both OFFs and the reference")
    parser.add_argument("--before-submit", action="store_true", help="Show lifetime CPU ticks at the early census")
    args = parser.parse_args()
    print(json.dumps(replay(args.controls, args.before_submit), indent=2, sort_keys=True))
