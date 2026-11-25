"""Independently reduce the twelve released raw arms and verify the frozen decision."""
from datetime import datetime
import hashlib
import json
from math import isclose
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
PILOT = HERE / "evidence/native/pilot"
DELAY = HERE.parent / "2026-11-24-dense-group-commit/evidence/native/release-replay.json"
ORDER = ((1, "stock-a"), (1, "candidate"), (1, "stock-b"),
         (2, "stock-b"), (2, "candidate"), (2, "stock-a"))
CLOCKS = ("submit_ms", "drain_ms", "middle_write_ms")
PG_COUNTERS = ("wal_records", "wal_fpi", "wal_bytes", "wal_buffers_full",
               "wal_write", "wal_sync", "wal_write_time", "wal_sync_time")
DISK_FIELDS = ("completed_flushes", "flush_time_ms", "completed_writes", "written_sectors", "write_time_ms")
METHOD = {"stock-a": "fdatasync", "stock-b": "fdatasync", "candidate": "open_datasync"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return json.loads(path.read_text())


def checksum(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def device(stream, start, end, shutdown):
    points = [json.loads(line) for line in stream]
    stamps = [point["epoch_us"] for point in points]
    series = []
    for point in points:
        row, = [line.split() for line in point["diskstats"] if line.split()[:3] == ["8", "0", "sda"]]
        series.append(tuple(int(row[i]) for i in (18, 19, 7, 9, 10)))
    require(stamps == sorted(set(stamps)) and all(
        all(a <= b for a, b in zip(left, right, strict=True)) for left, right in zip(series, series[1:])),
        "disk sample clock or counter reversal")

    def delta(left, right):
        return [b - a for a, b in zip(series[left], series[right], strict=True)]

    outer_start = max(i for i, t in enumerate(stamps[1:]) if t <= start)
    outer_end = next(i for i, t in enumerate(stamps) if t >= end)
    inner = [i for i, (lo, hi) in enumerate(zip(stamps, stamps[1:])) if start <= lo <= hi <= end]
    return dict(fields=list(DISK_FIELDS), inner=delta(inner[0], inner[-1]) if inner else [0] * len(DISK_FIELDS),
                outer=delta(outer_start, outer_end),
                outer_may_include_shutdown=outer_end + 1 == len(stamps) or stamps[outer_end + 1] > shutdown)


def reduce_arm(folder, phase, method, contract, verified_files):
    raw = folder / "raw.tar.gz"
    with tarfile.open(raw, "r:gz") as archive:
        def member(name):
            with archive.extractfile(name) as stream:
                return json.load(stream)

        invocation = member("invocation.json")
        request = read(folder / "request.json")
        require(all(invocation[k] == request[k] == value for k, value in
                    dict(attempt=request["attempt"], phase=phase, engine="mesh", profile="noop").items())
                and member("status.json")["exit_code"] == read(folder / "dispatch.json")["exit_code"] == 0,
                "arm invocation or status differs")
        host = member("host.json")
        mount = {"source": "/dev/sda", "target": "/", "fstype": "ext4", "maj:min": "8:0"}
        require(host["pg_mount"] == mount and host["verified_files"] == verified_files
                and host["affinity"] == contract["cpus"]
                and [host["boot_id"], host["kernel"], mount["maj:min"]] == contract["native_host"],
                "host or inventory identity differs")
        ledger = member("adapter/ledger.json")
        jobs = ledger["jobs"]
        enqueue, = ledger["transactions"]
        require(ledger["runners"] == 2 and ledger["slots"] == 64 and ledger["phase"] == phase
                and ledger["input_sha256"] == contract["input_sha256"] and ledger["local_smoke"] is False
                and len(jobs) == len({j["id"] for j in jobs}) == 1024
                and {j["job_key"]: j["id"] for j in jobs} == enqueue["enqueued_ids"]
                and len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
                and all(j["state"] == "completed" and j["attempts"] == j["effects"] == 1
                        and j["attempt_committed_us"] < j["effect_committed_us"] <= j["settled_us"] for j in jobs),
                "ledger phase, topology, or original writes differ")
        submit, commit = enqueue["submitted_us"], enqueue["committed_us"]
        settled = max(j["settled_us"] for j in jobs)
        effects = sorted(j["effect_committed_us"] for j in jobs)
        before, live, closure = (member(f"adapter/wal-{name}.json") for name in ("before", "live-after", "restored"))
        restored = closure["restored"]
        settings = dict(server_version_num="170011", wal_sync_method=method, synchronous_commit="on",
                        fsync="on", full_page_writes="on", track_commit_timestamp="on",
                        ssl="on", commit_delay="0", commit_siblings="5", debug_io_direct="", track_wal_io_timing="on")
        frames = [sample["frame"] for sample in (before, live, restored)]
        cluster, = {frame["system_id"] for frame in frames}
        require(all(all(frame["settings"][k] == v for k, v in settings.items()) and frame["observer_tls"]
                    and frame["other_clients"] == frame["wal_senders"] == 0 for frame in frames)
                and before["end"]["epoch_us"] <= submit < commit < settled <= live["begin"]["epoch_us"],
                "settings, client isolation, or timing brackets differ")
        support = member("adapter/wal-support.json")
        sync = support["o_sync"]
        dsync = support["o_dsync"]
        require(support["selected"] == method and support["platform"] == "linux"
                and support["kernel"] == host["kernel"] and support["device"] == mount["maj:min"]
                and all(support["wal_mount"][k] == v for k, v in mount.items())
                and 0 < dsync < sync and (dsync & sync) == dsync
                and all(support["files"][m]["bytes"] == 8192 and (support["files"][m]["flags"] & sync) == flag
                        for m, flag in (("fdatasync", 0), ("open_datasync", dsync))), "WAL support differs")
        old, mid, end = (frame["wal"] for frame in frames)
        require(old["stats_reset"] == mid["stats_reset"] == end["stats_reset"]
                and all(old[k] <= mid[k] <= end[k] for k in PG_COUNTERS), "WAL counter reversal")
        pg = {k: end[k] - old[k] for k in PG_COUNTERS}
        pg["wal_write_plus_sync_ms"] = pg["wal_write_time"] + pg["wal_sync_time"]
        require(pg["wal_write"] > 0 and pg["wal_write_time"] > 0 and pg["wal_bytes"] > 0
                and ((pg["wal_sync"] > 0 and pg["wal_sync_time"] > 0) if method == "fdatasync"
                     else pg["wal_sync"] == pg["wal_sync_time"] == 0), "WAL method accounting differs")
        with archive.extractfile("host-samples.jsonl") as stream:
            disk = device(stream, commit, settled, closure["stop_begin"]["epoch_us"])
    return dict(raw_sha256=checksum(raw), submit_ms=(settled - submit) / 1000,
                drain_ms=(settled - commit) / 1000, middle_write_ms=(effects[921] - effects[102]) / 1000,
                postgres_wal=pg, device=disk, cluster_id=cluster,
                postgres_bracket_us=[before["begin"]["epoch_us"], restored["end"]["epoch_us"]])


def replay():
    request = read(HERE / "request.json")
    release = read(PILOT / "controller-release.json")
    require(request["pilot"]["schedule"] == [list(row) for row in ORDER] and request["pilot"]["arms"] == 12
            and release["id"] == request["id"] + "-pilot" and release["exit_status"] == 0
            and release["attempts"] == [0] and release["groups"] == [1, 2]
            and datetime.fromisoformat(release["started"]) < datetime.fromisoformat(release["released_at"]),
            "pilot release or order differs")
    subblocks = {}
    for group, case in ORDER:
        name = f"{group}-{case}"
        path = PILOT / name
        series = read(path / "series.json")
        block, = (path / "blocks").iterdir()
        grant, = path.glob("grant-*.json")
        require(read(grant)["request_sha256"] == checksum(HERE / "request.json")
                and series["pins"]["case"] == case and series["pins"]["wal_sync_method"] == METHOD[case]
                and series["schedule"] == [read(block / "order.json")] == [
                    dict(group=group, order=["d2-on"], phases=["warmup", "measured"], profile="noop")],
                "sub-block seal or order differs")
        files = read(path / "staging/d2-on/inventory.json")["engines"]["mesh"]["verified_files"]
        full = read(block / "reconciled.json")
        arms = {}
        for phase in ("warmup", "measured"):
            label = f"{phase}-d2-on"
            arm = reduce_arm(block / label / f"{phase}-mesh", phase, METHOD[case], request, files)
            require(all(arm[k] == full[label][k] for k in ("raw_sha256", "cluster_id", "postgres_bracket_us")),
                    "raw arm identity disagrees with archived reconciliation")
            arms[phase] = arm
        subblocks[name] = arms

    arms = [a for phases in subblocks.values() for a in phases.values()]
    stamps = [t for arm in arms for t in arm["postgres_bracket_us"]]
    require(stamps == sorted(set(stamps)) and len({a["cluster_id"] for a in arms}) == len(arms) == 12,
            "phase order or cluster isolation differs")

    final = read(PILOT / "final-replay.json")
    require(final == read(PILOT / "summary.json"), "controller summaries differ")
    prior = read(DELAY)["rounds"]
    history = {clock: [] for clock in CLOCKS}
    for block in prior.values():
        a, b = (block[case]["d2-on"] for case in ("stock-a", "stock-b"))
        for clock, key in zip(CLOCKS, ("submit_jobs_per_s", "commit_jobs_per_s", "middle_write_ms"), strict=True):
            values = [x[key] if key == "middle_write_ms" else 1_024_000 / x[key] for x in (a, b)]
            history[clock].append(abs(values[0] - values[1]))

    gates = {}
    for group in ("1", "2"):
        cases = {case: subblocks[group + "-" + case]["measured"] for case in METHOD}
        for case, arm in cases.items():
            expected = final["rounds"][group][case]["d2-on"]
            require(all(arm[k] == expected[k] for k in (*CLOCKS, "raw_sha256", "postgres_wal"))
                    and all(arm["device"][k] == expected["device"][k] for k in arm["device"]),
                    "raw measurements disagree with published results")
        gates[group] = {}
        for clock in CLOCKS:
            stock_a, stock_b, candidate = (cases[case][clock] for case in METHOD)
            faster = min(stock_a, stock_b)
            gain = faster - candidate
            spread = abs(stock_a - stock_b)
            historic = max(history[clock])
            floor = max(faster / 20, spread, historic)
            gate = dict(gain_ms=gain, control_spread_ms=spread, historical_spread_ms=historic,
                        required_gain_ms=floor, passes=gain > floor)
            require(all(isclose(value, final["gates"][group][clock][key], abs_tol=1e-9)
                        if type(value) is float else value == final["gates"][group][clock][key]
                        for key, value in gate.items()), "gate disagrees with published results")
            gates[group][clock] = gate
    advance = all(gate["passes"] for group in gates.values() for gate in group.values())
    require(advance is final["advance_to_pricing"], "decision disagrees with published results")
    return dict(controller_release_sha256=checksum(PILOT / "controller-release.json"),
                controller_replay_sha256=checksum(PILOT / "final-replay.json"),
                request_sha256=checksum(HERE / "request.json"), subblocks=subblocks, gates=gates,
                advance_to_pricing=advance)


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2, allow_nan=False))
