#!/usr/bin/env python3
"""Replay local Linode Jobs evidence after each matrix group (Python 3.11+).

    python3 -B tools/durable-jobs-bench/analyze_stages.py --series SERIES
    python3 -B tools/durable-jobs-bench/analyze_stages.py --series SERIES --publish
    python3 -B tools/durable-jobs-bench/analyze_stages.py --setup

Use the first command independently for the control series. --group and
--profile restrict an incremental view. --publish requires the whole series.
"""

import argparse
from collections import Counter
from contextlib import suppress
from hashlib import sha256
from itertools import combinations
import json
from pathlib import Path
import re
from statistics import median
import sys
import tarfile
from tempfile import NamedTemporaryFile

import run as matrix
from verify import ENGINES, PROFILES, ROOT, SUITE, plan, positive_int, reconcile, require, xid


TARGET = ROOT / "target/linode-stage-analysis"
TIMESTAMP_FIELDS = ("entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")
SEGMENTS = {
    "commit_to_claim": "Producer commit → recorded queue claim",
    "claim_to_entry": "Recorded queue claim → handler entry",
    "entry_to_first": "Handler entry → first application transaction commit",
    "first_to_second": "First → second application transaction commit",
    "second_to_terminal": "Second application commit → durable terminal commit",
}
BOUNDED = ("commit_to_claim", "claim_to_entry", "entry_to_first", "second_to_terminal")
TOTALS = {"entry_to_terminal": SEGMENTS.keys() - {"commit_to_claim", "claim_to_entry"},
          "claim_to_terminal": SEGMENTS.keys() - {"commit_to_claim"}}
WINDOWS = ("commit", "submit", "middle80")
SETUP_NATIVE = ROOT / "target/linode-jobs-native/native-smoke"
SETUP_RECEIPTS = (
    (SETUP_NATIVE / "mesh-short-01/evidence.tar.gz", "./ledger.json", "mesh", "short",
     "6c0b70f715e3bdc1051ae6bfa1d8e53deac2d65758d08ab46b0c981e44fd3d9e"),
    (SETUP_NATIVE / "mesh-large-01/evidence.tar.gz", "./ledger.json", "mesh", "large",
     "01deb2b670e6bb3a4f194e2c4e90c5a4cce4a71ceba43c1e97a6f0eb88e77bfd"),
    (SETUP_NATIVE / "mesh-sparse-01/evidence.tar.gz", "./ledger.json", "mesh", "sparse",
     "df2760073035b1b84fe3f2bae2dac7eece3ac92ed143dd72a839854d4047ba57"),
    (SETUP_NATIVE / "oban-short-02/evidence.tar.gz", "./ledger.json", "oban", "short",
     "5ad06c1385dde0fab6d56cd4f00295d356b7fd3a0df7fe0fe6e53c419b6c8ec5"),
    (SETUP_NATIVE / "oban-sparse-01/evidence.tar.gz", "./ledger.json", "oban", "sparse",
     "35afee1e0b1315ac89cc150529b1bebba8d0668f4537ccb12d897eb190374e64"),
    (SUITE / "results/2026-11-23-river-smoke/native-short-verifier-01.tar.gz", "./ledger.json",
     "go", "short", "4e4da28f47b6c7f26346b4b322b2d24e47b9bf6e4e0901108c3359e277a0aa56"),
    (SUITE / "results/2026-11-23-river-submit/native-large.tar.gz", "./ledger.json",
     "go", "large", "9a02db89f833dca82ad41e5ad4d5c50bd01cb4c8b422a708d6484bc4787fb7a6"),
    (SUITE / "results/2026-11-23-apalis-smoke/sparse-trace-off-v1.tar.gz", "adapter/ledger.json",
     "rust", "sparse", "383edfbe68bc67b7c7f15098a5555f138a84b558735de720675713dfe7694bbd"),
)


def json_bytes(value):
    return (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()


def record_source(sources, path):
    name = path.resolve().relative_to(ROOT).as_posix()
    if name not in sources:
        sources[name] = matrix.digest(path)
    return sources[name]


def provenance(sources):
    for path in (Path(__file__), SUITE / "run.py", SUITE / "verify.py", SUITE / "worker.py",
                 ROOT / "scripts/jobs-parity.py", ROOT / "scripts/jobs-parity-profiles.json"):
        record_source(sources, path)
    return {"source_sha256": sha256(json_bytes(sources)).hexdigest(), "sources": sources}


def transaction_edges(ledger, predecoded):
    engine = ledger["engine"]
    owners, producer_xids = {}, set()
    previous_return = 0
    for tx in ledger["transactions"]:
        request = tx["request_id"]
        submitted, committed = tx["submitted_us"], tx["committed_us"]
        returned = positive_int(tx["acknowledged_us" if engine in ("mesh", "oban") else "returned_us"],
                                f"{request} producer return")
        require(previous_return <= submitted <= committed <= returned,
                f"{request}: invalid serial submit, commit or return timestamp")
        previous_return = returned
        if engine == "go" and not predecoded:
            read, decoded, called = (positive_int(tx[field], f"{request} {field}") for field in
                                     ("read_finished_us", "decoded_us", "enqueue_called_us"))
            require(submitted <= read <= decoded <= called <= committed,
                    f"{request}: invalid River read, decode or enqueue timestamp")
            positive_int(tx["input_bytes"], f"{request} input_bytes")
        identity = xid(tx["xid"], "producer XID") % (2**32)
        require(identity > 2 and identity not in producer_xids, f"{request}: reused or invalid producer XID")
        producer_xids.add(identity)
        owners.update(dict.fromkeys(tx["enqueued_ids"], tx))
    return owners, producer_xids


def stage_metrics(ledger, scores):
    # The caller has already reconciled every frozen ID, argument and work row.
    engine, profile = ledger["engine"], ledger["profile"]
    predecoded = engine == "go" and ledger["artifact_sha256"] == matrix.RIVER_PREDECODED_SHA256
    owners, producer_xids = transaction_edges(ledger, predecoded)
    jobs = ledger["jobs"]
    work_xids = {xid(job[field], field) for job in jobs for field in ("attempt_xid", "effect_xid")}
    require(producer_xids.isdisjoint(work_xids), "producer and application writes reused an XID")
    nonterminal_xids = producer_xids | work_xids
    use_claim = engine != "rust"
    segments = {name: [] for name in list(SEGMENTS)[0 if use_claim else 2:]}
    settlements, batch_sizes, terminals, claims = {}, Counter(), [], []
    for job in jobs:
        key = job["job_key"]
        tx = owners[key]
        has_claim = "claimed_us" in job
        require(has_claim == use_claim, f"{key}: {engine} has unexpected claim coverage")
        entry, first, second, terminal = [job[field] for field in TIMESTAMP_FIELDS]
        bounds = [entry, first, second, terminal]
        if has_claim:
            claimed = job["claimed_us"]
            claims.append(claimed)
            if engine == "mesh":
                require(claimed % 1000 == 0, f"{key}: Mesh claim is not stored in whole milliseconds")
            bounds = [tx["committed_us"], claimed, *bounds]
        for name, left, right in zip(segments, bounds[:-1], bounds[1:], strict=True):
            segments[name].append(right - left)
        terminal_xid = xid(job["settled_xid"], f"{key} terminal XID")
        require(terminal_xid not in nonterminal_xids, f"{key}: reused terminal XID")
        require(settlements.setdefault(terminal_xid, terminal) == terminal,
                f"{key}: one terminal transaction has multiple commit timestamps")
        batch_sizes[terminal_xid] += 1
        terminals.append(terminal)

    n = len(jobs)
    start_commit, start_submit, end = (scores[field] for field in
                                       ("first_commit_us", "first_submit_us", "last_settlement_us"))
    stats = {name: {"mean_ms": sum(values) / (1000 * n), "sum_us": sum(values),
                    "reversed": sum(value < 0 for value in values), "ties": values.count(0)}
             for name, values in segments.items()}
    residence = {name: sum(stats[field]["sum_us"] for field in fields) for name, fields in TOTALS.items()
                 if fields <= stats.keys()}

    ordered = sorted(terminals)
    trim = n // 10
    lower_rank, upper_rank = max(1, trim), n - trim
    lower_time, upper_time = ordered[lower_rank - 1], ordered[upper_rank - 1]
    middle_us = upper_time - lower_time
    middle_intervals = upper_rank - lower_rank
    spans = {"commit": end - start_commit, "submit": end - start_submit, "middle80": middle_us}
    bound = {}
    sparse = None
    if profile == "sparse":
        # Each interval is disjoint by the shared verifier's regulated arrival check.
        ordered_txs = ledger["transactions"]
        settled_by_key = {job["job_key"]: job["settled_us"] for job in jobs}
        idle = sum(right["submitted_us"] - settled_by_key[next(iter(left["enqueued_ids"]))]
                   for left, right in zip(ordered_txs, ordered_txs[1:]))
        sparse = {"settled_to_next_submit_gap_us": idle, "submit_to_terminal_active_us": spans["submit"] - idle}
    elif engine == "mesh" and all(metrics["reversed"] == 0 for metrics in stats.values()):
        # For each job i, terminal'_i >= terminal_i - phase_i when all prior
        # timestamps and all other durations are fixed. Max retains any bottleneck
        # job, even when another job was last in the observed run.
        bound = {name: end - max(ts - duration for ts, duration in zip(terminals, segments[name], strict=True))
                 for name in BOUNDED}

    return {
        "jobs": n, "producer_transactions": len(ledger["transactions"]),
        "claim_jobs": n if use_claim else 0, "predecoded_go": predecoded,
        "first_submit_us": start_submit, "first_commit_us": start_commit, "last_terminal_us": end,
        "first_submit_to_commit_ms": scores["first_submit_to_commit_ms"],
        "first_claim_from_commit_ms": (min(claims) - start_commit) / 1000 if claims else None,
        "first_to_last_claim_ms": (max(claims) - min(claims)) / 1000 if claims else None,
        "first_claim_to_last_terminal_ms": (end - min(claims)) / 1000 if claims else None,
        "window_us": spans,
        "jobs_per_s": {"commit": scores["commit_jobs_per_s"], "submit": scores["submit_jobs_per_s"],
                       "middle80": middle_intervals * 1_000_000 / middle_us if middle_us else None},
        "middle80_ranks": [lower_rank, upper_rank],
        "middle80_boundary_multiplicities": [ordered.count(lower_time), ordered.count(upper_time)],
        "middle80_completion_intervals": middle_intervals,
        "commit_pickup_p99_ms": scores["commit_to_pickup_p99_ms"],
        "submit_pickup_p99_ms": scores["submit_to_pickup_p99_ms"],
        "runner_cpu_ms_per_job": scores["runner_cpu_ms_per_job"],
        "segments": stats, "residence_us": residence,
        "distinct_terminal_times": len(set(terminals)), "terminal_transactions": len(settlements),
        "largest_terminal_batch": max(batch_sizes.values()),
        "sparse": sparse, "fixed_timeline_mesh_bound_saved_us": bound,
    }


def apalis_large_settlement(ledger, scores):
    jobs = sorted(ledger["jobs"], key=lambda job: (job["settled_us"], job["job_key"]))
    last = jobs[-1]
    # The large profile has one producer transaction for the entire batch.
    commit = ledger["transactions"][0]["committed_us"]
    return {
        "last_job": {field: last[field] for field in ("job_key", "fail_first", "attempt", "attempts", "effects")},
        "last_job_from_enqueue_commit_us": {field: last[field] - commit for field in TIMESTAMP_FIELDS},
        "penultimate_settlement_from_enqueue_commit_us": jobs[-2]["settled_us"] - commit,
        "jobs_at_last_settlement": sum(job["settled_us"] == last["settled_us"] for job in jobs),
        "submit_window_host": {field: scores[field] for field in (
            "affinity", "host_samples", "host_sample_span_ms", "cpu_steal_percent", "cpu_iowait_percent",
            "disk_busy_ms", "disk_flushes",
        ) if field in scores},
        "runner_cpu_seconds": ledger["runner_cpu_seconds"],
    }


def setup_report():
    sources, receipts = {}, []
    for path, member, engine, profile, expected in SETUP_RECEIPTS:
        require(record_source(sources, path) == expected, f"{path}: SETUP archive digest differs")
        with tarfile.open(path, "r:gz") as archive:
            ledger = matrix.file_json(archive, member)
        artifact = {field: ledger[field] for field in ("version", "artifact_sha256")}
        scores = reconcile(ledger, engine, profile, artifact)
        stages = stage_metrics(ledger, scores)
        # Exercise the same reader and stage calculations. Emit coverage only.
        receipts.append({"source": str(path.relative_to(ROOT)), "member": member, "engine": engine,
                         "profile": profile, **artifact, "jobs": stages["jobs"],
                         "producer_transactions": stages["producer_transactions"],
                         "claim_jobs": stages["claim_jobs"], "segments_exercised": list(stages["segments"]),
                         "predecoded_go": stages["predecoded_go"]})
    return {"kind": "SETUP only", "receipts": receipts, **provenance(sources)}


def series_directory(name):
    require(isinstance(name, str) and re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9.-]{0,63}", name),
            "expected a matrix series name")
    return matrix.RESULTS / name


def record_block(sources, directory, block):
    record_source(sources, directory / "order.json")
    for phase in block["phases"]:
        for engine in block["order"]:
            arm = directory / f"{phase}-{engine}"
            for filename in ("request.json", "raw.tar.gz"):
                record_source(sources, arm / filename)


def record_authority(sources, authority):
    directory = series_directory(authority["series"])
    record_source(sources, directory / "series.json")
    record_source(sources, directory / "inventory.json")
    for attempt in authority["validated_blocks"].values():
        path = directory / "blocks" / attempt
        record_block(sources, path, matrix.read_json(path / "order.json"))


def validate_series(series, directory, sources):
    kind, config, blocks = series["kind"], series["inventory"], series["plan"]
    require(kind in ("baseline", "repeat", "controls"), "only measured or rotation-control series qualify")
    matrix.check_inventory(config)
    require(matrix.read_json(directory / "inventory.json") == config, "staged inventory differs from the series")
    river = config["engines"]["go"]
    require(river["artifact_sha256"] != matrix.RIVER_PREDECODED_SHA256 and river.get("verified_files"),
            "matched analysis requires the replacement River binary and pinned wrapper files")
    profiles = {block["profile"] for block in blocks}
    require(blocks and blocks == [block for block in plan(kind) if kind != "repeat" or block["profile"] in profiles],
            "series block plan differs from the matrix crossover")

    if kind == "controls":
        return None
    authority = series["controls" if kind == "baseline" else "reference"]
    require(authority, "series is missing its required control or baseline reference")
    name = authority["series"]
    reference_directory = series_directory(name)
    if kind == "baseline":
        checked = matrix.verify_controls(name, config)
    else:
        reference = matrix.read_json(reference_directory / "series.json")
        controls = reference["controls"]
        series_directory(controls["series"])
        checked = matrix.verify_reference(name, config)
        record_authority(sources, controls)
    require(authority == checked, "selected control or baseline reference changed")
    record_authority(sources, checked)
    return tuple(checked["host"])


def read_block(directory, block, scores, config):
    """Check cross-arm identity and order, then read the selected scoring phase."""
    final_phase = block["phases"][-1]
    arms, spans = {}, []
    previous_end = 0
    for phase in block["phases"]:
        for engine in block["order"]:
            path = directory / f"{phase}-{engine}"
            request = matrix.read_json(path / "request.json")
            attempt = request["attempt"]
            require(isinstance(attempt, str) and re.fullmatch(r"[0-9a-f]{32}", attempt),
                    f"{path}: invalid matrix arm ID")
            require((request["phase"], request["engine"], request["profile"]) == (phase, engine, block["profile"]),
                    f"{path}: request is from another phase, engine or profile")
            with tarfile.open(path / "raw.tar.gz", "r:gz") as archive:
                host = matrix.file_json(archive, "host.json")
                require(host["architecture"] == "x86_64" and len(set(host["available_cpus"])) == 4
                        and set(config["cpus"]) <= set(host["available_cpus"])
                        and host["pg_mount"]["fstype"] in {"ext4", "xfs", "btrfs", "zfs"},
                        f"{path}: host is outside matrix native eligibility")
                ledger = matrix.file_json(archive, "adapter/ledger.json")
                adapter_phase = "diagnostic" if phase == "control" else phase
                if engine in ("mesh", "oban"):
                    require(ledger["phase"] == adapter_phase and ledger["local_smoke"] is False,
                            f"{path}: adapter ledger phase differs from the dispatched phase")
                else:
                    runtime = matrix.file_json(archive, "adapter/runtime.json")
                    require(runtime["phase"] == adapter_phase,
                            f"{path}: adapter runtime phase differs from the dispatched phase")
                    if engine == "go":
                        require(runtime["submit_boundary"] == "before_batch_read"
                                and runtime["artifact_sha256"] == ledger["artifact_sha256"],
                                f"{path}: missing replacement River submit boundary witness")
                finished = positive_int(matrix.file_json(archive, "status.json")["finished_us"], "arm finish")
            start = min(tx["submitted_us"] for tx in ledger["transactions"])
            end = max(job["settled_us"] for job in ledger["jobs"])
            require(previous_end <= start < end <= finished, f"{path}: arm windows violate serial matrix order")
            previous_end = finished
            spans.append((start, finished, attempt))
            if phase == final_phase:
                arms[engine] = {"attempt": attempt, **stage_metrics(ledger, scores[engine])}
                if (phase, engine, block["profile"]) == ("measured", "rust", "large"):
                    arms[engine]["settlement_detail"] = apalis_large_settlement(ledger, scores[engine])
    return arms, spans


def distribution(values):
    return {"n": len(values), "median": median(values), "min": min(values), "max": max(values)} if values else None


def direction(left, right):
    return (left > right) - (left < right)


def residence_sums(arm):
    return {**arm["residence_us"], **{phase: stage["sum_us"] for phase, stage in arm["segments"].items()}}


def residence_directions(arm_pairs):
    """Count whether the phase sums and the wall windows rank pairs the same way."""
    changes = {}
    for left, right in arm_pairs:
        spans = {"commit_window": (left["window_us"]["commit"], right["window_us"]["commit"]),
                 "post_claim_drain": (left["first_claim_to_last_terminal_ms"],
                                      right["first_claim_to_last_terminal_ms"])}
        right_sums = residence_sums(right)
        left_sums = residence_sums(left)
        for window, (left_span, right_span) in spans.items():
            if left_span is None or right_span is None:
                continue
            drain_sign = direction(left_span, right_span)
            for field in left_sums.keys() & right_sums.keys():
                sign = drain_sign * direction(left_sums[field], right_sums[field])
                counts = changes.setdefault(window, {}).setdefault(field, dict(opposite=0, same=0, tied=0))
                counts[{-1: "opposite", 0: "tied", 1: "same"}[sign]] += 1
    return changes


def throughput_ratio(mesh, peer, window):
    if mesh["jobs_per_s"][window] is not None and peer["jobs_per_s"][window] is not None:
        # Matched input fixes the job count and middle-window rank count.
        return peer["window_us"][window] / mesh["window_us"][window]
    return None


def summarize_profile(blocks):
    paired, order_effects = {}, {}
    for peer in ENGINES[1:]:
        comparisons = [(block["arms"]["mesh"], block["arms"][peer]) for block in blocks]
        paired[peer] = {
            "windows": {}, "residence_vs_wall_time": residence_directions(comparisons),
        }
        ratios = [{window: throughput_ratio(mesh, other, window) for window in WINDOWS} for mesh, other in comparisons]
        for window in WINDOWS:
            pairs = [pair for pair, ratio in zip(comparisons, ratios, strict=True) if ratio[window] is not None]
            signs = Counter(direction(other["window_us"][window], mesh["window_us"][window])
                            for mesh, other in pairs)
            gaps = [mesh["window_us"][window] - other["window_us"][window] for mesh, other in pairs]
            summary = {"mesh_to_peer": distribution([ratio[window] for ratio in ratios if ratio[window] is not None]),
                       "mesh_wins": signs[1], "peer_wins": signs[-1], "ties": signs[0],
                       "undefined_pairs": len(comparisons) - len(pairs),
                       "mesh_minus_peer_window_ms": distribution([gap / 1000 for gap in gaps])}
            if window != "middle80":
                eligible = [(mesh["fixed_timeline_mesh_bound_saved_us"], gap)
                            for (mesh, _), gap in zip(pairs, gaps, strict=True) if gap > 0]
                summary["fixed_timeline_gap_coverage"] = {
                    phase: {"slower_groups": len(eligible),
                            "with_bound": sum(phase in bound for bound, _ in eligible),
                            "can_cover": sum(bound.get(phase, 0) >= gap for bound, gap in eligible)}
                    for phase in BOUNDED
                }
            paired[peer]["windows"][window] = summary
        order_effects[peer] = [{"group": block["group"], "order": block["order"], "mesh_to_peer": ratio}
                               for block, ratio in zip(blocks, ratios, strict=True)]
    return {
        "paired": paired, "per_order": order_effects,
        "within_engine_residence_vs_wall_time": {
            engine: residence_directions(combinations((block["arms"][engine] for block in blocks), 2))
            for engine in ENGINES
        },
    }


def analyze_series(name, args):
    directory = series_directory(name)
    series = matrix.read_json(directory / "series.json")
    sources = {}
    for filename in ("series.json", "inventory.json"):
        record_source(sources, directory / filename)
    reference_host = validate_series(series, directory, sources)
    blocks = matrix.selected_blocks(series["plan"], args)
    require(blocks, "no blocks selected from the matrix plan")
    selected, issues = matrix.archive_index(directory, blocks, series["inventory"])
    rows, spans, hosts = [], [], set()
    for block in blocks:
        key = matrix.block_prefix(block)
        if key not in selected:
            continue
        attempt, scores = selected[key]
        path = directory / "blocks" / attempt
        record_block(sources, path, block)
        try:
            arms, arm_spans = read_block(path, block, scores, series["inventory"])
        except (ValueError, TypeError, KeyError) as error:
            raise ValueError(f"{path}: {error}") from error
        spans.extend(arm_spans)
        hosts.update((row["host_boot_id"], row["host_kernel"], row["pg_mount"]["maj:min"])
                     for row in scores.values())
        rows.append({"group": block["group"], "profile": block["profile"], "order": block["order"],
                     "block": attempt, "phase": block["phases"][-1], "arms": arms})
    require(len({attempt for _, _, attempt in spans}) == len(spans), "reused arm ID across selected blocks")
    ordered = sorted(spans)
    require(all(left[1] <= right[0] for left, right in zip(ordered, ordered[1:])),
            "selected blocks have overlapping arm windows")
    require(len(hosts) <= 1, "selected blocks span different boots, kernels or disks")
    require(not hosts or reference_host is None or hosts == {reference_host},
            "selected blocks differ in boot, kernel or disk from their reference")
    by_profile = {profile: summarize_profile([row for row in rows if row["profile"] == profile])
                  for profile in PROFILES if any(row["profile"] == profile for row in rows)}
    return {
        "series": name, "kind": series["kind"], "scoring_phase": blocks[0]["phases"][-1],
        "host": next(iter(hosts)) if hosts else None, "inventory": series["inventory"],
        "required_series_blocks": len(series["plan"]), "selected_plan": blocks, "blocks": rows,
        "missing_blocks": [matrix.block_prefix(block) for block in blocks if matrix.block_prefix(block) not in selected],
        "segment_definitions": SEGMENTS, "profiles": by_profile, **provenance(sources),
    }, issues


METHOD = """### Reading the tables

- Full commit and submit rates use the same end, the latest durable terminal
  commit. The starts are the earliest producer commit and earliest submit
  respectively. Matched River arms start submit before reading and decoding
  the batch. The original River receipt began after decoding and is SETUP only.
- Middle 80% uses terminal ranks floor(n/10) and n − floor(n/10), with a
  minimum starting rank of one. It divides the rank difference by the elapsed
  time, e.g. 820 intervals from ranks 102 to 922 for the 1,024-job stock
  comparison. Tied completions stay in the ranked sample. The companion JSON
  records how many jobs share each boundary timestamp. A zero span is
  undefined. The full and middle windows use the same selected arms.
- Pickup is durable enqueue commit or submit to handler entry, with the
  matrix's nearest-rank p99 and zero floor. Phase means use signed differences
  of stored timestamps. Reversals within the matrix tolerance and exact ties
  are counted in the JSON. Mesh claim is stored in floored milliseconds. Per-job
  differences near that resolution cannot distinguish order or real savings.
- The claim stamp records the queue's claim event. It can precede claim
  transaction completion. Apalis retains a last-lock field with different
  semantics and has no verified claim stamp. Its three common application and
  terminal segments and entry-to-terminal residence remain comparable.
- Commit → claim includes backlog residence. The first-claim delay and
  the span of claims distinguish startup from sustained admission. Late
  slot refill can lengthen the claim span even when the claim operation
  itself is fast.
- Summed residence includes concurrently resident jobs and uses job-ms. Both
  the commit window and the post-claim drain use wall-clock ms. The direction
  counts track whether each sum and each window move together, move oppositely,
  or tie. A shared terminal XID records a batch whose jobs can finish together.
- Sparse rates, including middle 80%, include regulated arrivals. The reported
  idle gap sums the previous terminal → next submit interval. Subtracting those
  disjoint gaps from the submit window gives active submit-to-terminal time.
- Arm cells summarize the median across groups. The companion JSON retains
  every group in matrix order, per-order ratios, and the range of each paired
  throughput and duration gap. It also includes direction counts for each
  phase sum and for pairs of groups within each engine. Wins and ties use
  the exact integer windows before rounding for display.
- Elapsed claim → entry can include claim response handling, payload transport
  and worker startup. Entry → first commit can include scheduling and database
  work. First → second includes the requested hold. Second → terminal can
  include reaping, acknowledgment batching and persistence. These are
  candidate areas to investigate. Runner CPU is separately measured for the
  full runner lifecycle and has no per-stage attribution.

### Conditional Mesh bounds

For each listed segment and each non-sparse Mesh arm with monotone recorded
stamps, set that segment to zero for every job. Keep all prior timestamps,
all other per-job durations, and the window start fixed. The earliest possible
end under those assumptions is max(job terminal − job segment). Subtract it
from the recorded latest terminal to obtain the largest possible window
saving under this model. The table shows whether that optimistic bound covers
the observed full-window gap in groups where Mesh was slower. This calculation
allows jobs to overlap and relaxes shared-resource and batching constraints.
For example, removing commit → claim would allow all pending jobs to
start admission at once. Its ceiling gives the outer limit under the model
for that observed wait. A faster claim operation can recover only some of
that wait. Changes that shift other timestamps, including regulated sparse
arrivals, fall outside the model. The hold-bearing segment and producer
interval are reported as direct measurements. Improvement selection belongs
to the matrix operators.
"""


def format_range(stats, digits=3):
    if stats is None:
        return "—"
    return f"{stats['median']:.{digits}f} [{stats['min']:.{digits}f}, {stats['max']:.{digits}f}]"


def apalis_large_markdown(series, blocks):
    arms = [block["arms"]["rust"] for block in blocks]
    slowest = max(blocks, key=lambda block: block["arms"]["rust"]["window_us"]["commit"])
    arm = slowest["arms"]["rust"]
    detail = arm["settlement_detail"]
    job = detail["last_job"]
    stamps = detail["last_job_from_enqueue_commit_us"]
    previous = detail["penultimate_settlement_from_enqueue_commit_us"]
    host = detail["submit_window_host"]
    archive = f"../{series}/blocks/{slowest['block']}/measured-rust/raw.tar.gz"
    lines = ["### Apalis large settlement", "",
             f"Observed rates across {len(arms)} measured Apalis large ledgers.", "",
             "| Window | Min jobs/s | Median jobs/s | Max jobs/s |", "| --- | ---: | ---: | ---: |"]
    for window, label in (("commit", "Enqueue commit to settlement"), ("submit", "Submit to settlement")):
        stats = distribution([entry["jobs_per_s"][window] for entry in arms])
        lines.append(f"| {label} | {stats['min']:.2f} | {stats['median']:.2f} | {stats['max']:.2f} |")
    lines.extend(["", f"In group {slowest['group']:02d}, {arm['jobs'] - detail['jobs_at_last_settlement']} of "
                  f"{arm['jobs']} jobs had settled before the final terminal timestamp. The penultimate job settled "
                  f"at {previous / 1000:,.3f} ms after the enqueue commit.", "",
                  f"The last job to settle, `{job['job_key']}`, entered at {stamps['entered_us'] / 1000:,.3f} ms, "
                  f"committed its first application write at {stamps['attempt_committed_us'] / 1000:,.3f} ms, "
                  f"and committed its second at {stamps['effect_committed_us'] / 1000:,.3f} ms. "
                  f"Its terminal commit landed at {stamps['settled_us'] / 1000:,.3f} ms, "
                  f"a {(stamps['settled_us'] - stamps['effect_committed_us']) / 1000:,.3f} ms settlement interval. "
                  f"All offsets start at the enqueue commit. The job had one expected attempt "
                  f"(`fail_first={str(job['fail_first']).lower()}`, `attempt={job['attempt']}`, "
                  f"`attempts={job['attempts']}`, `effects={job['effects']}`). "
                  f"[Group {slowest['group']:02d} archive, `adapter/ledger.json`]({archive})", "",
                  f"The {host['host_samples']} host samples bracketing the submit-to-terminal window "
                  f"span {host['host_sample_span_ms']:,.3f} ms. The pinned CPUs "
                  f"({', '.join(map(str, host['affinity']))}) recorded {host['cpu_steal_percent']:.3f}% steal and "
                  f"{host['cpu_iowait_percent']:.3f}% I/O wait. The ledger accounts for "
                  f"{detail['runner_cpu_seconds'] * 1000:,.3f} ms of runner CPU across the arm."])
    if "disk_busy_ms" in host and "disk_flushes" in host:
        lines[-1] += (f" The PostgreSQL mount device recorded {host['disk_busy_ms']} ms busy and "
                      f"{host['disk_flushes']} flushes across those samples.")
    lines[-1] += f" [Group {slowest['group']:02d} archive, `host-samples.jsonl`]({archive})"
    return lines + [""]


def report_markdown(document):
    lines = [f"# {document['series']} stage analysis", "",
             f"Scoring phase `{document['scoring_phase']}` · "
             f"{len(document['blocks'])}/{len(document['selected_plan'])} selected blocks · "
             f"{document['required_series_blocks']} planned in the series.",
             "", f"Replay with `{document['replay']}`. To publish a complete series, replay without selectors "
             "and append `--publish`.",
             "", f"[Per-arm evidence and sources](analysis.json) · SHA-256 `{document['source_sha256']}`.", ""]
    if document["missing_blocks"]:
        lines.extend(["Pending blocks " + ", ".join(document["missing_blocks"]) + ".", ""])
    for profile, summary in document["profiles"].items():
        blocks = [row for row in document["blocks"] if row["profile"] == profile]
        lines.extend([f"## {profile}", "",
                      "| Arm metric | Mesh | Oban | Go (River) | Rust (Apalis) |",
                      "| --- | ---: | ---: | ---: | ---: |"])
        cells = [(f"{window} jobs/s", ("jobs_per_s", window), 1) for window in WINDOWS]
        cells.extend((label, (field,), 1) for label, field in (
            ("First submit → first commit (ms)", "first_submit_to_commit_ms"),
            ("First commit → first claim (ms)", "first_claim_from_commit_ms"),
            ("First → last claim (ms)", "first_to_last_claim_ms"),
            ("First claim → last terminal (ms)", "first_claim_to_last_terminal_ms"),
            ("Commit → pickup p99 (ms)", "commit_pickup_p99_ms"),
            ("Submit → pickup p99 (ms)", "submit_pickup_p99_ms"),
            ("Runner CPU per job (ms)", "runner_cpu_ms_per_job"),
            ("Terminal transactions", "terminal_transactions"),
            ("Largest terminal transaction (jobs)", "largest_terminal_batch"),
        ))
        cells.extend((f"{label} mean (ms)", ("segments", name, "mean_ms"), 1)
                     for name, label in SEGMENTS.items())
        cells.extend((f"{scope.replace('_', ' ')} sum (job-ms)", ("residence_us", scope), 1000)
                     for scope in TOTALS)
        if profile == "sparse":
            cells.extend([
                ("Settled → next submit gaps (ms)", ("sparse", "settled_to_next_submit_gap_us"), 1000),
                ("Active submit → terminal time (ms)", ("sparse", "submit_to_terminal_active_us"), 1000),
            ])
        for label, keys, divisor in cells:
            values = []
            for engine in ENGINES:
                data = []
                for block in blocks:
                    value = block["arms"][engine]
                    for key in keys:
                        value = value.get(key, {}) if isinstance(value, dict) else None
                    if isinstance(value, (int, float)):
                        data.append(value / divisor)
                values.append(f"{median(data):.3f}" if len(data) == len(blocks) else "—")
            lines.append("| " + " | ".join([label, *values]) + " |")

        lines.extend(["", "| Peer | Window | Mesh:peer ratio median [min, max] | Mesh wins | Peer wins | Ties | Undefined | Mesh − peer ms median [min, max] |",
                      "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |"])
        for peer, pair in summary["paired"].items():
            for window, stats in pair["windows"].items():
                lines.append(f"| {peer} | {window} | {format_range(stats['mesh_to_peer'], 6)} | "
                             f"{stats['mesh_wins']} | {stats['peer_wins']} | {stats['ties']} | "
                             f"{stats['undefined_pairs']} | {format_range(stats['mesh_minus_peer_window_ms'])} |")
        lines.extend(["", "| Matched residence sum | Wall window | Opposite | Same | Tied |",
                      "| --- | --- | ---: | ---: | ---: |"])
        for peer, pair in summary["paired"].items():
            for window, directions in pair["residence_vs_wall_time"].items():
                for field in TOTALS:
                    counts = directions.get(field)
                    if counts is None:
                        continue
                    lines.append(f"| Mesh vs {peer}, {field.replace('_', ' ')} | {window.replace('_', ' ')} | "
                                 f"{counts['opposite']} | {counts['same']} | {counts['tied']} |")

        if any(block["arms"]["mesh"]["fixed_timeline_mesh_bound_saved_us"] for block in blocks):
            lines.extend(["", "| Mesh segment set to zero | Max saving, median [min, max] ms | Peer | Commit gaps covered / bounded / slower | Submit gaps covered / bounded / slower |",
                          "| --- | ---: | --- | ---: | ---: |"])
            for phase in BOUNDED:
                limits = [block["arms"]["mesh"]["fixed_timeline_mesh_bound_saved_us"] for block in blocks]
                data = [bound[phase] / 1000 for bound in limits if phase in bound]
                for peer, pair in summary["paired"].items():
                    coverage = [pair["windows"][window]["fixed_timeline_gap_coverage"][phase]
                                for window in ("commit", "submit")]
                    counts = ["{can_cover}/{with_bound}/{slower_groups}".format(**counts) for counts in coverage]
                    lines.append(f"| {SEGMENTS[phase]} | {format_range(distribution(data))} | {peer} | "
                                 f"{counts[0]} | {counts[1]} |")
        lines.append("")
        if profile == "large" and document["scoring_phase"] == "measured":
            lines.extend(apalis_large_markdown(document["series"], blocks))
    lines.append(METHOD)
    return "\n".join(lines)


def write_output(path, content):
    if path.exists() and path.read_bytes() == content:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile(dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(content)
            stream.close()
            temporary.replace(path)
        finally:
            with suppress(FileNotFoundError):
                temporary.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--series", help="matrix series name under tools/durable-jobs-bench/results")
    mode.add_argument("--setup", action="store_true", help="verify pinned native SETUP receipts without scoring")
    parser.add_argument("--group", action="append", choices=range(1, 6), type=int)
    parser.add_argument("--profile", action="append", choices=PROFILES)
    parser.add_argument("--publish", action="store_true", help="write the full series to results/SERIES-stage-analysis")
    args = parser.parse_args()

    if args.setup:
        require(not (args.publish or args.group or args.profile), "SETUP is a separate reader check")
        path = TARGET / "setup-reader.json"
        write_output(path, json_bytes(setup_report()))
        print(path.relative_to(ROOT))
        return

    document, issues = analyze_series(args.series, args)
    if issues:
        print(json_bytes({"invalid_or_incomplete_matrix_attempts": issues}).decode(), file=sys.stderr, end="")
    output = TARGET / args.series
    replay = [f"python3 -B {Path(__file__).resolve().relative_to(ROOT)}", f"--series {args.series}"]
    for option in ("group", "profile"):
        replay.extend(f"--{option} {value}" for value in sorted(set(getattr(args, option) or [])))
    if args.group or args.profile:
        output /= "-".join(replay[2:]).replace("--", "").replace(" ", "-")
    if args.publish:
        require(len(document["blocks"]) == document["required_series_blocks"] and not (args.group or args.profile),
                "publishing requires the entire validated series without selectors")
        output = matrix.RESULTS / f"{args.series}-stage-analysis"
    document["replay"] = " ".join(replay)
    write_output(output / "analysis.json", json_bytes(document))
    write_output(output / "report.md", report_markdown(document).encode())
    print(output.relative_to(ROOT))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, TypeError, KeyError, tarfile.TarError) as error:
        sys.exit(str(error))
