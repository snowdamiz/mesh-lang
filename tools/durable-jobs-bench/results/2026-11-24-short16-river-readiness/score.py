"""Reduce the exact PostgreSQL transaction clocks and pre-submit evidence."""

import json
import math
from statistics import median

INPUT = "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1"
CLOCKS = ("submit_jobs_per_s", "commit_jobs_per_s")
RIVER = "adapter/river/measured/"
EMPTY = dict(jobs=0, attempts=0, effects=0, products=0)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(archive, name):
    with archive.extractfile(name) as stream:
        return json.load(stream)


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def stage_times(ledger, score):
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    commit, submit = tx["committed_us"], tx["submitted_us"]
    require(len(jobs) == score["completed"] == 128 and score["failed"] == 0
            and ledger["input_sha256"] == INPUT, "incorrect short outcome")
    writes = {job[key] for job in jobs for key in ("attempt_xid", "effect_xid")}
    require(len(writes) == 256 and not writes.intersection(job["settled_xid"] for job in jobs),
            "application transaction identities are not independent")
    for job in jobs:
        require(job["attempt"] == job["attempts"] == job["effects"] == 1 and
                commit <= job["entered_us"] <= job["attempt_committed_us"]
                <= job["effect_committed_us"] <= job["settled_us"], "bad durable transaction order")
    result = {}
    for field, label in (("attempt_committed_us", "first_write"),
                         ("effect_committed_us", "second_write"), ("settled_us", "settlement")):
        stamps = sorted(job[field] for job in jobs)
        result[f"{label}_first16_ms"] = (stamps[15] - commit) / 1000
        result[f"{label}_tail112_ms"] = (stamps[-1] - stamps[15]) / 1000
    effect = max(job["effect_committed_us"] for job in jobs)
    terminal = max(job["settled_us"] for job in jobs)
    require((submit, commit, terminal) == tuple(score[field] for field in
            ("first_submit_us", "first_commit_us", "last_settlement_us")), "clock boundaries differ")
    result.update(settlement_after_last_write_ms=(terminal - effect) / 1000,
                  submit_to_settlement_ms=(terminal - submit) / 1000,
                  commit_to_settlement_ms=(terminal - commit) / 1000,
                  enqueue_ms=(commit - submit) / 1000)
    for key, start in zip(CLOCKS, (submit, commit), strict=True):
        require(math.isclose(score[key], len(jobs) * 1e6 / (terminal - start), rel_tol=1e-9),
                f"{key} differs from durable timestamps")
        result[key] = score[key]
    # Cohorts are defined by observed second-write commit order, with job key
    # as a deterministic tie break at PostgreSQL's microsecond resolution.
    ordered = sorted(jobs, key=lambda job: (job["effect_committed_us"], job["job_key"]))
    result["job_intervals_by_second_write_order_ms"] = {
        name: {label: distribution([(job[end] - job[start]) / 1000 for job in cohort])
               for start, end, label in (
                   ("entered_us", "attempt_committed_us", "entry_to_first_write"),
                   ("attempt_committed_us", "effect_committed_us", "first_to_second_write"),
                   ("effect_committed_us", "settled_us", "second_write_to_settlement"))}
        for name, cohort in (("first16", ordered[:16]), ("tail112", ordered[16:]))}
    return result


def connection_samples(archive, engine):
    if engine == "go":
        with archive.extractfile(RIVER + "samples.jsonl") as stream:
            samples = [json.loads(line) for line in stream]
        idle = [point for point in samples if point["idle"]]
        load = [point for point in samples if not point["idle"]]
        require(all(point["all_connections_tls"] for point in samples), "River sample was missing TLS")
    elif engine == "rust":
        samples = read(archive, "adapter/samples.json")
        idle, load = samples["idle"], samples["timed"]
        require(all(point["all_tls"] for point in idle + load), "Apalis sample was missing TLS")
    else:
        samples = read(archive, "adapter/raw.json")
        idle, load = samples["idle_samples"], samples["samples"]
    require(idle and load and all(point["terminal"] == 0 for point in idle)
            and load[-1]["terminal"] == 128, "samples did not bracket the workload")
    require(all(0 < point["connections"] <= 20 for point in idle + load), "physical cap exceeded")
    return dict(idle=[point["connections"] for point in idle],
                load=[point["connections"] for point in load],
                last_idle=idle[-1]["connections"], peak_during_load=max(point["connections"] for point in load))


def cpu_delta(after, before, clk_tck):
    checks = ("pid", "start_ticks")
    if before is not None:
        require(all(after[key] == before[key] for key in checks), "process identity changed")
    delta = {field: after[field] - (before[field] if before else 0)
             for field in ("user_ticks", "system_ticks")}
    require(min(delta.values()) >= 0 and after["threads"] > 0 and after["rss_bytes"] > 0,
            "invalid process resource counters")
    return dict(**delta, cpu_ms=sum(delta.values()) * 1000 / clk_tck,
                threads=after["threads"], rss_bytes=after["rss_bytes"])


def startup(archive, mode, score, clk_tck):
    evidence = read(archive, RIVER + "startup.json")
    producer = read(archive, "adapter/river/producer-startup.json")
    config = read(archive, "adapter/river/config.json")
    db = read(archive, "adapter/river/database.json")
    require(evidence["mode"] == mode and
            config["runner_pool_max"] == db["max_connections"] - 2 == 18 and
            db["database_connection_limit"] == 20 and config["producer_observer_pool_max"] == 1 and
            db["reserved_connections"] == db["superuser_reserved_connections"] == db["other_client_backends"] == 0 and
            db["user"] == "parity_subject" and config["poll_only"] is False and
            config["fetch_cooldown_ms"] == 5 and config["poll_ms"] == 250 and config["idle_ms"] == 3000 and
            config["profile_config"]["runners"] == 1 and config["profile_config"]["slots"] == 16,
            "River startup used a different cap, role or service configuration")
    points = evidence["points"]
    require([point["point"] for point in points] == ["before_runner", "ready", "idle_middle", "before_submit"],
            "missing pre-submit census")
    before, ready, middle, final = points
    stamps = [point["epoch_us"] for point in points]
    require(stamps == sorted(set(stamps)) and 0 <= score["first_submit_us"] - stamps[-1] <= 250_000 and
            final["elapsed_us"] - ready["elapsed_us"] >= 2_950_000,
            "startup samples did not bracket a three-second idle policy close to submit")
    observer = None
    identities = []
    listener_pids = []
    for point in points:
        backends = point["backends"]
        require(point["rows"] == EMPTY and 1 <= len(backends) <= 20 and
                all(backend["tls"] and not backend["superuser"] and backend["db_user"] == "parity_subject"
                    and backend["database"] == "river" for backend in backends),
                "missing TLS, wrong role, or application rows before submit")
        (current_observer,) = [row for row in backends if row["observer"]]
        require(current_observer["application_name"] == "river:producer_observer" and
                (observer is None or observer["pid"] == current_observer["pid"] and
                 observer["backend_start"] == current_observer["backend_start"]),
                "producer/observer backend changed")
        observer = current_observer
        listeners = [row["pid"] for row in backends if row["last_query_listen"]]
        listener_pids.append(listeners)
        ids = {(row["pid"], row["backend_start"]) for row in backends}
        require(len(ids) == len(backends) and {pid for pid, _ in ids} == {p["pid"] for p in point["postgres_backends"]},
                "invalid census or missing backend process counters")
        identities.append(ids)
    require(len(before["backends"]) == 1 and len(listener_pids[-1]) == 1 and
            listener_pids[-1] == listener_pids[-2], "detached listener was absent or changed near submit")
    if mode == "warm":
        fill = read(archive, RIVER + "pool-fill.json")
        require(fill["pool_max"] == 18 and len(fill["backend_pids"]) == len(set(fill["backend_pids"])) == 17 and
                0 <= fill["elapsed_us"] <= 10_000_000 and
                stamps[0] <= fill["started_us"] <= fill["finished_us"] <= stamps[1] and
                identities[1] == identities[2] == identities[3] and
                set(fill["backend_pids"]) == {pid for pid, _ in identities[3]} - {observer["pid"]},
                "warm fill did not retain all physical sessions including LISTEN")
    else:
        require(RIVER + "pool-fill.json" not in archive.getnames(), "cold control performed explicit fill")
    require(final["runner"]["pid"] == evidence["runner_pid"] and
            final["producer"]["pid"] == evidence["producer_pid"] == producer["after"]["pid"],
            "invalid service process identity")
    preexisting = {row["pid"]: row for row in before["postgres_backends"]}
    postgres = [cpu_delta(row, preexisting.get(row["pid"]), clk_tck) for row in final["postgres_backends"]]
    return dict(physical_at_submit=len(final["backends"]), pre_submit_elapsed_ms=final["elapsed_us"] / 1000,
                ready_elapsed_ms=ready["elapsed_us"] / 1000,
                pool_fill_ms=fill["elapsed_us"] / 1000 if mode == "warm" else None,
                producer_connection_elapsed_ms=producer["elapsed_us"] / 1000,
                producer_connection=cpu_delta(producer["after"], producer["before"], clk_tck),
                runner=cpu_delta(final["runner"], None, clk_tck),
                producer_after_schema=cpu_delta(final["producer"], before["producer"], clk_tck),
                postgres_client_cpu_ms=sum(item["cpu_ms"] for item in postgres),
                postgres_client_rss_bytes=sum(item["rss_bytes"] for item in postgres))
