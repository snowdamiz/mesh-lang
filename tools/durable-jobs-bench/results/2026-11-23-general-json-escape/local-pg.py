#!/usr/bin/env python3
"""Price the unchanged warm file broker against a matching runtime and PostgreSQL 17.11."""

import argparse
import json
import os
import statistics
import time

from measure import (STATE, WORKLOADS, binary_for, digest, import_sibling,
                     rotated, write)

local = import_sibling("local")
local.STATE = STATE


def one_trial(profile, arm, rotation, fixture, expected, out):
    ns = "ordinary_json_escape"
    binary = binary_for("broker", arm)
    cpu = local.CPU()
    with local.pg(out) as (cluster, scratch):
        subject, env, settings = local.prepare(cluster, scratch, out, binary, ns, "dense")
        with subject.warm_producer(env, out, time.monotonic() + 120) as send:
            path, = local.HARNESS["batch_files"](scratch, [fixture])
            assert digest(path) == expected["input_sha256"]
            producer = local.broker_pid(binary)
            initial_cpu = cpu.counters(producer)
            load_before = os.getloadavg()
            started = time.monotonic_ns()
            direct, committed = send(path, out / "receipt.log")
            round_trip_ms = (time.monotonic_ns() - started) / 1e6
            final_cpu = cpu.counters(producer)
            load_after = os.getloadavg()
            stamp, jobs, persisted = local.accepted(cluster, ns, fixture, direct, committed)
            enqueue, admissions, queries = local.counters(cluster)
            timing = cluster.transactions[fixture["request_id"]]
            assert timing["submitted_us"] <= stamp["commit_us"] <= timing["acknowledged_us"]
            assert len(jobs) == len(persisted) == len(fixture["jobs"])
            assert enqueue == expected["job_count"] and admissions == expected["chunks"], queries
            assert len({job["xid"] for job in jobs}) == expected["chunks"]
            cpu_ms = sum(b-a for a, b in zip(initial_cpu, final_cpu, strict=True)) * cpu.ns_per_unit / 1e6
            assert cpu_ms > 0
            row = dict(profile=profile, arm=arm, rotation=rotation, binary_sha256=digest(binary),
                       expected=expected, producer=direct, settings=settings, queries=queries,
                       jobs=jobs, persisted=persisted, transaction=timing, commit=stamp,
                       cpu_counters=dict(producer=producer, start=initial_cpu, end=final_cpu,
                                         ns_per_unit=cpu.ns_per_unit),
                       cpu_ms=cpu_ms, submit_to_commit_ms=(stamp["commit_us"]-timing["submitted_us"])/1000,
                       round_trip_ms=round_trip_ms, load_before=load_before, load_after=load_after)
            write(out / "result.json", row)
    return {key: val for key, val in row.items() if key not in ("jobs", "persisted", "producer")}


def summarize(rows):
    grouped = {}
    for row in rows:
        grouped.setdefault(row["profile"], {}).setdefault(row["arm"], []).append(row)
    for profiles in grouped.values():
        for a, arms in profiles.items():
            values = {"n": len(arms)}
            for key in ("cpu_ms", "round_trip_ms", "submit_to_commit_ms"):
                samples = [arm[key] for arm in arms]
                values[key] = [min(samples), statistics.median(samples), max(samples)]
            profiles[a] = values
        for key in ("cpu_ms", "submit_to_commit_ms"):
            base = statistics.median([profiles[arm][key][1] for arm in ("A", "B") if arm in profiles])
            for row in profiles.values():
                row[f"{key}_pct_vs_dup_control"] = (row[key][1] / base - 1) * 100
    return grouped


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--arms", nargs="+", required=True)
    parser.add_argument("--profiles", nargs="+", choices=list(WORKLOADS), default=list(WORKLOADS))
    parser.add_argument("--rotations", type=int, default=5)
    parser.add_argument("--label", default="pg")
    args = parser.parse_args()

    out = STATE / args.label
    out.mkdir()
    manifest = json.loads((STATE / "fixtures/manifest.json").read_text())
    proofs = json.loads((STATE / "correctness.json").read_text())
    cases = {}
    for profile in args.profiles:
        item = manifest[profile]
        input_path = STATE / "fixtures" / item["path"]
        assert digest(input_path) == item["sha256"]
        frozen = proofs[profile]["A"]
        cases[profile] = json.loads(input_path.read_text()), dict(
            input_sha256=item["sha256"], chunks=len(frozen["chunks"]),
            job_count=item["count"], per_job_payload_bytes=item["payload_bytes"],
            encode_bytes=frozen["encode_bytes"], payload_bytes=item["count"] * item["payload_bytes"])

    results = []
    for i in range(args.rotations):
        for profile, (fixture, expected) in cases.items():
            for arm in rotated(args.arms, i):
                instance = out / profile / f"{i:02}-{arm}"
                row = one_trial(profile, arm, i, fixture, expected, instance)
                results.append(row)
                write(out / "measurements.json", results)
                write(out / "summary.json", summarize(results))
                print(profile, i, arm, round(row["cpu_ms"], 3), round(row["submit_to_commit_ms"], 3),
                      flush=True)


if __name__ == "__main__":
    os.umask(0o077)
    main()
