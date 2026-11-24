#!/usr/bin/env python3
"""Read the sealed short ledgers and price terminal-only changes. Python 3.10+."""
from collections import Counter, defaultdict
from hashlib import sha256
from itertools import groupby, pairwise
import json
from pathlib import Path
from statistics import median
import sys
import tarfile

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
ROOT = SUITE.parents[1]
sys.path.insert(0, str(SUITE))
from run import sample_window
from verify import reconcile, require, xid

REPAIR = HERE.parent / "2026-11-23-ordinary-completion-writer-native-repair"
COMBINED = HERE.parent / "2026-11-23-ordinary-completion-writer" / "combined"
SERIES = REPAIR / "evidence/native-timing/bulk-v2-completion-writer-exit-r2-identity-retry"
WRITERS = ("writer-off", "writer-zero")
PEERS = ("river", "apalis", "oban")
RANKS = (1, 16, 32, 64, 96, 112, 128)
MILESTONES = {
    "claim": "claimed_us", "entry": "entered_us", "write1": "attempt_committed_us",
    "write2": "effect_committed_us", "settled": "settled_us",
}
NO_TERMINAL_MODELS = ("fixed_effect_schedule", "slot_feedback_replay",
                      "zero_dispatch_slack_cohort_floor", "slot_capacity_floor")
APALIS_SOURCES = ("Cargo.lock", "src/main.rs", "jobs.sql", "writes.sql", "counters.sql", "run.py")
ARCHIVED_MEMBERS = (
    "compiler/mesh-rt/src/actor/scheduler.rs", "compiler/mesh-rt/src/db/pool.rs",
    "compiler/mesh-rt/src/jobs/runner.rs", "compiler/mesh-rt/src/jobs/completion_writer.rs",
    "compiler/mesh-rt/src/jobs/trusted.rs", "compiler/mesh-rt/src/jobs/store.rs",
    "compiler/mesh-rt/src/jobs/schema.sql", "fixture/main.mpl", "fixture/writes.mpl",
)


def ranked(values):
    values = sorted(values)
    return [values[rank - 1] for rank in RANKS]


def distribution(values):
    values = list(values)
    return [min(values), median(values), max(values)]


def archive_json(archive, member):
    with archive.extractfile(member) as stream:
        return json.load(stream)


def stage_progress(commit, milestones):
    # Differences between last crossings are a wall-clock partition. They
    # generally join different jobs, and are not per-job service times.
    entry, first, second, settled = (milestones[stage][-1] for stage in (
        "entry", "write1", "write2", "settled",
    ))
    first_entry = milestones["entry"][0]
    return dict(first_entry=first_entry - commit, entry_spread=entry - first_entry,
                after_last_entry_to_last_write1=first - entry, last_write1_to_last_write2=second - first,
                final_drain=settled - second)


def terminal_models(jobs, slots):
    """Fixed work and claim cohorts. All numbers are absolute microseconds."""
    # Mesh stores SQL statement claim time rounded down to a millisecond.
    # Coalesce identical millisecond markers and allow the latest instant
    # in each bucket, clipped to earliest entry. It is still not commit time.
    cohorts = []
    prior_terminal = []
    previous = jobs[0]["claimed_us"]
    for index, (claim, batch) in enumerate(groupby(jobs, key=lambda row: row["claimed_us"])):
        batch = list(batch)
        if index == 0:
            require(len(batch) == slots, "expected a full first claim marker cohort")
        claim_upper = min(claim + 999, min(row["entered_us"] for row in batch))
        past_required = len(prior_terminal) + len(batch) - slots
        observed_gate = previous
        if past_required > 0:
            observed_gate = max(observed_gate, sorted(prior_terminal)[past_required - 1])
        require(observed_gate <= claim_upper, "recorded cohort exceeds the SQL capacity window")
        claim_model = max(claim, observed_gate)
        cohorts.append((claim_model - observed_gate, batch, claim_model, past_required))
        prior_terminal.extend(row["settled_us"] for row in batch)
        previous = claim_model

    def replay(remove_terminal, keep_slack=True):
        previous = jobs[0]["claimed_us"]
        terminals = []
        for slack, batch, observed_claim, past_required in cohorts:
            gate = previous
            if past_required > 0:
                gate = max(gate, sorted(terminals)[past_required - 1])
            claim = gate + (slack if keep_slack else 0)
            if not remove_terminal:
                require(claim == observed_claim, "slot recurrence does not reproduce baseline")
            require(len(batch) + sum(terminal > claim for terminal in terminals) <= slots,
                    "simulated claim exceeds configured slots")
            shift = claim - observed_claim
            for row in batch:
                entry, write1, write2, terminal = (row[field] + shift for field in (
                    "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us",
                ))
                require(claim <= entry <= write1 < write2 <= terminal,
                        "simulated application commits lost their ordering")
                terminals.append(write2 if remove_terminal else terminal)
            previous = claim
        return max(terminals)

    observed_end = max(row["settled_us"] for row in jobs)
    require(observed_end == replay(False), "terminal feedback baseline differs")
    fixed_effect_end = max(row["effect_committed_us"] for row in jobs)
    fixed_claim_to_write2 = [row["effect_committed_us"] - claim
                             for _, batch, claim, _ in cohorts for row in batch]
    first_claim = cohorts[0][2]
    capacity_floor = first_claim + max(max(fixed_claim_to_write2),
                                      -(-sum(fixed_claim_to_write2) // slots))
    # Mathematical floor allowing any ordering/packing at the same slots,
    # and an earliest replay with fixed cohort order but zero dispatch slack.
    zero_slack_end = replay(True, keep_slack=False)
    feedback_end = replay(True)
    require(capacity_floor <= zero_slack_end <= feedback_end <= fixed_effect_end <= observed_end,
            "terminal counterfactual bounds are inconsistent")
    return dict(fixed_effect_schedule=fixed_effect_end, slot_feedback_replay=feedback_end,
                slot_capacity_floor=capacity_floor, zero_dispatch_slack_cohort_floor=zero_slack_end,
                claim_marker_cohorts=len(cohorts),
                markers_shifted_within_ms=sum(claim != batch[0]["claimed_us"]
                                             for _, batch, claim, _ in cohorts),
                observed_dispatch_slack_us=sum(slack for slack, _, _, _ in cohorts))


def apalis_proof(archive, jobs, tx, terminal_groups):
    build = archive_json(archive, "adapter/build.json")
    for name in APALIS_SOURCES:
        require(sha256((SUITE / "rust" / name).read_bytes()).hexdigest() == build["source_sha256"][name],
                f"Apalis source {name} differs from the captured build")
    raw_jobs = archive_json(archive, "adapter/raw-jobs.json")
    require(raw_jobs == jobs and all(row["native_state"] == "Done" and row["last_result"] == {"Ok": None}
                                     for row in raw_jobs), "Apalis raw terminal SQL differs")
    raw = archive_json(archive, "adapter/raw-writes.json")
    require(len(raw["product"]) == 1 and all(raw["product"][0][field] == tx[field]
                for field in ("xid", "request_id", "committed_us")), "Apalis product SQL differs")
    for label, prefix, rows in (("attempts", "attempt", raw["attempts"]), ("effects", "effect", raw["effects"])):
        by_key = {row["job_key"]: row for row in rows}
        require(len(by_key) == len(rows) == len(jobs), f"incorrect Apalis {label}")
        for job in jobs:
            row = by_key[job["job_key"]]
            require((row["job_id"], row["xid"], row["committed_us"]) ==
                    (job["id"], job[f"{prefix}_xid"], job[f"{prefix}_committed_us"]),
                    f"Apalis {label} row differs from ledger")
    require(archive_json(archive, "adapter/transactions.json") ==
            {"committed": [tx], "rollback": None}, "Apalis enqueue receipts differ")
    counters = archive_json(archive, "adapter/counters.json")
    calls = {key: counters["after"][key] - counters["after_idle"][key]
             for key in ("claims", "application", "runner")}
    require(calls["application"] == 2 * len(jobs), "wrong Apalis application SQL call count")
    grouped = [batch for batch in terminal_groups.values() if len(batch) > 1]
    first_ack = min(row["settled_us"] for row in jobs)
    last_write = max(row["effect_committed_us"] for row in jobs)
    return dict(calls_after_idle=calls,
                entry_before_last_lock=sum(row["entered_us"] < row["last_lock_us"] for row in jobs),
                second_writes_before_first_settlement=sum(row["effect_committed_us"] < first_ack for row in jobs),
                jobs_settled_before_last_write2=sum(row["settled_us"] < last_write for row in jobs),
                grouped_rows=sum(map(len, grouped)),
                distinct_second_commits_in_every_group=all(
                    len({row["effect_xid"] for row in batch}) == len(batch) for batch in grouped),
                grouped_ack_write2_spread_us=distribution(
                    max(row["effect_committed_us"] for row in batch) -
                    min(row["effect_committed_us"] for row in batch) for batch in grouped))


def writer_sql(archive):
    split = archive_json(archive, "adapter/split-executables.json")
    statements = archive_json(archive, "adapter/queue-policy.json")["queue_statements"]
    application = archive_json(archive, "adapter/trusted-application-statements.json")
    prefixes = dict(claim="SELECT status, id, token, context,", scalar="SELECT coalesce(mesh_jobs.settle_state(",
                    grouped="SELECT ordinal::text FROM mesh_jobs.complete_admitted_v3(",
                    attempt_insert="INSERT INTO public.mesh_jobs_parity_attempts",
                    effect_insert="INSERT INTO public.mesh_jobs_parity_effects", checkout_ping="SELECT $1")
    result = {}
    for label, prefix in prefixes.items():
        records = [row for row in (application if label in ("checkout_ping", "attempt_insert", "effect_insert")
                                   else statements) if row["query"].startswith(prefix)]
        require(len(records) == 1, f"missing or ambiguous {label} SQL count")
        result[label] = {field: records[0][field] for field in ("calls", "rows", "execution_ms")}
    require([result[label]["calls"] for label in ("checkout_ping", "attempt_insert", "effect_insert")]
            == [256, 128, 128], "wrong writer application statement counts")
    raw = archive_json(archive, "adapter/raw.json")
    return dict(statements=result, claim_calls_after_idle=raw["raw_claim_calls"],
                warmed_application_sessions=split["warmed_application_sessions"],
                runner_sha256=split["runner_sha256"], mode=split["writer_mode"])


def read_arm(path, digest, lane, sealed, label):
    require(sha256(path.read_bytes()).hexdigest() == digest, f"{path}: sealed digest mismatch")
    with tarfile.open(path, "r:gz") as archive:
        ledger = archive_json(archive, "adapter/ledger.json")
        engine = lane["engine"]
        inventory = lane["inventory"]["engines"][engine]
        identity = {field: inventory[field] for field in ("version", "artifact_sha256")}
        score = reconcile(ledger, engine, "short", identity)
        require(archive_json(archive, "status.json")["exit_code"] == 0, "failed archive")
        host = archive_json(archive, "host.json")
        require(host["affinity"] == [0, 1], "wrong CPU mask")
        with archive.extractfile("host-samples.jsonl") as stream:
            samples = [json.loads(line) for line in stream]
        host_score = sample_window(host, samples, score)
        require(all(sealed[key] == value for key, value in host_score.items()), "host window differs from seal")
        require(all(sealed[key] == score[key] for key in (
            "first_submit_us", "first_commit_us", "last_settlement_us", "completed", "failed",
        )), "ledger scores differ from seal")
        tx, = ledger["transactions"]
        submit, commit = tx["submitted_us"], tx["committed_us"]
        returned = tx.get("returned_us", tx.get("acknowledged_us"))
        jobs = ledger["jobs"]
        producer = xid(tx["xid"], "producer XID") % (2**32)
        work = {xid(row[key], key) for row in jobs for key in ("attempt_xid", "effect_xid")}
        groups = defaultdict(list)
        use_claim = label != "apalis"
        for row in jobs:
            require(("claimed_us" in row) == use_claim, "wrong original claim coverage")
            require(commit <= row["entered_us"] <= row["attempt_committed_us"] <
                    row["effect_committed_us"] <= row["settled_us"], "job timestamps reversed")
            if use_claim:
                require(commit <= row["claimed_us"] <= row["entered_us"], "claim timestamp reversed")
            groups[xid(row["settled_xid"], "terminal XID")].append(row)
        require(submit <= commit <= returned and len(work) == 256 and producer not in work
                and not (groups.keys() & (work | {producer})), "invalid disjoint durable XIDs")
        require(all(len({row["settled_us"] for row in batch}) == 1 for batch in groups.values()),
                "one terminal XID has multiple timestamps")
        fields = {name: field for name, field in MILESTONES.items() if use_claim or name != "claim"}
        milestones = {name: ranked(row[field] for row in jobs) for name, field in fields.items()}
        progress = stage_progress(commit, milestones)
        require(sum(progress.values()) == score["last_settlement_us"] - commit, "stage partition mismatch")
        service_fields = {f"{a}_to_{b}": (left, right)
                          for (a, left), (b, right) in pairwise(fields.items())}
        per_job = {label: distribution(row[right] - row[left] for row in jobs)
                   for label, (left, right) in service_fields.items()}
        last_settled = score["last_settlement_us"]
        final_batch = [row for row in jobs if row["settled_us"] == last_settled]
        result = dict(
            first_submit_us=submit, first_commit_us=commit, submit_to_commit_us=commit - submit,
            commit_to_return_us=returned - commit, commit_to_end_us=sum(progress.values()),
            submit_to_end_us=score["last_settlement_us"] - submit,
            commit_jobs_per_s=score["commit_jobs_per_s"], submit_jobs_per_s=score["submit_jobs_per_s"],
            ranked_from_commit_us={name: [time - commit for time in times] for name, times in milestones.items()},
            progress_us=progress, job_intervals_us=per_job, terminal_xids=len(groups),
            terminal_group_size_histogram=dict(sorted(Counter(map(len, groups.values())).items())),
            peak_connections=score["peak_connections"], ledger_runner_cpu_ms_per_job=score["runner_cpu_ms_per_job"],
            host={key: host_score[key] for key in ("cpu_steal_percent", "cpu_iowait_percent",
                  "host_samples", "host_sample_span_ms", "host_kernel", "host_boot_id", "affinity", "pg_mount")},
            final_settlement_job_count=len(final_batch),
            final_settlement_jobs_write2_to_terminal_us=distribution(
                row["settled_us"] - row["effect_committed_us"] for row in final_batch),
            last_write2_job_settlement_us=max(row["settled_us"] - commit for row in jobs
                                             if row["effect_committed_us"] == milestones["write2"][-1]),
        )
        if label in WRITERS:
            result["sql"] = writer_sql(archive)
            require(result["sql"]["mode"] == label.removeprefix("writer-")
                    and result["sql"]["runner_sha256"] == lane["runner_seal"]["elf"]
                    and result["sql"]["warmed_application_sessions"] == 8, "wrong sealed split writer")
            ordered = sorted(jobs, key=lambda row: row["claimed_us"])
            result["terminal_models_from_commit_us"] = {
                name: value - commit if name in NO_TERMINAL_MODELS else value
                for name, value in terminal_models(ordered, ledger["slots"]).items()
            }
        if label == "apalis":
            result["apalis_sql_proof"] = apalis_proof(archive, jobs, tx, groups)
        if label == "river":
            edges = [tx[key] for key in ("submitted_us", "read_finished_us", "decoded_us",
                                        "enqueue_called_us", "committed_us")]
            require(edges == sorted(edges), "River producer edges reversed")
            result["river_producer_us"] = dict(zip(("read", "decode", "pre_call", "call_to_commit"),
                                                   (b - a for a, b in pairwise(edges)), strict=True))
    return result


def pair(mesh, peer):
    a, b = mesh["progress_us"], peer["progress_us"]
    intervals = {key: a[key] - b[key] for key in a}
    intervals["producer"] = mesh["submit_to_commit_us"] - peer["submit_to_commit_us"]
    commit_gap = mesh["commit_to_end_us"] - peer["commit_to_end_us"]
    submit_gap = mesh["submit_to_end_us"] - peer["submit_to_end_us"]
    require(sum(intervals.values()) == submit_gap and submit_gap - intervals["producer"] == commit_gap,
            "paired decomposition differs from ledger clocks")
    common = mesh["ranked_from_commit_us"].keys() & peer["ranked_from_commit_us"].keys()
    return dict(
        submit_gap_us=submit_gap, commit_gap_us=commit_gap, intervals_us=intervals,
        ranked_gaps_us={key: [left - right for left, right in zip(
            mesh["ranked_from_commit_us"][key], peer["ranked_from_commit_us"][key], strict=True)] for key in common},
        terminal_sensitivity={
            label: dict(commit_gap_us=end - peer["commit_to_end_us"],
                        submit_gap_us=mesh["submit_to_commit_us"] + end - peer["submit_to_end_us"])
            for label, end in mesh["terminal_models_from_commit_us"].items() if label in NO_TERMINAL_MODELS
        },
    )


def main():
    sources = {}

    def read(path):
        data = path.read_bytes()
        sources[path.relative_to(ROOT).as_posix()] = sha256(data).hexdigest()
        return data

    seal = json.loads(read(REPAIR / "evidence/verified-timing.json"))
    series = json.loads(read(SERIES / "series.json"))
    require(not seal["pending"] and [row["group"] for row in seal["matched"]] == list(range(1, 6))
            and len(series["plan"]) == 5, "five released groups required")
    source_info = json.loads(read(COMBINED / "evidence/teardown-source.json"))
    archive_path = COMBINED / "combined-source.tar.gz"
    require(sha256(read(archive_path)).hexdigest() == source_info["combined"]["source_sha256"],
            "combined product source archive differs")
    with tarfile.open(archive_path, "r:gz") as archive:
        for member in ARCHIVED_MEMBERS:
            with archive.extractfile(f"source/{member}") as stream:
                sources[f"{archive_path.relative_to(ROOT)}!source/{member}"] = sha256(stream.read()).hexdigest()
    for path in (
        HERE / "replay.py", SUITE / "verify.py", SUITE / "run.py", SUITE / "native/ledger.py",
        SUITE / "native/arm.py", *(SUITE / "rust" / name for name in APALIS_SOURCES), SUITE / "go/evidence.go",
        SUITE / "go/worker.go", SUITE / "go/db.go", ROOT / "scripts/jobs-parity.py",
        ROOT / "scripts/jobs-parity-profiles.json", COMBINED / "replay/release-exit.patch",
    ):
        read(path)
    groups = []
    lanes = series["config"]["lanes"]
    for block, planned in zip(seal["matched"], series["plan"], strict=True):
        folder = SERIES / "blocks" / block["attempt"]
        plan = json.loads(read(folder / "order.json"))
        receipt = json.loads(read(folder / "reconciled.json"))
        require(plan == planned and plan["group"] == block["group"] and plan["phases"] == ["measured"]
                and plan["profile"] == "short" and set(plan["order"]) == set(lanes) and len(lanes) == 6,
                "wrong block order or scope")
        require(receipt == {key: block[key] for key in receipt}, "block receipt differs")
        arms = {}
        for position, label in enumerate(plan["order"], 1):
            lane = lanes[label]
            key = f"measured-{label}/measured-{lane['engine']}"
            digest = block["raw_sha256"][key]
            archive = folder / key / "raw.tar.gz"
            arm = read_arm(archive, digest, lane, block["scores"][label], label)
            sources[archive.relative_to(ROOT).as_posix()] = digest
            arms[label] = dict(arm, order=position)
        require(all(arms[left]["first_submit_us"] + arms[left]["submit_to_end_us"] <
                    arms[right]["first_submit_us"] for left, right in pairwise(plan["order"])),
                "measured arm windows overlap or do not follow recorded order")
        require(all([arm["host"]["host_boot_id"], arm["host"]["host_kernel"], arm["host"]["pg_mount"]["maj:min"]]
                    == series["config"]["host"] for arm in arms.values()), "Linux host identity changed")
        pairs = {f"{writer} vs {peer}": pair(arms[writer], arms[peer])
                 for writer in WRITERS for peer in PEERS}
        groups.append(dict(group=block["group"], order=plan["order"], arms=arms, pairs=pairs))
    print(json.dumps(dict(ranks=RANKS, distribution_order=["min", "median", "max"],
                          time_unit="microseconds except explicitly named fields",
                          groups=groups, sources_sha256=sources), sort_keys=True, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
