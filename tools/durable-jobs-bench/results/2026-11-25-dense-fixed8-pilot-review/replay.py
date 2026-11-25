"""Replay both clock windows and bracket device activity for the fixed-hold control."""
from bisect import bisect_left, bisect_right
from hashlib import sha256
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "2026-11-25-dense-grouped-refill/evidence/native-pilot"


def disk(sample):
    device, = (row.split()[3:] for row in sample["diskstats"] if row.split()[:2] == ["8", "0"])
    return {name: int(device[pos]) for name, pos in (
        ("writes", 4), ("write_ms", 7), ("busy_ms", 9), ("flushes", 15), ("flush_ms", 16))}


def interval(samples, start, stop):
    stamps = [s["epoch_us"] for s in samples]
    assert all(a < b for a, b in zip(stamps, stamps[1:]))
    left, right = bisect_left(stamps, start), bisect_right(stamps, stop) - 1
    assert 0 < left <= right < len(stamps)-1
    counts = [disk(samples[i]) for i in (left-1, left, right, right+1)]
    bounds = {k: [counts[2][k]-counts[1][k], counts[3][k]-counts[0][k]] for k in counts[0]}
    assert all(0 <= a <= b for a, b in bounds.values())
    return dict(counts=bounds, interior_samples=right-left+1, duration_us=stop-start)


def capture(path, score):
    raw_sha = sha256(path.read_bytes()).hexdigest()
    assert raw_sha == score["raw_sha256"]
    with tarfile.open(path) as t:
        host = json.load(t.extractfile("host.json"))
        assert host["pg_mount"]["maj:min"] == "8:0" and host["affinity"] == [0,1]
        samples = [json.loads(line) for line in t.extractfile("host-samples.jsonl")]
        ledger = json.load(t.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    assert len(jobs) == 1024 and all(j["state"] == "completed" and j["attempts"] == 1 for j in jobs)
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    assert all(j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"] for j in jobs)
    commits = sorted(j["effect_committed_us"] for j in jobs)
    terminal = max(j["settled_us"] for j in jobs)
    assert commits[-1] == score["last_effect_us"] and terminal == score["last_settlement_us"]
    xids = len({j["settled_xid"] for j in jobs})
    assert xids == score["terminal_xids"]
    return dict(raw_sha256=raw_sha, terminal_xids=xids, submit_us=terminal-score["first_submit_us"],
                drain_us=terminal-score["first_commit_us"], last_effect_us=commits[-1]-score["first_commit_us"],
                runner_cpu_ms=round(1024*score["runner_cpu_ms_per_job"],3), claim_calls=score["claim_sql_calls"],
                middle=interval(samples, commits[102], commits[921]),
                drain=interval(samples, score["first_commit_us"], terminal))


def replay():
    release = json.loads((SOURCE / "controller-release.json").read_bytes())
    assert release["groups"] == [1,2] and release["exit_status"] == 0
    s = json.loads((SOURCE / "summary.json").read_bytes())
    assert s["advances"] is False and not s["failures"] and not s["missing"]
    groups = {}
    for group, b in sorted(s["blocks"].items()):
        arms = {}
        for label, score in sorted(b["scores"].items()):
            phase = label.split("-", 1)[0]
            path = SOURCE / "blocks" / b["attempt"] / label / (phase+"-mesh") / "raw.tar.gz"
            arms[label] = capture(path, score)
        assert len(arms) == 10
        fixed = arms["measured-fixed8"]
        a, b_ctrl = (arms[key] for key in ("measured-stock-a", "measured-stock-b"))
        contrast = {}
        for k in ("submit_us", "drain_us", "last_effect_us", "terminal_xids"):
            faster = min(a[k], b_ctrl[k])
            contrast[k] = dict(saving=faster-fixed[k], duplicate_spread=abs(a[k]-b_ctrl[k]))
        groups[group] = dict(arms=arms, fixed_vs_controls=contrast, original_refill_advanced=b["gate"]["advances"])
    return groups


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
