"""Replay sealed public stock arms without starting PostgreSQL or a runner."""
from hashlib import file_digest
import json
from pathlib import Path
import re
from statistics import median
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PRODUCT = HERE.parent / "2026-11-24-trusted-jobs-product-composition"
ARCHIVE = PRODUCT / "evidence/native/timing-closure-retry"
REVISION = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
ELF = "af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355"
MANIFEST = "979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55"
INPUT = "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"
HOST = ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
LANES = ("large", "large-external", "slow", "sparse", "dense-stock", "dense-equal", "dense-mixed")
ENGINES = dict(product="mesh", apalis="rust", oban="oban", river="go")
SLOTS = {"large": (64, 8, 64, 96), "large-external": (64, 8, 64, 96),
         "slow": (512, 1, 64, 68), "sparse": (24, 1, 1, 5),
         "dense-stock": (1024, 8, 64, 96), "dense-equal": (1024, 2, 64, 96),
         "dense-mixed": (1024, 2, 64, 96)}


def read(path):
    return json.loads(path.read_bytes())


def member(raw, name):
    return json.load(raw.extractfile("adapter/" + name + ".json"))


def digest(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def statement(name):
    q = " ".join(name.casefold().split())
    for symbol, label in (("claim_batch_set_v1", "set"), ("claim_batch_v1", "cursor")):
        if "from mesh_jobs." + symbol + "(" in q:
            return label
    if q.startswith("with clock as materialized") and "j.scheduled_at_ms >" in q:
        return "probe_due" if "j.scheduled_at_ms > (select now_ms from clock)" in q else "due_since"
    return {"begin isolation level read committed": "begin", "commit": "commit",
            "rollback": "rollback"}.get(q)


def verify_seal(block):
    names = set()
    for line in (block / "SHA256SUMS").read_text().splitlines():
        sha, name = line.split("  ", 1)
        assert digest(block / name) == sha, (block, name)
        names.add(name)
    return names


def timings(score):
    return dict(submit_ms=(score["last_settlement_us"] - score["first_submit_us"]) / 1000,
                drain_ms=(score["last_settlement_us"] - score["first_commit_us"]) / 1000,
                enqueue_ms=(score["first_commit_us"] - score["first_submit_us"]) / 1000)


def stock_mesh(path, score, lane):
    with tarfile.open(path) as raw:
        jobs, writer, policy, counts, build, storage, db = (
            member(raw, k) for k in ("ledger", "producer-writer", "queue-policy", "raw", "build",
                                     "storage-topology", "postgres-settings"))
        n, runners, slots, cap = SLOTS[lane]
        assert score["affinity"] == [0, 1]
        assert [score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]] == HOST
        assert build["files"]["mesh"] == ELF and build["version"] == jobs["version"] == REVISION
        assert jobs["artifact_sha256"] == MANIFEST and counts["statement_counter_deallocations"] == 0
        assert (len(jobs["jobs"]), jobs["runners"], jobs["slots"], db["max_connections"]) == (n, runners, slots, cap)
        assert db["version"] == "17.11" and all(db[k] for k in ("fsync","full_page_writes","synchronous_commit","tls"))
        external = lane == "large-external"
        assert (storage["storage"], storage["args_storage"], storage["work_mem"]) == (
            ("external-4m", "e", "4MB") if external else ("default", "x", "4MB"))
        transactions = jobs["transactions"]
        assert len(transactions) == (24 if lane == "sparse" else 1)
        txn_by_job = {key: (t, id) for t in transactions for key, id in t["enqueued_ids"].items()}
        assert len(txn_by_job) == n
        first_submit = min(t["submitted_us"] for t in transactions)
        first_commit = min(t["committed_us"] for t in transactions)
        rows = jobs["jobs"]
        assert len({j["job_key"] for j in rows}) == n
        assert all(j["state"] == "completed" and j["attempt"] == j["attempts"] == j["effects"] == 1
                   and not j["fail_first"] and txn_by_job[j["job_key"]][1] == j["id"]
                   and txn_by_job[j["job_key"]][0]["committed_us"] - 1000 <= j["claimed_us"] <= j["entered_us"]
                   <= j["attempt_committed_us"] < j["effect_committed_us"] <= j["settled_us"] for j in rows)
        assert len({str(int(t["xid"]) % 2**32) for t in transactions} |
                   {j[key] for j in rows for key in ("attempt_xid", "effect_xid", "settled_xid")}) == 3*n+len(transactions)
        assert writer["enabled"] and policy["queue_policy"] == jobs["queue_policy"] == "set_based"
        assert (policy["terminal_xids"], policy["largest_terminal_cohort"], policy["completion_cohort_queries"]) == (n, 1, 0)
        assert (first_submit, first_commit, max(j["settled_us"] for j in rows)) == (
            score["first_submit_us"], score["first_commit_us"], score["last_settlement_us"])
        assert score["completed"] == score["ordered_independent_writes"] == n and not score["failed"]
        sql = {}
        for q in policy["queue_statements"]:
            label = statement(q["query"])
            if label:
                assert label not in sql
                sql[label] = {k: q[k] for k in ("calls", "rows", "execution_ms")}
        calls = lambda label: sql.get(label, {}).get("calls", 0)
        result_rows = lambda label: sql.get(label, {}).get("rows", 0)
        assert calls("set") == policy["admitted_queries"]
        logs = raw.extractfile("adapter/postgres.log").read().decode()
        errors = len(re.findall(r"\bERROR:\s", logs))
        lock_timeout = logs.count("canceling statement due to lock timeout")
        set_timeout = len(re.findall(r"PL/pgSQL function mesh_jobs\.claim_batch_set_v1\([^\n]*line 20 at SQL statement", logs))
        reports = [json.loads(raw.extractfile(f"adapter/runner-{index}.log").read().splitlines()[-1])
                   for index in range(runners)]
        assert all(r["last_error"] is None and r["drained"] and r["pending_claims"] == 0 and
                   r["claimed"] == r["finished"] == r["outcomes"]["completed"] for r in reports)
        assert sum(r["claimed"] for r in reports) == n
        assert calls("begin") == calls("set") + calls("cursor") + calls("rollback")
        assert calls("commit") == calls("set") + calls("cursor")
        assert errors == lock_timeout == set_timeout == calls("rollback")
        # Every successful ordinary probe was observed after an acknowledged
        # fallback and every empty-cutoff probe consumes exactly one empty marker.
        assert calls("probe_due") == calls("cursor")
        fallback_rows, empty_rows = calls("probe_due"), calls("due_since")
        set_grants = result_rows("set") - fallback_rows - empty_rows
        assert 0 <= set_grants <= n
        hints = sum(r["batch_contention_hints"] for r in reports)
        if lane.startswith("large"):
            assert jobs["input_sha256"] == INPUT and (writer["bulk"], writer["scalar"]) == (0, n)
            assert set_grants == 0
            assert result_rows("cursor") - n == hints - set_timeout
        else:
            assert not fallback_rows and not result_rows("cursor") and set_grants == n
        return dict(archive=str(path.relative_to(ROOT)), sha256=score["raw_sha256"],
                    storage=storage, sql=sql, set_grant_rows=set_grants, fallback_markers=fallback_rows,
                    empty_markers=empty_rows, set_lock_timeouts=set_timeout, cursor_grant_rows=n-set_grants,
                    cursor_contention_hints=hints-set_timeout, writer={k: writer[k] for k in ("bulk", "scalar")},
                    durable_enqueue_transactions=len(transactions), app_xids=2*n, terminal_xids=n,
                    claims_between_observer_snapshots=counts["raw_claim_calls"], jobs=n,
                    last_grant_ms=(max(j["claimed_us"] for j in rows)-first_commit)/1000,
                    payload_sha256={j["job_key"]: j["payload_sha256"] for j in rows} if lane.startswith("large") else {},
                    **timings(score))


def peer_receipt(path, lane, key, score, payloads):
    result = dict(timings(score), sha256=score["raw_sha256"])
    if lane not in ("large", "large-external"):
        return result
    with tarfile.open(path) as raw:
        ledger = member(raw, "ledger")
        assert ledger["input_sha256"] == INPUT
        assert (ledger["runners"], ledger["slots"]) == (8, 64)
        assert {j["job_key"]: j["payload_sha256"] for j in ledger["jobs"]} == payloads
        if lane == "large":
            assert "adapter/payload-storage.json" not in raw.getnames()
        else:
            proof = member(raw, "payload-storage")
            stored = proof["persisted"]
            assert proof["engine"] == ENGINES[key] and proof["policy"] == "EXTERNAL"
            assert proof["setup"]["rows_before"] == 0 and stored["trigger_absent"]
            assert proof["setup"]["ddl_end_us"] <= score["first_submit_us"]
            assert score["last_settlement_us"] <= proof["verify_start_us"]
            assert stored["metadata"]["attstorage"] == proof["setup"]["after"]["attstorage"] == "e"
            assert {j["job_key"]: j["payload_sha256"] for j in stored["rows"]} == payloads
            assert len(stored["rows"]) == 64 and all(j["compression"] is None and j["payload_bytes"] == 524288
                                                    for j in stored["rows"])
            result["verified_external"] = dict(rows=len(stored["rows"]), column=stored["metadata"]["column"])
    return result


def audit():
    selected, other = {}, {}
    for lane in LANES:
        groups = {}
        for block in sorted((ARCHIVE / lane / "blocks").glob("*")):
            if not (block / "SHA256SUMS").is_file() or not (block / "reconciled.json").is_file():
                other[str(block.relative_to(ARCHIVE))] = {
                    str(p.relative_to(block)): digest(p) for p in sorted(block.glob("*/*/raw.tar.gz"))}
                continue
            names = verify_seal(block)
            group = read(block / "order.json")["group"]
            assert group not in groups
            scores = read(block / "reconciled.json")["scores"]
            assert scores.keys() == ENGINES.keys()
            for key, engine in ENGINES.items():
                p = block / f"measured-{key}/measured-{engine}/raw.tar.gz"
                assert str(p.relative_to(block)) in names
                assert digest(p) == scores[key]["raw_sha256"]
                assert scores[key]["completed"] == scores[key]["ordered_independent_writes"] == SLOTS[lane][0]
                assert not scores[key]["failed"]
                assert scores[key]["affinity"] == [0, 1]
                assert [scores[key]["host_boot_id"], scores[key]["host_kernel"],
                        scores[key]["pg_mount"]["maj:min"]] == HOST
            product = stock_mesh(block / "measured-product/measured-mesh/raw.tar.gz", scores["product"], lane)
            product["peer"] = {
                name: peer_receipt(block / f"measured-{name}/measured-{ENGINES[name]}/raw.tar.gz",
                                   lane, name, s, product["payload_sha256"])
                for name, s in scores.items() if name != "product"
            }
            groups[group] = product
        selected[lane] = groups
    ranges = {}
    for lane in ("large", "large-external"):
        if not selected[lane]:
            continue
        vals = list(selected[lane].values())
        gaps = [r["drain_ms"] - min(s["drain_ms"] for s in r["peer"].values()) for r in vals]
        assert all(v["payload_sha256"] == vals[0]["payload_sha256"] for v in vals)
        ranges[lane] = dict(matched_fastest_peer_drain_gap_ms=gaps, median_drain_gap_ms=median(gaps),
                            stock_drain_range_ms=max(r["drain_ms"] for r in vals)-min(r["drain_ms"] for r in vals),
                            median_gap_floor_ms=max(0, median(gaps)/4))
    return dict(selected=selected, unselected=other, large_baseline=ranges)


if __name__ == "__main__":
    summary = audit()
    print(json.dumps(summary, indent=2, sort_keys=True, allow_nan=False))
    if "--complete" in sys.argv:
        sys.exit(not all(set(groups) == set(range(1,6)) for groups in summary["selected"].values()))
