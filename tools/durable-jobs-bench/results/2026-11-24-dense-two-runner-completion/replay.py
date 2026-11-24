"""Reconcile the two-runner local rotation and replay native stage boundaries."""
from collections import Counter
import hashlib
import json
from pathlib import Path
from statistics import median
import sys
import tarfile

HERE = Path(__file__).resolve().parent
NATIVE = HERE.parent / "2026-11-23-dense-matched-topology"
sys.path.insert(0, str(NATIVE))
import matrix
from qualification import diagnostic_reconciler, mesh_helpers
from verify import require as check

BROKER = "25f1f9bb0bee23a14e93d0090a01f394f49ccea6515eb791ccd143dfd7942ba8"
RUNNER = "38da7dfc2512f7ab33b9209f2d231c8fb8b1068380cbbf1e46ecf23bac08dd28"
BROKER_MANIFEST = "ac239c8296a50d6439b1c5f86e626a138c70a36b4442a8c27d72fc979564d18a"
RUNNER_MANIFEST = "2e1c6a1b56a2b8a67e0659c7f8960420311a47ea80225affb35bfb8be498d3cb"
ROLES = {"parity_subject": 8, "parity_producer": 1, "compiled_caller": 32}


def stages(ledger):
    jobs = ledger["jobs"]
    origin, = ledger["transactions"]
    return {field: (max(j[field] for j in jobs) - origin["committed_us"]) / 1000
            for field in ("entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")}


def local_arm(data, label):
    def read(name):
        return json.loads(data(name))

    ledger = read("ledger.json")
    row = read("row.json")
    split = read("split-executables.json")
    build = read("build.json")
    check(split["runner"] == RUNNER and split["broker"] == build["files"]["mesh"] == BROKER
          and split["runner_manifest"] == RUNNER_MANIFEST and split["broker_manifest"] == BROKER_MANIFEST,
          "compiled artifact changed")
    check(split["mode"] == ("zero" if label == "zero" else "off") and split["slots_per_runner"] == 32
          and split["scheduler_workers_per_runner"] == 2, "writer or topology mismatch")
    score = diagnostic_reconciler(2)(ledger, "mesh", "noop", dict(version=build["version"],
                                            artifact_sha256=BROKER_MANIFEST))
    jobs = ledger["jobs"]
    check(row["local_smoke"] and row["phase"] == "diagnostic" and row["queue_policy"] == "success_cohort"
          and row["committed_work_records"] == 2048 and row["completed"] == len(jobs) == 1024,
          "wrong comparison or lost work")
    tls = read("postgres-tls-sessions.json")
    check(Counter(r["role"] for r in tls) == ROLES and all(r["tls"] for r in tls)
          and len({r["backend"] for r in tls}) == 41 and score["peak_connections"] == 41,
          "physical TLS connection budget differs")
    reports = [json.loads(data(f"runner-{i}.log").splitlines()[-1]) for i in range(2)]
    counts = Counter(j["worker_pid"] for j in jobs)
    check(set(split["pids"]) == counts.keys() and len(counts) == 2, "runner identity changed")
    for report, pid in zip(reports, split["pids"], strict=True):
        check(report["drained"] and report["last_error"] is None and report["queues"]["default"]["limit"] == 32
              and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == counts[pid]
              and not any(report[key] for key in ("undrained", "stale_acks", "recovered", "pending_claims")),
              "runner failed to drain")
    mesh_helpers()["verify_app"](read, jobs, reports[0]["namespace"])
    check(reports[0]["namespace"] == reports[1]["namespace"], "runner namespace mismatch")
    cohorts = Counter(j["settled_xid"] for j in jobs)
    policy = read("queue-policy.json")
    producer = read("producer-writer.json")
    app_xids = {j[key] for j in jobs for key in ("attempt_xid", "effect_xid")}
    origin, = ledger["transactions"]
    check(origin["xid"] not in cohorts and cohorts.keys().isdisjoint(app_xids) and len(app_xids) == 2048
          and max(cohorts.values()) == policy["largest_terminal_cohort"] <= 8
          and len(cohorts) == policy["terminal_xids"] and producer["bulk"] == 8 and producer["scalar"] == 0,
          "transaction identity or batching mismatch")
    last = stages(ledger)
    return dict(submit_ms=(score["last_settlement_us"] - score["first_submit_us"]) / 1000,
                drain_ms=last["settled_us"], last_entry_ms=last["entered_us"],
                last_effect_ms=last["effect_committed_us"], terminal_xids=len(cohorts),
                runner_cpu_ms_per_job=score["runner_cpu_ms_per_job"], admitted_queries=policy["admitted_queries"],
                pickup_p99_ms=score["commit_to_pickup_p99_ms"])


def local():
    blobs = {}
    with tarfile.open(HERE / "local-evidence.tar.gz", mode="r|gz") as archive:
        for member in archive:
            check(member.isfile() and member.name not in blobs, "unexpected member")
            blobs[member.name] = archive.extractfile(member).read()
    data = blobs.__getitem__
    checks = dict(line.split("  ", 1)[::-1] for line in data("SHA256SUMS").decode().splitlines())
    check(blobs.keys() == set(checks) | {"SHA256SUMS"}, "archive member set changed")
    for path, expected in checks.items():
        check(hashlib.sha256(data(path)).hexdigest() == expected, f"raw digest mismatch for {path}")
    for kind, binary, seal in (("broker", BROKER, BROKER_MANIFEST), ("runner", RUNNER, RUNNER_MANIFEST)):
        saved = data(f"builds/{kind}-artifacts.json")
        check(hashlib.sha256(saved).hexdigest() == seal and json.loads(saved)["files"]["mesh"] == binary,
              f"frozen {kind} binary changed")

    def group_run(folder, label):
        check(json.loads(data(folder + ".status.json"))["exit"] == 0, "unsuccessful arm")
        return local_arm(lambda name: data(folder + "/" + name), label)

    smoke = group_run("smoke-01/01-zero", "zero")
    original = json.loads(data("rotated-01/freeze.json"))
    initial = {}
    for i, order in enumerate(original["orders"][:3], 1):
        for label in order:
            folder = f"rotated-01/{i:02}-{label}"
            if folder == "rotated-01/03-off-b":
                failure = json.loads(data(folder + "/failure.json"))
                check(failure["type"] == "FileExistsError"
                      and "postgres-tls-sessions.json" in failure["message"]
                      and json.loads(data(folder + ".status.json"))["exit"] == 1,
                      "original pre-submit failure differs")
                for index in range(2):
                    check(json.loads(data(f"{folder}/runner-{index}.log").splitlines()[-1])["claimed"] == 0,
                          "failed preflight reached a job")
            else:
                initial[folder] = group_run(folder, label)
    check(original["source_sha256"] == hashlib.sha256(data("rotated-01/initial-run.py")).hexdigest(),
          "original campaign source mismatch")
    freeze = json.loads(data("rotated-02/freeze.json"))
    check(original["decision"] == freeze["decision"] and original["orders"] == freeze["orders"],
          "scoring or order changed")
    check(freeze["source_sha256"] == hashlib.sha256((HERE / "run.py").read_bytes()).hexdigest(),
          "campaign source mismatch")
    groups = []
    for i, order in enumerate(freeze["orders"], 1):
        check(sorted(order) == ["off-a", "off-b", "zero"], "missing duplicates")
        groups.append({label: group_run(f"rotated-02/{i:02}-{label}", label) for label in order})
    check(len({tuple(order) for order in freeze["orders"]}) == len(groups) == 6, "wrong rotations")
    metrics = {}
    for field in smoke:
        wins, margins, deltas, spreads = 0, 0, [], []
        for g in groups:
            a, b, z = [g[label][field] for label in ("off-a", "off-b", "zero")]
            delta = min(a, b) - z
            spread = abs(b - a)
            wins += delta > 0
            margins += delta > spread
            deltas.append(delta)
            spreads.append(spread)
        metrics[field] = dict(medians={mode: median(g[mode][field] for g in groups) for mode in groups[0]},
                              median_saving_vs_faster=median(deltas), median_control_spread=median(spreads),
                              wins_vs_both=wins, wins_beyond_spread=margins)
    def clears(g, key):
        a, b, z = [g[lane][key] for lane in ("off-a","off-b","zero")]
        return min(a, b) - z > abs(a-b)

    qualifying = sum(clears(g, "submit_ms") and clears(g, "drain_ms") and
                     g["zero"]["last_effect_ms"] < min(g[m]["last_effect_ms"] for m in ("off-a","off-b"))
                     for g in groups)
    return dict(smoke=smoke, interrupted_arms=initial, groups=groups, metrics=metrics, qualifying_groups=qualifying,
                advance=qualifying >= 4)


def occupancy(jobs):
    """Occupied job intervals measured from the floored claim stamp to terminal commit."""
    claims = sorted(j["claimed_us"] for j in jobs)
    lo, hi = claims[102], claims[921]
    events = sorted((j[field], delta) for j in jobs
                    for field, delta in (("claimed_us", 1), ("settled_us", -1)))
    previous = events[0][0]
    count = 0
    durations = Counter()
    for stamp, delta in events:
        durations[count] += max(0, min(hi, stamp) - max(lo, previous))
        count += delta
        previous = stamp
    check(count == 0 and sum(durations.values()) == hi - lo, "occupancy window incomplete")
    return dict(mean=sum(n * us for n, us in durations.items()) / (hi - lo),
                percent_at_most_48=100 * sum(us for n, us in durations.items() if n <= 48) / (hi - lo))


def native():
    config, schedule, pins = matrix.inputs("stock")
    summary = matrix.summarize(NATIVE / "evidence/stock", config, schedule, pins)
    check(summary["qualified_rotations"] == 5, "stock rotations changed")
    result = {}
    for block, arms in sorted(summary["blocks"].items()):
        result[block] = {}
        for label in ("mesh-v1-2x32", "oban-8x8", "river-8x8", "apalis-8x8"):
            lane = "measured-" + label
            folder = NATIVE / "evidence/stock/blocks" / block / lane
            raw, = folder.glob("*/raw.tar.gz")
            with tarfile.open(raw) as archive:
                ledger = matrix.run.file_json(archive, "adapter/ledger.json")
            check(hashlib.sha256(raw.read_bytes()).hexdigest() == arms[lane]["raw_sha256"],
                    "native source changed")
            sample = stages(ledger)
            jobs = ledger["jobs"]
            effects = sorted(j["effect_committed_us"] for j in jobs)
            sample["middle_effects_per_s"] = 819_000_000 / (effects[921] - effects[102])
            sample["last_effect_to_last_settlement_ms"] = sample["settled_us"] - sample["effect_committed_us"]
            if label == "mesh-v1-2x32":
                sample["claim_interval_occupancy"] = occupancy(jobs)
            result[block][label] = sample
    return result


if __name__ == "__main__":
    results = {"native": native(), "local": local()}
    print(json.dumps(results, indent=2, sort_keys=True, allow_nan=False))
