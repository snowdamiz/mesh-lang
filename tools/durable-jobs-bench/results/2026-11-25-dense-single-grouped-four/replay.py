"""Price public single-dispatch grouping from released native archives."""
import hashlib
import io
import json
import subprocess
import tarfile
from collections import Counter, defaultdict
from pathlib import Path

BASE = "tools/durable-jobs-bench/results/"
SCALAR = "2026-11-25-dense-single-dispatch/evidence"
TRANSFER = "2026-11-25-public-dense-3c4032d-transfer/logger-off-recovery/evidence/pilot"
EIGHT = "2026-11-25-public-dense-8x8/path-recovery/evidence/pilot"
SCALAR_REF = "e76083c5c"
TRANSFER_REF = "f0ae21054"
EIGHT_REF = "dad83049a"
PUBLIC_ELF = "4924b5880247059032786f43f490cf99bde8a0a69adc790994ee01a51ab91058"
SCALAR_ELF = "c9f029a001e2f8d058a00c0af8a376bc283304bff7ed9156baf299455c622cb9"
BRIDGE_ELF = "d56d0e6112a94ff601bebf6f777061068ed2fe0f2f2da586963dd6e1326a220a"
D388 = "d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45"
PUBLIC_SOURCE = "3c4032d06b10bc54a47290c673562fb063b47789"
TIMESTAMPS = ("claimed_us", "entered_us", "attempt_committed_us",
              "effect_committed_us", "settled_us")
PHASES = ("claim_to_entry", "entry_to_first", "first_to_second", "second_to_terminal")
ROOT = Path(__file__).resolve().parents[3]


def git_file(ref, path):
    return subprocess.check_output(["git", "show", f"{ref}:{BASE}{path}"], cwd=ROOT)


def release(ref, evidence, status):
    receipt = json.loads(git_file(ref, f"{evidence}/controller-release.json"))
    assert receipt["exit_status"] == status
    return receipt["id"]


def bounded_body_digest():
    schema = subprocess.check_output(
        ["git", "show", f"{PUBLIC_SOURCE}:compiler/mesh-rt/src/jobs/grouped_schema.sql"], cwd=ROOT)
    claim = schema.split(b"\nREVOKE EXECUTE", 1)[0]
    original = claim.replace(b"mesh_jobs.claim_batch_bounded_v1(",
                             b"mesh_jobs.claim_batch_set_v1(", 1)
    assert hashlib.sha256(original).hexdigest() == D388
    return hashlib.sha256(claim.split(b"$$")[1]).hexdigest()


def occupancy(intervals, begin, end):
    events = Counter()
    for left, right in intervals:
        left, right = max(left, begin), min(right, end)
        if left < right:
            events[left] += 1
            events[right] -= 1
    active = peak = area = 0
    previous = begin
    for stamp, change in sorted(events.items()):
        area += active * (stamp - previous)
        active += change
        peak = max(peak, active)
        previous = stamp
    assert active == 0
    return {"peak": peak, "mean": round(area / (end - begin), 3)}


def progress(jobs, begin):
    first, second = [sorted(j[key] for j in jobs) for key in TIMESTAMPS[2:4]]
    left, right = second[102], second[921]
    active = {}
    for label, start, stop in zip(PHASES, TIMESTAMPS, TIMESTAMPS[1:]):
        active[label] = occupancy(((j[start], j[stop]) for j in jobs), left, right)
    active["claim_to_terminal"] = occupancy(
        ((j["claimed_us"], j["settled_us"]) for j in jobs), left, right)
    clocks = {}
    for label, stamps in (("first", first), ("second", second)):
        clocks[label] = {f"rank_{rank}_from_enqueue_us": stamps[rank - 1] - begin
                         for rank in (1, 64, 512, 1024)}
        clocks[label]["middle_820_us"] = stamps[921] - stamps[102]
    return {"writes": clocks, "middle_occupancy": active}


def statement(policy, identifier):
    rows = [row for row in policy["queue_statements"] if identifier in row["query"]]
    return {"calls": sum(row["calls"] for row in rows),
            "rows": sum(row["rows"] for row in rows),
            "execution_ms": round(sum(row["execution_ms"] for row in rows), 3)}


def score(ref, path, expected, elf, runners, bounded_digest):
    data = git_file(ref, path)
    digest = hashlib.sha256(data).hexdigest()
    assert digest == expected["raw_sha256"], path
    with tarfile.open(fileobj=io.BytesIO(data)) as raw:
        assert json.load(raw.extractfile("status.json"))["exit_code"] == 0

        def adapter(name):
            return json.load(raw.extractfile(f"adapter/{name}.json"))

        ledger = adapter("ledger")
        policy = adapter("queue-policy")
        sessions = adapter("postgres-tls-sessions")
        identity = adapter("process-identity")
        if elf == PUBLIC_ELF and expected["terminal_xids"] < 1024:
            assert adapter("build")["version"] == PUBLIC_SOURCE
            bounded = adapter("public-grants")["claim_batch_bounded_v1"]
            assert bounded == {
                "body_sha256": bounded_digest, "worker": True, "producer": False, "app": False,
                "configuration": ["search_path=pg_catalog, pg_temp", "lock_timeout=100ms"],
                "owner": "parity_migrator", "security_definer": False,
            }

    assert len(identity) == runners + 1 and all(
        item["executable_sha256"] == elf for item in identity)
    assert ledger["runners"] == runners and ledger["slots"] == 64
    assert sum(s["role"] == "compiled_caller" and s["tls"] for s in sessions) == 32
    jobs = ledger["jobs"]
    assert len(jobs) == len({j["id"] for j in jobs}) == 1024
    enqueue, = ledger["transactions"]
    submit, begin = enqueue["submitted_us"], enqueue["committed_us"]
    assert int(enqueue["xid"]) > 0 and submit < begin
    end = max(j["settled_us"] for j in jobs)
    assert end - begin == expected["drain_us"] and end - submit == expected["submit_us"]

    members = defaultdict(list)
    app_xids = set()
    for job in jobs:
        assert job["state"] == "completed" and job["attempts"] == job["effects"] == 1
        assert [job[k] for k in TIMESTAMPS] == sorted(job[k] for k in TIMESTAMPS)
        attempt, effect, settled = (int(job[k]) for k in
                                    ("attempt_xid", "effect_xid", "settled_xid"))
        assert int(enqueue["xid"]) < attempt < effect < settled
        app_xids.update((attempt, effect))
        members[settled].append(job)
    assert len(app_xids) == 2048 and not app_xids.intersection(members)
    assert len(members) == policy["terminal_xids"] == expected["terminal_xids"]

    writers = defaultdict(Counter)
    for cohort in members.values():
        assert len(cohort) <= 8
        (pid, _), = {(j["worker_pid"], j["settled_us"]) for j in cohort}
        writers[pid][len(cohort)] += 1
    assert set(writers) == {p["pid"] for p in identity if p["argv"][-1] == "run"}
    widths = Counter(map(len, members.values()))

    scalar = statement(policy, "mesh_jobs.settle_state(")
    grouped = statement(policy, "mesh_jobs.complete_batch_v1(")
    assert scalar["calls"] <= widths[1] and scalar["calls"] == scalar["rows"]
    assert scalar["calls"] + grouped["calls"] == len(members)
    assert scalar["rows"] + grouped["rows"] == 1024
    routes = {route: statement(policy, route) for route in
              ("mesh_jobs.claim_batch_set_v1(", "mesh_jobs.claim_batch_bounded_v1(")}
    if elf == PUBLIC_ELF and grouped["calls"]:
        assert routes["mesh_jobs.claim_batch_bounded_v1("]["calls"] > 0
        assert routes["mesh_jobs.claim_batch_set_v1("]["calls"] == 0

    total = scalar["execution_ms"] + grouped["execution_ms"]
    result = progress(jobs, begin)
    result.update(raw_sha256=digest, elf_sha256=elf, runners=runners,
                  terminal_xids=len(members), terminal_widths=dict(widths),
                  width_by_runner={pid: dict(hist) for pid, hist in writers.items()},
                  terminal_statements={"scalar": scalar, "grouped": grouped},
                  terminal_statement_total_ms=round(total, 3),
                  mean_terminal_statement_ms=round(total / len(members), 6),
                  busiest_writer_statement_floor_in_observed_run_ms=round(total / runners, 3),
                  claim_statements=routes, submit_us=end - submit, drain_us=end - begin,
                  last_write_to_terminal_us=end - max(j["effect_committed_us"] for j in jobs),
                  peak_claim_to_terminal=occupancy(
                      ((j["claimed_us"], j["settled_us"]) for j in jobs), begin, end)["peak"])
    return result


def reduction(single, stocks, public_group):
    price = public_group["mean_terminal_statement_ms"]
    retained = single["terminal_statement_total_ms"]
    targets = {}
    for clock in ("submit_us", "drain_us"):
        fast, slow = sorted(row[clock] for row in stocks)
        floor = max(fast * 0.05, slow - fast)
        targets[clock] = round((single[clock] - fast + floor) / 1000, 3)
    match_drain = (single["drain_us"] - public_group["drain_us"]) / 1000
    return {
        "historical_scalar_stock_gate_improvement_ms": targets,
        "max_mean_ms_per_cohort_at_128_xids_to_clear_scalar_drain_gate_from_sql_alone": round(
            (retained - targets["drain_us"]) / 128, 6),
        "remaining_gap_after_removing_all_scalar_terminal_statement_ms": round(
            match_drain - retained, 3),
        "projected_serial_statement_reclaim_at_public_group_count_ms": round(
            retained - public_group["terminal_statement_total_ms"], 3),
        "projected_serial_statement_reclaim_at_128_xids_and_same_mean_price_ms": round(
            retained - 128 * price, 3),
        "reclaim_needed_to_match_public_grouped_drain_ms": round(match_drain, 3),
        "reclaim_needed_to_match_public_grouped_submit_ms": round(
            (single["submit_us"] - public_group["submit_us"]) / 1000, 3),
    }


def main():
    scalar = json.loads(git_file(SCALAR_REF, f"{SCALAR}/released-replay.json"))
    transfer = json.loads(git_file(TRANSFER_REF, f"{TRANSFER}/replay.json"))
    eight = json.loads(git_file(EIGHT_REF, f"{EIGHT}/replay.json"))
    bounded = bounded_body_digest()
    result = {
        "releases": {
            "scalar": release(SCALAR_REF, f"{SCALAR}/native", 1),
            "transfer": release(TRANSFER_REF, TRANSFER, 0),
            "canonical_eight": release(EIGHT_REF, EIGHT, 0),
        },
        "public_bounded_claim_body_sha256": bounded,
        "blocks": {},
    }
    for number, block in scalar["blocks"].items():
        group_id = f"{int(number):02d}-noop"
        scalar_rows = {}
        for label, expected in block["primary"].items():
            path = f"{SCALAR}/native/blocks/{block['attempt']}/measured-{label}/measured-mesh/raw.tar.gz"
            scalar_rows[label] = score(SCALAR_REF, path, expected, SCALAR_ELF,
                                       1 if label == "single-four" else 2, bounded)
        transfer_rows = {}
        for label, expected in transfer["groups"][number]["measured"].items():
            path = f"{TRANSFER}/blocks/{group_id}/measured-{label}/measured-mesh/raw.tar.gz"
            transfer_rows[label] = score(TRANSFER_REF, path, expected,
                                         BRIDGE_ELF if label == "C" else PUBLIC_ELF, 2, bounded)
        grouped = transfer_rows["B"]
        path = f"{EIGHT}/blocks/{group_id}/measured-B/measured-mesh/raw.tar.gz"
        canonical = score(EIGHT_REF, path, eight["groups"][number]["measured-B"],
                          PUBLIC_ELF, 8, bounded)
        single = scalar_rows["single-four"]
        stocks = [scalar_rows[label] for label in ("stock-a", "stock-b")]
        result["blocks"][number] = {
            "scalar": scalar_rows,
            "transfer": transfer_rows,
            "canonical_eight_grouped": canonical,
            "serial_statement_budget": reduction(single, stocks, grouped),
        }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
