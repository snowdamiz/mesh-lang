#!/usr/bin/env python3
"""Dispatch serial native Jobs arms, checkpoint evidence, and regenerate a comparison."""

import argparse
import csv
from datetime import datetime, timezone
import fcntl
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shlex
from statistics import median
import subprocess
import sys
import tarfile
import time
from uuid import uuid4

from verify import ENGINES, PROFILES, ROOT, SUITE, expected_inputs, plan, reconcile, require


RESULTS = SUITE / "results"
ACCESS = ROOT / "target/linode-jobs-access"
REMOTE = PurePosixPath("/var/tmp/mesh-durable-jobs-matrix")
SSH = ["ssh", "-F", "/dev/null", "-o", "ConnectTimeout=5", "-o", "ConnectionAttempts=1",
       "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=yes",
       "-o", f"UserKnownHostsFile={ACCESS / 'known_hosts'}",
       "-i", str(ACCESS / "id_ed25519"), "root@172.234.135.126"]


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def read_json(path):
    return json.loads(path.read_text())


def write_json(path, document):
    with path.open("x") as stream:
        json.dump(document, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_inventory(config):
    require(set(config) == {"cpus", "engines"}, "inventory requires cpus and engines")
    cpus = config["cpus"]
    require(len(cpus) == len(set(cpus)) == 2 and all(type(cpu) is int and cpu >= 0 for cpu in cpus),
            "inventory must pin two distinct CPUs")
    require(set(config["engines"]) == set(ENGINES), "inventory must describe all four engines")
    for engine, spec in config["engines"].items():
        require(set(spec) - {"run_as"} == {"argv", "postgres_storage", "version", "artifact_sha256", "evidence_globs"},
                f"{engine}: invalid inventory fields")
        if "run_as" in spec:
            require(re.fullmatch(r"[a-z_][a-z_0-9-]*", spec["run_as"]) and spec["run_as"] != "root",
                    f"{engine}: invalid benchmark user")
        argv = spec["argv"]
        require(argv and argv[0].startswith("/") and all(re.fullmatch(r"[a-zA-Z0-9_./=-]+", arg) for arg in argv),
                f"{engine}: use absolute program and credential-free fixed arguments")
        require(spec["postgres_storage"].startswith("/")
                and re.fullmatch(r"[a-zA-Z0-9_./-]+", spec["postgres_storage"]),
                f"{engine}: expected an absolute owned PostgreSQL storage directory")
        require(re.fullmatch(r"[0-9a-f]{64}", spec["artifact_sha256"]), f"{engine}: invalid artifact digest")
        require(isinstance(spec["version"], str) and re.fullmatch(r"[A-Za-z0-9_.+/@ -]+", spec["version"]),
                f"{engine}: invalid version")
        require(all(re.fullmatch(r"[A-Za-z0-9_./*?-]+", name) and not name.startswith("/")
                    and ".." not in Path(name).parts for name in spec["evidence_globs"]),
                f"{engine}: invalid evidence glob")


def ssh(args, **kwargs):
    command = args if isinstance(args, str) else shlex.join(map(str, args))
    return subprocess.run([*SSH, command], check=False, **kwargs)


def check_host(out):
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"ssh-{uuid4().hex}.txt"
    with path.open("xb") as log:
        log.write(f"{utc_now()} root@172.234.135.126:22\n".encode())
        log.flush()
        result = ssh(["true"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=12)
        log.write(result.stdout.replace(b"\r\n", b"\n"))
        log.write(f"exit_status={result.returncode}\n".encode())
    print(path)
    return result.returncode == 0


def stage(path, remote, log):
    target = shlex.quote(str(remote / path.name))
    # Upload only the named driver and public inventory. On resume verify the
    # existing bytes. A failed transfer leaves its temporary file on the host.
    checksum = shlex.quote(digest(path))
    script = (f"umask 077 && mkdir -p {shlex.quote(str(remote))} && "
              f"if test -f {target}; then cat >/dev/null && "
              f"test \"$(sha256sum {target} | cut -d ' ' -f 1)\" = {checksum}; "
              f"else staged=$(mktemp {target}.partial.XXXXXX) && cat >\"$staged\" && "
              f"test \"$(sha256sum \"$staged\" | cut -d ' ' -f 1)\" = {checksum} && "
              f"mv -n \"$staged\" {target}; fi")
    with path.open("rb") as source:
        result = ssh(script, stdin=source, stdout=log, stderr=log, timeout=30)
    require(result.returncode == 0, f"could not stage {path.name} over SSH")


def file_json(archive, name):
    member = archive.getmember(name)
    require(member.isfile(), f"archive member {name} must be a regular file")
    with archive.extractfile(member) as stream:
        return json.load(stream)


def archive_arm(directory, remote, attempt):
    if (directory / "raw.tar.gz").exists():
        return
    transfer = uuid4().hex
    partial = directory / f"download-{transfer}.partial"
    with partial.open("xb") as stream, (directory / f"download-{transfer}.txt").open("xb") as log:
        try:
            result = ssh(["python3", "-B", remote / "worker.py", "export", attempt], stdout=stream,
                         stderr=log, timeout=180)
        except subprocess.TimeoutExpired:
            log.write(b"SSH evidence transfer exceeded 180 seconds\n")
            return
    if result.returncode != 0:
        return
    try:
        with tarfile.open(partial, "r:gz") as archive:
            invocation = file_json(archive, "invocation.json")
            require(invocation["attempt"] == attempt, "downloaded wrong arm")
            file_json(archive, "status.json")
    except (OSError, ValueError, KeyError, tarfile.TarError) as exc:
        with (directory / f"download-{transfer}.txt").open("a") as log:
            print(f"Downloaded evidence failed validation ({type(exc).__name__})", file=log)
        return
    partial.rename(directory / "raw.tar.gz")


def dispatch_arm(path, remote, block, phase, engine):
    attempt = uuid4().hex
    directory = path / f"{phase}-{engine}"
    directory.mkdir()
    write_json(directory / "request.json", {"attempt": attempt, "phase": phase, "engine": engine,
                                             "profile": block["profile"], "created": utc_now()})
    args = ["python3", "-B", remote / "worker.py", "run", attempt, "--engine", engine,
            "--profile", block["profile"], "--phase", phase]
    try:
        with (directory / "dispatch.txt").open("xb") as log:
            result = ssh(args, stdout=log, stderr=log, timeout=930)
        outcome = {"exit_code": result.returncode}
    except subprocess.TimeoutExpired:
        outcome = {"interrupted": "SSH wait exceeded 930 seconds"}
    else:
        print(f"{path.name} {phase} {engine}: {result.returncode}", flush=True)
    try:
        write_json(directory / "dispatch.json", {**outcome, "finished": utc_now()})
    finally:
        archive_arm(directory, remote, attempt)


def selected_blocks(blocks, args):
    return [block for block in blocks if (not args.group or block["group"] in args.group)
            and (not args.profile or block["profile"] in args.profile)]


def block_prefix(block):
    return f"{block['group']:02d}-{block['profile']}"


def sample_window(host, samples, result):
    start, end = result["first_submit_us"], result["last_settlement_us"]
    require(samples and samples[0]["epoch_us"] <= start < end <= samples[-1]["epoch_us"],
            "server clock samples do not bracket the full submit window")
    require(all(left["epoch_us"] <= right["epoch_us"] and left["monotonic_ns"] < right["monotonic_ns"]
                for left, right in zip(samples, samples[1:])), "host timing clock moved backwards")
    lower = max(index for index, point in enumerate(samples) if point["epoch_us"] <= start)
    upper = next(index for index, point in enumerate(samples) if point["epoch_us"] >= end)
    bounding = samples[lower:upper + 1]
    require(all(right["monotonic_ns"] - left["monotonic_ns"] <= 1_000_000_000
                for left, right in zip(bounding, bounding[1:])), "host counter recorder missed the window")
    require(not any(point["builders"] for point in samples), "build process overlapped arm")
    first = bounding[0]["cpu"]
    last = bounding[-1]["cpu"]
    deltas = [[after - before for before, after in zip(first[f"cpu{cpu}"], last[f"cpu{cpu}"], strict=True)]
              for cpu in host["affinity"]]
    require(all(min(delta) >= 0 and sum(delta) > 0 for delta in deltas), "invalid Linux CPU counter window")
    ticks = sum(map(sum, deltas))
    steal_pct = 100 * sum(delta[7] for delta in deltas) / ticks
    require(steal_pct <= 1, f"CPU steal {steal_pct:.2f}% exceeds the 1% comparison limit")
    mount = host["pg_mount"]
    major_minor = mount["maj:min"].split(":")
    disk_rows = []
    for point in (bounding[0], bounding[-1]):
        rows = [line.split() for line in point["diskstats"]]
        disk_rows.append(next((row for row in rows if row[:2] == major_minor), None))
    disk = {}
    if all(disk_rows):
        before, after = disk_rows
        for name, field, scale in (("disk_read_bytes", 5, 512), ("disk_write_bytes", 9, 512),
                                   ("disk_busy_ms", 12, 1), ("disk_flushes", 18, 1)):
            if field < min(len(before), len(after)):
                disk[name] = (int(after[field]) - int(before[field])) * scale
    return {"host_boot_id": host["boot_id"], "host_kernel": host["kernel"],
            "pg_mount": mount, "affinity": host["affinity"],
            "cpu_steal_percent": steal_pct,
            "cpu_iowait_percent": 100 * sum(delta[4] for delta in deltas) / ticks,
            "host_sample_span_ms": (bounding[-1]["epoch_us"] - bounding[0]["epoch_us"]) / 1000,
            "host_samples": len(bounding), **disk}


def score_arm(directory, block, phase, engine, config):
    with tarfile.open(directory / "raw.tar.gz", "r:gz") as archive:
        invocation = file_json(archive, "invocation.json")
        request = read_json(directory / "request.json")
        for field, value in {"attempt": request["attempt"], "phase": phase,
                             "engine": engine, "profile": block["profile"]}.items():
            require(invocation[field] == value, f"archive invocation {field} mismatched")
        require(file_json(archive, "status.json")["exit_code"] == 0, "adapter reported a failed exit")
        host = file_json(archive, "host.json")
        require(host["affinity"] == config["cpus"], "host sample mask differs from required affinity")
        artifact = {field: config["engines"][engine][field] for field in ("artifact_sha256", "version")}
        result = reconcile(file_json(archive, "adapter/ledger.json"), engine, block["profile"], artifact)
        with archive.extractfile("host-samples.jsonl") as stream:
            samples = [json.loads(line) for line in stream]
        result.update(sample_window(host, samples, result))
        return result


def verify_block(path, block, config):
    require(read_json(path / "order.json") == block, "recorded block order differs from the plan")
    last = {}
    host_ids = set()
    for phase in block["phases"]:
        last = {engine: score_arm(path / f"{phase}-{engine}", block, phase, engine, config)
                for engine in block["order"]}
        host_ids.update((row["host_boot_id"], row["host_kernel"], row["pg_mount"]["maj:min"])
                        for row in last.values())
    require(len(host_ids) == 1, "block spans different host boots, kernels, or disks")
    return last


def archive_index(out, blocks, config):
    selected = {}
    issues = {}
    for block in blocks:
        prefix = block_prefix(block)
        attempts = sorted((out / "blocks").glob(f"{prefix}.*"))
        for path in attempts:
            try:
                scores = verify_block(path, block, config)
            except (OSError, ValueError, KeyError, TypeError, tarfile.TarError) as exc:
                issues[path.name] = str(exc)
            else:
                if prefix not in selected:
                    selected[prefix] = (path.name, scores)
    return selected, issues


def summarize(out):
    series = read_json(out / "series.json")
    blocks = series["plan"]
    selected, issues = archive_index(out, blocks, series["inventory"])
    print(json.dumps({"kind": series["kind"], "required_four_engine_blocks": len(blocks),
                      "validated_blocks": {key: name for key, (name, _) in selected.items()},
                      "missing_blocks": [block_prefix(block) for block in blocks if block_prefix(block) not in selected],
                      "invalid_or_incomplete_attempts": issues}, indent=2), file=sys.stderr, flush=True)
    if len(selected) != len(blocks):
        return 1
    scores = [(block, *selected[block_prefix(block)]) for block in blocks]
    identities = {(row["host_boot_id"], row["host_kernel"], row["pg_mount"]["maj:min"])
                  for _, _, arms in scores for row in arms.values()}
    require(len(identities) == 1, "complete candidate series spans different host boots, kernels, or disks")
    if series["kind"] not in ("baseline", "repeat"):
        return 0
    if series["kind"] == "baseline":
        controls = series["controls"]
        require(controls, "baseline has no retained initial controls")
        require(controls == verify_controls(controls["series"], series["inventory"]),
                "initial control evidence changed since dispatch")
        require(identities == {tuple(controls["host"])}, "baseline ran on a different boot, kernel, or disk than its controls")
    else:
        reference = series["reference"]
        require(reference, "repeat has no retained reference baseline")
        require(reference == verify_reference(reference["series"], series["inventory"]),
                "reference baseline changed since dispatch")
        require(identities == {tuple(reference["host"])}, "repeat ran on a different boot, kernel, or disk")
    fields = ("commit_jobs_per_s", "submit_jobs_per_s", "commit_to_pickup_p99_ms", "submit_to_pickup_p99_ms",
              "commit_to_settlement_p99_ms", "submit_to_settlement_p99_ms", "runner_cpu_ms_per_job",
              "peak_runner_rss_bytes", "peak_connections", "cpu_steal_percent", "cpu_iowait_percent")
    writer = csv.writer(sys.stdout, lineterminator="\n")
    writer.writerow(["profile", "engine", "metric", "n", "median", "min", "max"])
    for profile in dict.fromkeys(block["profile"] for block in blocks):
        for engine in ENGINES:
            for field in fields:
                data = [arms[engine][field] for block, _, arms in scores if block["profile"] == profile]
                writer.writerow([profile, engine, field, len(data), median(data), min(data), max(data)])
    return 0


def initialize(out, kind, config, controls=None, reference=None, profiles=None):
    blocks = [block for block in plan(kind) if not profiles or block["profile"] in profiles]
    identity = {"kind": kind, "inventory": config, "plan": blocks, "controls": controls, "reference": reference,
                "worker_sha256": digest(SUITE / "worker.py"),
                "scorer_sha256": digest(SUITE / "verify.py"),
                "driver_sha256": digest(SUITE / "run.py")}
    manifest = out / "series.json"
    if manifest.exists():
        prior = read_json(manifest)
        require(all(prior[key] == value for key, value in identity.items()),
                "resume requires the frozen plan, drivers and inventory; start a new series for changes")
        if not (out / "inventory.json").exists():
            write_json(out / "inventory.json", config)
        require(read_json(out / "inventory.json") == config, "staged inventory differs from the frozen plan")
        return prior
    identity.update({"host_series": out.name + "-" + uuid4().hex, "created": utc_now()})
    write_json(manifest, identity)
    write_json(out / "inventory.json", config)
    return identity


def verify_controls(series, config):
    directory = RESULTS / series
    control = read_json(directory / "series.json")
    blocks = plan("controls")
    require(control["kind"] == "controls" and control["inventory"] == config and control["plan"] == blocks,
            "initial controls must use the same inventory and both reversed orders")
    selected, _ = archive_index(directory, blocks, config)
    require(len(selected) == len(blocks), "initial four-engine controls are incomplete")
    hosts = {(row["host_boot_id"], row["host_kernel"], row["pg_mount"]["maj:min"])
             for _, arms in selected.values() for row in arms.values()}
    require(len(hosts) == 1, "initial controls span different host boots, kernels, or disks")
    return {"series": series, "host": list(hosts.pop()),
            "validated_blocks": {key: name for key, (name, _) in selected.items()}}


def verify_reference(series, config):
    directory = RESULTS / series
    baseline = read_json(directory / "series.json")
    old = baseline["inventory"]
    blocks = plan("baseline")
    require(baseline["kind"] == "baseline" and baseline["plan"] == blocks and baseline["controls"],
            "repeat requires a controlled baseline")
    require(old["cpus"] == config["cpus"] and all(old["engines"][engine] == config["engines"][engine]
            for engine in ENGINES if engine != "mesh"), "repeat changed the CPU mask or a peer engine")
    require(old["engines"]["mesh"]["artifact_sha256"] != config["engines"]["mesh"]["artifact_sha256"],
            "repeat requires a different Mesh artifact")
    selected, _ = archive_index(directory, blocks, old)
    require(len(selected) == len(blocks), "reference baseline is incomplete")
    control = baseline["controls"]
    require(control == verify_controls(control["series"], old), "reference control evidence changed")
    hosts = {(row["host_boot_id"], row["host_kernel"], row["pg_mount"]["maj:min"])
             for _, arms in selected.values() for row in arms.values()}
    require(hosts == {tuple(control["host"])}, "reference and controls disagree on host, kernel, or disk")
    return {"series": series, "host": control["host"], "mesh": old["engines"]["mesh"]["artifact_sha256"],
            "validated_blocks": {key: name for key, (name, _) in selected.items()}}


def dispatch(out, args):
    config = read_json(args.inventory)
    check_inventory(config)
    for profile in PROFILES:
        expected_inputs(profile)
    if not check_host(out / "access"):
        return 1
    controls = verify_controls(args.controls_series, config) if args.kind == "baseline" else None
    reference = verify_reference(args.reference_series, config) if args.kind == "repeat" else None
    out.mkdir(exist_ok=True)
    series = initialize(out, args.kind, config, controls, reference, args.profile if reference else None)
    remote = REMOTE / series["host_series"]
    with (out / f"stage-{uuid4().hex}.txt").open("xb") as log:
        stage(SUITE / "worker.py", remote, log)
        stage(out / "inventory.json", remote, log)
    blocks = selected_blocks(series["plan"], args)
    require(blocks, "no blocks selected")
    completed, _ = archive_index(out, blocks, config)
    for block in blocks:
        prefix = block_prefix(block)
        if prefix in completed:
            continue
        path = out / "blocks" / (prefix + "." + str(time.time_ns()))
        path.mkdir(parents=True)
        write_json(path / "order.json", block)
        for phase in block["phases"]:
            for engine in block["order"]:
                dispatch_arm(path, remote, block, phase, engine)
        try:
            checked = verify_block(path, block, config)
        except (OSError, ValueError, KeyError, TypeError, tarfile.TarError) as exc:
            print(f"{path.name} incomplete: {exc}", file=sys.stderr, flush=True)
        else:
            write_json(path / "reconciled.json", checked)
            print(f"{path.name} reconciled", flush=True)
    completed, _ = archive_index(out, blocks, config)
    return 0 if len(completed) == len(blocks) else 1


def recover(out):
    series = read_json(out / "series.json")
    remote = REMOTE / series["host_series"]
    for request in sorted((out / "blocks").glob("*/*/request.json")):
        archive_arm(request.parent, remote, read_json(request)["attempt"])
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("plan", "check-host", "dispatch", "recover", "summarize"))
    parser.add_argument("--kind", choices=("controls", "baseline", "diagnostic", "repeat"), default="baseline")
    parser.add_argument("--series")
    parser.add_argument("--controls-series")
    parser.add_argument("--reference-series")
    parser.add_argument("--inventory", type=Path)
    parser.add_argument("--profile", choices=PROFILES, action="append")
    parser.add_argument("--group", type=int, choices=range(1, 6), action="append")
    args = parser.parse_args()
    if args.action == "plan":
        print(json.dumps(selected_blocks(plan(args.kind), args), indent=2))
        return 0
    if not args.series or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9.-]{0,63}", args.series):
        parser.error("a safe --series name is required")
    out = RESULTS / args.series
    if args.action == "check-host":
        return 0 if check_host(out / "access") else 1
    if args.action == "summarize":
        return summarize(out)
    if args.action == "dispatch" and args.inventory is None:
        parser.error("--inventory is required to dispatch")
    if args.action == "dispatch" and args.kind == "baseline" and (
            not args.controls_series or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9.-]{0,63}", args.controls_series)):
        parser.error("baseline dispatch requires --controls-series with validated reversed controls")
    if args.action == "dispatch" and args.kind == "repeat" and (
            not args.reference_series or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9.-]{0,63}", args.reference_series)):
        parser.error("repeat dispatch requires --reference-series with a validated baseline")
    # A single local owner dispatches and recovers. The host worker also fences
    # concurrent arm starts, including after an SSH disconnect.
    with (SUITE / ".timing.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return dispatch(out, args) if args.action == "dispatch" else recover(out)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
