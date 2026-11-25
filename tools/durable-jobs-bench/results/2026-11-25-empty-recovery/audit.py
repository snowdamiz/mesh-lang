"""Replay the recovery UPDATE counters from the four measured native trace arms."""
from hashlib import sha256
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "2026-11-24-public-dense-set-claim/evidence/native"
QUERY_ID = -2584306270883932604
FIELDS = ("calls", "rows", "shared_blks_hit", "shared_blks_read", "total_exec_time")


def statement(rows):
    found = [r for r in rows if r["queryid"] == QUERY_ID and not r["toplevel"]]
    if not found:
        return {k: 0 for k in FIELDS}
    row, = found
    assert row["query"].startswith("WITH due AS (")
    assert "UPDATE mesh_jobs.jobs j SET state" in row["query"]
    return row


def audit():
    groups = json.loads((SOURCE / "final-replay.json").read_text())["groups"]
    assert set(groups) == {"1", "2"}
    results = []
    for group, (attempt, scores) in sorted(groups.items()):
        for label, runners in (("equal-trace", 2), ("stock-trace", 8)):
            score = scores["arms"][label]
            archive, = (SOURCE / "blocks" / attempt).glob(f"measured-{label}/*/raw.tar.gz")
            digest = sha256(archive.read_bytes()).hexdigest()
            assert digest == score["raw_sha256"]
            with tarfile.open(archive) as raw:
                def load(name):
                    return json.load(raw.extractfile(f"adapter/{name}.json"))
                ledger = load("ledger")
                frames = load("claim-frames")
                after = load("claim-off-clock")
            assert ledger["runners"] == runners and ledger["slots"] == 64
            assert ledger["phase"] == "measured" and ledger["profile"] == "noop"
            db = ledger["database"]
            assert db["version"] == "17.11" and db["tls"] and db["fsync"] and db["synchronous_commit"]
            jobs = ledger["jobs"]
            assert len(jobs) == len({j["job_key"] for j in jobs}) == 1024
            assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
            assert all(j["state"] == "completed" and j["attempts"] == j["effects"] == 1
                       and j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"] for j in jobs)
            initial, ready, settled = frames
            assert initial["end_us"] < ready["begin_us"]
            assert ready["end_us"] <= score["first_submit_us"] < score["first_commit_us"]
            assert score["last_settlement_us"] == max(j["settled_us"] for j in jobs)
            assert score["last_settlement_us"] <= settled["begin_us"] <= settled["end_us"] <= after["begin_us"]
            assert all(f["calls"]["dealloc"] == 0 for f in frames) and after["dealloc"] == 0
            windows = {}
            for name, left, right in (("idle", initial, ready), ("submit_to_observer", ready, settled),
                                      ("post_observer", settled, after)):
                before, end = (statement(f["statements"]) for f in (left, right))
                delta = {k: end[k] - before[k] for k in FIELDS}
                reference = statement(score["elapsed_sql"][name])
                assert all(abs(delta[k] - reference[k]) < 1e-7 for k in FIELDS)
                assert all(v >= 0 for v in delta.values())
                windows[name] = {**delta, "total_exec_time": round(delta["total_exec_time"], 6)}
            results.append(dict(group=int(group), runners=runners, archive=str(archive.relative_to(SOURCE)),
                                sha256=digest, windows=windows))
    return dict(query_id=QUERY_ID, source=SOURCE.parent.parent.name, arms=results)


if __name__ == "__main__":
    print(json.dumps(audit(), indent=2, sort_keys=True))
