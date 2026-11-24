"""Run a matched WAL group-commit sensitivity through the existing fenced controller."""
from datetime import datetime, timezone
import importlib.util
import json
from hashlib import sha256
from tempfile import NamedTemporaryFile
from pathlib import Path
import re
import runpy
import subprocess
import sys

HERE = Path(__file__).resolve().parent
PREVIOUS = HERE.parent / "2026-11-24-dense-wal-cost"
ID = HERE.name
PACKET = HERE / "request.json"
CASES = ("stock-a", "delay", "stock-b")
ROOT = HERE.parents[3]
STATE = ROOT / "target/linode-jobs-stepchange"
QUEUE = STATE / "requests"
STAGES = ROOT / "target/dense-group-commit/stages"
NATIVE = HERE / "evidence/native"
sys.path.insert(0, str(PREVIOUS))
spec = importlib.util.spec_from_file_location("matched_wal", PREVIOUS / "controller.py")
wal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wal)
run = wal.run
require = wal.require


def adapter(delay):
    settings = dict(wal.SETTINGS, commit_delay=str(delay), commit_siblings="1" if delay else "5")
    source = (PREVIOUS / "wal_capture.py").read_text()
    anchor = '"commit_delay": "0",'
    config = 'config.write("track_wal_io_timing = on\\n")'
    require(source.count(anchor) == source.count(config) == 1, "historical observer changed")
    siblings = settings["commit_siblings"]
    source = source.replace(anchor, f'"commit_delay": "{delay}", "commit_siblings": "{siblings}",')
    conf = f"track_wal_io_timing = on\ncommit_delay = {delay}\ncommit_siblings = {settings['commit_siblings']}\n"
    source = source.replace(config, f"config.write({conf!r})")
    return source, settings


def stage_files(folder, source, digests):
    path = STAGES / folder
    path.mkdir(parents=True, exist_ok=True)
    for name, data in (("wal_capture.py", source.encode()), ("native-wal.sh", (PREVIOUS / "native-wal.sh").read_bytes())):
        require(sha256(data).hexdigest() == digests[name], f"cannot reproduce {folder}/{name}")
        dest = path / name
        if not dest.exists():
            with NamedTemporaryFile(dir=path, delete=False) as tmp:
                tmp.write(data)
            Path(tmp.name).replace(dest)
        require(dest.read_bytes() == data, f"changed staged {dest}")
    return path


def check():
    packet = run.read_json(PACKET)
    require(packet["id"] == ID and packet["schedule"] == schedule(), "different schedule")
    for name, digest in packet["pins"].items():
        require(run.digest(ROOT / name) == digest, f"changed {name}")
    config, pins = wal.inputs()
    require(config["cpus"] == [0, 1], "wrong CPU set")
    for delay, folder in ((0, "stock"), (100, "delay")):
        source, settings = adapter(delay)
        path = stage_files(folder, source, packet["stage_sha256"][folder])
        compile(source, str(path), "exec")
        verification = runpy.run_path(str(path / "wal_capture.py"))
        require(verification["SETTINGS"] == settings and "'commit_siblings'" in verification["FRAME"], "missing settings witness")
    historical = run.read_json(PREVIOUS / "evidence/native/final-replay.json")
    require(not historical["missing"] and not historical["failures"], "historical source incomplete")
    for item in historical["groups"].values():
        for name, arm in item["arms"].items():
            phase, label = name.split("-", 1)
            engine = config["lanes"][label]["engine"]
            raw = PREVIOUS / "evidence/native/blocks" / item["attempt"] / name / f"{phase}-{engine}" / "raw.tar.gz"
            require(run.digest(raw) == arm["raw_sha256"], "historical archive differs")
    return config, {**pins, "group_commit_request": run.digest(PACKET)}


def schedule():
    return [[group, case] for group, cases in ((1, CASES), (2, CASES[::-1])) for case in cases]


def grant(_=None):
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?", (QUEUE/"STATUS.md").read_text(), re.M)
    phase, key, start, end = events[-1]
    require(phase == "TIMING" and key == ID and
            datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
            and PACKET.read_bytes() == (QUEUE / (ID + ".json")).read_bytes(), "requires the sealed native grant")
    return dict(start=start, end=end, request_sha256=run.digest(PACKET))


def subtrial(action, group, case):
    config, pins = check()
    require([group, case] in schedule(), "unknown subtrial")
    delay = 100 if case == "delay" else 0
    _, settings = adapter(delay)
    wal.HERE = STAGES / ("delay" if delay else "stock")
    wal.ID = f"{ID}-{group}-{case}"
    wal.RESULTS = NATIVE / f"{group}-{case}"
    wal.SETTINGS = settings
    wal.BLOCKS = [wal.BLOCKS[group-1]]
    wal.grant = grant
    pins = dict(pins, policy=case, commit_delay=delay, commit_siblings=int(settings["commit_siblings"]))
    wal.check_stages(config)
    if action == "run-one":
        grant()
        NATIVE.mkdir(parents=True, exist_ok=True)
        return wal.timing(config, pins)
    wal.configure_matrix()
    receipt = wal.summary(wal.RESULTS, config, wal.BLOCKS, pins)
    require(not receipt["missing"], "incomplete trial")
    print(json.dumps(receipt, sort_keys=True))
    return 0


def launch(action, group, case):
    return [sys.executable, "-B", str(HERE/"controller.py"), action, str(group), case]


def clock_gate(name, controls_and_treatment):
    durations = [arm[name] for arm in controls_and_treatment]
    if name != "middle_write_ms":
        durations = [1024000 / rate for rate in durations]
    a, treatment, b = durations
    faster = min(a, b)
    gain = faster - treatment
    spread = abs(a - b)
    return dict(gain_ms=gain, control_spread_ms=spread, passes=gain > max(0.05 * faster, spread))


def replay():
    rounds, gates = {}, {}
    clocks = ("submit_jobs_per_s", "commit_jobs_per_s", "middle_write_ms")
    for group, case in schedule():
        receipt = json.loads(subprocess.check_output(launch("read-one", group, case)))
        data = receipt["groups"][str(group)]["arms"]
        measured = {}
        for label in wal.ORDER:
            arm = data["measured-" + label]
            values = {key: arm[key] for key in ("raw_sha256", *clocks)}
            values["wal_sync"] = arm["postgres_wal"]["whole_workload_delta"]["wal_sync"]
            measured[label] = values
        rounds.setdefault(group, {})[case] = measured
    for group, cases in rounds.items():
        gates[group] = {}
        for label in wal.ORDER:
            series = [cases[case][label] for case in CASES]
            a, treatment, b = [arm["wal_sync"] for arm in series]
            gates[group][label] = dict(clocks={key: clock_gate(key, series) for key in clocks},
                                       fewer_syncs=treatment < min(a, b))
    qualifies = all(group["d2-on"]["fewer_syncs"] and all(m["passes"] for m in group["d2-on"]["clocks"].values())
                    for group in gates.values())
    return dict(rounds=rounds, gates=gates, material_in_both_rotations=qualifies)


def main(args):
    action, *tail = args
    if action in ("run-one", "read-one"):
        return subtrial(action, int(tail[0]), tail[1])
    config, _ = check()
    if action == "check":
        for case in ("stock","delay"):
            wal.HERE = STAGES / case
            wal.check_stages(config)
        return 0
    if action == "timing":
        grant()
        for group, case in schedule():
            subprocess.run(launch("run-one", group, case), check=True)
        run.write_json(NATIVE / "final-replay.json", replay())
        return 0
    if action == "replay":
        print(json.dumps(replay(), sort_keys=True, indent=2))
        return 0
    require(action == "intake" and tail == [run.digest(PACKET)], "unknown command or seal")
    receipt = QUEUE / f"{ID}-groups-1-2.released.json"
    if receipt.exists():
        return run.read_json(receipt)["exit_status"]
    pkt = run.read_json(PACKET)
    require(PACKET.read_bytes() == (QUEUE / f"{ID}.json").read_bytes(), "sealed intake missing")
    pkt.update(timing_grant_id=ID, timing_window_minutes=25, command_attempts=1)
    return runpy.run_path(str(STATE/"completion-writer-r2.py"))["measure"](pkt, "timing_command", [1,2])


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
