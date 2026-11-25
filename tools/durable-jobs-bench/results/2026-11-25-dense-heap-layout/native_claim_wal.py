"""Read native claim and tuple counters from the original PostgreSQL exports."""
from hashlib import sha256
import json
from pathlib import Path
import tarfile

NATIVE = Path(__file__).resolve().parents[1] / "2026-11-24-public-dense-set-claim/evidence/native"


def replay():
    series = json.loads((NATIVE / "final-replay.json").read_bytes())
    assert set(series["groups"]) == {"1", "2"} and not series["failed"]
    rows = []
    for group, (attempt, scores) in series["groups"].items():
        for lane in ("equal-trace", "stock-trace"):
            score = scores["arms"][lane]
            path = NATIVE / "blocks" / attempt / f"measured-{lane}/measured-mesh/raw.tar.gz"
            assert sha256(path.read_bytes()).hexdigest() == score["raw_sha256"]
            with tarfile.open(path) as archive:
                snapshot = json.load(archive.extractfile("adapter/claim-off-clock.json"))
                ledger = json.load(archive.extractfile("adapter/ledger.json"))
            jobs = ledger["jobs"]
            assert len(jobs) == score["completed"] == 1024 and score["failed"] == 0
            assert len({j[key] for j in jobs for key in ("attempt_xid", "effect_xid")}) == 2048
            relation, = (r for r in snapshot["relations"] if r["relname"] == "jobs")
            statement, = (s for s in snapshot["statements"] if s["queryid"] == 6827047345048828466)
            assert not statement["toplevel"] and statement["rows"] == len(jobs)
            rows.append(dict(group=group, lane=lane, inserted=relation["n_tup_ins"],
                             updated=relation["n_tup_upd"], dead_estimate=relation["n_dead_tup"],
                             nested_wal_bytes=int(statement["wal_bytes"]), raw_sha256=score["raw_sha256"]))
    return rows


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
