"""Reduce the original Linode archives and the earlier equivalent guest replay."""

import hashlib
import io
import json
from pathlib import Path
import runpy
from statistics import median
import tarfile


PACKET = Path(__file__).resolve().parent
ROOT = PACKET.parents[3]
RESULTS = PACKET.parent
INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
JOBS = 1024
FLIGHTS = 2 * JOBS


def read_json(path):
    return json.loads(path.read_text())


def tar_json(archive, name):
    with archive.extractfile(name) as stream:
        return json.load(stream)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def interval_union_us(windows):
    end = total = 0
    for start, stop in sorted(windows):
        total += max(0, stop - max(start, end))
        end = max(end, stop)
    return total


def native_arm(block, recorded, lane, engine):
    relative = f"measured-{lane}/measured-{engine}"
    path = block / relative / "raw.tar.gz"
    sha = digest(path)
    assert sha == recorded["raw_sha256"][relative], path
    score = recorded["scores"][lane]
    with tarfile.open(path, "r:gz") as archive:
        host = tar_json(archive, "host.json")
        ledger = tar_json(archive, "adapter/ledger.json")
        status = tar_json(archive, "status.json")
        assert status["exit_code"] == score["failed"] == 0, path
        assert host["architecture"] == "x86_64" and host["affinity"] == [0, 1], path
        assert host["boot_id"] == score["host_boot_id"], path
        assert host["kernel"] == score["host_kernel"], path
        assert host["pg_mount"]["maj:min"] == score["pg_mount"]["maj:min"], path
        assert score["cpu_steal_percent"] <= 1, path
        assert ledger["input_sha256"] == INPUT and ledger["profile"] == "noop", path
        invocation = tar_json(archive, "invocation.json")
        assert invocation["phase"] == "measured" and invocation["engine"] == engine, path
        assert not ledger.get("local_smoke", False), path
        database = ledger["database"]
        assert database["version"] == "17.11" and database["max_connections"] == 96, path
        assert all(database[k] for k in ("synchronous_commit", "fsync", "tls",
                                         "full_page_writes", "track_commit_timestamp")), path
        jobs = ledger["jobs"]
        transaction, = ledger["transactions"]
        commit = transaction["committed_us"]
        settled = max(job["settled_us"] for job in jobs)
        assert score["first_commit_us"] == commit and score["last_settlement_us"] == settled, path
        assert len(jobs) == score["completed"] == score["ordered_independent_writes"] == JOBS, path
        assert {job["job_key"]: job["id"] for job in jobs} == transaction["enqueued_ids"], path
        assert all(job["state"] == "completed" and job["attempt"] == job["attempts"] == 1
                   and job["effects"] == 1 and commit < job["attempt_committed_us"]
                   < job["effect_committed_us"] < job["settled_us"] for job in jobs), path
        app_xids = {job[field] for job in jobs for field in ("attempt_xid", "effect_xid")}
        terminal_xids = {job["settled_xid"] for job in jobs}
        assert len(app_xids) == FLIGHTS and app_xids.isdisjoint(terminal_xids), path
        row = dict(raw_sha256=sha, artifact_sha256=ledger["artifact_sha256"],
                   version=ledger["version"], boot_id=host["boot_id"],
                   disk=host["pg_mount"]["maj:min"], kernel=host["kernel"],
                   cpu_steal_percent=score["cpu_steal_percent"],
                   drain_us=settled - commit, submit_to_terminal_us=settled - transaction["submitted_us"],
                   app_xids=len(app_xids), terminal_xids=len(terminal_xids))
        if engine != "mesh":
            return row

        raw = tar_json(archive, "adapter/raw.json")
        tls = tar_json(archive, "adapter/postgres-tls-sessions.json")
        app_backends = {s["backend"] for s in tls if s["role"] == "compiled_caller" and s["tls"]}
        masks = tar_json(archive, "adapter/process-affinity.json")
        assert len(app_backends) == 32 and ledger["runners"] == 8 and ledger["slots"] == 64, path
        assert len(masks) == 9 and all(mask == [0, 1] for mask in masks.values()), path
        assert JOBS == len(terminal_xids) == len({j["worker_pid"] for j in jobs}), path
        assert raw["raw_application_sql_calls"] == 2 * FLIGHTS, path
        assert raw["statement_counter_deallocations"] == 0, path
        resets = None
        if lane == "admission-v3":
            counters = tar_json(archive, "adapter/admission-ledger.json")
            assert counters["mode"] == "strict" and counters["cohort_turns"] == 0, path
            assert counters["terminal_xids"] == JOBS and counters["admission_turns"] > 0, path
            resets = counters["resets"]
            assert resets == FLIGHTS + len(app_backends), path

        windows = [(job["attempt_committed_us"], job["settled_us"]) for job in jobs]
        row.update(app_calls_after_idle=raw["raw_application_sql_calls"],
                   full_reset_calls_including_startup=resets,
                   post_commit_window_sum_us=sum(stop - start for start, stop in windows),
                   post_commit_window_union_us=interval_union_us(windows),
                   last_effect_commit_to_last_terminal_us=settled - max(
                       job["effect_committed_us"] for job in jobs),
                   app_tls_backends=len(app_backends), fresh_worker_pids=JOBS,
                   candidate_flights_per_arm=FLIGHTS)
        return row


def native_series():
    blocks = RESULTS / "2026-11-23-admission-v3-strict" / "blocks"
    lanes = {"current": "mesh", "admission-v3": "mesh", "oban": "oban",
             "river": "go", "apalis": "rust"}
    series = {}
    for file in sorted(blocks.glob("*-noop.*/reconciled.json")):
        recorded = read_json(file)
        group = read_json(file.with_name("order.json"))["group"]
        assert group not in series, file
        arms = {lane: native_arm(file.parent, recorded, lane, engine) for lane, engine in lanes.items()}
        assert len({(a["boot_id"], a["kernel"], a["disk"]) for a in arms.values()}) == 1, file
        gap = {peer: arms["admission-v3"]["drain_us"] - arms[peer]["drain_us"]
               for peer in ("oban", "river", "apalis")}
        series[group] = dict(block=file.parent.name, arms=arms, strict_gap_us=gap,
                             serial_savings_us_per_flight_to_match_peer={
                                 peer: round(us / FLIGHTS, 3) for peer, us in gap.items()})
    assert set(series) == set(range(1, 6))
    for lane in lanes:
        assert len({r["arms"][lane]["artifact_sha256"] for r in series.values()}) == 1, lane
    return series


def earlier_guest():
    lane = ROOT / "target/linux-queue-perf"
    read_serial = runpy.run_path(str(lane / "archived_transport.py"))["read_serial"]
    # The recovered timing run increased the transfer limit. The serial and
    # digest checks in the archived reader still apply.
    read_serial.__globals__["MAX_COMPRESSED"] = 4 * 1024**2
    result = {}
    for name, traced in (("dense-pipeline-proof", True), ("dense-warm-02", False)):
        serial = lane / f"{name}.serial"
        with serial.open("rb") as stream:
            data, status = read_serial(stream)
        assert status == 0, serial
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            config = tar_json(archive, "config.json")
            assert config["trace"] == traced and config["profile"] == "noop", serial
            assert config["full_input_warmup"] == (not traced), serial
            metrics = tar_json(archive, "metrics.json")
            assert [row["engine"] for row in metrics] == config["arms"], serial
            assert all(row["completed"] == JOBS and row["input_sha256"] == INPUT
                       for row in metrics), serial
            if not traced:
                warmups = tar_json(archive, "warmups.json")
                assert [(row["engine"], row["completed"], row["input_sha256"])
                        for row in warmups] == [(engine, JOBS, INPUT) for engine in config["arms"]]
            arms = {}
            for engine in ("pipeline-control", "pipeline"):
                writes = tar_json(archive, f"{engine}/ordered-writes.json")
                assert writes["ordered"] == writes["pids"] == JOBS, serial
                assert len(set(writes["worker_pids"])) == JOBS, serial
                if traced:
                    assert writes["exact_consuming_waits"] == JOBS, serial
                metric = next(row for row in metrics if row["engine"] == engine)
                arms[engine] = dict(elf_sha256=config["hashes"][f"proof/mesh-{engine}"],
                                    jobs_per_s=metric["jobs_per_second"],
                                    runner_cpu_ms_per_job=1000 * metric["cpu_seconds_per_job"],
                                    ordered_pairs=JOBS, exact_waits=writes.get("exact_consuming_waits"))
            result[name] = dict(serial_sha256=digest(serial), embedded_archive_sha256=hashlib.sha256(
                data).hexdigest(), arms=arms, pipeline_rate_change_percent=100 * (
                    arms["pipeline"]["jobs_per_s"] / arms["pipeline-control"]["jobs_per_s"] - 1))
    return result


def observed_native_cpu():
    folder = RESULTS / "2026-11-23-native-worker-cost" / "recovered"
    reduced = read_json(folder / "reduced.json")
    capture = reduced["arms"]["03-mesh-on"]
    waits = capture["exact_child_waits"]
    assert waits["workers"] == JOBS and len(waits["jobs"]) == JOBS
    assert capture["score"]["ordered_independent_writes"] == JOBS
    bounds = capture["windows"]["drain"]["stable_backends"]
    child_ms = waits["child_total_cpu_ms"]
    jobs_pg_lower_ms, _ = bounds["parity_subject"]["busy_cpu_ms_bounds"]
    observer_shift = reduced["observer_perturbation"]["mesh"]["drain"]["on_delta_vs_off_midpoint"]
    return dict(reduced_sha256=digest(folder / "reduced.json"),
                archive_sha256=digest(folder / "evidence.tar.gz"),
                exact_child_cpu_ms=child_ms, jobs_pg_cpu_lower_ms=jobs_pg_lower_ms,
                app_pg_cpu_ms_range=bounds["compiled_caller"]["busy_cpu_ms_bounds"],
                child_only_fixed_work_wall_floor_ms=child_ms / 2,
                child_plus_jobs_pg_fixed_work_wall_floor_ms=(child_ms + jobs_pg_lower_ms) / 2,
                observer_drain_shift_percent=100 * observer_shift)


if __name__ == "__main__":
    series = native_series()
    print(json.dumps(dict(native_series=series, earlier_equivalent=earlier_guest(),
                          observed_native_stock_cpu=observed_native_cpu(),
                          strict_median_drain_us=median(r["arms"]["admission-v3"]["drain_us"]
                                                        for r in series.values())),
                     indent=2, sort_keys=True))
