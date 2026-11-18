#!/usr/bin/env python3
"""Run contract.md workloads and report independent, write-once benchmark runs."""

import argparse
from collections import defaultdict
from contextlib import contextmanager
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
import hashlib
import http.client
import ipaddress
from io import BytesIO
import json
import math
import os
from pathlib import Path
import platform
import random
import re
import resource
import shlex
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import time


LANGUAGES = ("mesh", "elixir", "go", "rust")
LOCAL = "127.0.0.1"
HERE = Path(__file__).resolve().parent
PROBE_TIMEOUT = 1
PROBE_INTERVAL = 0.002
SAMPLE_INTERVAL = 0.25
BLOCKS = 5
OPEN_FILES_LIMIT = 65_536
SLO_MS = 50
SLO_ERROR_FRACTION = 0.001


@dataclass(frozen=True)
class Trial:
    language: str
    mode: str
    cpus: int
    block: int
    pairs: int = 0
    iterations: int = 0
    rate: int = 0


def timestamp():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def compact(value):
    return json.dumps(value, separators=(",", ":"), allow_nan=False)


def write_json(path, value):
    with path.open("x") as file:
        json.dump(value, file, indent=2, allow_nan=False)
        file.write("\n")


def fixture_records():
    return [
        {"id": 100 + row, "values": [100 + (17 * col + 253 * row) % 900 for col in range(250)]}
        for row in range(4)
    ]


def score(record):
    return record["id"] + sum(i * value for i, value in enumerate(record["values"], 1))


def cpu_checksum(records, iterations):
    cycles, remainder = divmod(iterations, len(records))
    scores = [score(record) for record in records]
    return cycles * sum(scores) + sum(scores[:remainder])


def clean_env():
    return {key: os.environ[key] for key in ("PATH", "TMPDIR") if key in os.environ}


def subject_env(trial, fixture_path, port, warmup):
    workers = str(trial.cpus)
    env = {
        "PERF_MODE": "serve" if trial.rate else trial.mode,
        "PERF_WORKERS": workers,
        "PERF_FIXTURES": str(fixture_path),
        "PERF_PORT": str(port),
        "PERF_WARMUP": str(warmup if trial.mode in ("cpu", "actor") else 0),
    }
    if trial.pairs:
        env["PERF_PAIRS"] = str(trial.pairs)
    if trial.iterations:
        env["PERF_ITERS"] = str(trial.iterations)
    env.update({
        "mesh": {"MESH_SCHEDULER_MIN_WORKERS": workers, "MESH_SCHEDULER_MAX_WORKERS": workers},
        "elixir": {"ERL_FLAGS": f"+S {workers}:{workers}", "RELEASE_DISTRIBUTION": "none"},
        "go": {"GOMAXPROCS": workers},
        "rust": {},
    }[trial.language])
    return env


@contextmanager
def process(command, directory, name, env, stdin=subprocess.DEVNULL):
    with (directory / f"{name}.stdout").open("xb") as output, (
        directory / f"{name}.stderr"
    ).open("xb") as errors:
        child = subprocess.Popen(
            command, stdin=stdin, stdout=output, stderr=errors, env=env, start_new_session=True,
        )
        try:
            yield child
        finally:
            if child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                    child.wait(timeout=2)
                except ProcessLookupError:
                    pass
                except subprocess.TimeoutExpired:
                    pass
            # The group can still contain children after its leader exits.
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait(timeout=2)


def wait(child, timeout, tick):
    deadline = time.monotonic() + timeout
    while True:
        tick()
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            if child.poll() is not None:
                return child.returncode
            raise TimeoutError(f"Process {child.pid} exceeded {timeout:g}s")
        try:
            return child.wait(timeout=min(remaining, SAMPLE_INTERVAL))
        except subprocess.TimeoutExpired:
            pass


def proc_read(pid, phase):
    proc = Path("/proc") / str(pid)
    try:
        status = (proc / "status").read_text()
        stat = (proc / "stat").read_text().rsplit(")", 1)[1].split()
        memory = {}
        for line in status.splitlines():
            key, _, value = line.partition(":")
            if key in ("VmRSS", "VmHWM"):
                memory[key + "_kib"] = int(value.split()[0])
        return {
            "at_utc": timestamp(),
            "monotonic_ns": time.monotonic_ns(),
            "phase": phase,
            "user_ticks": int(stat[11]),
            "system_ticks": int(stat[12]),
            **memory,
        }
    except OSError:
        return None


def sample(file, child, phase):
    point = proc_read(child.pid, phase)
    if point is not None:
        file.write(compact(point) + "\n")
        file.flush()


def validate_result(trial, records, result):
    if not isinstance(result, dict):
        raise ValueError("Expected one JSON result object")
    if trial.mode == "actor":
        ops = trial.pairs * trial.iterations
        checksum = ops * (trial.iterations + 1) // 2
        count = trial.pairs * ((trial.iterations - 1) // 64 + 1)
        samples = result.get("rtt_ns")
        if not isinstance(samples, list) or len(samples) != count or any(
            type(item) is not int or item < 0 for item in samples
        ):
            raise ValueError(f"Expected {count} nonnegative integer RTT samples")
    elif trial.mode == "cpu":
        ops = trial.iterations
        checksum = cpu_checksum(records, ops)
    else:
        ops = checksum = 1
    for key, expected in (("ops", ops), ("checksum", checksum)):
        if type(result.get(key)) is not int or result[key] != expected:
            raise ValueError(f"Expected {key}={expected}, got {result.get(key)!r}")
    if type(result.get("elapsed_ns")) is not int or result["elapsed_ns"] <= 0:
        raise ValueError("Expected a positive integer elapsed_ns")


def request(port, route, body=None, timeout=PROBE_TIMEOUT):
    method = "GET" if body is None else "POST"
    headers = f"{method} {route} HTTP/1.1\r\nHost: {LOCAL}:{port}\r\nConnection: close\r\n"
    if body is not None:
        headers += f"Content-Type: application/json\r\nContent-Length: {len(body)}\r\n"
    raw = bytearray()
    deadline = time.monotonic() + timeout
    with socket.create_connection((LOCAL, port), timeout=timeout) as conn:
        conn.sendall((headers + "\r\n").encode() + (body or b""))
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"{route} did not complete and close its connection")
            conn.settimeout(remaining)
            data = conn.recv(4096)
            if not data:
                break
            raw.extend(data)
            if len(raw) > 64 * 1024:
                raise ValueError(f"{route} response exceeded 64 KiB")

    class CapturedSocket:
        def makefile(self, *_args):
            return BytesIO(raw)

    with http.client.HTTPResponse(CapturedSocket(), method=method) as response:
        response.begin()
        if response.version != 11:
            raise ValueError(f"{route} must respond with HTTP/1.1")
        return (
            response.status,
            response.headers.get_content_type(),
            response.read(),
        )


def expect_json(response, expected):
    status, media_type, body = response
    if status != 200 or media_type != "application/json":
        raise ValueError(f"Expected HTTP 200 application/json, got {status} {media_type}")
    if json.dumps(expected, sort_keys=True) != json.dumps(json.loads(body), sort_keys=True):
        raise ValueError(f"Expected {expected!r}, got {body[:200]!r}")


def readiness(child, port, start_ns, readiness_timeout):
    deadline = time.monotonic() + readiness_timeout
    attempts = 0
    lower_ns = 0
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise ValueError(f"Server exited before readiness with status {child.returncode}")
        attempts += 1
        probe_started = time.monotonic_ns()
        try:
            response = request(port, "/health", timeout=min(PROBE_TIMEOUT, max(0, deadline - time.monotonic())))
        except (ConnectionRefusedError, ConnectionResetError, BrokenPipeError, TimeoutError):
            lower_ns = probe_started - start_ns
            time.sleep(PROBE_INTERVAL)
            continue
        expect_json(response, {"ok": True})
        if child.poll() is not None:
            raise ValueError("Server exited during readiness")
        return {
            "ready_ns": time.monotonic_ns() - start_ns, "lower_ns": lower_ns,
            "attempts": attempts, "at_utc": timestamp(),
        }
    raise TimeoutError("Server did not become ready before the startup deadline")


def smoke_http(port, records):
    records = [*records, {"id": 1_000_000, "values": [-1_000_000, 1_000_000], "extra": True}]
    for record in records:
        response = request(port, "/score", compact(record).encode())
        expect_json(response, {"id": record["id"], "score": score(record)})
    expect_json(
        request(port, "/score", b'{"id":1.0,"values":[-1.0,3e0]}'),
        {"id": 1, "score": 6},
    )
    invalid = (
        b"{", b"[]", b"null", b"{}", b'{"id":1}', b'{"id":-1,"values":[1]}',
        b'{"id":"1","values":[1]}', b'{"id":1,"values":null}',
        b'{"id":1,"values":[true]}', b'{"id":1,"values":[1.5]}',
        b'{"id":1000001,"values":[]}', b'{"id":1,"values":[-1000001]}',
        compact({"id": 0, "values": [0] * 257}).encode(),
    )
    for body in invalid:
        status, _, _ = request(port, "/score", body)
        if status != 400:
            raise ValueError(f"Expected HTTP 400 for {body!r}, got {status}")
    return {"validated": len(records) + 1, "rejected": len(invalid), "connection_close": True}


def k6_command(args, config=None):
    command = [args.k6, "version"]
    if config is not None:
        command = [args.k6, "run", "--quiet", "--address=", "--env", f"PERF_CONFIG={compact(config)}", "-"]
    if not getattr(args, "load_host", None):
        return command
    if config is not None:
        command = ["timeout", "--kill-after=3s", f"{config['seconds'] + 20}s", *command]
    clean = 'env -i PATH="$PATH" XDG_CONFIG_HOME=/nonexistent K6_NO_USAGE_REPORT=true '
    return ssh_command(args.load_host, clean + shlex.join(command))


def ssh_command(host, script):
    return [
        "ssh", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5",
        "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2",
        "--", host, script,
    ]


def capture(command, env=None):
    return subprocess.run(
        command, check=True, capture_output=True, text=True, timeout=20, env=env,
    ).stdout.strip()


def read_if_present(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return None


def host_metadata():
    info = {
        "platform": platform.uname()._asdict(),
        "python": platform.python_version(),
        "logical_cpus": os.cpu_count(),
        "load_average": os.getloadavg(),
        "boot_id": read_if_present("/proc/sys/kernel/random/boot_id"),
    }
    if sys.platform == "linux":
        info["available_cpus"] = sorted(os.sched_getaffinity(0))
        info["clock_ticks_per_second"] = os.sysconf("SC_CLK_TCK")
        info["cpu_quota"] = read_if_present("/sys/fs/cgroup/cpu.max")
        info["cpu_governor"] = read_if_present("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        info["smt_active"] = read_if_present("/sys/devices/system/cpu/smt/active")
        if shutil.which("lscpu"):
            info["lscpu"] = capture(["lscpu", "--json"], {**clean_env(), "LC_ALL": "C"})
    elif sys.platform == "darwin":
        info["cpu_model"] = capture(["sysctl", "-n", "machdep.cpu.brand_string"])
    return info


def load_metadata(args, server_boot_id):
    env = {**clean_env(), "XDG_CONFIG_HOME": "/nonexistent", "K6_NO_USAGE_REPORT": "true"}
    version = capture(k6_command(args), None if getattr(args, "load_host", None) else env)
    expected = getattr(args, "k6_version", None)
    if expected and re.search(r"\bk6 v?" + re.escape(expected.lstrip("v")) + r"(?:\s|$)", version) is None:
        raise ValueError(f"Expected k6 {expected}, got {version}")
    info = {"k6": version}
    if getattr(args, "load_host", None):
        report = capture(ssh_command(
            args.load_host,
            "cat /proc/sys/kernel/random/boot_id && command -v timeout && uname -a &&"
            " date -u +%Y-%m-%dT%H:%M:%SZ && nproc && cat /proc/loadavg && lscpu"
            " && cat /proc/sys/net/ipv4/ip_local_port_range /proc/sys/net/ipv4/tcp_tw_reuse"
            " && ulimit -n",
        ))
        load_boot_id = report.splitlines()[0]
        if load_boot_id == server_boot_id:
            raise ValueError("SSH load generator must run on a different host")
        info.update({"ssh_destination": args.load_host, "system": report})
    return info


def load_result(summary, target, seconds):
    metrics = summary["metrics"]

    def count(name):
        number = metrics.get(name, {}).get("values", {}).get("count", 0)
        if type(number) is not int or number < 0:
            raise ValueError(f"Invalid k6 count {name}={number!r}")
        return number

    started = count("score_started")
    success = count("score_success")
    errors = count("score_errors")
    within_slo = count("score_within_slo")
    dropped = count("dropped_iterations")
    if within_slo > success or success + errors > started or started < count("iterations"):
        raise ValueError("Inconsistent k6 request counters")
    offered = started + dropped
    missing = max(0, target - offered)
    interrupted = started - success - errors
    failed = errors + dropped + interrupted + missing
    error_fraction = failed / max(target, offered)
    latency = metrics.get("score_e2e_ms", {}).get("values", {})
    p99 = latency.get("p(99)")
    return {
        "target_offered": target, "offered": offered, "started": started,
        "success": success, "errors": errors, "dropped": dropped,
        "interrupted": interrupted, "missing": missing,
        "error_fraction": error_fraction, "e2e_ms": latency,
        "success_per_s": success / seconds, "within_slo": within_slo,
        "slo_goodput_per_s": within_slo / seconds,
        "slo_met": p99 is not None and p99 <= SLO_MS and error_fraction < SLO_ERROR_FRACTION,
    }


def run_load(args, directory, server, samples, port, records, phase, rate, seconds):
    if server.poll() is not None:
        raise ValueError("Server exited before load")
    remote = bool(getattr(args, "load_host", None))
    address = args.server_address if remote else LOCAL
    config = {
        "url": f"http://{address}:{port}/score",
        "rate": rate, "seconds": seconds,
        "slo_ms": SLO_MS,
        "vus": getattr(args, "k6_vus", None) or max(4, math.ceil(rate / 5)),
        "cases": [{"record": record, "expected": {"id": record["id"], "score": score(record)}}
                  for record in records],
    }
    write_json(directory / f"{phase}.config.json", config)
    start = timestamp()
    env = {**clean_env(), "XDG_CONFIG_HOME": "/nonexistent", "K6_NO_USAGE_REPORT": "true"}

    def tick():
        if server.poll() is not None:
            raise ValueError("Server exited under load")
        sample(samples, server, phase)

    with (directory.parent / "load.js").open("rb") as script, process(
        k6_command(args, config), directory, phase, None if remote else env, stdin=script,
    ) as child:
        code = wait(child, seconds + 40, tick)
        tick()
        if code:
            raise ValueError(f"k6 {phase} exited with status {code}")
    result = load_result(json.loads((directory / f"{phase}.stdout").read_text()), rate * seconds, seconds)
    result.update({"started_at_utc": start, "finished_at_utc": timestamp(), "seconds": seconds})
    write_json(directory / f"{phase}.json", result)
    return result


def choose_port(fixed):
    with socket.socket() as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind(("0.0.0.0", fixed))
        return sock.getsockname()[1]


def run_trial(args, directory, trial, cores, fixture_path, records, k6, *,
              executable=None, check_http=False, grouped_actor=False):
    server = trial.mode in ("http", "sustained", "serve")
    port = choose_port(getattr(args, "port", 0)) if server else 0
    subject = str(executable if executable is not None else args.bin_dir / trial.language)
    command = ["taskset", "--cpu-list", ",".join(map(str, cores)), subject] if cores else [subject]
    env = subject_env(trial, fixture_path, port, args.warmup_ops)
    if grouped_actor:
        env["PERF_ACTOR_PLACEMENT"] = "grouped"
    write_json(directory / "subject.json", {"command": command, "environment": env})
    start_ns = time.monotonic_ns()
    with process(command, directory, "subject", {**clean_env(), **env}) as child, (
        directory / "process.jsonl"
    ).open("x") as samples:
        if not server:
            code = wait(child, args.timeout, lambda: sample(samples, child, trial.mode))
            finished_ns = time.monotonic_ns()
            if code:
                raise ValueError(f"Subject exited with status {code}")
            data = json.loads((directory / "subject.stdout").read_text())
            validate_result(trial, records, data)
            if grouped_actor and data.get("actor_placement") != "grouped":
                raise ValueError("Mesh executable did not report grouped actor placement")
            return {"subject": data, "process_wall_ns": finished_ns - start_ns}

        ready = readiness(child, port, start_ns, args.readiness_timeout)
        write_json(directory / "readiness.json", ready)
        sample(samples, child, "ready")
        if trial.mode == "serve":
            result = {"readiness": ready, "http": smoke_http(port, records)}
            if k6:
                load = run_load(args, directory, child, samples, port, records, "smoke", 4, 1)
                if load["success"] < load["target_offered"] or load["error_fraction"]:
                    raise ValueError(f"k6 smoke failed with {load['success']} valid responses "
                                     f"and {load['error_fraction']:.2%} failed load")
                result["load"] = load
            return result

        if check_http:
            write_json(directory / "http-contract.json", smoke_http(port, records))
        if args.warmup_seconds:
            warmup = run_load(args, directory, child, samples, port, records, "warmup",
                              trial.rate, args.warmup_seconds)
            if check_http and warmup["error_fraction"]:
                raise ValueError(f"HTTP warmup had {warmup['error_fraction']:.2%} failed arrivals")
        seconds = args.sustained_seconds if trial.mode == "sustained" else args.measure_seconds
        load = run_load(args, directory, child, samples, port, records, "measure", trial.rate, seconds)
        return {"readiness": ready, "load": load}


def trial_order(args):
    if args.command == "smoke":
        return [
            Trial(language, mode, 1, 1, pairs, iterations)
            for language in args.languages
            for mode, pairs, iterations in (
                ("actor", 3, 129), ("cpu", 0, 13), ("recovery", 0, 0), ("serve", 0, 0),
            )
        ]
    rng = random.Random(args.seed)
    trials = []
    for block in range(1, args.blocks + 1):
        current = []
        for language in args.languages:
            for cpus in (1, 4):
                current.extend(
                    Trial(language, "actor", cpus, block, pairs, args.actor_iters)
                    for pairs in sorted({1, args.pairs})
                )
                current.append(Trial(language, "recovery", cpus, block))
                current.extend(Trial(language, "http", cpus, block, rate=rate) for rate in args.http_rates)
            current.append(Trial(language, "cpu", 1, block, iterations=args.cpu_iters))
            if block == args.blocks:
                current.append(Trial(language, "sustained", 4, block, rate=args.sustained_rate))
        rng.shuffle(current)
        trials.extend(current)
    return trials


def run_suite(args):
    host = host_metadata()
    smoke = args.command == "smoke"
    available = host.get("available_cpus", [])
    if not smoke and (len(available) < 4 or shutil.which("taskset") is None):
        raise ValueError("run requires Linux, taskset, and at least four available logical CPUs")
    if not smoke:
        _, hard_limit = resource.getrlimit(resource.RLIMIT_NOFILE)
        limit = OPEN_FILES_LIMIT if hard_limit == resource.RLIM_INFINITY else min(hard_limit, OPEN_FILES_LIMIT)
        resource.setrlimit(resource.RLIMIT_NOFILE, (limit, limit))
        host["subject_open_files_limit"] = limit
    k6 = not smoke or args.require_k6 or args.k6_version or shutil.which(args.k6) is not None
    loadgen = load_metadata(args, host["boot_id"]) if k6 else None
    args.bin_dir = args.bin_dir.resolve()
    binary_hashes = {}
    for language in args.languages:
        binary = args.bin_dir / language
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError(f"Expected executable {binary}")
        with binary.open("rb") as file:
            digest = hashlib.sha256()
            for chunk in iter(lambda: file.read(1024 * 1024), b""):
                digest.update(chunk)
        binary_hashes[language] = {"path": str(binary), "sha256": digest.hexdigest()}
    release = args.bin_dir / "elixir-release"
    if "elixir" in binary_hashes and release.is_dir():
        digest = hashlib.sha256()
        for file in sorted(release.rglob("*")):
            if file.is_file() and file.name != "COOKIE":
                digest.update(compact([str(file.relative_to(release)),
                                       hashlib.sha256(file.read_bytes()).hexdigest()]).encode())
        binary_hashes["elixir"]["release_sha256"] = digest.hexdigest()

    records = fixture_records()
    trials = trial_order(args)
    out = args.out.resolve()
    out.mkdir(parents=True)
    fixture_path = out / "fixtures.json"
    fixture_path.write_text(compact(records) + "\n")
    if k6:
        shutil.copyfile(HERE / "load.js", out / "load.js")
    metadata = {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()}
    write_json(out / "run.json", {
        "schema": 1, "started_at_utc": timestamp(), "settings": metadata,
        "host": host, "load_generator": loadgen, "binaries": binary_hashes,
        "build_versions": read_if_present(args.bin_dir / "versions.txt"),
        "fixtures_sha256": hashlib.sha256(fixture_path.read_bytes()).hexdigest(),
        "kind": "smoke" if smoke else "localhost diagnostic" if args.localhost else
                "remote pilot" if args.blocks < BLOCKS else "remote",
        "trials": [asdict(trial) for trial in trials],
    })
    print(f"Artifacts {out}", flush=True)
    failed = 0
    attempted = 0
    for index, trial in enumerate(trials, 1):
        directory = out / f"{index:04}"
        directory.mkdir()
        started_at = timestamp()
        label = f"{trial.language} {trial.mode} {trial.cpus} CPU block {trial.block}"
        if trial.rate:
            label += f" {trial.rate}/s"
        attempted += 1
        try:
            result = run_trial(args, directory, trial, available[:trial.cpus], fixture_path, records, k6)
        except (ValueError, TypeError, KeyError, OSError, http.client.HTTPException, subprocess.SubprocessError) as error:
            failed += 1
            print(f"FAIL {label} ({directory.name}): {error}", flush=True)
            write_json(directory / "result.json", {
                "status": "failed", "started_at_utc": started_at,
                "finished_at_utc": timestamp(), "error": str(error),
            })
            continue
        write_json(directory / "result.json", {
            "status": "ok", "started_at_utc": started_at, "finished_at_utc": timestamp(), **result,
        })
        detail = ""
        if "subject" in result:
            data = result["subject"]
            detail = f" ops={data['ops']} checksum={data['checksum']}"
            if "rtt_ns" in data:
                detail += f" rtt_samples={len(data['rtt_ns'])}"
        if "http" in result:
            data = result["http"]
            detail = f" valid={data['validated']} rejected={data['rejected']} connection_close=yes"
            if "load" in result:
                load = result["load"]
                detail += f" k6_valid={load['success']}/{load['offered']}"
            else:
                detail += " k6=skipped (unavailable)"
        if trial.rate:
            load = result["load"]
            print(f"RECORDED {label} nominal={load['target_offered']} offered={load['offered']} "
                  f"valid={load['success']} errors={load['errors']} dropped={load['dropped']} "
                  f"interrupted={load['interrupted']} missing={load['missing']}", flush=True)
        else:
            print(f"PASS {label}{detail}", flush=True)

    write_json(out / "finished.json", {
        "finished_at_utc": timestamp(), "planned": len(trials), "attempted": attempted, "failed": failed,
    })
    outcome = "passed" if smoke else "recorded without runner errors"
    print(f"{args.command}: {attempted - failed}/{len(trials)} trials {outcome}", flush=True)
    return int(failed > 0)


def percentile(values, percent):
    ordered = sorted(values)
    return ordered[math.ceil(len(ordered) * percent / 100) - 1]


def resource_metrics(path, phase, tick_hz):
    with path.open() as file:
        points = [point for line in file if (point := json.loads(line))["phase"] == phase]
    metrics = {}
    for source, label in (("VmRSS_kib", "sampled RSS MiB"), ("VmHWM_kib", "process HWM MiB")):
        values = [point[source] for point in points if source in point]
        if values:
            metrics[label] = max(values) / 1024
    rss = [point["VmRSS_kib"] for point in points if "VmRSS_kib" in point]
    if len(rss) > 1:
        metrics["RSS change MiB"] = (rss[-1] - rss[0]) / 1024
    if len(points) > 1 and tick_hz:
        first, last = points[0], points[-1]
        ticks = last["user_ticks"] + last["system_ticks"] - first["user_ticks"] - first["system_ticks"]
        seconds = (last["monotonic_ns"] - first["monotonic_ns"]) / 1e9
        metrics["sampled CPU cores"] = ticks / tick_hz / seconds
    return metrics


def report_metrics(directory, trial, result, tick_hz):
    values = {}
    phase = trial["mode"]
    if "subject" in result:
        data = result["subject"]
        if phase == "recovery":
            values["recovery ms"] = data["elapsed_ns"] / 1e6
        else:
            values["ops/s"] = data["ops"] * 1e9 / data["elapsed_ns"]
        for percent in (50, 95, 99):
            if "rtt_ns" in data:
                values[f"RTT p{percent} ms"] = percentile(data["rtt_ns"], percent) / 1e6
    if "readiness" in result:
        ready = result["readiness"]
        values["startup upper bound ms"] = ready["ready_ns"] / 1e6
        values["startup probe window ms"] = (ready["ready_ns"] - ready["lower_ns"]) / 1e6
    if "load" in result:
        data = result["load"]
        for key, name in (("med", "p50"), ("p(95)", "p95"), ("p(99)", "p99"), ("max", "max")):
            if key in data["e2e_ms"]:
                values[f"HTTP {name} ms"] = data["e2e_ms"][key]
        values.update({
            "successful/s": data["success_per_s"], "SLO goodput/s": data["slo_goodput_per_s"],
            "error %": 100 * data["error_fraction"], "offered": data["offered"],
            "request errors": data["errors"], "dropped": data["dropped"],
            "interrupted": data["interrupted"], "missing": data["missing"],
        })
        phase = "measure"
    values.update(resource_metrics(directory / "process.jsonl", phase, tick_hz))
    return values


def report(paths):
    baseline = None
    for path in paths:
        metadata = json.loads((path / "run.json").read_text())
        if metadata["schema"] != 1:
            raise ValueError(f"Unsupported run schema in {path}")
        host = metadata["host"]
        load = metadata["load_generator"]
        settings = metadata["settings"]
        print(f"\n{path} ({metadata['kind']})")
        print(f"{settings['label']} | {metadata['started_at_utc']} | "
              f"{host['platform']['node']} {host['platform']['release']} {host['platform']['machine']}")
        print(f"CPU IDs: {host.get('available_cpus', 'unpinned')} | quota: {host.get('cpu_quota')} | "
              f"load: {host['load_average']} | seed: {settings.get('seed', 'n/a')}")
        if load:
            print(f"Load generator: {load.get('ssh_destination', 'localhost')} | {load['k6']}")
        if metadata["kind"] == "smoke":
            passed = sum(
                json.loads(file.read_text())["status"] == "ok" for file in path.glob("*/result.json")
            )
            print(f"Smoke correctness: {passed}/{len(metadata['trials'])} trials passed.")
            continue
        comparison = (
            metadata["trials"], metadata.get("fixtures_sha256"), metadata.get("build_versions"),
            *(settings[key] for key in ("warmup_ops", "warmup_seconds", "measure_seconds",
                                        "sustained_seconds", "k6_vus")),
        )
        if baseline is not None and comparison != baseline:
            print("Comparison warning: the trial matrix, fixture, build, or load settings differ.")
        baseline = comparison
        print(f"Warmup: {settings['warmup_seconds']}s HTTP / {settings['warmup_ops']} untimed ops | "
              f"measurement: {settings['measure_seconds']}s HTTP / {settings['sustained_seconds']}s sustained")
        print("Values are block medians [min, max]. HTTP latency uses valid replies. "
              f"SLO blocks require p99 <= {SLO_MS}ms and <{SLO_ERROR_FRACTION:.1%} failures "
              "including dropped or missing load.")
        groups = defaultdict(list)
        tested_rates = defaultdict(dict)
        for index, trial in enumerate(metadata["trials"], 1):
            key = tuple(trial[name] for name in ("mode", "cpus", "pairs", "iterations", "rate", "language"))
            directory = path / f"{index:04}"
            file = directory / "result.json"
            groups[key].append((directory, trial, json.loads(file.read_text()) if file.exists() else {}))
        results = [result for group in groups.values() for _, _, result in group]
        failed = sum(result.get("status") == "failed" for result in results)
        not_recorded = sum(not result for result in results)
        if failed or not_recorded:
            print(f"Incomplete results: {failed} failed and {not_recorded} unrecorded of {len(results)} trials.")
        for (mode, cpus, pairs, iters, rate, language), group in sorted(groups.items()):
            measured = defaultdict(list)
            passed = slo_blocks = 0
            for directory, trial, result in group:
                if result.get("status") != "ok":
                    continue
                passed += 1
                slo_blocks += int(result.get("load", {}).get("slo_met", False))
                for name, value in report_metrics(directory, trial, result, host.get("clock_ticks_per_second")).items():
                    measured[name].append(value)
            workload = f" {rate}/s" if rate else f" {iters} iterations" if iters else ""
            if pairs:
                workload += f" × {pairs} pairs"
            slo = f" | SLO {slo_blocks}/{passed} measured blocks" if rate and passed else ""
            if mode == "http":
                tested_rates[cpus, language][rate] = (
                    slo_blocks == len(group) if passed == len(group) else None
                )
            print(f"\n{mode} {cpus} CPU{workload} | {language} | {passed}/{len(group)} trials{slo}")
            for name, values in measured.items():
                print(f"  {name}: {statistics.median(values):.4g} [{min(values):.4g}, {max(values):.4g}]"
                      f"{'' if len(values) == passed else f' (n={len(values)})'}")
        print("\nHighest tested HTTP rate meeting the SLO in every block")
        for (cpus, language), rates in sorted(tested_rates.items()):
            passed = [rate for rate, verdict in rates.items() if verdict is True]
            result = f"{max(passed)} req/s" if passed else "none"
            if None in rates.values():
                result += " (some rates have unrecorded or failed trials)"
            print(f"  {cpus} CPU | {language} | {result}")
    return 0


def positive(text):
    value = int(text)
    if value <= 0:
        raise argparse.ArgumentTypeError("Expected a positive integer")
    return value


def nonnegative(text):
    value = int(text)
    if value < 0:
        raise argparse.ArgumentTypeError("Expected a nonnegative integer")
    return value


def private_address(text):
    try:
        address = ipaddress.ip_address(text)
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from error
    networks = ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")
    if not any(address in ipaddress.ip_network(network) for network in networks):
        raise argparse.ArgumentTypeError("Expected a private IPv4 address")
    return str(address)


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    smoke = commands.add_parser("smoke", help="verify the selected subjects without reporting timings")
    run = commands.add_parser("run", help="run shuffled Linux blocks for each configuration")
    renderer = commands.add_parser("report", help="print a read-only report for each specified run directory")
    renderer.add_argument("runs", type=Path, nargs="+", metavar="RUN_DIR")
    for command in (smoke, run):
        command.add_argument("--out", type=Path, required=True, help="new artifact directory")
        command.add_argument("--bin-dir", type=Path, required=True, help="directory with the selected executables")
        command.add_argument("--languages", nargs="+", choices=LANGUAGES, default=LANGUAGES,
                             help="subjects to run (default: all four)")
        command.add_argument("--label", default="unlabeled", help="build or host label recorded in the report")
        command.add_argument("--timeout", type=positive, default=120, help="per-process actor/CPU/recovery timeout in seconds")
        command.add_argument("--readiness-timeout", type=positive, default=15, help="server startup timeout in seconds")
        command.add_argument("--k6", default="k6", help="k6 binary on the load host, or on PATH")
        command.add_argument("--k6-version", help="require this exact version of k6")
    smoke.set_defaults(warmup_ops=3)
    smoke.add_argument("--require-k6", action="store_true", help="fail if local k6 is unavailable")
    run.add_argument("--blocks", type=positive, default=BLOCKS, help="repeat blocks (use 1 for a pilot)")
    run.add_argument("--seed", type=int, default=26084)
    run.add_argument("--pairs", type=positive, default=64, help="concurrent pair count tested alongside one pair")
    run.add_argument("--actor-iters", type=positive, default=10_000)
    run.add_argument("--cpu-iters", type=positive, default=200_000)
    run.add_argument("--warmup-ops", type=nonnegative, default=100)
    run.add_argument("--http-rates", type=positive, nargs="+", required=True, metavar="RPS",
                     help="shared HTTP rates in requests per second, including a sparse low rate")
    run.add_argument("--warmup-seconds", type=nonnegative, default=15)
    run.add_argument("--measure-seconds", type=positive, default=30)
    run.add_argument("--sustained-rate", type=positive, required=True,
                     help="shared sustained rate in requests per second on four CPUs")
    run.add_argument("--sustained-seconds", type=positive, default=300)
    run.add_argument("--k6-vus", type=positive, help="override preallocated VUs (default max(4, ceil(rate / 5)))")
    placement = run.add_mutually_exclusive_group(required=True)
    placement.add_argument("--load-host", metavar="SSH_DEST", help="SSH destination with k6 and GNU timeout")
    placement.add_argument("--localhost", action="store_true", help="explicitly run a localhost diagnostic")
    run.add_argument("--server-address", type=private_address, help="private SUT address reachable from the load host")
    run.add_argument("--port", type=positive, default=0, help="fixed TCP port (default: ephemeral per trial)")
    args = parser.parse_args()
    if args.command in ("run", "smoke") and len(set(args.languages)) != len(args.languages):
        parser.error("--languages must be distinct")
    if args.command == "run":
        if args.load_host and (args.load_host.startswith("-") or any(char.isspace() for char in args.load_host)):
            parser.error("--load-host must be an SSH destination without whitespace")
        if args.load_host and not args.server_address:
            parser.error("--load-host requires --server-address")
        if args.localhost and args.server_address:
            parser.error("--server-address requires --load-host")
        if args.port > 65535:
            parser.error("--port must be between 1 and 65535")
        if len(set(args.http_rates)) != len(args.http_rates):
            parser.error("--http-rates must be distinct")
        if args.pairs > 64 or max(args.actor_iters, args.cpu_iters, args.warmup_ops) > 100_000_000:
            parser.error("use at most 64 pairs and 100000000 iterations")
    return args


def interrupt(_signal, _frame):
    raise KeyboardInterrupt


def main():
    args = parse_args()
    signal.signal(signal.SIGTERM, interrupt)
    try:
        if args.command == "report":
            return report(args.runs)
        return run_suite(args)
    except KeyboardInterrupt:
        print("Benchmark interrupted. Partial artifacts remain in the run directory.", file=sys.stderr)
        return 130
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Benchmark error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
