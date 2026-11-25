"""Stage and replay the frozen dense runner with a per-cluster PostgreSQL WAL method."""
import argparse
from datetime import datetime, timezone
from hashlib import sha256
import importlib.util
import inspect
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PREVIOUS = HERE.parent / "2026-11-24-dense-wal-cost"
DELAY = HERE.parent / "2026-11-24-dense-group-commit"
ID = HERE.name
PACKET = HERE / "request.json"
STATE = ROOT / "target/linode-jobs-stepchange"
QUEUE = STATE / "requests"
STAGES = ROOT / "target" / ID / "stages"
NATIVE = HERE / "evidence/native"

sys.path.insert(0, str(PREVIOUS))
spec = importlib.util.spec_from_file_location("frozen_wal_controller", PREVIOUS / "controller.py")
wal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wal)

run, require, matrix = wal.run, wal.require, wal.matrix
STOCK = "fdatasync"
CANDIDATE = "open_datasync"
METHODS = {"stock-a": STOCK, "stock-b": STOCK, "candidate": CANDIDATE}
CASES = ("stock-a", "candidate", "stock-b")
CLOCKS = ("submit_ms", "drain_ms", "middle_write_ms")
DELAY_CLOCKS = dict(zip(CLOCKS, ("submit_jobs_per_s", "commit_jobs_per_s", "middle_write_ms"), strict=True))


def schedule(campaign):
    cases = CASES if campaign == "pilot" else ("stock-a", "candidate")
    return [[group, case] for group, order in ((1, cases), (2, cases[::-1])) for case in order]


def replace(source, old, new):
    require(source.count(old) == 1, f"frozen source anchor changed: {old}")
    return source.replace(old, new, 1)


def adapter(method):
    require(method in METHODS.values(), "unknown WAL method")
    source = (PREVIOUS / "wal_capture.py").read_text()
    source = replace(source, '"wal_sync_method": "fdatasync",',
                     f'"wal_sync_method": "{method}", "commit_siblings": "5", "debug_io_direct": "",')
    config = ("track_wal_io_timing = on\ncommit_delay = 0\ncommit_siblings = 5\n"
              f"debug_io_direct = ''\nwal_sync_method = {method}\n")
    source = replace(source, 'config.write("track_wal_io_timing = on\\n")',
                     f"config.write({config!r})\n"
                     '            probe = probe_support(self.data)\n'
                     '            (output / "wal-support.json").write_text(json.dumps(probe) + "\\n")')
    return replace(source, 'if __name__ == "__main__":',
                   (HERE / "probe.py").read_text() + '\n\nif __name__ == "__main__":')


def stage(method, digest):
    folder = STAGES / method
    folder.mkdir(parents=True, exist_ok=True)
    launcher, observer = folder / "native-wal.sh", folder / "wal_capture.py"
    for path, data in ((launcher, (PREVIOUS / launcher.name).read_bytes()),
                       (observer, adapter(method).encode())):
        require(sha256(data).hexdigest() == digest[path.name], f"stage pin changed: {path.name}")
        if not path.exists():
            path.write_bytes(data)
        require(path.read_bytes() == data, f"stage contents changed: {path}")
    return folder, runpy.run_path(str(observer))


def check():
    packet = run.read_json(PACKET)
    require(packet["id"] == ID and packet["controller"] == "linode-native-stepchange-controller-v1"
            and packet["ready"] is True and packet["cpus"] == [0, 1]
            and packet["input_sha256"] == wal.dense.INPUT
            and all(packet[c]["schedule"] == schedule(c) for c in ("pilot", "pricing")),
            "packet identity or schedule changed")
    for name, digest in packet["pins"].items():
        require(run.digest(ROOT / name) == digest, f"changed input: {name}")
    original = run.read_json(PREVIOUS / "evidence/native/series.json")
    old_request = run.read_json(PREVIOUS / "controller-packet.json")
    config, pins = original["config"], original["pins"]
    require(config["cpus"] == packet["cpus"] and config["host"] == packet["native_host"]
            and tuple(sorted(config["lanes"])) == tuple(sorted(wal.ORDER))
            and all(lane["runners"] == 2 for lane in config["lanes"].values())
            and config["mesh_runner"]["d2-on"] == old_request["runner"]
            and config["mesh_broker"]["d2-on"] == old_request["broker"]
            and wal.BLOCKS == original["schedule"]
            and pins["request"] == run.digest(PREVIOUS / "controller-packet.json"), "prior native contract changed")
    for name, digest in pins.items():
        if name.startswith("tools/"):
            require(run.digest(ROOT / name) == digest, f"changed frozen input: {name}")
    wal.HERE = PREVIOUS
    for label in wal.ORDER:
        remote = run.REMOTE / original["remotes"][label]
        archived = run.read_json(PREVIOUS / "evidence/native/staging" / label / "inventory.json")
        require(wal.stages(config, label, remote)[1] == archived, f"archived native inventory differs: {label}")
    for method in (STOCK, CANDIDATE):
        wal.HERE, observer = stage(method, packet["stage_sha256"][method])
        require(observer["SETTINGS"] == dict(wal.SETTINGS, wal_sync_method=method,
                                             commit_siblings="5", debug_io_direct=""),
                "observer settings changed")
        wal.check_stages(config)
    return config, dict(pins, write_method_request=run.digest(PACKET))


def controller_events():
    return re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (QUEUE / "STATUS.md").read_text(), re.M)


def grant(campaign):
    key = f"{ID}-{campaign}"
    events = controller_events()
    require(events and events[-1][:2] == ("TIMING", key), "requires a sole-controller TIMING grant")
    _, _, start, end = events[-1]
    require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
            and PACKET.read_bytes() == (QUEUE / f"{key}.json").read_bytes(), "grant expired or seal differs")
    return dict(start=start, end=end, request_sha256=run.digest(PACKET))


def install_replay(method):
    # Keep all archived PG and host checks. Only change the method-specific charge assertion
    # and enrich the same device bracket with write counters from the same diskstats samples.
    charge = inspect.getsource(wal.verify_wal)
    charge = replace(charge, 'final["wal_sync"] > old["wal_sync"] and final["wal_sync_time"] > old["wal_sync_time"]',
                     '(final["wal_sync"] > old["wal_sync"] and final["wal_sync_time"] > old["wal_sync_time"]'
                     ' if SETTINGS["wal_sync_method"] == "fdatasync" else'
                     ' final["wal_sync"] == old["wal_sync"] and final["wal_sync_time"] == old["wal_sync_time"])'
                     ' and final["wal_write"] > old["wal_write"] and final["wal_bytes"] > old["wal_bytes"]')
    device = inspect.getsource(wal.device)
    device = replace(device, '(int(row[18]), int(row[19]))', 'tuple(int(row[i]) for i in (18, 19, 7, 9, 10))')
    device = replace(device, 'else [0, 0]', 'else [0] * 5')
    device = replace(device, '["completed_flushes", "flush_time_ms"]',
                     '["completed_flushes", "flush_time_ms", "completed_writes", "written_sectors", "write_time_ms"]')
    exec(compile(charge + "\n" + device, str(wal.__file__), "exec"), wal.__dict__)
    base_score = wal.score_arm

    def score_arm(folder, phase, label, config, remote):
        score = base_score(folder, phase, label, config, remote)
        with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
            support = run.file_json(archive, "adapter/wal-support.json")
            before = run.file_json(archive, "adapter/wal-before.json")
            closure = run.file_json(archive, "adapter/wal-restored.json")
        verify_support(support, method, config["host"][1])
        for delta in ("live_delta", "whole_workload_delta", "after_live_publication_shutdown_readback"):
            counters = score["postgres_wal"][delta]
            counters["wal_write_plus_sync_ms"] = counters["wal_write_time"] + counters["wal_sync_time"]
        return dict(score, wal_support=support, cluster_id=before["frame"]["system_id"],
                    postgres_bracket_us=[before["begin"]["epoch_us"], closure["restored"]["end"]["epoch_us"]])

    wal.score_arm = score_arm


def verify_support(support, method, kernel):
    mount = support["wal_mount"]
    require(support["platform"] == "linux" and support["kernel"] == kernel
            and support["selected"] == method and support["device"] == wal.MOUNT["maj:min"]
            and {key: mount[key] for key in wal.MOUNT} == wal.MOUNT
            and not {"ro", "nobarrier", "barrier=0", "noload", "norecovery"}.intersection(mount["options"].split(",")),
            "WAL support or mount witness changed")
    dsync, sync = support["o_dsync"], support["o_sync"]
    files = support["files"]
    require(0 < dsync < sync and (dsync & sync) == dsync and set(files) == {STOCK, CANDIDATE}
            and all(files[m]["bytes"] == 8192 and files[m]["flags"] & sync == flag
                    for m, flag in ((STOCK, 0), (CANDIDATE, dsync))),
            "missing Linux write and sync support")


def ordered_arms(arms):
    values = list(arms)
    brackets = [time for arm in values for time in arm["postgres_bracket_us"]]
    require(brackets == sorted(set(brackets)) and len({a["cluster_id"] for a in values}) == len(values),
            "arms overlap, execute out of order, or reuse a PostgreSQL cluster")


def summary(directory, config, blocks, pins):
    manifest = run.read_json(directory / "series.json")
    require(all(manifest[k] == value for k, value in dict(config=config, schedule=blocks, pins=pins).items()),
            "series stamp differs")
    attempts = sorted((directory / "blocks").glob(f"{run.block_prefix(blocks[0])}.*"))
    require(len(attempts) == 1, "requires one attempt per sub-block")
    attempt = attempts[0]
    try:
        arms = wal.replay_block(attempt, blocks[0], config, manifest["remotes"])
        ordered_arms(arms.values())
    except (*matrix.ERRORS, RuntimeError) as error:
        return dict(missing=[blocks[0]["group"]], failures={attempt.name: str(error)})
    return dict(missing=[], failures={}, attempt=attempt.name, arms=arms)


def configure_trial(campaign, group, case):
    config, pins = check()
    require([group, case] in schedule(campaign), "unknown sub-block")
    method = METHODS[case]
    wal.HERE = STAGES / method
    wal.ID = f"{ID}-{campaign}-{group}-{case}"
    wal.RESULTS = NATIVE / campaign / f"{group}-{case}"
    wal.SETTINGS = dict(wal.SETTINGS, wal_sync_method=method, commit_siblings="5", debug_io_direct="")
    wal.BLOCKS = [dict(wal.BLOCKS[group - 1])]
    if campaign == "pilot":
        config["lanes"] = {"d2-on": config["lanes"]["d2-on"]}
        wal.BLOCKS[0]["order"] = ["d2-on"]
    wal.grant = lambda _=None: grant(campaign)
    wal.summary = summary
    install_replay(method)
    return config, dict(pins, campaign=campaign, case=case, wal_sync_method=method)


def subtrial(action, campaign, group, case):
    config, pins = configure_trial(campaign, group, case)
    if action == "run-one":
        grant(campaign)
        if campaign == "pricing":
            released_pilot()
        require(not (wal.RESULTS / "series.json").exists(), "existing native attempt requires controller reconciliation")
        wal.RESULTS.parent.mkdir(parents=True, exist_ok=True)
        return wal.timing(config, pins)
    receipt = summary(wal.RESULTS, config, wal.BLOCKS, pins)
    require(not receipt["missing"], f"unreconciled sub-block: {receipt}")
    print(json.dumps(receipt, allow_nan=False))
    return 0


def command(action, campaign, *tail):
    return [sys.executable, "-B", str(HERE / "controller.py"), action, campaign, *map(str, tail)]


def clocks(arm):
    out = dict(submit_ms=(arm["last_settlement_us"] - arm["first_submit_us"]) / 1000,
               drain_ms=(arm["last_settlement_us"] - arm["first_commit_us"]) / 1000,
               middle_write_ms=arm["middle_write_ms"])
    require(all(0 < value < float("inf") for value in out.values()), "invalid end-to-end clock")
    return out


def historical_spread():
    archive = DELAY / "evidence/native"
    release = run.read_json(archive / "controller-release.json")
    prior = run.read_json(archive / "release-replay.json")
    require(release["id"] == DELAY.name and release["groups"] == [1, 2]
            and release["exit_status"] == 0 and release["released_at"]
            and prior["material_in_both_rotations"] is False, "historical delay release differs")
    return {key: max(g["d2-on"]["clocks"][name]["control_spread_ms"] for g in prior["gates"].values())
            for key, name in DELAY_CLOCKS.items()}


def clock_gate(a, candidate, b, noise):
    control = min(a, b)
    gain, spread = control - candidate, abs(a - b)
    margin = max(0.05 * control, spread, noise)
    return dict(gain_ms=gain, control_spread_ms=spread, historical_spread_ms=noise,
                required_gain_ms=margin, passes=gain > margin)


def replay(campaign):
    blocks = [json.loads(subprocess.check_output(command("read-one", campaign, group, case)))
              for group, case in schedule(campaign)]
    ordered_arms(arm for block in blocks for arm in block["arms"].values())
    rounds = {}
    for (group, case), block in zip(schedule(campaign), blocks, strict=True):
        rounds.setdefault(group, {})[case] = {
            name.removeprefix("measured-"): dict(clocks(arm), raw_sha256=arm["raw_sha256"],
                postgres_wal=arm["postgres_wal"]["whole_workload_delta"], device=arm["disk"])
            for name, arm in block["arms"].items() if name.startswith("measured-")
        }
    if campaign == "pilot":
        noise = historical_spread()
        gates = {group: {key: clock_gate(*(cases[case]["d2-on"][key] for case in CASES), noise[key])
                         for key in CLOCKS} for group, cases in rounds.items()}
        return dict(rounds=rounds, gates=gates,
                    advance_to_pricing=all(m["passes"] for group in gates.values() for m in group.values()))
    comparisons = {group: {case: {label: {
        key: dict(mesh_minus_peer_ms=arms["d2-on"][key] - arms[label][key],
                  mesh_latency_reduction_pct=100 * (1 - arms["d2-on"][key] / arms[label][key]))
        for key in CLOCKS} for label in wal.ORDER[1:]} for case, arms in cases.items()}
        for group, cases in rounds.items()}
    return dict(rounds=rounds, same_setting_comparisons=comparisons)


def released_pilot():
    receipt = run.read_json(QUEUE / f"{ID}-pilot-groups-1-2.released.json")
    require(receipt["id"] == f"{ID}-pilot" and receipt["exit_status"] == 0 and receipt["released_at"],
            "pricing requires a released pilot")
    result = replay("pilot")
    require(result["advance_to_pricing"], "pilot did not clear the pricing gate")


def intake(campaign, seal):
    require(seal == run.digest(PACKET), "intake digest differs")
    key = f"{ID}-{campaign}"
    receipt = QUEUE / f"{key}-groups-1-2.released.json"
    if receipt.exists():
        return run.read_json(receipt)["exit_status"]
    require(PACKET.read_bytes() == (QUEUE / f"{key}.json").read_bytes(), "sealed intake missing")
    if campaign == "pricing":
        released_pilot()
    events = controller_events()
    require(events and events[-1][0] == "RELEASED", "controller must release its previous grant first")
    packet = run.read_json(PACKET)
    packet.update(timing_grant_id=key, timing_window_minutes=packet[campaign]["minutes"], command_attempts=1,
                  timing_command=packet[campaign]["timing_command"], evidence_destination=str(NATIVE / campaign))
    return runpy.run_path(str(STATE / "completion-writer-r2.py"))["measure"](packet, "timing_command", [1, 2])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "intake", "timing", "replay", "run-one", "read-one"))
    parser.add_argument("campaign", nargs="?", choices=("pilot", "pricing"), default="pilot")
    parser.add_argument("tail", nargs="*")
    args = parser.parse_args()
    action, campaign, tail = args.action, args.campaign, args.tail
    if action in ("run-one", "read-one"):
        return subtrial(action, campaign, int(tail[0]), tail[1])
    check()
    if action == "check":
        return 0
    if action == "replay":
        print(json.dumps(replay(campaign), indent=2, sort_keys=True, allow_nan=False))
        return 0
    if action == "intake":
        return intake(campaign, *tail)
    grant(campaign)
    if campaign == "pricing":
        released_pilot()
    for group, case in schedule(campaign):
        subprocess.run(command("run-one", campaign, group, case), check=True)
    run.write_json(NATIVE / campaign / "final-replay.json", replay(campaign))
    return 0


if __name__ == "__main__":
    sys.exit(main())
