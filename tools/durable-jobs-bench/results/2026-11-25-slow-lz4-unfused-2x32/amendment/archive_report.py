"""Verify retained releases and derive paired timing from the exported raw archives."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
from statistics import median
import tarfile

HERE = Path(__file__).resolve().parent
QUEUE = HERE.parents[4] / "target/linode-jobs-stepchange/requests"
INVENTORY = HERE / "inventory.json"
ENGINES = ("bulk", "river", "oban", "apalis")
CLOCKS = ("submit_us", "drain_us", "enqueue_us")
PROGRESS = ("commit_to_rank64_us", "rank64_to_rank448_us", "rank448_to_last_effect_us",
            "last_effect_to_terminal_us", "rank64_to_last_first_write_us")
NOISE = ("terminal_to_worker_exit_us", "terminal_to_last_sample_us", "recorder_wall_ns",
         "submit_window_steal_pct", "recorder_steal_pct", "exit_sample_steal_pct", "submit_window_iowait_pct")
COLD = ("postgres_start_to_ready_ns", "runner_dispatch_to_ready_observation_ns",
        "ready_to_submit_wall_us", "adapter_wall_ns")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_bytes())


def member(archive, name):
    entry = archive.getmember(name)
    require(entry.isfile(), f"{name} must be a regular archived file")
    return archive.extractfile(entry).read()


def get(archive, name):
    return json.loads(member(archive, name))


def share(points, cpus, field):
    counters = [(points[0]["cpu"][f"cpu{cpu}"], points[-1]["cpu"][f"cpu{cpu}"]) for cpu in cpus]
    changes = [[b - a for a, b in zip(before, after, strict=True)] for before, after in counters]
    require(min(min(line) for line in changes) >= 0, "CPU counters moved backward")
    total = sum(map(sum, changes))
    return 100 * sum(row[field] for row in changes) / total if total else None


def duration_metrics(archive, host, submitted, committed, terminal, exit_us, shape):
    points = [json.loads(line) for line in member(archive, "host-samples.jsonl").splitlines()]
    require(all(a["epoch_us"] <= b["epoch_us"] and a["monotonic_ns"] < b["monotonic_ns"]
                for a, b in zip(points, points[1:])) and not any(row["builders"] for row in points),
            "recorder clock or builder overlap differs")
    lo = max(i for i, point in enumerate(points) if point["epoch_us"] <= submitted)
    hi = next(i for i, point in enumerate(points) if point["epoch_us"] >= terminal)
    bracket = points[lo:hi + 1]
    require(hi > lo and all(b["monotonic_ns"] - a["monotonic_ns"] <= 1_000_000_000
                           for a, b in zip(bracket, bracket[1:])), "missed durable clock bracket")
    cpus = host["affinity"]
    steal = share(bracket, cpus, 7)
    require(steal is not None and steal <= 1 and terminal <= exit_us, "submit-window steal or worker exit differs")
    row = dict(terminal_to_worker_exit_us=exit_us-terminal,
               terminal_to_last_sample_us=points[-1]["epoch_us"]-terminal,
               recorder_wall_ns=points[-1]["monotonic_ns"]-points[0]["monotonic_ns"],
               submit_window_steal_pct=steal, recorder_steal_pct=share(points, cpus, 7),
               exit_sample_steal_pct=share(points[hi:], cpus, 7),
               submit_window_iowait_pct=share(bracket, cpus, 4))
    if shape == "two":
        stamps = get(archive, "adapter/startup-clock.json")
        ready = stamps.get("verified_tls_roster", stamps["runners_ready"])
        start, dispatch, runners, close = (stamps[k]["monotonic_ns"] for k in
                                          ("adapter_start", "runner_launch_dispatch", "runners_ready", "adapter_end"))
        pg_start, pg_ready = (stamps[k]["monotonic_ns"] for k in ("postgres_start", "postgres_ready"))
        require(start <= pg_start <= pg_ready <= dispatch <= runners <= ready["monotonic_ns"] < close
                and ready["epoch_ns"] // 1000 <= submitted <= committed <= terminal
                <= stamps["adapter_end"]["epoch_ns"] // 1000, "startup or shutdown clock order changed")
        row.update(postgres_start_to_ready_ns=pg_ready-pg_start, runner_dispatch_to_ready_observation_ns=runners-dispatch,
                   ready_to_submit_wall_us=submitted-ready["epoch_ns"]//1000, adapter_wall_ns=close-start,
                   adapter_close_after_terminal_wall_us=stamps["adapter_end"]["epoch_ns"]//1000-terminal)
    return row


def archive_score(archive, inventory, lane, label, attempt, status, raw_sha, verified):
    shape, _, policy = label.split("-")
    engine = lane["engine"]
    host = get(archive, "host.json")
    spec = lane["inventory"]["engines"][engine]
    require(host["affinity"] == inventory["cpus"] and host["verified_files"] == spec["verified_files"]
            and [host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == inventory["host"],
            "host or verified files differ")
    pg = get(archive, "adapter/codec-postgres.json")
    settings = pg["settings"]
    postgres = inventory["postgres"]
    cap = 71 if shape == "two" else 68
    require(pg.get("path", pg.get("postgres")) == postgres["prefix"] + "/bin/postgres"
            and pg.get("sha256", pg.get("postgres_sha256")) == postgres["installed"]["bin/postgres"]
            and pg.get("server_version", settings.get("server_version_num")) == "170011"
            and settings["default_toast_compression"] == policy and int(settings["max_connections"]) == cap
            and settings["shared_preload_libraries"] == "pg_stat_statements"
            and all(settings[key] == "on" for key in
                    ("ssl", "fsync", "synchronous_commit", "full_page_writes", "track_commit_timestamp")),
            "PG17.11 executable, codec, TLS or durability settings differ")
    ledger = get(archive, "adapter/ledger.json")
    txn, = ledger["transactions"]
    jobs = ledger["jobs"]
    require(ledger["engine"] == engine and ledger["profile"] == "slow" and ledger["runners"] == (2 if shape == "two" else 1)
            and ledger["slots"] == 64 and ledger["artifact_sha256"] == spec["artifact_sha256"]
            and len(jobs) == len({job["job_key"] for job in jobs}) == 512,
            "ledger identity, roster or fixture size differs")
    submitted, committed = txn["submitted_us"], txn["committed_us"]
    require(submitted <= committed, "enqueue clock order differs")
    for job in jobs:
        require(job["hold_ms"] == 100 and job["attempts"] == job["effects"] == 1 and job["state"] == "completed"
                and committed <= job["attempt_committed_us"] <= job["effect_committed_us"] <= job["settled_us"],
                "job hold, writes or durable timestamps differ")

    storage = get(archive, "adapter/payload-storage.json")
    persisted = storage["persisted"]["rows"]
    column = storage["persisted"]["metadata"]
    codecs = Counter(row["compression"] for row in persisted)
    expected = {job["job_key"]: job["payload_sha256"] for job in jobs}
    require(len(persisted) == 512 and {row["job_key"]: row["payload_sha256"] for row in persisted} == expected
            and all(row["payload_bytes"] == 16384 for row in persisted)
            and column["type"] == ("bytea" if engine == "rust" else "jsonb")
            and codecs == {"lz4" if policy == "lz4" else None: 512},
            "stored row, bytes, or strict compression witness differs")
    security = storage["security"]
    require(security["observer_tls"] is True and security["database_limit"] == cap and security["codec_overrides"] == 0,
            "stored row observer TLS, database cap or role policy differs")

    first_writes = sorted(job["attempt_committed_us"] for job in jobs)
    rank64, rank448, last_first = (first_writes[rank - 1] for rank in (64, 448, 512))
    last_effect = max(job["effect_committed_us"] for job in jobs)
    terminal = max(job["settled_us"] for job in jobs)
    row = dict(attempt_id=attempt, raw_sha256=raw_sha, submit_us=terminal-submitted, drain_us=terminal-committed,
               enqueue_us=committed-submitted, commit_to_rank64_us=rank64-committed,
               rank64_to_rank448_us=rank448-rank64, rank448_to_last_effect_us=last_effect-rank448,
               last_effect_to_terminal_us=terminal-last_effect, rank64_to_last_first_write_us=last_first-rank64,
               compression_counts={"lz4" if policy == "lz4" else "uncompressed": 512},
               stored_column_bytes=sum(item["stored_column_bytes"] for item in persisted),
               total_relation_bytes=storage["persisted"]["total_bytes"])
    if rank448 > rank64:
        overlap = sum(max(0, min(job["effect_committed_us"], rank448) - max(job["attempt_committed_us"], rank64))
                      for job in jobs)
        row["mean_committed_holds_rank64_to_rank448"] = overlap / (rank448 - rank64)
    row.update(duration_metrics(archive, host, submitted, committed, terminal, status["finished_us"], shape))

    if verified is not None:
        require(verified["raw_sha256"] == raw_sha and verified["attempt_id"] == attempt
                and round(verified["submit_ms"] * 1000) == row["submit_us"]
                and round(verified["drain_ms"] * 1000) == row["drain_us"]
                and round(verified["commit_to_64th_first_ms"] * 1000) == row["commit_to_rank64_us"]
                and round(verified["first64_to_last_effect_ms"] * 1000) == last_effect-rank64
                and round(verified["last_effect_to_terminal_ms"] * 1000) == row["last_effect_to_terminal_us"],
                "independent durable arithmetic differs from controller replay")
    if engine == "mesh":
        writer = storage.get("writer", [])
        row["producer_sql_exec_ms"] = sum(line["total_exec_time"] for line in writer)
    if engine == "go":
        samples = [json.loads(line) for line in member(archive, "adapter/river/measured/samples.jsonl").splitlines()]
        idle = [sample["connections"] for sample in samples if sample["idle"]]
        require(idle == ([7, 7] if shape == "two" else [4, 4]), "River stock idle census differs")
        row["river_idle_tls_clients"] = idle
        row["river_peak_sample_clients"] = max(sample["connections"] for sample in samples if not sample["idle"])
    return row


def archived_release(packet_path, folder, inventory_path, consumed, *, prior=False):
    packet = read(packet_path)
    inv = read(inventory_path)
    release_path = folder / "controller-release.json"
    release_sha = digest(release_path)
    release = read(release_path)
    canonical = QUEUE / (packet["id"] + ".released.json")
    require(release["id"] == packet["id"] and release["request_sha256"] == digest(packet_path)
            and release["request_sha256"] == read(folder / "grant.json")["packet_sha256"]
            and digest(inventory_path) == packet["inventory_sha256"]
            and (not canonical.exists() or canonical.read_bytes() == release_path.read_bytes()),
            f"{packet['id']}: release, inventory or grant hash differs")
    raw = {p.relative_to(folder).as_posix(): digest(p) for p in sorted(folder.rglob("raw.tar.gz"))}
    require(raw == release["raw_sha256"] and (not (folder / "raw-sha256.json").exists()
            or raw == read(folder / "raw-sha256.json")), "released archive file set differs from raw manifest")
    scores = folder / ("verified-replay.json" if prior else "replay.json")
    replay = read(scores) if scores.is_file() else {}
    if release["exit_status"] == 0:
        independent = read(folder / "verified-replay.json")
        if "fresh_equal_topology_scores" in independent:
            require(independent["fresh_release_sha256"] == release_sha
                    and independent["fresh_equal_topology_scores"] == replay["0"]["scores"],
                    "composite proof replay differs from its release")
        else:
            require(replay == independent, "saved controller and release replays differ")
    groups = {}
    for group in packet["groups"]:
        planned = [(phase, label) for phase in packet["phases"] for label in packet["orders"][str(group)]]
        recorded = folder / f"{group:02d}/order.json"
        order_matches = recorded.is_file() and read(recorded) == [list(cell) for cell in planned]
        group_result = replay.get(str(group), {})
        accepted, gaps, unscored = {}, {}, {}
        for phase, label in planned:
            lane = inv["lanes"][label]
            key = f"{phase}-{label}"
            arm_path = folder / f"{group:02d}" / key / f"{phase}-{lane['engine']}"
            raw_path = arm_path / "raw.tar.gz"
            rel = raw_path.relative_to(folder).as_posix()
            if rel not in raw:
                gaps[key] = {"reason": "no sealed archive"}
                if (arm_path / "dispatch.json").is_file():
                    gaps[key]["dispatch"] = read(arm_path / "dispatch.json")
                continue
            with tarfile.open(raw_path, "r:gz") as archive:
                status = get(archive, "status.json")
                invocation = get(archive, "invocation.json")
                attempt = invocation["attempt"]
                require(attempt not in consumed, "reused native attempt")
                consumed.add(attempt)
                request = read(arm_path / "request.json")
                require(all(invocation[k] == request[k] == v for k, v in
                            dict(engine=lane["engine"], phase=phase, profile="slow", attempt=attempt).items()),
                        f"{key}: effective invocation differs")
                verified = group_result.get("scores", {}).get(key)
                if status["exit_code"] != 0 or verified is None:
                    gaps[key] = dict(status=status, raw_sha256=raw[rel], attempt_id=attempt,
                                     reason=group_result.get("rejected", {}).get(key, "no eligible controller score"))
                    if status["exit_code"] == 0:
                        try:
                            unscored[key] = archive_score(archive, inv, lane, label, attempt, status, raw[rel], None)
                        except (KeyError, TypeError, ValueError, StopIteration) as error:
                            gaps[key]["archive_arithmetic_error"] = str(error)
                    continue
                accepted[key] = archive_score(archive, inv, lane, label, attempt, status, raw[rel], verified)
        eligible = order_matches and group_result.get("eligible") is True and not gaps
        if release["exit_status"] == 0:
            require(eligible and len(accepted) == len(planned), f"successful release has incomplete group {group}")
        groups[str(group)] = dict(order_matches=order_matches, eligible=eligible, scores=accepted, gaps=gaps,
                                  unscored_archive_arithmetic=unscored)
    require(len(raw) <= packet["arms"] and (release["exit_status"] != 0 or len(raw) == packet["arms"]),
            "released arm count differs from the request")
    return dict(id=packet["id"], release_sha256=release_sha, release_status=release["exit_status"],
                released_at=release["released_at"], planned_arms=packet["arms"], raw_arms=len(raw), groups=groups)


def distribution(rows):
    return {metric: dict(n=len(values), median=median(values), minimum=min(values), maximum=max(values),
                         negative=sum(value < 0 for value in values), positive=sum(value > 0 for value in values))
            for metric in rows[0]
            if (values := [row[metric] for row in rows if row[metric] is not None])}


def pairings(releases):
    order_results = {}
    measured_by_lane = {}
    for release in releases:
        for group, entry in release["groups"].items():
            if not entry["eligible"]:
                continue
            measured = {key.removeprefix("measured-"): row for key, row in entry["scores"].items()
                        if key.startswith("measured-")}
            if not measured:
                continue
            for label, row in measured.items():
                measured_by_lane.setdefault(label, []).append(row)

            def diff(left, right, metrics=(*CLOCKS, *PROGRESS)):
                return {k: measured[left][k] - measured[right][k] for k in metrics}

            result = {}
            for name in ENGINES:
                result[f"{name}-pglz-minus-lz4"] = diff(f"two-{name}-pglz", f"two-{name}-lz4")
                result[f"{name}-one-minus-two-pglz"] = diff(f"one-{name}-pglz", f"two-{name}-pglz")
                if name != "bulk":
                    for policy in ("pglz", "lz4"):
                        result[f"mesh-minus-{name}-{policy}"] = diff(f"two-bulk-{policy}", f"two-{name}-{policy}")
                    left, right = (result[f"mesh-minus-{name}-{policy}"] for policy in ("lz4", "pglz"))
                    result[f"lz4-minus-pglz-change-in-mesh-margin-vs-{name}"] = {k: left[k]-right[k] for k in left}
            order_results[group] = result

    def selected_metrics(row):
        return {key: row[key] for key in (*CLOCKS, *PROGRESS, *NOISE, *COLD, "mean_committed_holds_rank64_to_rank448")
                if key in row}

    ordered = list(order_results.values())
    return dict(per_order=order_results,
                paired_distributions={key: distribution([result[key] for result in ordered])
                                      for key in ordered[0]} if ordered else {},
                measured_by_lane={lane: distribution([selected_metrics(row) for row in rows])
                                  for lane, rows in measured_by_lane.items()})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("through", choices=("proof", "pilot", "full"))
    args = parser.parse_args()
    consumed = set()
    original = archived_release(HERE.parent / "request-proof.json", HERE.parent / "evidence/proof",
                                HERE.parent / "inventory.json", consumed, prior=True)
    proof = archived_release(HERE / "request-proof.json", HERE / "evidence/proof", INVENTORY, consumed)
    custody = read(HERE / "custody.json")
    reused = original["groups"]["0"]["scores"]
    require(original["release_status"] == 1 and proof["release_status"] == 0
            and original["release_sha256"] == custody["release_sha256"]
            and {key.removeprefix("diagnostic-"): row["raw_sha256"] for key, row in reused.items()} == custody["bridges"],
            "original four-cell bridge binding differs")
    timing = []
    if args.through != "proof":
        timing.append(archived_release(HERE / "request-pilot.json", HERE / "evidence/pilot", INVENTORY, consumed))
    if args.through == "full":
        timing.append(archived_release(HERE / "request-full.json", HERE / "evidence/full", INVENTORY, consumed))
    print(json.dumps(dict(original_proof=original, amended_proof=proof, timing=timing, **pairings(timing)),
                     indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    main()
