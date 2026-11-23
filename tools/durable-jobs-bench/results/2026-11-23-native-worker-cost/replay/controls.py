"""Replay the 40 scored and 24 warmup arms directly from the frozen Git blobs."""

from collections import Counter
import hashlib
import io
import json
import math
from pathlib import Path
from statistics import mean, median
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SUITE = ROOT / "tools/durable-jobs-bench"
SERIES = "tools/durable-jobs-bench/results/2026-11-23-controls"
REF = "a3e8e341ac422e4253d06195560946f0b1da7e1a"
sys.path.insert(0, str(SUITE))
from verify import reconcile, require  # noqa: E402


def blob(path):
    return subprocess.check_output(["git", "show", f"{REF}:{path}"], cwd=ROOT)


def document(archive, name):
    with archive.extractfile(name) as stream:
        return json.load(stream)


def bracket(points, start, end, field, keys, tick_hz):
    """Enclosing and enclosed counter deltas with explicit edge allowances."""
    lo = max(i for i, p in enumerate(points) if p["epoch_us"] <= start)
    hi = next(i for i, p in enumerate(points) if p["epoch_us"] >= end)
    inside_lo = next(i for i, p in enumerate(points) if p["epoch_us"] >= start)
    inside_hi = max(i for i, p in enumerate(points) if p["epoch_us"] <= end)
    require(all(b["monotonic_ns"] > a["monotonic_ns"] and b["epoch_us"] >= a["epoch_us"]
                for a, b in zip(points, points[1:])), "nonmonotonic host samples")

    def delta(i, j):
        return [sum(points[j][field][key][n] - points[i][field][key][n] for key in keys)
                for n in range(8)]

    # Exclude idle, iowait and steal. Count IRQ and softirq as busy. Guest
    # fields are already included in user/nice and should not be added.
    def busy(values):
        return sum(values[i] for i in (0, 1, 2, 5, 6)) * 1000 / tick_hz

    outer = delta(lo, hi)
    inner = delta(inside_lo, inside_hi) if inside_lo < inside_hi else [0] * 8
    require(min(outer) >= 0 and min(inner) >= 0 and sum(outer) > 0, "invalid CPU delta")
    edges = [(start - points[lo]["epoch_us"]) / 1000, (points[hi]["epoch_us"] - end) / 1000]
    # This is an engineering allowance for procstat tick granularity and the
    # sampler's unbracketed /proc/stat read, not an exact scheduler CPU bound.
    allowance = 2 * len(keys) * 1000 / tick_hz
    lower = max(0, busy(inner) - allowance, busy(outer) - len(keys) * sum(edges) - allowance)
    upper = min(len(keys) * (end - start) / 1000, busy(outer) + allowance)
    require(lower <= upper, "host tick allowance does not reconcile")
    return {
        "outer_ticks": outer, "inner_ticks": inner, "tick_hz": tick_hz,
        "outer_busy_cpu_ms": busy(outer), "inner_busy_cpu_ms": busy(inner),
        "busy_cpu_ms_estimated_bounds": [lower, upper],
        "iowait_pct_outer": 100 * outer[4] / sum(outer),
        "steal_pct_outer": 100 * outer[7] / sum(outer),
        "edge_ms_before_after": edges,
        "max_host_sample_gap_ms": max((b["monotonic_ns"] - a["monotonic_ns"]) / 1e6
                                      for a, b in zip(points[lo:hi], points[lo + 1:hi + 1])),
        "indexes": [lo, hi],
    }


def disk(points, indexes, mount):
    device = mount["maj:min"].split(":")
    rows = [next(line.split() for line in points[index]["diskstats"] if line.split()[:2] == device)
            for index in indexes]
    return {
        name: (int(rows[1][i]) - int(rows[0][i])) * scale
        for name, i, scale in (("reads", 3, 1), ("read_bytes", 5, 512), ("writes", 7, 1),
                               ("write_bytes", 9, 512), ("busy_ms", 12, 1), ("flushes", 18, 1))
        if len(rows[0]) > i
    }


def distribution(values):
    ordered = sorted(values)
    return {"mean": mean(values), "p50": median(values),
            "p99": ordered[math.ceil(len(ordered) * .99) - 1], "max": ordered[-1]}


def dense_stages(ledger, score):
    jobs = ledger["jobs"]
    n = len(jobs)
    commit = score["first_commit_us"]
    end = score["last_settlement_us"]
    settle = sorted(j["settled_us"] for j in jobs)
    # Between the ceil(10%) and ceil(90%) settlement ranks.
    low, high = (settle[math.ceil(n * p) - 1] for p in (.1, .9))
    result = {"middle90_minus_10_us": [low, high],
              "middle80_jobs_per_s": (math.ceil(n * .9) - math.ceil(n * .1)) * 1e6 / (high - low),
              "distinct_terminal_xids": len({j.get("settled_xid") for j in jobs}),
              "largest_terminal_xid_jobs": max(Counter(j.get("settled_xid") for j in jobs).values()),
              "distinct_worker_pids": len({j["worker_pid"] for j in jobs}) if "worker_pid" in jobs[0] else None}
    chain = ["claimed_us", "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us"]
    for field in chain[:-1]:
        if any(field not in job for job in jobs):
            continue
        result[field + "_from_commit_ms"] = distribution([(j[field] - commit) / 1000 for j in jobs])
        result[field + "_to_settle_mean_occupancy"] = sum(
            j["settled_us"] - max(j[field], commit) for j in jobs
        ) / (end - commit)
        result[field + "_to_settle_middle_occupancy"] = sum(
            max(0, min(j["settled_us"], high) - max(j[field], low)) for j in jobs
        ) / (high - low)
    for a, b in zip(chain, chain[1:]):
        if a in jobs[0]:
            result[a + "_to_" + b + "_ms"] = distribution([(j[b] - j[a]) / 1000 for j in jobs])
    return result


def read_arm(path, profile, engine, phase, spec):
    payload = blob(path + "/raw.tar.gz")
    request = json.loads(blob(path + "/request.json"))
    with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
        invocation = document(archive, "invocation.json")
        status = document(archive, "status.json")
        host = document(archive, "host.json")
        ledger = document(archive, "adapter/ledger.json")
        with archive.extractfile("host-samples.jsonl") as stream:
            points = [json.loads(line) for line in stream]
        require(status["exit_code"] == 0 and invocation["phase"] == phase and invocation["engine"] == engine
                and invocation["profile"] == profile and invocation["attempt"] == request["attempt"],
                f"incorrect arm identity: {path}")
        require(host["affinity"] == [0, 1] and host["architecture"] == "x86_64",
                f"incorrect native host geometry: {path}")
        require(host["verified_files"] == spec.get("verified_files", {}),
                f"incorrect file hashes: {path}")
        require(not any(p["builders"] for p in points), f"builder overlapped {path}")
        artifact = {field: spec[field] for field in ("version", "artifact_sha256")}
        score = reconcile(ledger, engine, profile, artifact)
        raw = document(archive, "adapter/raw.json") if engine in {"mesh", "oban"} else {}
        if raw:
            writes = document(archive, "adapter/ordered-writes.json")
            app = document(archive, "adapter/application-records.json")
            for label, field in (("attempts", "attempt_xid"), ("effects", "effect_xid")):
                require({(j["job_key"], j[field]) for j in ledger["jobs"]} ==
                        {(row["job_key"], row["xid"]) for row in app[label]}
                        and len(app[label]) == len(ledger["jobs"]), f"{path}: app writes disagree")
            require(writes["ordered"] == writes["jobs"] == score["completed"] == len(raw["attempts"]),
                    f"{path}: raw counts disagree")
            raw_identity = {(j["job_key"], j["id"], j["payload_sha256"]) for j in raw["attempts"]}
            require(raw_identity == {(j["job_key"], j["id"], j["payload_sha256"]) for j in ledger["jobs"]},
                    f"{path}: raw and ledger job identities disagree")
            require(writes["worker_pids"] == len({j["worker_pid"] for j in ledger["jobs"]}),
                    f"{path}: raw worker PID cardinality disagrees")
            require(raw["runner_cpu_seconds_including_startup_and_idle"] == ledger["runner_cpu_seconds"],
                    f"{path}: lifecycle wait4 CPU differs")
        cpu_keys = [f"cpu{cpu}" for cpu in host["affinity"]]
        tick_hz = 100  # Linux x86-64 USER_HZ. Record diagnostics' SC_CLK_TCK independently.
        submit = score["first_submit_us"]
        commit = score["first_commit_us"]
        end = score["last_settlement_us"]
        start_sample, end_sample = (points[i]["epoch_us"] for i in (0, -1))
        windows = {}
        for name, start, stop in (("submit", submit, end), ("drain", commit, end),
                                  ("bracketed_arm", start_sample, end_sample)):
            window = bracket(points, start, stop, "cpu", cpu_keys, tick_hz)
            window.update(disk(points, window["indexes"], host["pg_mount"]))
            window["wall_ms"] = (stop - start) / 1000
            windows[name] = window
        result = {"path": path, "archive_sha256": hashlib.sha256(payload).hexdigest(),
                  "engine": engine, "profile": profile, "phase": phase,
                  "windows": windows, "boot_id": host["boot_id"], "score": score,
                  "producer": [{k: v for k, v in tx.items() if k != "enqueued_ids"}
                               for tx in ledger["transactions"]]}
        if raw:
            before = document(archive, "adapter/host-before.json")["stat"].splitlines()[0].split()[1:9]
            after = document(archive, "adapter/host-after.json")["stat"].splitlines()[0].split()[1:9]
            totals = [int(b) - int(a) for a, b in zip(before, after, strict=True)]
            result["all_guest_adapter_lifecycle_cpu_ms"] = sum(totals[i] for i in (0, 1, 2, 5, 6)) * 10
            result["raw_counters"] = {k: raw[k] for k in raw if k.startswith("raw_") or k in
                                      ("wakeups", "statement_counter_deallocations", "producer_mode")}
            result["raw_samples"] = raw["samples"]
            result["exact_distinct_worker_pids"] = writes["worker_pids"]
        if engine in {"rust", "go"}:
            result["adapter_counters"] = document(archive, "adapter/counters.json")
        if profile == "noop":
            result["stages"] = dense_stages(ledger, score)
        return result


def main():
    index = json.loads(blob(SERIES + "/index.json"))["validated_blocks"]
    series = json.loads(blob(SERIES + "/series.json"))
    verifier_path = SUITE / "verify.py"
    require(hashlib.sha256(verifier_path.read_bytes()).hexdigest() == series["scorer_sha256"],
            "live ledger verifier does not match the frozen scorer digest")
    inventory = series["inventory"]["engines"]
    report = []
    for planned in series["plan"]:
        profile = planned["profile"]
        key = f"{planned['group']:02d}-{profile}"
        directory = f"{SERIES}/blocks/{index[key]}"
        require(json.loads(blob(directory + "/order.json")) == planned, f"order differs in {key}")
        arms = [read_arm(f"{directory}/{phase}-{engine}", profile, engine, phase, inventory[engine])
                for phase in planned["phases"] for engine in planned["order"]]
        report.append({"key": key, **planned, "arms": arms})
    totals = {}
    for group in report:
        for arm in group["arms"]:
            phase = totals.setdefault(arm["phase"], Counter())
            phase.update(arms=1, jobs=arm["score"]["completed"],
                         application_commits=2 * arm["score"]["ordered_independent_writes"])
    json.dump({"commit": REF, "totals": totals, "groups": report}, sys.stdout, indent=2, allow_nan=False)
    print()


if __name__ == "__main__":
    main()
