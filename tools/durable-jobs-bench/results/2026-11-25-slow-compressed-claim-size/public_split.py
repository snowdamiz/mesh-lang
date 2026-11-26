"""Count public LZ4 fallback branches and isolate producer SQL from released split arms."""
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[4]
REVISION = "dccc1ee41c3e61688e30073cd811bb7c5e69fdd1"
PILOT = "tools/durable-jobs-bench/results/2026-11-25-public-slow-lz4-2x32-transfer/evidence/pilot"
RELEASE = "fddf299250fc3038d86c268d9d617e01f77f060ff0092651ee5d7f83e0ad7728"


def blob(path, digest):
    data = subprocess.check_output(["git", "show", f"{REVISION}:{PILOT}/{path}"], cwd=ROOT)
    assert sha256(data).hexdigest() == digest
    return data


def inspect(archive, lane):
    def read(name):
        return json.load(archive.extractfile(f"adapter/{name}.json"))

    transfer = read("lz4-transfer")
    assert transfer["layout"]["compression"] == {"lz4": 512}
    ledger = read("ledger")
    txn, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert txn["enqueued_ids"] == {j["job_key"]: j["id"] for j in jobs}
    assert len(jobs) == 512 and all(j["state"] == "completed" for j in jobs)
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 1536
    enqueue = (txn["committed_us"] - txn["submitted_us"]) / 1000
    bulk, = [s for s in read("slow-transfer")["producer_statements"] if "mesh_jobs.enqueue_batch_v1(" in s["query"]]
    scalar, = [s for s in read("slow-transfer")["producer_statements"] if "mesh_jobs.enqueue(" in s["query"]]
    assert (bulk["calls"], bulk["rows"], scalar["calls"], scalar["rows"]) == (8, 504, 8, 8)
    sql_ms = bulk["execution_ms"] + scalar["execution_ms"]
    result = dict(enqueue_ms=enqueue, writer_execution_ms=round(sql_ms, 3),
                  remaining_enqueue_ms=round(enqueue - sql_ms, 3),
                  drain_ms=(max(j["settled_us"] for j in jobs) - txn["committed_us"]) / 1000)
    if lane == "pub-set":
        assert transfer["route"] == "set_based"
        routes = transfer["routes"]
        set_route, cursor = routes["set_route"], routes["cursor_route"]
        assert routes["begin"]["calls"] == routes["commit"]["calls"] == set_route["calls"] + cursor["calls"]
        assert routes["rollback"]["calls"] == 0
        due = [q for q in read("queue-policy")["queue_statements"] if q["query"].startswith("WITH clock AS MATERIALIZED")]
        cutoff, = [q for q in due if "j.scheduled_at_ms > $4::bigint" in q["query"]]
        uncapped, = [q for q in due if q != cutoff]
        assert cursor["calls"] == uncapped["calls"]
        # The set dispatcher's two disjoint branches exhaust the observed set calls.
        assert cutoff["calls"] + cursor["calls"] == set_route["calls"] == set_route["rows"]
        result.update(fallback_calls=cursor["calls"], empty_with_cutoff=cutoff["calls"],
                      set_exec_ms=round(set_route["execution_ms"], 3),
                      cursor_exec_ms=round(cursor["execution_ms"], 3))
    return result


def replay():
    release = json.loads(blob("controller-release.json", RELEASE))
    assert release["exit_status"] == 0 and len(release["raw_sha256"]) == 16
    result = {}
    for path, digest in sorted(release["raw_sha256"].items()):
        group, label, _, _ = path.split("/")
        phase, lane = label.split("-", 1)
        if not lane.startswith("pub-"):
            continue
        with tarfile.open(fileobj=BytesIO(blob(path, digest))) as archive:
            result[f"{group}/{phase}/{lane}"] = dict(raw_sha256=digest, **inspect(archive, lane))
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
