"""Reconcile released grant counts and price wider refills on fixed native timelines."""

from bisect import bisect_right
from collections import Counter, defaultdict
from hashlib import sha256
from itertools import accumulate
import json
from pathlib import Path
import tarfile


HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.parent / "2026-11-25-dense-logical-grant-key-native"
EVIDENCE = ORIGINAL / "evidence/native"
PACKET = "cc5eff4abce062383d0217a836ae36b0fddfba0947573d5874dcc35b2240b18f"
SQL = "d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45"
GRANT = "FROM mesh_jobs.claim_batch_set_v1("
JOBS = 1024
SLOTS = 32
MAX_WIDTH = 16
STAMP_US = 1000


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def statement(rows, fragment):
    match, = (row for row in rows if fragment in row["query"])
    return match


def minimum_calls(stream, release, lead_us, row_edge_us):
    # Preserve grant-time order. Within a tied bucket, moving the earliest
    # release first gives the model the most room at the next grant.
    jobs = sorted((job["claimed_us"], job[release]) for job in stream)
    stamps = [stamp for stamp, _ in jobs]
    turns = Counter(stamps)
    best = {0: 0}  # Number granted through this turn -> fewest calls.
    needed = 0
    for stamp, width in turns.items():
        needed += width
        latest = len(jobs) if lead_us is None else bisect_right(stamps, stamp + lead_us)
        active = list(accumulate(
            (release_us >= stamp + row_edge_us for _, release_us in jobs),
            initial=0,
        ))
        next_best = {}
        for granted, calls in best.items():
            if granted >= needed:
                next_best[granted] = min(next_best.get(granted, len(turns)), calls)
            room = min(MAX_WIDTH, SLOTS - active[granted], latest - granted)
            for end in range(max(needed, granted + 1), granted + room + 1):
                next_best[end] = min(next_best.get(end, len(turns)), calls + 1)
        best = next_best
    return best[len(jobs)]


def capacity_model(streams, release):
    result = {}
    for edge in (0, STAMP_US):
        result[str(edge)] = {
            str(lead) if lead is not None else "unrestricted": sum(
                minimum_calls(stream, release, lead, edge) for stream in streams
            )
            for lead in (0, 1000, 2000, 4000, None)
        }
    return result


def reduce_arm(path, score, diagnostic):
    assert digest(path) == score["raw_sha256"]
    with tarfile.open(path) as archive:
        def read(name):
            return json.load(archive.extractfile(name))

        def adapter(name):
            return read(f"adapter/{name}.json")

        assert ("adapter/grant-frames.json" in archive.getnames()) == diagnostic
        raw, ledger, queue = (adapter(name) for name in ("raw", "ledger", "queue-policy"))
        jobs = ledger["jobs"]
        assert ledger["phase"] == "measured" and (ledger["runners"], ledger["slots"]) == (2, 64)
        assert ledger["queue_policy"] == queue["queue_policy"] == "success_cohort"
        assert raw["idle_seconds"] >= 3
        assert len(jobs) == len({job["id"] for job in jobs}) == JOBS
        assert len(raw["transactions"]) == len(ledger["transactions"]) == 1
        assert all(
            job["attempt"] == job["attempts"] == job["effects"] == 1
            and job["state"] == "completed"
            and job["claimed_us"] % STAMP_US == 0
            and job["claimed_us"] <= job["entered_us"] <= job["attempt_committed_us"]
            < job["effect_committed_us"] <= job["settled_us"]
            for job in jobs
        )
        app_xids = {job[key] for job in jobs for key in ("attempt_xid", "effect_xid")}
        terminal = Counter(job["settled_xid"] for job in jobs)
        assert len(app_xids) == 2 * JOBS and app_xids.isdisjoint(terminal)
        enqueue, = ledger["transactions"]
        assert enqueue["xid"] not in app_xids | terminal.keys()
        assert len(enqueue["enqueued_ids"]) == JOBS
        for n in (0, 1):
            report = json.loads(archive.extractfile(
                f"adapter/runner-{n}.log"
            ).read().splitlines()[-1])
            assert report["drained"] and report["last_error"] is None
            assert report["recovered"] == report["stale_acks"] == report["batch_contention_hints"] == 0
            assert report["finished"] == report["claimed"] == report["outcomes"]["completed"]

        streams = defaultdict(list)
        for job in jobs:
            streams[job["worker_pid"]].append(job)
        assert len(streams) == 2
        buckets = Counter((job["worker_pid"], job["claimed_us"]) for job in jobs)
        widths = Counter(buckets.values())
        assert max(widths) <= MAX_WIDTH
        rows = queue["queue_statements"]
        grant = statement(rows, GRANT)
        assert not any("claim_batch_v1(" in row["query"] or row["query"] == "ROLLBACK" for row in rows)
        transactions = {row["query"]: row["calls"] for row in rows}
        begin, commit = (transactions[command]
                         for command in ("BEGIN ISOLATION LEVEL READ COMMITTED", "COMMIT"))
        assert begin == commit == grant["calls"] == queue["admitted_queries"]
        markers = grant["rows"] - JOBS
        positive = len(buckets)
        # Each bucket needs a positive call and each extra row needs its own
        # marker call. Equality leaves zero calls for split buckets or zero rows.
        assert markers >= 0 and positive + markers == grant["calls"]
        assert statement(rows, "WITH clock AS MATERIALIZED")["calls"] == markers
        scalar = statement(rows, "SELECT coalesce(mesh_jobs.settle_state(")
        grouped = statement(rows, "FROM mesh_jobs.complete_batch_v1(")
        assert scalar["calls"] == scalar["rows"] == terminal_width(terminal, 1)
        assert grouped["rows"] + scalar["rows"] == JOBS
        assert grouped["calls"] == queue["completion_cohort_queries"]
        assert len(terminal) == scalar["calls"] + grouped["calls"] == queue["terminal_xids"]
        measured = raw["raw_claim_calls"]
        result = dict(
            raw_sha256=score["raw_sha256"],
            lifetime_grant_transactions=grant["calls"], workload_grant_transactions=measured,
            outside_workload_grant_transactions=grant["calls"] - measured,
            lifetime_empty_grants=markers, workload_empty_grants=measured - positive,
            workload_positive_grants=positive, width_histogram=dict(widths),
            workload_runner_sql_calls=raw["raw_runner_sql_calls"],
            priming_idle_runner_sql_calls=raw["raw_idle_sql_calls"],
            workload_other_runner_sql_calls=raw["raw_runner_sql_calls"] - 3 * measured - len(terminal),
            terminal_transactions=len(terminal), scalar_terminal_transactions=scalar["calls"],
            grouped_terminal_transactions=grouped["calls"],
            lifetime_grant_statement_elapsed_ms=grant["execution_ms"],
        )
        assert 0 <= result["workload_empty_grants"] <= markers
        if diagnostic:
            frames = adapter("grant-frames")
            counts = [
                statement([row for row in frame["statements"]
                           if row["toplevel"] and row["role"] == "parity_subject"], GRANT)["calls"]
                for frame in [*frames, adapter("grant-closed")]
            ]
            assert counts[-2] == counts[-1] == grant["calls"]
            assert counts[-2] - counts[1] == measured
            return dict(result, frame_grant_calls=counts, sampled_plans=score["plans"])
        assert len(terminal) == score["terminal_xids"]
        return dict(result, submit_us=score["submit_us"], drain_us=score["drain_us"],
                    runner_lifetime_cpu_ms=score["runner_cpu_ms"], pg_bracket_cpu_ms=score["pg_cpu"]["ms"],
                    host_cpu_ms=score["host_cpu_ms"],
                    minimum_calls_at_terminal_record=capacity_model(streams.values(), "settled_us"),
                    minimum_calls_at_second_app_record=capacity_model(streams.values(), "effect_committed_us"))


def terminal_width(cohorts, width):
    return sum(size == width for size in cohorts.values())


def replay():
    assert digest(ORIGINAL / "controller-packet.json") == PACKET
    assert digest(ORIGINAL / "candidate.sql") == SQL
    release = EVIDENCE / "controller/controller-timing-523a54a77bb8.json"
    receipt = json.loads(release.read_text())
    assert receipt["exit_status"] == 0 and receipt["groups"] == [0, 1, 2]
    verified = json.loads((EVIDENCE / "final-replay.json").read_text())
    assert verified["failures"] == verified["additional_complete_attempts"] == {}
    assert verified["next_peer_gate"]["passed"] is False
    result = {}
    for group, block in verified["groups"].items():
        folder, = EVIDENCE.glob(f"blocks/{int(group):02d}-noop.*")
        result[group] = {
            lane: reduce_arm(folder / f"measured-{lane}/measured-mesh/raw.tar.gz", score, group == "0")
            for lane, score in block["measured"].items()
        }
    assert len(result["0"]) == 2 and len(result["1"]) == len(result["2"]) == 3
    comparisons = {}
    for group in ("1", "2"):
        arms = result[group]
        treatment = arms["key"]
        ceiling = max(arms[stock]["runner_lifetime_cpu_ms"] for stock in ("stock-a", "stock-b"))
        excess_cpu = treatment["runner_lifetime_cpu_ms"] - ceiling
        avoided = treatment["workload_positive_grants"] - treatment[
            "minimum_calls_at_terminal_record"
        ]["1000"]["unrestricted"]
        comparisons[group] = dict(excess_runner_cpu_ms=round(excess_cpu, 3),
                                  cpu_ms_needed_per_removed_grant=round(excess_cpu / avoided, 3))
    return dict(packet_sha256=PACKET, logical_sql_sha256=SQL, groups=result,
                released_at=receipt["released_at"], original_peer_gate=verified["next_peer_gate"],
                later_reservation_break_even=comparisons)


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
