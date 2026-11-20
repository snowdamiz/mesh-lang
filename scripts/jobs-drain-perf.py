#!/usr/bin/env python3
"""Compare J5 committed drains using existing native/debug toolchains and local PG.

Creates a private SCRAM cluster and fresh databases inside this checkout. Build
and seed outside the timed window. See receipt.json for timings and row proofs.
Exit 1 on a failed run or proof, 2 on a measured regression above 10%.
"""

import argparse
import contextlib
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import secrets
import shlex
import socket
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "scripts/fixtures/jobs-drain-perf"
BASELINE = "7c1620563614a04a573aef806a6d019a2af4dfe9"
SIDES = ("main", "candidate")
NAMESPACE = "drain_perf"
QUEUE = "proof"
BUDGET_RATIO = 1.10
MIN_PAIRS = 5

ROWS_SQL = """
    SELECT jsonb_agg(to_jsonb(observed) ORDER BY position)
    FROM (
        SELECT j.id, j.position, j.args, j.state, j.attempt, j.run, j.queue,
               j.worker, j.version, j.scheduled_at_ms, j.attempted_at_ms,
               j.finished_at_ms, j.errors, j.lease_token, j.lease_expires_at_ms,
               e.at_ms AS effect_at_ms, e.context
        FROM mesh_jobs.jobs j
        LEFT JOIN public.j5_effects e ON e.job_id = j.id
        WHERE j.namespace = 'drain_perf'
    ) observed
"""


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def checked(command, *, env=None, cwd=ROOT, log=None, timeout=30):
    result = subprocess.run(command, cwd=cwd, env=env, text=True, capture_output=True,
                            timeout=timeout)
    if log is not None:
        log.write_text(result.stdout + result.stderr)
    require(result.returncode == 0, f"{Path(command[0]).name} exited {result.returncode}: "
            f"{log or result.stderr.strip()}")
    return result.stdout.strip()


def git(*args):
    return checked(["git", *args])


def load():
    return {"utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "load_avg_1_5_15m": os.getloadavg()}


@contextlib.contextmanager
def postgres(bindir, scratch, evidence, env):
    data = scratch / "pgdata"
    password = secrets.token_hex(20)
    passfile = scratch / "init-password"
    passfile.touch(mode=0o600)
    passfile.write_text(password + "\n")
    try:
        checked([bindir / "initdb", "-D", data, "-U", "j5_owner",
                 "--no-locale", "--encoding=UTF8", "--auth-local=trust",
                 "--auth-host=scram-sha-256", f"--pwfile={passfile}"],
                env=env, log=evidence / "initdb.log")
    finally:
        passfile.unlink()
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    with (data / "postgresql.conf").open("a") as config:
        config.write(f"\nlisten_addresses = '127.0.0.1'\nport = {port}\n"
                     "unix_socket_directories = ''\nssl = off\ntimezone = 'UTC'\n")
    control = [bindir / "pg_ctl", "-D", data, "-w", "-t", "20"]

    def sql(database, query):
        return checked([bindir / "psql", "-XqwAt", "-v", "ON_ERROR_STOP=1", "-h", "127.0.0.1",
                        "-p", str(port), "-U", "j5_owner", "-d", database, "-c", query],
                       env=dict(env, PGPASSWORD=password, PGSSLMODE="disable"))

    try:
        checked([*control, "-l", str(evidence / "postgres.log"), "start"], env=env)
        sql("postgres", f"CREATE ROLE j5_fixture LOGIN PASSWORD '{password}'")
        yield sql, f"postgresql://j5_fixture:{password}@127.0.0.1:{port}"
    finally:
        if (data / "postmaster.pid").is_file():
            checked([*control, "-m", "fast", "stop"], env=env)


def artifact_hashes(directory, *, direct=False):
    names = ["meshc", "libmesh_rt.a"]
    if direct:
        names.append("libmesh_test_rt.a")
    return {name: sha256(directory / name) for name in names}


def build(side, artifacts, scratch, evidence, env):
    project = scratch / side
    project.mkdir()
    source = (FIXTURE / "common.mpl").read_text() + (FIXTURE / f"{side}-drain.mpl").read_text()
    (project / "main.mpl").write_text(source)
    binary = scratch / f"{side}-drain"
    checked([artifacts / "meshc", "build", project, "--output", binary, "--opt-level", "0"],
            env=dict(env, MESH_RT_LIB_PATH=str(artifacts / "libmesh_rt.a")),
            cwd=scratch, log=evidence / f"build-{side}.log", timeout=180)
    return binary


def verify_report(side, report, size):
    require(report["namespace"] == NAMESPACE and report["claimed"] == size
            and report["finished"] == size and report["recovered"] == 0,
            f"unexpected {side} counts: {report}")
    require(report["outcomes"].get("completed") == size
            and all(count == 0 for kind, count in report["outcomes"].items() if kind != "completed"),
            f"unexpected {side} outcomes: {report}")
    if side == "main":
        require(report["drained"] and report["stale_acks"] == 0 and report["last_error"] is None,
                f"unclean legacy drain: {report}")
    else:
        require(report["stop_reason"] == "idle" and report["remaining"] is False
                and report["budget_hit"] is False, f"unclean bounded drain: {report}")


def verify_rows(seed, receipts, before, after, due_count):
    require(len(receipts) == len(before) == len(after) == len(seed), "wrong row count")
    previous_finish = 0
    timings = []
    for index, (spec, receipt, initial, final) in enumerate(zip(seed, receipts, before, after)):
        due = index < due_count
        key = f"{receipt['id']}/1"
        require(receipt["effect_key"] == key, f"wrong enqueue effect key: {receipt}")
        expected = {
            "args": spec["args"], "scheduled_at_ms": spec["scheduled_at_ms"],
            "worker": "proof", "version": 1, "queue": QUEUE, "run": 1,
            "id": receipt["id"], "attempt": 0, "state": "available" if due else "scheduled",
            "errors": [], "lease_token": None, "lease_expires_at_ms": None,
            "attempted_at_ms": None, "finished_at_ms": None, "context": None, "effect_at_ms": None,
        }
        require(all(initial[key] == value for key, value in expected.items()),
                f"wrong pre-drain row for {spec['args']}: {initial}")
        if not due:
            require(final == initial, f"future row changed: {initial} -> {final}")
            continue
        claimed, effect, finished = (final[k] for k in
                                     ("attempted_at_ms", "effect_at_ms", "finished_at_ms"))
        require(all(type(stamp) is int for stamp in (claimed, effect, finished))
                and previous_finish <= claimed <= effect <= finished,
                f"bad claim/effect/settlement timestamps or overlapping jobs: {final}")
        previous_finish = finished
        context = {field: initial[field] for field in
                   ("id", "args", "worker", "version", "queue", "scheduled_at_ms")}
        expected.update(state="completed", attempt=1, attempted_at_ms=claimed,
                        finished_at_ms=finished, effect_at_ms=effect,
                        context=dict(context, effect_key=key, attempt=1, max_attempts=1,
                                     metadata={}, tags=[]))
        require(final == dict(expected, position=initial["position"]), f"wrong outcome: {final}")
        timings.append({"job_id": final["id"], "label": final["args"]["label"],
                        "claim_to_effect_ms": effect - claimed,
                        "effect_to_settlement_ms": finished - effect,
                        "claim_to_settlement_ms": finished - claimed})
    return timings


def trial(side, pair, seed, cutoff, size, sql, url, binary, env, evidence):
    db = f"j5_{side}_{pair}"
    sql("postgres", f"CREATE DATABASE {db} OWNER j5_fixture")
    sql(db, """SET ROLE j5_fixture;
        CREATE TABLE public.j5_effects (
          job_id uuid PRIMARY KEY, context jsonb NOT NULL,
          at_ms bigint NOT NULL DEFAULT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint
        )""")
    config = {"namespace": NAMESPACE, "queues": {QUEUE: 1},
              "workers": [{"worker": "proof", "version": 1}]}
    if side == "main":
        config["drain"] = True
    options = {"max_jobs": size + 1, "cutoff_ms": cutoff, "recursive": True, "on_error": "continue"}
    settings = dict(env, J5_DATABASE_URL=f"{url}/{db}?sslmode=disable", J5_SEED=json.dumps(seed),
                    J5_CONFIG=json.dumps(config), J5_OPTIONS=json.dumps(options))
    receipts = json.loads(checked([binary, "seed"], env=settings, log=evidence / f"{db}-seed.log"))
    before = json.loads(sql(db, ROWS_SQL))
    db_clock_before = int(sql(db, "SELECT mesh_jobs.now_ms()"))
    host_before = load()
    started = time.perf_counter_ns()
    result = json.loads(checked([binary, "drain"], env=settings,
                                log=evidence / f"{db}-drain.log", timeout=120))
    process_elapsed = time.perf_counter_ns() - started
    host_after = load()
    db_clock_after = int(sql(db, "SELECT mesh_jobs.now_ms()"))
    after = json.loads(sql(db, ROWS_SQL))
    effect_count = int(sql(db, "SELECT count(*) FROM public.j5_effects"))
    (evidence / f"{db}-rows.json").write_text(json.dumps({"before": before, "after": after}, indent=2)
                                             + "\n")
    require(seed[0]["scheduled_at_ms"] <= cutoff <= db_clock_before <= db_clock_after
            < seed[-1]["scheduled_at_ms"], f"{db}: clock did not preserve identical eligibility")
    require(0 < result["elapsed_ns"] <= process_elapsed, f"{db}: invalid timing interval")
    verify_report(side, result["report"], size)
    job_timings = verify_rows(seed, receipts, before, after, size)
    require(effect_count == size and all(
        db_clock_before <= row["attempted_at_ms"] <= row["finished_at_ms"] <= db_clock_after
        for row in after[:size]), f"{db}: unexpected effects or work outside the drain window")
    sql("postgres", f"DROP DATABASE {db}")
    ms = result["elapsed_ns"] / 1_000_000
    return {"pair": pair, "side": side, "warmup": pair == 0, "config": config,
            "options": options if side == "candidate" else None,
            "runner_report": result["report"], "drain_ms": ms, "process_elapsed_ns": process_elapsed,
            "drain_ms_per_job": ms / size, "jobs_per_second": 1000 * size / ms,
            "verified_effects": effect_count, "verified_untouched_future": len(seed) - size,
            "db_clock_before_ms": db_clock_before, "db_clock_after_ms": db_clock_after,
            "host_before": host_before, "host_after": host_after, "jobs": job_timings}


def spread(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}


def summarize(samples):
    groups = {side: [s for s in samples if s["side"] == side and not s["warmup"]] for side in SIDES}
    summary = {
        side: {metric: spread([sample[metric] for sample in group])
               for metric in ("drain_ms", "drain_ms_per_job", "jobs_per_second")}
        for side, group in groups.items()
    }
    for side, group in groups.items():
        for metric in ("claim_to_effect_ms", "effect_to_settlement_ms", "claim_to_settlement_ms"):
            summary[side][metric] = spread(
                [job[metric] for sample in group for job in sample["jobs"]])
    pairs = list(zip(groups["main"], groups["candidate"]))
    ratios = [candidate["drain_ms"] / main["drain_ms"] for main, candidate in pairs]
    observed = spread(ratios)
    status = "insufficient_pairs"
    if len(pairs) >= MIN_PAIRS:
        status = "pass" if observed["median"] <= BUDGET_RATIO else "fail"
    return {"sides": summary, "pairs": len(pairs), "paired_ratios_candidate_over_main": ratios,
            "paired_ratio_summary": observed, "pairs_over_budget": sum(r > BUDGET_RATIO for r in ratios),
            "budget": {"max_median_paired_ratio": BUDGET_RATIO, "status": status}}


def direct_probe(artifacts, scratch, evidence, env):
    project = scratch / "direct"
    project.mkdir()
    (project / "mesh.toml").write_text('[package]\nname = "j5-direct"\nversion = "0.1.0"\n')
    test = project / "direct.test.mpl"
    test.write_bytes((FIXTURE / test.name).read_bytes())
    before = load()
    output = checked([artifacts / "meshc", "test", project], cwd=scratch,
                     env=dict(env, MESH_TEST_RT_LIB_PATH=str(artifacts / "libmesh_test_rt.a")),
                     log=evidence / "direct.log", timeout=180)
    samples = [json.loads(line.removeprefix("J5_DIRECT ")) for line in output.splitlines()
               if line.startswith("J5_DIRECT ")]
    require(len(samples) == 7 and all(s["completed"] == 1000 and s["elapsed_ns"] > 0 for s in samples),
            "missing or invalid compiled direct-handler timings")
    return {"path": "candidate Test.perform_job; compiled Mesh test, no PostgreSQL or worker process",
            "host_before": before, "host_after": load(), "samples": samples,
            "ns_per_call": spread([sample["elapsed_ns"] / sample["completed"] for sample in samples])}


def run(args, receipt):
    evidence = args.output
    temp = ROOT / "target"
    temp.mkdir(exist_ok=True)
    # Match default runtime scheduler settings. Do not inherit a database,
    # runtime override, or worker-internal environment from another proof.
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("PG", "MESH_", "J5_"))}
    with tempfile.TemporaryDirectory(prefix="j5-drain-", dir=temp) as location:
        scratch = Path(location)
        env.update(TMPDIR=str(scratch))
        artifacts = {"main": args.main_artifacts, "candidate": args.candidate_artifacts}
        source = {"main": BASELINE, "candidate": git("rev-parse", args.candidate_source)}
        require((artifacts["main"] / "source-sha").read_text().strip() == BASELINE,
                "main artifact source-sha does not match the J5 pin")
        receipt["toolchains"] = {}
        for side in SIDES:
            digest = artifact_hashes(artifacts[side], direct=args.direct and side == "candidate")
            receipt["toolchains"][side] = {
                "directory": str(artifacts[side]), "source_sha": source[side],
                "source_evidence": "artifact source-sha" if side == "main" else "caller-supplied",
                "file_sha256": digest,
            }
        binaries = {side: build(side, artifacts[side], scratch, evidence, env) for side in SIDES}
        for side, binary in binaries.items():
            receipt["toolchains"][side]["fixture_binary_sha256"] = sha256(binary)

        with postgres(args.pg_bin, scratch, evidence, env) as (sql, url):
            receipt["postgres"] = dict(
                line.split("=", 1) for line in sql("postgres", """
                    SELECT name || '=' || setting FROM pg_settings WHERE name IN
                       ('server_version', 'server_version_num', 'fsync', 'synchronous_commit',
                        'autovacuum', 'shared_buffers', 'ssl', 'wal_level') ORDER BY name
                """).splitlines()
            )
            require(receipt["postgres"]["fsync"] == receipt["postgres"]["synchronous_commit"] == "on",
                    "committed drain comparison requires fsync and synchronous_commit enabled")
            cutoff = int(sql("postgres", (
                "SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint"
            )))
            seed = [
                {"worker": "proof", "queue": QUEUE, "max_attempts": 1,
                 "args": {"label": f"job-{i:03d}"},
                 "scheduled_at_ms": cutoff + (-60_000 if i < args.jobs else 3_600_000)}
                for i in range(args.jobs + args.future)
            ]
            receipt.update(cutoff_ms=cutoff, seed=seed)
            for pair in range(args.pairs + 1):
                for side in SIDES:
                    sample = trial(side, pair, seed, cutoff, args.jobs, sql, url,
                                   binaries[side], env, evidence)
                    receipt["trials"].append(sample)
                    print(f"pair {pair} {side}: {sample['drain_ms']:.2f} ms", flush=True)
        receipt["summary"] = summarize(receipt["trials"])
        if args.direct:
            receipt["direct"] = direct_probe(artifacts["candidate"], scratch, evidence, env)
        for side in SIDES:
            require(receipt["toolchains"][side]["file_sha256"]
                    == artifact_hashes(artifacts[side], direct=args.direct and side == "candidate"),
                    f"{side} toolchain artifacts changed during the comparison")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for side in SIDES:
        parser.add_argument(f"--{side}-artifacts", required=True, type=lambda s: Path(s).resolve())
    parser.add_argument("--candidate-source", required=True, help="source commit of the supplied pair")
    parser.add_argument("--pg-bin", required=True, type=lambda s: Path(s).resolve())
    parser.add_argument("--output", required=True, type=lambda s: Path(s).resolve(),
                        help="new receipt directory inside this checkout")
    parser.add_argument("--pairs", type=int, default=7, help="measured pairs after one warmup pair")
    parser.add_argument("--jobs", type=int, default=24, help="due jobs per trial")
    parser.add_argument("--future", type=int, default=4, help="future jobs per trial")
    parser.add_argument("--direct", action="store_true", help="run the candidate's compiled test helper")
    args = parser.parse_args()
    if not args.output.is_relative_to(ROOT) or args.output.exists():
        parser.error("--output must name a new directory inside this checkout")
    if args.pairs < 1 or args.jobs < 1 or args.future < 1 or args.jobs + args.future > 128:
        parser.error("require at least one pair and one job of each kind, up to 128 total jobs")
    args.output.mkdir(parents=True)

    receipt = {
        "command": shlex.join(["python3", *sys.argv]), "checkout_head": git("rev-parse", "HEAD"),
        "inputs_sha256": {str(path.relative_to(ROOT)): sha256(path)
                          for path in [Path(__file__).resolve(), *sorted(FIXTURE.glob("*.mpl"))]},
        "host": {"platform": platform.platform(), "logical_cpus": os.cpu_count(), "start": load()},
        "scope": {
            "build": "prebuilt native debug toolchains; fixture opt level 0",
            "timed_region": "start + join or Jobs.drain, with seed and pool open outside",
            "cutoff": "fixed on candidate; main uses database now, bracketed for equal eligibility",
            "workload": "one isolated worker per claim writes an effect over a fresh SCRAM connection",
            "settings": "one Jobs slot, one pool slot, default runtime, telemetry and batching off",
        },
        "trials": [],
    }
    status = 0
    try:
        run(args, receipt)
        if receipt["summary"]["budget"]["status"] == "fail":
            status = 2
    except (OSError, RuntimeError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        receipt["error"] = str(error)
        print(error, file=sys.stderr)
        status = 1
    finally:
        receipt["host"]["end"] = load()
        receipt["exit_status"] = status
        path = args.output / "receipt.json"
        path.write_text(json.dumps(receipt, indent=2) + "\n")
        print(path)
    return status


if __name__ == "__main__":
    sys.exit(main())
