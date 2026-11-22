#!/usr/bin/env python3
"""Compare bounded Jobs maintenance under compiled J1 traffic using prebuilt artifacts."""

import argparse
import importlib.util
import json
import os
import platform
import re
import shutil
import signal
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

# Reuse the J1 database, fixture, reconciliation and runner lifecycle helpers.
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("jobs_parity", Path(__file__).with_name("jobs-parity.py"))
j1 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(j1)

TRAFFIC = "parity_0000000000000001"
MAINT = "parity_0000000000000002"
BALLAST = "parity_0000000000000003"
INDEX = "mesh_jobs_ready"
ARMS = ("main-native", "candidate-native", "candidate-script")
SOURCE_SCOPE = ("Cargo.toml", "Cargo.lock", ".cargo", "compiler",
                ":(exclude,glob)compiler/**/tests/**")
LIMIT = 1.15
QUIET_LOAD_PER_CPU = 0.75
ADMIN_TIMEOUT_MS = 30_000

# Same cutoff, ordering and lock semantics as Jobs prune in both pinned sources.
PRUNE = """
WITH cutoff AS MATERIALIZED (SELECT 1::bigint AS at_ms), stale AS (
    SELECT id FROM mesh_jobs.jobs CROSS JOIN cutoff
    WHERE namespace = '{namespace}' AND finished_at_ms <= cutoff.at_ms AND queue = 'default'
    ORDER BY finished_at_ms, id LIMIT {size} FOR UPDATE OF jobs SKIP LOCKED
), removed AS (
    DELETE FROM mesh_jobs.jobs j USING stale
    WHERE j.namespace = '{namespace}' AND j.id = stale.id RETURNING j.id
) SELECT coalesce(json_agg(id ORDER BY id), '[]') FROM removed;
"""


def require(condition, message):
    if not condition:
        raise j1.QueueMismatch(message)


def git(*args):
    return j1.checked(["git", *args])


def source(ref):
    return git("rev-parse", "--verify", f"{ref}^{{commit}}")


def native_pair(directory, ref):
    files = {name: directory.resolve() / name for name in ("meshc", "libmesh_rt.a")}
    marker = directory / "source-sha"
    if marker.is_file() and marker.read_text().strip() != ref:
        raise j1.FixtureError(f"source-sha disagrees with the declared artifact ref in {directory}")
    return {"declared_source": ref, "source_marker_present": marker.is_file(),
            "files": {name: {"path": str(path), "sha256": j1.digest(path)} for name, path in files.items()}}


def verify_artifact_pin(pairs, manifest):
    try:
        expected = json.loads(manifest.read_text())["artifact_pairs"]
        for side, pair in pairs.items():
            pinned = expected[side]
            if (pair["declared_source"] != pinned["declared_source"]
                    or any(file["sha256"] != pinned["files"][name]["sha256"]
                           for name, file in pair["files"].items())):
                raise j1.FixtureError(f"{side} compiler/runtime pair differs from {manifest}")
    except (KeyError, TypeError, ValueError) as error:
        raise j1.FixtureError(f"invalid artifact pin manifest: {manifest}") from error


def compile_fixture(pair, scratch, evidence, env, target, build_profile):
    project = scratch / "project"
    shutil.copytree(j1.MESH_SOURCE, project)
    j1.checked(["cc", "-std=c11", "-O2", "-c", "worker_identity.c", "-o", "worker_identity.o"],
               cwd=project, env=env, log=evidence / f"{scratch.name}-cc.log")
    j1.checked(["ar", "rcs", "libworker_identity.a", "worker_identity.o"], cwd=project, env=env)
    manifest = (project / "mesh.toml.in").read_text().replace("@TARGET@", target)
    (project / "mesh.toml").write_text(manifest.replace("@SHA256@", j1.digest(project / "libworker_identity.a")))
    files = pair["files"]
    binary = scratch / "fixture"
    j1.checked([files["meshc"]["path"], "build", str(project), "--target", target, "--output", str(binary),
                "--opt-level", "2" if build_profile == "release" else "0"],
               cwd=project, env=dict(env, MESH_RT_LIB_PATH=files["libmesh_rt.a"]["path"]),
               timeout=180, log=evidence / f"{scratch.name}-compile.log")
    for artifact in files.values():
        if j1.digest(Path(artifact["path"])) != artifact["sha256"]:
            raise j1.FixtureError(f"native artifact changed while compiling: {artifact['path']}")
    pair["compiled_fixture_sha256"] = j1.digest(binary)
    return binary


def batches(count, seed, profile, label, hold_override=None):
    workload = dict(profile, minimum_jobs=count, jobs_per_slot=0)
    result, = j1.inputs(workload, 1, seed)
    result["request_id"] = label
    for job in result["jobs"]:
        job["key"] = f"{label}_{job['key']}"
        if hold_override is not None:
            job["hold_ms"] = hold_override
    return result


def job_ids(cluster, namespace):
    return json.loads(cluster.sql("mesh", f"""
        SELECT coalesce(json_object_agg(args->>'key', id::text), '{{}}')
        FROM mesh_jobs.jobs WHERE namespace = '{namespace}'
    """))


def receipts(cluster, namespace):
    observed = json.loads(cluster.sql("mesh", f"""
        SELECT coalesce(json_object_agg(request_key, receipt), '{{}}')
        FROM mesh_jobs.receipts WHERE namespace = '{namespace}'
    """))
    require(all(value["effect_key"] == f"{value['id']}/1" for value in observed.values()),
            "enqueue receipt effect key differs from its persisted job/run identity")
    return observed


def check_identities(rows, before, expected):
    require({row["job_key"]: row["id"] for row in rows} == before
            and set(before) == set(expected) and len(set(before.values())) == len(before),
            "persisted job identities differ from the enqueue manifest")


def verify_writes(cluster, rows, namespace, products):
    # Compare individual effect keys to the persisted job/run identity. Account
    # for extra writes across namespaces in the trial-wide check after both drain.
    sql = f"""
        SELECT json_build_object(
            'attempts', (SELECT coalesce(json_agg(json_build_array(engine, job_key, effect_key, attempt)), '[]')
                         FROM public.mesh_jobs_parity_attempts WHERE namespace='{namespace}'),
            'effects', (SELECT coalesce(json_agg(json_build_array(engine, job_key, effect_key)), '[]')
                        FROM public.mesh_jobs_parity_effects WHERE namespace='{namespace}'),
            'product', (SELECT coalesce(json_agg(json_build_array(engine, request_id)), '[]')
                        FROM public.mesh_jobs_parity_product WHERE namespace='{namespace}')
        )
    """
    writes = json.loads(cluster.sql("mesh", sql))
    keys = [["mesh", row["job_key"], f"{row['id']}/1"] for row in rows]
    require(sorted(writes["attempts"]) == sorted(key + [row["attempt"]] for key, row in zip(keys, rows, strict=True))
            and sorted(writes["effects"]) == sorted(keys)
            and sorted(writes["product"]) == sorted(["mesh", request] for request in products),
            "handler attempts, durable effects or product writes differ from expected identities")
    return {kind: len(values) for kind, values in writes.items()}


def prepare_maintenance(cluster, size):
    # Synthetic history uses compiled enqueues and real receipts. Expired rows
    # model a lease lost before handler entry. Distinct namespaces give the
    # probe sole control of the maintenance pages while J1 runs.
    cluster.sql("mesh", f"""
        UPDATE mesh_jobs.jobs SET state='completed', attempt=1, finished_at_ms=1
        WHERE namespace='{MAINT}' AND args->>'key' LIKE 'prune%';
        UPDATE mesh_jobs.jobs SET state='executing', attempt=1, attempted_at_ms=1,
            lease_token=gen_random_uuid(), lease_expires_at_ms=1
        WHERE namespace='{MAINT}' AND args->>'key' LIKE 'expired%';
        UPDATE mesh_jobs.jobs SET state='scheduled', scheduled_at_ms=1
        WHERE namespace='{MAINT}' AND args->>'key' LIKE 'due%';
        INSERT INTO mesh_jobs.queues(namespace,name,concurrency) VALUES ('{BALLAST}','default',0);
        INSERT INTO mesh_jobs.jobs(namespace, worker, version, queue, args, metadata, tags, state,
            priority, max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms, inserted_at_ms, scheduled_at_ms)
        SELECT '{BALLAST}', 'parity', 1, 'default', '{{}}', '{{}}', '[]', 'scheduled', 0,
            3, 10000, 1000, 1000, 1, 4102444800000 + ordinal
        FROM generate_series(1, {size}) AS rows(ordinal);
        ANALYZE mesh_jobs.jobs;
    """)


def index_state(cluster):
    return json.loads(cluster.sql("mesh", f"""
        SELECT json_build_object('oid', indexrelid::bigint, 'bytes', pg_relation_size(indexrelid),
                                 'valid', indisvalid, 'ready', indisready)
        FROM pg_index WHERE indexrelid='mesh_jobs.{INDEX}'::regclass
    """))


def load_sample():
    return {"unix_ms": time.time_ns() / 1e6, "averages": os.getloadavg()}


def admin(cluster, app, log, *, query=None, script=False):
    env = dict(cluster.env, PATH=f"{cluster.bin}{os.pathsep}{cluster.env['PATH']}",
               PGHOST="127.0.0.1", PGPORT=str(cluster.port), PGUSER="parity_owner",
               PGPASSWORD=cluster.password, PGDATABASE="mesh", PGSSLMODE="disable",
               PGAPPNAME=app, PGCONNECT_TIMEOUT="5",
               PGOPTIONS=f"-c log_min_duration_statement=0 -c statement_timeout={ADMIN_TIMEOUT_MS} -c lock_timeout=2000")
    command = (["bash", str(j1.ROOT / "scripts/jobs-reindex.sh"), INDEX, str(ADMIN_TIMEOUT_MS)] if script else
               [str(cluster.bin / "psql"), "-XqwAt", "-v", "ON_ERROR_STOP=1", "-c", query.strip()])
    started = time.monotonic_ns()
    result = j1.checked(command, env=env, timeout=45, log=log)
    return result, (time.monotonic_ns() - started) / 1e6


def statement_times(postgres_log, app, statement):
    # REINDEX's statement duration excludes setup done by the wrapper. Unix
    # epoch milliseconds in the PG log share the J1 commit/claim clock.
    pattern = re.compile(rf"^(\d+\.\d+) \[\d+\] {re.escape(app)} LOG:\s+"
                         rf"duration: ([\d.]+) ms\s+statement: {statement}", re.MULTILINE)
    matches = pattern.findall(postgres_log)
    if len(matches) != 1:
        raise j1.FixtureError(f"expected one recorded SQL completion for {app}, got {len(matches)}")
    ended, elapsed = map(float, matches[0])
    return {"start_unix_ms": ended * 1000 - elapsed, "end_unix_ms": ended * 1000, "statement_ms": elapsed}


def drain(cluster, namespace, count, process, timeout):
    def done():
        require(process.poll() is None, f"runner exited early for {namespace}")
        return cluster.observe("mesh", namespace)["terminal"] >= count
    j1.wait_for(done, time.monotonic() + timeout, f"{namespace} terminal rows")


def runner_report(log, completed):
    report = j1.mesh_shutdown_report(log)
    require(report["last_error"] is None and report["stale_acks"] == report["recovered"] == report["pruned"] == 0
            and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == completed,
            f"unexpected claim, maintenance or shutdown results in {log}")
    return report


def claim_stall(samples, commits, reindex):
    lower = reindex["start_unix_ms"]
    upper = reindex["end_unix_ms"]
    claims = sorted(row["claimed_ms"] for row in samples)
    # A committed first-wave job claimed after REINDEX proves eligible backlog
    # throughout the statement. Require an ordinary claim on either side so
    # the wait from the last claim through resumption is measured.
    if (commits["traffic_before"] > lower or claims[0] >= lower or claims[-1] <= upper
            or not any(row["job_key"].startswith("traffic_before") and row["claimed_ms"] > upper for row in samples)):
        raise j1.FixtureError("ordinary claims did not bracket reindex with a witnessed eligible backlog")
    return {
        "max_bracketing_claim_gap_ms": max(b - a for a, b in zip(claims, claims[1:]) if a < upper and b > lower),
        "claims_during_statement": sum(lower <= stamp <= upper for stamp in claims),
        "enqueue_commits_during_statement": sum(lower <= stamp <= upper for stamp in commits.values()),
    }


def run_trial(cluster, binary, paths, workload, args, settings, env, evidence, arm, prefix):
    cluster.prepare_database("mesh")
    require(cluster.owned("mesh"), "database ownership assertion failed")
    subject = j1.Subject("mesh", [str(binary)], binary.parent, cluster, env)
    traffic = subject.environment(TRAFFIC, args.slots, args.slots, "dense", settings["poll_ms"])
    maint = subject.environment(MAINT, args.slots, args.slots, "dense", settings["poll_ms"])
    to_prune, expired, due, before, during = (workload[key] for key in ("prune", "expired", "due", "traffic_before", "traffic_during"))
    timeout = settings["trial_timeout_seconds"]
    tolerance = settings["clock_tolerance_ms"]
    commits = {}
    enqueue_wall_ms = {}

    def enqueue(batch, environment, suffix=""):
        key = batch["request_id"]
        start = time.monotonic_ns()
        count, commit = subject.enqueue(environment, paths[key], evidence / f"enqueue-{key}{suffix}.log")
        require(count == len(batch["jobs"]) and commit is not None, f"invalid committed enqueue for {key}")
        enqueue_wall_ms[key + suffix] = (time.monotonic_ns() - start) / 1e6
        return commit

    subject.call("migrate", traffic, evidence / "migrate-traffic.log")
    subject.call("migrate", maint, evidence / "migrate-maintenance.log")
    seed_start = time.monotonic_ns()
    for batch in (to_prune, expired, due):
        commits[batch["request_id"]] = enqueue(batch, maint)
    prepare_maintenance(cluster, args.index_rows)
    seed_ms = (time.monotonic_ns() - seed_start) / 1e6
    seeded = job_ids(cluster, MAINT)
    initial_receipts = receipts(cluster, MAINT)
    require({key: value["id"] for key, value in initial_receipts.items()} == seeded,
            "seed receipt identity mismatch")
    prune_ids = {seeded[job["key"]] for job in to_prune["jobs"]}
    recovered_jobs = {job["key"]: job for batch in (expired, due) for job in batch["jobs"]}
    old_index = index_state(cluster)
    load = [load_sample()]
    runner_log = evidence / "runner-traffic.log"
    runner = subject.run(traffic, 0, runner_log)
    pages = []
    clean = False
    try:
        j1.wait_for(lambda: j1.runner_ready([runner], [runner_log], track=False),
                    time.monotonic() + 30, "compiled traffic runner")
        commits["traffic_before"] = enqueue(before, traffic)
        first_wave_ids = job_ids(cluster, TRAFFIC)
        j1.wait_for(lambda: cluster.observe("mesh", TRAFFIC)["executing"] > 0,
                    time.monotonic() + 30, "first ordinary claim", interval=0.02)
        load.append(load_sample())
        with ThreadPoolExecutor(max_workers=2) as pool:
            rebuild = pool.submit(admin, cluster, prefix + "_reindex", evidence / "reindex.log",
                                  query=f"REINDEX INDEX CONCURRENTLY mesh_jobs.{INDEX};",
                                  script=arm == "candidate-script")
            producer = pool.submit(enqueue, during, traffic)
            removed = []
            for page in range(args.pages):
                label = prefix + f"_prune_{page}"
                deleted, prune_wall = admin(cluster, label, evidence / f"prune-{page}.log",
                                            query=PRUNE.format(namespace=MAINT, size=args.page_size))
                ids = json.loads(deleted)
                require(len(ids) == args.page_size, "prune did not delete exactly one bounded page")
                removed.extend(ids)
                recovery_label = prefix + f"_recover_{page}"
                recovered, recovery_wall = admin(
                    cluster, recovery_label, evidence / f"recover-{page}.log",
                    query=f"SELECT mesh_jobs.recover('{MAINT}', {args.page_size}, NULL);")
                states = json.loads(cluster.sql("mesh", f"""
                    SELECT json_build_object(
                        'staged', count(*) FILTER (WHERE args->>'key' LIKE 'due%' AND state='available'),
                        'released', count(*) FILTER (WHERE args->>'key' LIKE 'expired%' AND state='retryable'
                            AND attempt=1 AND lease_token IS NULL AND lease_expires_at_ms IS NULL))
                    FROM mesh_jobs.jobs WHERE namespace='{MAINT}'
                """))
                total = (page + 1) * args.page_size
                require(int(recovered) == args.page_size and states == {"staged": total, "released": total},
                        "recovery did not expire and stage exactly one bounded page each")
                pages.append({"prune": {"application": label, "wall_ms": prune_wall, "ids": ids},
                              "recover": {"application": recovery_label, "wall_ms": recovery_wall,
                                          "expired": int(recovered), "staged": args.page_size}})
            _, reindex_wall = rebuild.result()
            commits["traffic_during"] = producer.result()
        load.append(load_sample())
        require(len(set(removed)) == len(removed) and set(removed) == prune_ids, "prune selected different identities")
        ready_ids = job_ids(cluster, MAINT)
        require(ready_ids == {key: seeded[key] for key in recovered_jobs}, "page work changed maintenance job identities")
        index = index_state(cluster)
        require(old_index["oid"] != index["oid"] and index["valid"] and index["ready"],
                "reindex did not replace the target with a valid and ready index")
        drain(cluster, TRAFFIC, len(before["jobs"]) + len(during["jobs"]), runner, timeout)
        clean = True
    finally:
        j1.stop_runners([runner], require_clean=clean)
    traffic_report = runner_report(runner_log, len(before["jobs"]) + len(during["jobs"]))
    load.append(load_sample())

    expected = {job["key"]: (batch["request_id"], job) for batch in (before, during) for job in batch["jobs"]}
    rows = cluster.rows("mesh", TRAFFIC)
    traffic_receipts = receipts(cluster, TRAFFIC)
    check_identities(rows, {key: value["id"] for key, value in traffic_receipts.items()}, expected)
    require({row["job_key"]: row["id"] for row in rows if row["job_key"].startswith("traffic_before")} == first_wave_ids,
            "first-wave job identities changed during maintenance")
    samples = j1.account(rows, expected, commits, tolerance)
    traffic_writes = verify_writes(cluster, rows, TRAFFIC, ("traffic_before", "traffic_during"))

    pg_log = (cluster.evidence / "postgres.log").read_text()
    reindex = {"wall_ms": reindex_wall, "before": old_index, "after": index,
               **statement_times(pg_log, prefix + "_reindex", r'REINDEX INDEX CONCURRENTLY mesh_jobs\."?mesh_jobs_ready"?;')}
    reindex.update(claim_stall(samples, commits, reindex))
    for page in pages:
        for op, query in (("prune", "WITH cutoff AS MATERIALIZED"), ("recover", r"SELECT mesh_jobs\.recover")):
            detail = page[op]
            detail.update(statement_times(pg_log, detail["application"], query))
    maintenance_start = min(reindex["start_unix_ms"], *(p[op]["start_unix_ms"] for p in pages for op in p))
    maintenance_end = max(reindex["end_unix_ms"], *(p[op]["end_unix_ms"] for p in pages for op in p))
    during_maintenance = [row for row in samples if row["commit_ms"] <= maintenance_end
                          and row["entered_ms"] >= maintenance_start]
    if not any(row["job_key"].startswith("traffic_before") and row["claimed_ms"] > maintenance_end for row in samples):
        raise j1.FixtureError("the ordinary backlog did not last through all maintenance pages")
    last_settlement = max(row["settled_ms"] for row in samples)

    enqueue(to_prune, maint, "-replay")
    require(job_ids(cluster, MAINT) == ready_ids and receipts(cluster, MAINT) == initial_receipts,
            "replaying pruned requests changed jobs or receipts")
    runner_log = evidence / "runner-recovered.log"
    runner = subject.run(maint, 0, runner_log)
    clean = False
    try:
        j1.wait_for(lambda: j1.runner_ready([runner], [runner_log], track=False),
                    time.monotonic() + 30, "compiled recovery runner")
        drain(cluster, MAINT, len(recovered_jobs), runner, timeout)
        clean = True
    finally:
        j1.stop_runners([runner], require_clean=clean)
    maintenance_report = runner_report(runner_log, len(recovered_jobs))
    recovered_rows = cluster.rows("mesh", MAINT)
    check_identities(recovered_rows, ready_ids, recovered_jobs)
    for row in recovered_rows:
        expected_attempt = 2 if row["job_key"].startswith("expired") else 1
        require(row["state"] == "completed" and row["attempt"] == row["first_handler_attempt"] == expected_attempt,
                "maintenance row did not settle at the expected attempt")
    maintenance_writes = verify_writes(cluster, recovered_rows, MAINT, ("prune", "expired", "due"))
    for kind, table in (("attempts", "attempts"), ("effects", "effects"), ("product", "product")):
        actual = int(cluster.sql("mesh", f"SELECT count(*) FROM public.mesh_jobs_parity_{table}"))
        require(actual == traffic_writes[kind] + maintenance_writes[kind], f"unexpected trial-wide {kind}")

    return {
        "arm": arm, "load": load, "enqueue_commits_unix_ms": commits, "enqueue_wall_ms": enqueue_wall_ms,
        "setup_wall_ms": seed_ms, "pages": pages, "reindex": reindex,
        "dispatch_p99_ms": j1.percentile([row["pickup_ms"] for row in samples], 0.99),
        "dispatch_samples_in_maintenance": len(during_maintenance),
        "maintenance_overlap_dispatch_p99_ms": j1.percentile([row["pickup_ms"] for row in during_maintenance], 0.99),
        "catch_up_after_last_enqueue_ms": j1.within_clock(
            last_settlement - max(commits[key] for key in ("traffic_before", "traffic_during")), tolerance),
        "catch_up_after_maintenance_ms": max(0, last_settlement - maintenance_end),
        "traffic_writes": traffic_writes, "maintenance_writes": maintenance_writes,
        "traffic": samples, "maintenance": recovered_rows, "receipts": initial_receipts,
        "runner_reports": {"ordinary": traffic_report, "maintenance": maintenance_report},
    }


def summarize(rounds, args, settings, cpus):
    rows = [trial for round_trials in rounds for trial in round_trials.values()]
    comparisons = []
    for trials in rounds:
        main, native, script = (trials[arm] for arm in ARMS)
        baseline = main["dispatch_p99_ms"]
        require(baseline > 0 and native["reindex"]["statement_ms"] > 0, "invalid comparison denominator")
        comparisons.append({
            "candidate_script_over_main_dispatch_p99": script["dispatch_p99_ms"] / baseline,
            "candidate_native_over_main_dispatch_p99": native["dispatch_p99_ms"] / baseline,
            "candidate_script_over_native_reindex_wall": script["reindex"]["wall_ms"] / native["reindex"]["wall_ms"],
            "candidate_script_over_native_reindex_statement":
                script["reindex"]["statement_ms"] / native["reindex"]["statement_ms"],
        })
    metrics = {}
    for arm in ARMS:
        trials = [round_trials[arm] for round_trials in rounds]
        series = {
            "dispatch_p99_ms": [trial["dispatch_p99_ms"] for trial in trials],
            "maintenance_overlap_dispatch_p99_ms": [trial["maintenance_overlap_dispatch_p99_ms"] for trial in trials],
            "catch_up_after_last_enqueue_ms": [trial["catch_up_after_last_enqueue_ms"] for trial in trials],
            "catch_up_after_maintenance_ms": [trial["catch_up_after_maintenance_ms"] for trial in trials],
            "reindex_wall_ms": [trial["reindex"]["wall_ms"] for trial in trials],
            "reindex_statement_ms": [trial["reindex"]["statement_ms"] for trial in trials],
            "prune_statement_ms": [page["prune"]["statement_ms"] for trial in trials for page in trial["pages"]],
            "recovery_and_staging_statement_ms": [page["recover"]["statement_ms"] for trial in trials for page in trial["pages"]],
        }
        metrics[arm] = {key: {"min": min(data), "median": statistics.median(data), "max": max(data)}
                        for key, data in series.items()}

    peak_load = max(sample["averages"][0] / cpus for row in rows for sample in row["load"])
    min_samples = min(row["dispatch_samples_in_maintenance"] for row in rows)
    reasons = []
    if not args.qualify:
        reasons.append("qualification was not requested")
    if args.build_profile != "release" or len(rounds) < 5:
        reasons.append("qualification requires release artifacts and at least five rounds")
    if peak_load > QUIET_LOAD_PER_CPU:
        reasons.append("observed one-minute load per logical CPU exceeded the noise limit")
    if min_samples < 100:
        reasons.append("at least one trial had fewer than 100 dispatch samples intersecting maintenance")
    poll_budget = 2 * settings["poll_ms"]
    latency_pass = all(row["candidate_script_over_main_dispatch_p99"] <= LIMIT for row in comparisons)
    stall_pass = all(row["reindex"]["max_bracketing_claim_gap_ms"] <= poll_budget
                     for row in rows if row["arm"] == "candidate-script")
    return {
        "verdict": "diagnostic" if reasons else "pass" if latency_pass and stall_pass else "fail",
        "diagnostic_reasons": reasons, "by_arm_ms": metrics, "paired_ratios": comparisons,
        "peak_one_minute_load_per_cpu": peak_load, "quiet_load_per_cpu_limit": QUIET_LOAD_PER_CPU,
        "dispatch_definition": "durable enqueue commit to first handler entry over both fixed J1 waves",
        "minimum_dispatch_samples": min_samples, "dispatch_p99_limit": LIMIT, "dispatch_p99_within_limit": latency_pass,
        "claim_gap_limit_ms": poll_budget, "claim_gap_within_limit": stall_pass,
    }


def record(args):
    settings = json.loads(j1.CONFIG.read_text())
    main, candidate, through = map(source, (args.main_ref, args.candidate_ref, args.candidate_through))
    if git("diff", "--name-only", candidate, through, "--", *SOURCE_SCOPE):
        raise j1.FixtureError("candidate compiler/runtime source scope differs between the declared refs")
    if git("diff", "--name-only", through, "--", *SOURCE_SCOPE):
        raise j1.FixtureError("checkout compiler/runtime source scope differs from the candidate through ref")
    excluded_changes = git("diff", "--name-status", candidate, through, "--", ":(glob)compiler/**/tests/**")
    pairs = {"main": native_pair(args.main_artifacts, main), "candidate": native_pair(args.candidate_artifacts, candidate)}
    if args.artifact_manifest:
        verify_artifact_pin(pairs, args.artifact_manifest)
    cpus = os.cpu_count()
    workload = {
        label: batches(count, settings["seed"] + index, settings["workloads"]["slow"], label, hold)
        for index, (label, count, hold) in enumerate((
            ("prune", args.page_size * args.pages, 0), ("expired", args.page_size * args.pages, 0),
            ("due", args.page_size * args.pages, 0), ("traffic_before", args.jobs, None),
            ("traffic_during", args.slots * 2, None),
        ))
    }
    evidence = args.output.resolve()
    root = j1.ROOT / "target" / "jobs-parity"
    if not evidence.is_relative_to(root.resolve()):
        raise j1.FixtureError(f"output must be inside {root}")
    evidence.mkdir(parents=True)
    j1.write_json(evidence / "inputs.json", workload)
    rust = j1.checked(["rustc", "-vV"])
    target = next(line.removeprefix("host: ") for line in rust.splitlines() if line.startswith("host: "))
    rounds = []
    with j1.workspace(evidence) as scratch:
        cluster = j1.Cluster(scratch, evidence, args.slots * 2 + 8)
        # Match J1's polling, one runner and scheduler count for all arms.
        env = {key: value for key, value in cluster.env.items()
               if not key.startswith(("MESH_", "JOBS_PARITY_"))}
        env.update(MESH_SCHEDULER_MIN_WORKERS=str(settings["runner_schedulers"]),
                   MESH_SCHEDULER_MAX_WORKERS=str(settings["runner_schedulers"]))
        binaries = {}
        for side, pair in pairs.items():
            build_dir = scratch / side
            build_dir.mkdir()
            binaries[side] = compile_fixture(pair, build_dir, evidence, env, target, args.build_profile)
        paths = dict(zip(workload, j1.batch_files(scratch, workload.values()), strict=True))
        j1.write_json(evidence / "manifest.json", {
            "command": sys.argv, "head": source("HEAD"), "candidate_scope_through": through,
            "rust_scope_pathspecs": SOURCE_SCOPE, "excluded_compiler_test_changes": excluded_changes.splitlines(),
            "artifact_pairs": pairs, "build_profile": args.build_profile,
            "rustc": rust, "host": platform.platform(), "logical_cpus": cpus,
            "postgres_bin": str(cluster.bin), "expected_server_version": args.pg_version,
            "workload": {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()},
            "input_sha256": j1.digest(evidence / "inputs.json"), "poll_ms": settings["poll_ms"],
            "runner_schedulers": settings["runner_schedulers"], "clock_tolerance_ms": settings["clock_tolerance_ms"],
            "harness_sha256": {str(path.relative_to(j1.ROOT)): j1.digest(path) for path in (
                Path(__file__).resolve(), Path(spec.origin), j1.CONFIG, j1.ROOT / "scripts/jobs-reindex.sh",
                *sorted(j1.MESH_SOURCE.iterdir()),
            ) if path.is_file()},
        })
        try:
            cluster.start()
            server = cluster.sql("postgres", "SHOW server_version")
            if server.split()[0] != args.pg_version:
                raise j1.FixtureError(f"expected PostgreSQL {args.pg_version}, observed {server}")
            cluster.sql("postgres", "ALTER SYSTEM SET log_line_prefix = '%n [%p] %a '")
            cluster.sql("postgres", "SELECT pg_reload_conf()")
            j1.wait_for(lambda: cluster.sql("postgres", "SHOW log_line_prefix").rstrip() == "%n [%p] %a",
                        time.monotonic() + 10, "PostgreSQL timing log format")
            j1.write_json(evidence / "postgres.json", {"server_version": server, "connection_cap": cluster.connection_cap,
                "settings": json.loads(cluster.sql("postgres", """
                    SELECT json_object_agg(name, setting || coalesce(' ' || unit, '')) FROM pg_settings
                    WHERE name IN ('fsync', 'synchronous_commit', 'track_commit_timestamp',
                        'shared_buffers', 'maintenance_work_mem', 'autovacuum', 'max_connections')
                """))})
            for index in range(args.pairs):
                # Rotate the three starting positions. Reverse every other block
                # of three so the same predecessor does not always warm an arm.
                order = ARMS if index // len(ARMS) % 2 == 0 else ARMS[::-1]
                offset = index % len(ARMS)
                trials = {}
                for arm in order[offset:] + order[:offset]:
                    path = evidence / f"round-{index + 1}-{arm}"
                    path.mkdir()
                    print(f"round {index + 1} {arm}", flush=True)
                    binary = binaries[arm.split("-")[0]]
                    trials[arm] = run_trial(cluster, binary, paths, workload, args, settings, env,
                                            path, arm, f"j6_{index}_{arm}")
                    j1.write_json(path / "raw.json", trials[arm])
                rounds.append(trials)
        finally:
            cluster.stop()
    summary = summarize(rounds, args, settings, cpus)
    j1.write_json(evidence / "summary.json", summary)
    print(json.dumps(summary, indent=2))
    return int(args.qualify and summary["verdict"] != "pass")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__,
        epilog="Set PG_BIN_DIR to a local PostgreSQL installation. The probe starts and removes its own cluster. "
               "Use a new output directory per invocation. Exit 1 means qualification did not pass and "
               "exit 2 means the trial could not be reconciled or completed.")
    parser.add_argument("--main-artifacts", type=Path, required=True, help="directory with pinned meshc and libmesh_rt.a")
    parser.add_argument("--candidate-artifacts", type=Path, required=True, help="directory with pinned meshc and libmesh_rt.a")
    parser.add_argument("--artifact-manifest", type=Path, help="require the source refs and artifact hashes in a prior manifest.json")
    parser.add_argument("--main-ref", default="7c1620563614a04a573aef806a6d019a2af4dfe9")
    parser.add_argument("--candidate-ref", required=True, help="source ref declared for the candidate artifacts")
    parser.add_argument("--candidate-through", default="HEAD", help="verify the compiler/runtime scope through this ref")
    parser.add_argument("--output", type=Path, required=True, help="new evidence directory under target/jobs-parity")
    parser.add_argument("--pg-version", default="17.11", help="exact local patch version, using PG_BIN_DIR")
    parser.add_argument("--build-profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--pairs", type=int, default=5, help="number of matched three-arm rounds")
    parser.add_argument("--jobs", type=int, default=128, help="ordinary jobs in the first J1 wave")
    parser.add_argument("--slots", type=int, default=4, help="runner and database-wide concurrency")
    parser.add_argument("--page-size", type=int, default=16)
    parser.add_argument("--pages", type=int, default=4, help="pages per maintenance operation")
    parser.add_argument("--index-rows", type=int, default=50000, help="additional, future-scheduled index rows")
    parser.add_argument("--qualify", action="store_true", help="require quiet, repeated release trials and pass the gates")
    args = parser.parse_args()
    if (min(args.pairs, args.slots, args.pages, args.index_rows) < 1 or args.slots > 64
            or not 1 <= args.page_size <= 1000 or args.jobs < 4 * args.slots):
        parser.error("use 1..64 slots, 1..1000 rows per page, at least four jobs per slot, and positive rounds, pages and index rows")
    if not j1.POSTGRES_VERSION.fullmatch(args.pg_version):
        parser.error("use a PostgreSQL major.patch version")

    def interrupted(signum, _frame):
        raise KeyboardInterrupt(f"received signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    try:
        return record(args)
    except (j1.FixtureError, j1.QueueMismatch, OSError, ValueError, subprocess.SubprocessError, KeyboardInterrupt) as error:
        print(f"{type(error).__name__}: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
