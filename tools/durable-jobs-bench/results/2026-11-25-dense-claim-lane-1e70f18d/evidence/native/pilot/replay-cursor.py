"""Reconcile the released warmup cursor without running the timing scorer."""

import hashlib
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
RESULT = HERE.parents[2]
RESULTS = RESULT.parent
SET = "FROM mesh_jobs.claim_batch_set_v1("
CURSOR = "FROM mesh_jobs.claim_batch_v1("
SETTLE = "SELECT coalesce(mesh_jobs.settle_state("
JOBS = 1024


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path):
    return json.loads(path.read_bytes())


def require(condition, message):
    if not condition:
        raise ValueError(message)


def warmup(path, manifest):
    with tarfile.open(path) as raw:
        def read_member(name):
            with raw.extractfile(name) as stream:
                return json.load(stream)

        require(read_member("invocation.json")["phase"] == "warmup"
                and read_member("status.json")["exit_code"] == 0, "unexpected arm invocation")
        require(read_member("adapter/build.json") == manifest, "warmup ELF binding differs")
        ledger = read_member("adapter/ledger.json")
        counter = read_member("adapter/queue-policy.json")
        work = read_member("adapter/raw.json")
        require(work["statement_counter_deallocations"] == 0, "statement counters were evicted")
        statements = counter["queue_statements"]

        def count(query):
            items = [row for row in statements if query(row["query"])]
            return {key: sum(row[key] for row in items) for key in ("calls", "rows")}

        set_count = count(lambda query: SET in query)
        cursor = count(lambda query: CURSOR in query)
        terminal = count(lambda query: query.startswith(SETTLE))
        begins = count(lambda query: query == "BEGIN ISOLATION LEVEL READ COMMITTED")["calls"]
        commits = count(lambda query: query == "COMMIT")["calls"]
        rollbacks = count(lambda query: query.startswith("ROLLBACK"))["calls"]
        empty_probes = count(lambda query: "SELECT greatest(j.scheduled_at_ms" in query)["calls"]
        fallback_probes = count(lambda query: "SELECT (j.scheduled_at_ms -" in query)["calls"]
        reports = [
            json.loads(raw.extractfile(f"adapter/runner-{index}.log").read().splitlines()[-1])
            for index in range(2)
        ]
    require(ledger["phase"] == "warmup" and ledger["queue_policy"] == counter["queue_policy"] == "set_based"
            and set_count["calls"] == counter["admitted_queries"], "claim policy differs")
    require(begins == commits == set_count["calls"] + cursor["calls"] and rollbacks == 0,
            "a checked SELECT lacks a matching commit")
    require(terminal == {"calls": JOBS, "rows": JOBS}
            and counter["terminal_xids"] == JOBS and counter["largest_terminal_cohort"] == 1
            and counter["completion_cohort_queries"] == 0
            and not count(lambda query: "complete_batch_v1(" in query)["calls"], "terminal cardinality differs")
    jobs = ledger["jobs"]
    require(len(jobs) == len({job["id"] for job in jobs}) == JOBS
            and len({job["settled_xid"] for job in jobs}) == JOBS
            and all(job["state"] == "completed" and job["attempt"] == job["attempts"] == job["effects"] == 1
                    for job in jobs), "job or terminal ledger differs")
    require(len({job[key] for job in jobs for key in ("attempt_xid", "effect_xid", "settled_xid")}) == 3 * JOBS,
            "application writes share a transaction")
    require(all(report["state"] == "stopped" and report["drained"] and report["last_error"] is None
                and report["claimed"] == report["finished"] == report["outcomes"]["completed"]
                and not any(report[key] for key in ("recovered", "stale_acks", "pending_claims"))
                for report in reports) and sum(report["claimed"] for report in reports) == JOBS,
            "runner teardown or successful grants differ")

    # In SetBased mode a set reply flags contention only after a rollback.
    # With that route excluded, a consumed hint came from the checked cursor.
    hints = [report["batch_contention_hints"] for report in reports]
    require(cursor["calls"] in (0, 1) and cursor["calls"] == cursor["rows"] == sum(hints)
            and cursor["calls"] == fallback_probes, "cannot classify the cursor reply")
    markers = set_count["rows"] - JOBS
    require(markers == empty_probes + cursor["calls"], "set marker accounting differs")
    productive = set_count["calls"] - markers
    stamp_groups = {(job["worker_pid"], job["claimed_us"]) for job in jobs}
    require(len(stamp_groups) == productive, "productive set call bounds differ")
    return {
        "raw_sha256": digest(path), "set": set_count, "cursor": cursor,
        "begin": begins, "commit": commits, "rollback": rollbacks,
        "terminal": terminal, "terminal_xids": counter["terminal_xids"],
        "next_due_since": empty_probes, "next_due": fallback_probes,
        "contention_hints_by_runner": hints,
        "set_productive_calls": productive, "set_markers": markers,
        "cursor_disposition": "contention_hint" if cursor["calls"] else "no_call",
        "cursor_grants": 0,
    }


def main():
    release = read(HERE / "controller-release.json")
    summary = read(HERE / "summary.json")
    series = read(HERE / "native/series.json")
    setup = read(HERE.parent / "setup/verified.json")
    manifest = setup["jobs_proof"]["manifest"]
    require(release == read(HERE / "controller/controller-timing-68d526da8594.json")
            and release["exit_status"] == 1 and release["attempts"] == [1], "first release differs")
    require(summary["pilot_decision"] == "incomplete" and not summary["advances"]
            and summary["qualified_rotations"] == 0 and summary["missing"] == [1, 2]
            and list(summary["failures"].values()) == ["ValueError: checked set admission or individual settlement changed"],
            "original disposition differs")
    require(series["pins"]["request"] == digest(RESULT / "archive/timing.json")
            and series["pins"]["setup"] == digest(HERE.parent / "setup/raw.tar.gz")
            and series["config"]["product"]["manifest"] == manifest
            and manifest["source_sha256"] == digest(RESULT / "archive/native-stage.tar"), "series binding differs")
    scorer = RESULTS / "2026-11-24-dense-individual-hot-pool/score.py"
    scorer_pin = str(scorer.relative_to(RESULT.parents[3]))
    require(digest(scorer) == series["pins"][scorer_pin]
            and 'and not any(CURSOR in item["query"]' in scorer.read_text(), "zero-cursor gate differs")
    raw_paths = list((HERE / "native/blocks").rglob("raw.tar.gz"))
    require({str(path.relative_to(HERE / "native")): digest(path) for path in raw_paths} == summary["raw"]
            and len(raw_paths) == 4 and all(path.parent.name.startswith("warmup-") for path in raw_paths),
            "first-failure raw inventory differs")
    arms = {}
    for lane, calls in (("checked-a", 0), ("hot", 1)):
        path, = (path for path in raw_paths if path.parent.parent.name == f"warmup-{lane}")
        arms[lane] = warmup(path, manifest)
        require(arms[lane]["cursor"]["calls"] == calls, f"unexpected cursor count: {lane}")
    print(json.dumps({"source": manifest["version"], "elf_sha256": manifest["files"]["mesh"],
                      "exit_status": release["exit_status"], "released_at": release["released_at"],
                      "qualified_rotations": summary["qualified_rotations"], "warmups": arms},
                     indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
