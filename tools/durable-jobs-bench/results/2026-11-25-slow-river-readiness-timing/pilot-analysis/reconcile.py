"""Reconcile the rejected SLOW readiness timing captures using released files only."""

from bisect import bisect_left, bisect_right
import hashlib
import json
from pathlib import Path
import tarfile


HERE = Path(__file__).resolve().parent
PACKET = HERE.parent
PILOT = PACKET / "evidence/pilot"
RELEASE_SHA = "1b753afe56b6a31c939ac31411bed3268e64286e1175bac70ac0815c7c0f9246"
ENGINE = {"mesh": "mesh", "oban": "oban"}
BOUNDARIES = ("submit", "enqueue", "write64", "write448", "write512", "last_effect", "terminal")
HOST_WINDOWS = ("submit_to_terminal", "write64_to_write448", "last_effect_to_terminal")
CPU_TICK_MS = 10  # CLK_TCK=100 in the sealed cap-71 design.
METRICS = ("cpu_active_ms", "cpu_idle_ms", "cpu_iowait_ms", "cpu_steal_ms",
           "disk_read_bytes", "disk_write_bytes", "disk_busy_ms", "disk_flushes")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_bytes())


def release():
    all_files = {str(p.relative_to(PILOT)) for p in PILOT.rglob("*") if p.is_file()}
    manifest = {}
    for line in (HERE / "archive.sha256").read_text().splitlines():
        digest, filename = line.split("  ", 1)
        path = Path(filename)
        name = path.as_posix()
        require(not path.is_absolute() and ".." not in path.parts and name not in manifest
                and sha(PILOT / path) == digest, f"archive file differs: {filename}")
        manifest[name] = digest
    require(manifest.keys() == all_files and len(manifest) == 145, "pilot archive inventory differs")
    document = read(PILOT / "controller-release.json")
    request = read(PACKET / "request-pilot.json")
    grant = read(PILOT / "grant.json")
    require(manifest["controller-release.json"] == RELEASE_SHA and document["exit_status"] == 1
            and document["id"] == request["id"] == grant["id"]
            and document["request_sha256"] == grant["packet_sha256"] == sha(PACKET / "request-pilot.json"),
            "official rejection or intake seal differs")
    raws = {name: digest for name, digest in manifest.items() if name.endswith("/raw.tar.gz")}
    require(len(raws) == request["arms"] == 24
            and raws == read(PILOT / "raw-sha256.json") == document["raw_sha256"],
            "raw arm seal differs")
    pinned = ("public-code-access.log", "retained-inputs.json", "retained-native-sha256.log")
    require({name: manifest[name] for name in pinned} == document["retained_custody_sha256"],
            "retained custody seal differs")
    return document, request


def counters(point, disk):
    cpu = [sum(pair) * CPU_TICK_MS for pair in zip(point["cpu"]["cpu0"], point["cpu"]["cpu1"], strict=True)]
    dev, = (row for line in point["diskstats"] if (row := line.split())[:2] == disk)
    return [sum(cpu[i] for i in (0, 1, 2, 5, 6)), cpu[3], cpu[4], cpu[7],
            int(dev[5]) * 512, int(dev[9]) * 512, int(dev[12]), int(dev[18])]


def bounds(start, end, times, values):
    before = bisect_right(times, start) - 1
    after = bisect_left(times, end)
    first_inside = bisect_left(times, start)
    last_inside = bisect_right(times, end) - 1
    require(0 <= before < after < len(times), "host samples fail to bracket durable window")
    observations = {}
    for index, key in enumerate(METRICS):
        series = [sample[index] for sample in values]
        outer = series[after] - series[before]
        if any(series[i] > series[i + 1] for i in range(before, after)):
            observations[key] = dict(outer_net=outer, lower=None, upper=None)
        else:
            lower = series[last_inside] - series[first_inside] if last_inside > first_inside else 0
            observations[key] = dict(outer_net=outer, lower=lower, upper=outer)
    return dict(window_us=end - start, leading_slack_us=start - times[before],
                trailing_slack_us=times[after] - end,
                outer_samples=after - before + 1, inner_samples=max(0, last_inside - first_inside + 1),
                max_gap_us=max(times[i + 1] - times[i] for i in range(before, after)), counters=observations)


def arm(path, score, invocation, digest):
    with tarfile.open(path, "r:gz") as raw:
        def doc(name):
            return json.load(raw.extractfile(name))

        identity = doc("invocation.json")
        require(all(identity[k] == invocation[k] for k in ("attempt", "phase", "engine", "profile"))
                and identity["attempt"] == score["attempt_id"] and score["raw_sha256"] == digest
                and doc("status.json")["exit_code"] == 0, f"{path}: arm identity or exit differs")
        ledger, marks, host = (doc(name) for name in ("adapter/ledger.json", "adapter/startup-clock.json", "host.json"))
        require(host["affinity"] == [0, 1] and [host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] ==
                [score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]],
                f"{path}: host identity differs")
        samples = [json.loads(line) for line in raw.extractfile("host-samples.jsonl")]
        txn, = ledger["transactions"]
        jobs = ledger["jobs"]
        writes = sorted(job["attempt_committed_us"] for job in jobs)
        effects = sorted(job["effect_committed_us"] for job in jobs)
        terminal = max(job["settled_us"] for job in jobs)
        app_xids = {job[key] for job in jobs for key in ("attempt_xid", "effect_xid")}
        terminal_xids = {job["settled_xid"] for job in jobs}
        tail = [job for job in jobs if job["settled_us"] > effects[-1]]
        require(len(jobs) == len({j["job_key"] for j in jobs}) == score["completed"] == 512 and
                len(app_xids) == score["write_xids"] == 1024 and
                len(terminal_xids) == score["terminal_xids"] and
                txn["xid"] not in (app_xids | terminal_xids) and app_xids.isdisjoint(terminal_xids) and
                all(job["hold_ms"] == 100 and txn["committed_us"] <= job["attempt_committed_us"]
                    and job["attempt_committed_us"] + 100_000 <= job["effect_committed_us"] <= job["settled_us"]
                    for job in jobs), f"{path}: frozen hold or XID contract differs")
        epochs = dict(zip(BOUNDARIES, (txn["submitted_us"], txn["committed_us"], writes[63], writes[447],
                                       writes[-1], effects[-1], terminal), strict=True))
        require(list(epochs.values()) == sorted(epochs.values()), "nonmonotonic durable landmarks")
        offsets = {boundary: stamp - epochs["submit"] for boundary, stamp in epochs.items()}
        intervals = {f"{a}_to_{b}": epochs[b] - epochs[a] for a, b in zip(BOUNDARIES, BOUNDARIES[1:])}
        for end in BOUNDARIES[2:]:
            name = "first_write_" + end.removeprefix("write") if end.startswith("write") else end
            require(score["progress_us"][name] == {"submit": offsets[end], "drain": epochs[end] - epochs["enqueue"]},
                    f"{path}: recorded {end} differs")
        require(offsets["terminal"] / 1000 == score["submit_ms"]
                and (terminal - epochs["enqueue"]) / 1000 == score["drain_ms"]
                and intervals["last_effect_to_terminal"] == score["terminal_tail_us"],
                f"{path}: official clocks or terminal tail differ")
        if "runner_launch_to_last_settlement_us" in score:
            startup = doc("adapter/river/measured/startup.json")
            ready, = [point for point in startup["points"] if point["point"] == "ready"]
            runner_launch = ready["epoch_us"] - ready["elapsed_us"]
            launch_to_ready = ready["elapsed_us"]
            clock_key = "runner_launch_to_last_settlement_us"
            origin = "rebuilt River first runner launch"
            runner_extra = dict(producer_connection_setup_us=doc("adapter/river/producer-startup.json")["elapsed_us"],
                                per_runner_fill_us=[fill["elapsed_us"] for fill in score.get("pool_fills", [])])
        else:
            runner_launch = marks["runner_launch_dispatch"]["epoch_ns"] // 1000
            launch_to_ready = (marks["runners_ready"]["monotonic_ns"] -
                               marks["runner_launch_dispatch"]["monotonic_ns"]) // 1000
            clock_key = "runner_dispatch_to_last_settlement_us"
            origin = "bridge dispatch including Go migration" if invocation["engine"] == "go" else "first runner fork"
            runner_extra = {}
        adapter_start = marks["adapter_start"]["epoch_ns"] // 1000
        require(adapter_start <= runner_launch < epochs["submit"] and terminal - runner_launch == score[clock_key]
                and (terminal - adapter_start) / 1000 == score["adapter_start_to_terminal_ms"],
                f"{path}: recorded launch clock differs")
        if clock_key == "runner_launch_to_last_settlement_us":
            require(launch_to_ready == score["runner_launch_to_ready_us"], "River ready clock differs")
    times = [point["epoch_us"] for point in samples]
    require(times == sorted(set(times)) and not any(point["builders"] for point in samples), "host clock or builder evidence differs")
    values = [counters(point, host["pg_mount"]["maj:min"].split(":")) for point in samples]
    host_windows = {}
    for name in HOST_WINDOWS:
        start, end = name.split("_to_")
        host_windows[name] = bounds(epochs[start], epochs[end], times, values)
    outer = host_windows["submit_to_terminal"]
    net = {key: value["outer_net"] for key, value in outer["counters"].items()}
    total_cpu_ms = sum(net[key] for key in METRICS[:4])
    require(outer["outer_samples"] == score["host_samples"]
            and outer["leading_slack_us"] + outer["window_us"] + outer["trailing_slack_us"] ==
            round(score["host_sample_span_ms"] * 1000)
            and all(net[key] == score[key] for key in METRICS[4:])
            and 100 * net["cpu_iowait_ms"] / total_cpu_ms == score["cpu_iowait_percent"]
            and 100 * net["cpu_steal_ms"] / total_cpu_ms == score["cpu_steal_percent"],
            f"{path}: controller host bracket differs")
    return dict(attempt=identity["attempt"], raw_sha256=digest, epochs_us=epochs,
                submit_offsets_us=offsets, interval_us=intervals, jobs=len(jobs), application_xids=len(app_xids),
                terminal_xids=len(terminal_xids),
                second_write_ranks_since_submit_us={str(r): effects[r - 1] - epochs["submit"] for r in (64, 448, 512)},
                terminal_after_last_effect=dict(jobs=len(tail), xids=len({j["settled_xid"] for j in tail}),
                                                final_commit_jobs=sum(j["settled_us"] == terminal for j in jobs)),
                startup=dict(origin=origin, launch_to_ready_us=launch_to_ready, launch_to_submit_us=epochs["submit"] - runner_launch,
                             launch_to_terminal_us=terminal - runner_launch, adapter_start_to_terminal_us=terminal - adapter_start,
                             **runner_extra),
                ready_physical_clients=score.get("ready_physical_clients"),
                host=dict(cpu_steal_percent=score["cpu_steal_percent"], cpu_iowait_percent=score["cpu_iowait_percent"],
                          windows=host_windows))


def contrasts(arms, request):
    pairs = {}
    for group in request["groups"]:
        for phase in request["phases"]:
            prefix = f"{group}/{phase}-"
            a, b, warm = (arms[prefix + name] for name in ("lazy-a", "lazy-b", "warm-river"))
            slow, fast = sorted((a, b), key=lambda row: row["submit_offsets_us"]["terminal"], reverse=True)
            delta = {key: slow["interval_us"][key] - fast["interval_us"][key] for key in slow["interval_us"]}
            submit_gap = slow["submit_offsets_us"]["terminal"] - fast["submit_offsets_us"]["terminal"]
            require(sum(delta.values()) == submit_gap, "lazy phase gaps do not telescope")

            def launch_mid(field):
                return (a["startup"][field] + b["startup"][field]) / 2

            midpoint_submit = (a["submit_offsets_us"]["terminal"] + b["submit_offsets_us"]["terminal"]) / 2
            pairs[f"{group}/{phase}"] = dict(
                slower_lazy="lazy-a" if slow is a else "lazy-b", submit_gap_us=submit_gap,
                slower_minus_faster_interval_us=delta,
                slower_minus_faster_launch_prelude_us=slow["startup"]["launch_to_submit_us"] - fast["startup"]["launch_to_submit_us"],
                slower_minus_faster_launch_to_terminal_us=slow["startup"]["launch_to_terminal_us"] - fast["startup"]["launch_to_terminal_us"],
                lazy_midpoint_minus_warm_submit_us=midpoint_submit - warm["submit_offsets_us"]["terminal"],
                lazy_midpoint_minus_warm_launch_to_terminal_us=launch_mid("launch_to_terminal_us") - warm["startup"]["launch_to_terminal_us"],
                warm_minus_lazy_midpoint_launch_prelude_us=warm["startup"]["launch_to_submit_us"] - launch_mid("launch_to_submit_us"),
            )
    return pairs


def main():
    document, request = release()
    recorded = read(PILOT / "replay.json")
    arms = {}
    for group in request["groups"]:
        block = PILOT / f"{group:02d}"
        order = [[phase, lane] for phase in request["phases"] for lane in request["orders"][str(group)]]
        observation = recorded[str(group)]
        require(order == read(block / "order.json") and observation["eligible"] is False
                and observation["gate_errors"] and not observation["rejected"],
                f"block {group}: released ordering or control rejection differs")
        for phase, lane in order:
            name = f"{phase}-{lane}"
            folder = block / name / f"{phase}-{ENGINE.get(lane, 'go')}"
            path = folder / "raw.tar.gz"
            arms[f"{group}/{name}"] = arm(path, observation["scores"][name], read(folder / "request.json"),
                                           document["raw_sha256"][path.relative_to(PILOT).as_posix()])
    require(len({value["attempt"] for value in arms.values()}) == len(arms) == 24, "pilot attempt reuse")
    result = dict(release_sha256=RELEASE_SHA, release_exit=document["exit_status"], released_at=document["released_at"],
                  archive_manifest_sha256=sha(HERE / "archive.sha256"), arms=arms,
                  controls={group: row["gate_errors"] for group, row in recorded.items()},
                  lazy_and_warm_contrasts=contrasts(arms, request))
    print(json.dumps(result, sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
