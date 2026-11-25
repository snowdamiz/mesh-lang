"""Identify dedicated claim backends from the statements in the retained server log."""
from collections import Counter
from hashlib import sha256
import io
import json
import tarfile

from grant_capacity_frontier import EVIDENCE, SEAL
from late_capacity_io import queue_timeout_records


def measure(block, score):
    raw = (block / "measured-B/measured-mesh/raw.tar.gz").read_bytes()
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
    for pid, role in roles.items():
        a, b = (frame["rows"][str(pid)] for frame in (first, last))
        assert a[:2] == b[:2] and a[0] == cpu["postmaster"] and b[2] >= a[2]
        by_pid[pid] = b[2] - a[2]
        ticks[role] += by_pid[pid]
    assert sum(ticks.values()) * 10 == score["pg_cpu_ms"]
    claims = Counter()
    for stamp, _, pid in records:
        assert roles[pid] == "parity_subject" and commit <= stamp <= stamp + 999 <= settled
        claims[pid] += 1
    assert len(claims) <= 8
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


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["request_sha256"] == SEAL and release["exit_status"] == 0
    scores = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    results = {}
    for n in range(1, 6):
        block = EVIDENCE / "blocks" / f"{n:02}-noop"
        score = scores[str(n)]["measured-B"]
        assert score == json.loads((block / "reconciled.json").read_bytes())["measured-B"]
        results[n] = measure(block, score)
    return results


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
