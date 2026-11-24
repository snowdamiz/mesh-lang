"""Audit the selected public dense receipts without executing a workload."""

from collections import Counter
from hashlib import sha256
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "2026-11-24-trusted-jobs-product-composition"
ARCHIVE = SOURCE / "evidence/native/timing-closure-retry"
INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
ELF = "af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355"
MANIFEST = "979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55"
REVISION = "b3557273d72be3802bf189b284e7caaf5f3caa0f"


def read(path):
    return json.loads(path.read_bytes())


def member(raw, name):
    return json.load(raw.extractfile("adapter/" + name + ".json"))


def digest(path):
    with path.open("rb") as file:
        return sha256(file.read()).hexdigest()


def lifecycle(ledger, writer, policy):
    jobs = ledger["jobs"]
    txn, = ledger["transactions"]
    assert ledger["version"] == REVISION and ledger["artifact_sha256"] == MANIFEST
    assert ledger["input_sha256"] == INPUT and ledger["queue_policy"] == "set_based"
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 1024
    assert all(j["state"] == "completed" and j["attempt"] == j["attempts"] == j["effects"] == 1
               and j["hold_ms"] == 0 and not j["fail_first"]
               and txn["enqueued_ids"][j["job_key"]] == j["id"]
               and txn["committed_us"] - 1000 <= j["claimed_us"] <= j["entered_us"]
               <= j["attempt_committed_us"] < j["effect_committed_us"] <= j["settled_us"] for j in jobs)
    assert len({j[key] for j in jobs for key in ("attempt_xid", "effect_xid", "settled_xid")}
               | {str(int(txn["xid"]) % (2**32))}) == 3073
    assert writer["enabled"] and (writer["bulk"], writer["scalar"]) == (8, 0)
    assert policy["queue_policy"] == "set_based" and policy["terminal_xids"] == 1024
    assert policy["largest_terminal_cohort"] == 1 and policy["completion_cohort_queries"] == 0
    return jobs, txn


def mesh(path, score):
    with tarfile.open(path) as raw:
        ledger, writer, policy = (member(raw, name) for name in ("ledger", "producer-writer", "queue-policy"))
        jobs, txn = lifecycle(ledger, writer, policy)
        trial = member(raw, "raw")
        assert trial["statement_counter_deallocations"] == 0
        assert member(raw, "build")["files"]["mesh"] == ELF
        timings = policy["queue_statements"]
        claim, = [s for s in timings if "FROM mesh_jobs.claim_batch_set_v1(" in s["query"]]
        assert not [s for s in timings if "FROM mesh_jobs.claim" in s["query"] and s != claim]
        assert claim["calls"] == policy["admitted_queries"]
        assert claim["rows"] >= 1024
        assert digest(path) == score["raw_sha256"]
        wake = Counter()
        reports = []
        for i in range(ledger["runners"]):
            with raw.extractfile(f"adapter/runner-{i}.log") as log:
                report = json.loads(log.read().splitlines()[-1])
            assert report["last_error"] is None and report["drained"]
            assert report["claimed"] == report["finished"] and report["pending_claims"] == 0
            assert report["wakeups"] == trial["wakeups"][i]
            reports.append(dict(claimed=report["claimed"], hints=report["batch_contention_hints"]))
            wake.update({k: v for k, v in report["wakeups"].items() if type(v) is int})
        assert sum(r["claimed"] for r in reports) == 1024
        with raw.extractfile("adapter/postgres.log") as log:
            pglog = log.read().decode()
        timeouts = pglog.count("canceling statement due to lock timeout")
        assert timeouts == sum(r["hints"] for r in reports)
        assert pglog.count("line 20 at SQL statement") == timeouts
        assert "duration:" not in pglog
        end = max(j["settled_us"] for j in jobs)
        effects = sorted(j["effect_committed_us"] for j in jobs)
        assert (txn["submitted_us"], txn["committed_us"], end) == (
            score["first_submit_us"], score["first_commit_us"], score["last_settlement_us"])
        return dict(zip(("runners", "slots", "connections", "peak_executing"),
                        (ledger[k] for k in ("runners", "slots", "peak_connections", "peak_executing"))),
                    archive=str(path.relative_to(ARCHIVE)), sha256=score["raw_sha256"],
                    idle_seconds=trial["idle_seconds"], idle_runner_sql_calls=trial["raw_idle_sql_calls"],
                    off_clock_plus_workload_claim_calls=claim["calls"],
                    claim_calls_between_observer_snapshots=trial["raw_claim_calls"],
                    calls_outside_observer_delta=claim["calls"]-trial["raw_claim_calls"],
                    claim_rows=claim["rows"], non_grant_result_rows=claim["rows"]-1024,
                    claim_total_execution_ms=claim["execution_ms"],
                    hints=sum(r["hints"] for r in reports), queue_lock_timeouts=timeouts, wakeups=dict(wake), runners_detail=reports,
                    enqueue_ms=(txn["committed_us"]-txn["submitted_us"])/1000,
                    drain_ms=(end-txn["committed_us"])/1000,
                    claim_front_ms=(min(j["claimed_us"] for j in jobs)-txn["committed_us"])/1000,
                    claim_last_ms=(max(j["claimed_us"] for j in jobs)-txn["committed_us"])/1000,
                    claim_to_entry_p50_ms=score["claim_to_entry_p50_ms"],
                    middle_effect_ms=(effects[921]-effects[102])/1000)


def main():
    selected = {}
    failures = {}
    layouts = {}
    for lane in ("dense-stock", "dense-mixed", "dense-equal"):
        folder = ARCHIVE / lane
        config = read(folder / "controller-evidence/series.json")["config"]
        layouts[lane] = {name: arm["inventory"]["engines"][arm["engine"]]["argv"]
                         for name, arm in config["lanes"].items()}
        groups = {}
        for block in sorted((folder / "blocks").iterdir()):
            group = read(block / "order.json")["group"]
            statuses = {}
            for path in block.glob("*/*/raw.tar.gz"):
                with tarfile.open(path) as raw:
                    statuses[str(path.relative_to(block))] = json.load(raw.extractfile("status.json"))["exit_code"]
            if not (block / "reconciled.json").exists():
                failures[block.name] = statuses
                continue
            assert group not in groups and len(statuses) == 8 and set(statuses.values()) == {0}
            scores = read(block / "reconciled.json")["scores"]
            assert set(scores) == {"product", "river", "oban", "apalis"}
            assert all(s["completed"] == s["ordered_independent_writes"] == 1024 and not s["failed"]
                       for s in scores.values())
            entry = mesh(block / "measured-product/measured-mesh/raw.tar.gz", scores["product"])
            entry["peer_drain_ms"] = {
                k: (s["last_settlement_us"] - s["first_commit_us"]) / 1000
                for k, s in scores.items() if k != "product"}
            entry["free_mesh_enqueue_residual_to_river_ms"] = (
                entry["drain_ms"] - (scores["river"]["last_settlement_us"] -
                                     scores["river"]["first_submit_us"]) / 1000)
            groups[group] = entry
        assert set(groups) == set(range(1, 6))
        selected[lane] = groups
    assert layouts["dense-mixed"]["product"] == layouts["dense-equal"]["product"]
    assert len({item["sha256"] for series in selected.values() for item in series.values()}) == 15
    print(json.dumps(dict(selected=selected, failed_blocks=failures, argv=layouts),
                     indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    main()
