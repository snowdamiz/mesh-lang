"""Run short, local A/B trials against Elixir or Rust."""

import argparse
from dataclasses import asdict
import hashlib
import os
from pathlib import Path
import random
import shutil
import signal
import statistics
import subprocess
import sys

import run


def run_comparison(args):
    out = args.out.resolve()
    baseline = args.baseline.resolve(strict=True)
    subjects = {"mesh-baseline": ("mesh", baseline)}
    if args.candidate:
        subjects["mesh-candidate"] = ("mesh", args.candidate.resolve(strict=True))
    if args.rust:
        subjects["rust"] = ("rust", args.rust.resolve(strict=True))
    else:
        subjects["elixir"] = ("elixir", (args.elixir or baseline.with_name("elixir")).resolve(strict=True))
    binaries = {}
    for name, (_, path) in subjects.items():
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError(f"Expected executable {path}")
        with path.open("rb") as file:
            binaries[name] = {"path": str(path), "sha256": hashlib.file_digest(file, "sha256").hexdigest()}

    modes = ("actor", "http") if args.workload == "both" else (args.workload,)
    with_http = "http" in modes
    k6 = run.capture(run.k6_command(args)) if with_http else None
    host = run.host_metadata()
    available = host.get("available_cpus", [])
    cores = available[:4] if len(available) >= 4 and shutil.which("taskset") else []
    out.mkdir(parents=True)
    records = run.fixture_records()
    fixture_path = out / "fixtures.json"
    run.write_json(fixture_path, records)
    if with_http:
        shutil.copyfile(run.HERE / "load.js", out / "load.js")
    run.write_json(out / "run.json", {
        "started_at_utc": run.timestamp(),
        "kind": "localhost diagnostic",
        "settings": {key: str(value) if isinstance(value, Path) else value
                     for key, value in vars(args).items()},
        "host": host,
        "pinned_cpus": cores,
        "binaries": binaries,
        "k6": k6,
    })
    print(f"Artifacts {out}", flush=True)

    rng = random.Random(args.seed)
    measurements = {}
    invalid_http = 0
    for block in range(1, args.blocks + 1):
        trials = []
        for name, (language, _) in subjects.items():
            if "actor" in modes:
                trials.extend((name, run.Trial(language, "actor", workers, block,
                                              args.pairs, args.actor_iters))
                              for workers in (1, 4))
            if with_http:
                trials.append((name, run.Trial(language, "http", 4, block, rate=args.http_rate)))
        rng.shuffle(trials)
        for name, trial in trials:
            directory = out / f"{block}-{name}-{trial.mode}-{trial.cpus}"
            directory.mkdir()
            grouped_actor = trial.mode == "actor" and (
                (name == "mesh-candidate" and args.grouped_candidate)
                or (name == "mesh-baseline" and args.grouped_baseline)
            )
            result = run.run_trial(args, directory, trial, cores[:trial.cpus], fixture_path,
                                   records, with_http, executable=subjects[name][1],
                                   check_http=True, grouped_actor=grouped_actor)
            run.write_json(directory / "result.json", {"trial": asdict(trial), **result})
            if trial.mode == "actor":
                data = result["subject"]
                value = data["ops"] * 1e9 / data["elapsed_ns"]
                unit = "round trips/s"
                samples = data["rtt_ns"]
                median_us = statistics.median(samples) / 1000
                p99_us = run.percentile(samples, 99) / 1000
                within_us = 100 * sum(ns <= 1000 for ns in samples) / len(samples)
                for metric, observation in (("µs RTT p50", median_us), ("µs RTT p99", p99_us),
                                            ("% RTT ≤1 µs", within_us)):
                    measurements.setdefault((trial.mode, trial.cpus, name, metric), []).append(observation)
                detail = f"  p50 {median_us:.2f} µs  p99 {p99_us:.2f} µs  ≤1 µs {within_us:.1f}%"
                valid = True
            else:
                load = result["load"]
                value = load["e2e_ms"].get("p(99)")
                valid = value is not None and load["error_fraction"] == 0
                unit = "ms p99"
                detail = ""
            if valid:
                measurements.setdefault((trial.mode, trial.cpus, name, unit), []).append(value)
                print(f"Block {block} {name} {trial.mode} {trial.cpus} workers  {value:,.2f} {unit}{detail}",
                      flush=True)
            else:
                invalid_http += 1
                print(f"Block {block} {name} invalid HTTP trial  "
                      f"{load['error_fraction']:.2%} failed arrivals", flush=True)

    print("\nMedians and ranges of valid trials")
    for (mode, workers, name, unit), values in sorted(measurements.items()):
        print(f"{mode} {workers} workers {name}  {statistics.median(values):,.2f} "
              f"[{min(values):,.2f}, {max(values):,.2f}] {unit} ({len(values)}/{args.blocks})")
    if invalid_http:
        print(f"{invalid_http} HTTP trials were invalid. See the raw trial directories.", file=sys.stderr)
    return int(invalid_http > 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True, help="Path to the original Mesh executable")
    parser.add_argument("--candidate", type=Path, help="Path to the modified Mesh executable")
    parser.add_argument("--grouped-baseline", action="store_true",
                        help="Run the Mesh baseline's actor workload with grouped spawns")
    parser.add_argument("--grouped-candidate", action="store_true",
                        help="Run the Mesh candidate's actor workload with grouped spawns")
    reference = parser.add_mutually_exclusive_group()
    reference.add_argument("--elixir", type=Path, help="Elixir launcher (default is next to the baseline)")
    reference.add_argument("--rust", type=Path, help="Rust benchmark executable")
    parser.add_argument("--out", type=Path, required=True, help="New directory for raw trials")
    parser.add_argument("--workload", choices=("actor", "http", "both"), default="both")
    parser.add_argument("--blocks", type=run.positive, default=5)
    parser.add_argument("--seed", type=int, default=89)
    parser.add_argument("--pairs", type=run.positive, choices=range(1, 65), default=64, metavar="1..64")
    parser.add_argument("--actor-iters", type=run.positive, default=12_000)
    parser.add_argument("--http-rate", type=run.positive, default=200)
    parser.add_argument("--measure-seconds", type=run.positive, default=3)
    parser.add_argument("--warmup-seconds", type=run.nonnegative, default=1)
    parser.add_argument("--warmup-ops", type=run.nonnegative, default=20_000)
    parser.add_argument("--k6", default="k6")
    parser.set_defaults(timeout=60, readiness_timeout=15)
    args = parser.parse_args()
    if args.grouped_baseline and args.workload == "http":
        parser.error("--grouped-baseline requires an actor workload")
    if args.grouped_candidate and (not args.candidate or args.workload == "http"):
        parser.error("--grouped-candidate requires --candidate and an actor workload")
    if max(args.actor_iters, args.warmup_ops) > 100_000_000:
        parser.error("use at most 100000000 actor iterations and warmup operations per pair")
    signal.signal(signal.SIGTERM, run.interrupt)
    try:
        return run_comparison(args)
    except KeyboardInterrupt:
        print("Benchmark interrupted. Partial artifacts remain in the run directory.", file=sys.stderr)
        return 130
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"Benchmark error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
