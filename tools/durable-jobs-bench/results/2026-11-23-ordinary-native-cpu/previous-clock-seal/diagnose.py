"""Controller-owned native role diagnostic and offline replay. Based on diagnose-roles.py."""
import argparse
from datetime import datetime, timedelta, timezone
import fcntl
import json
from pathlib import Path
import re
import shutil
from statistics import median
import tarfile
from uuid import uuid4

from reduce import HERE, ROOT, arm, run, verdict

ID = "ordinary-native-cpu"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
PIN_FILE = HERE / "inventory.json"
BLOCK = {"profile": "noop"}
LANES = ("ordinary", "oban", "river")
ORDERS = {"forward": LANES, "reverse": LANES[::-1]}
MODES = ("off", "on", "off")
HOST_IDLE = """
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
"""


def config_for_replay():
    config = run.read_json(PIN_FILE)
    run.require(config["cpus"] == [0, 1] and config["clock_ticks"] == 100
                and config["profiles"] == ["noop"], "invalid dense platform pins")
    for label in LANES:
        lane = config["lanes"][label]
        run.check_inventory(lane["inventory"])
        run.require(lane["inventory"]["cpus"] == config["cpus"], "trial mask changed")
    return config


def assignment(min_seconds):
    status = (QUEUE / "STATUS.md").read_text()
    events = re.findall(r"^(SETUP|TIMING|RELEASED)\s+([^\s]+)[^\n]*", status, re.M)
    run.require(events and events[-1] == ("SETUP", ID), "controller exclusive SETUP is not the current assignment")
    grants = re.findall(rf"^SETUP {ID} (\S+) (\S+)\s*\n([\s\S]*?)(?=^\S|\Z)", status, re.M)
    # Grant lines must describe the named diagnostic and be explicit in STATUS.md.
    run.require(grants, "controller did not post an exact UTC SETUP assignment")
    start, stop, _ = grants[-1]
    begin, end = [datetime.fromisoformat(value.replace("Z", "+00:00")) for value in (start, stop)]
    now = datetime.now(timezone.utc)
    run.require(begin <= now and now + timedelta(seconds=min_seconds) < end,
                "outside controller SETUP bounds or insufficient time for a new arm")
    return {"request": ID, "utc_start": start, "utc_end": stop}


def check_inputs():
    request = run.read_json(QUEUE / f"{ID}.json")
    run.require(request["id"] == ID and request["ready"] is True, "request is not sealed and ready")
    for relative, expected in request["local_sha256"].items():
        path = ROOT / relative
        run.require(run.digest(path) == expected, f"local frozen input changed: {relative}")
    return request


def layout(out, config):
    for order, lanes in ORDERS.items():
        for label in lanes:
            for index, mode in enumerate(MODES, 1):
                lane = config["lanes"][label]
                parent = out / order / label / f"{index:02}-{mode}"
                yield order, label, mode, parent, parent / f"diagnostic-{lane['engine']}"


def replay(out, config):
    results = []
    for order, label, mode, _, raw in layout(out, config):
        row = {"order": order, "lane": label, "mode": mode, "path": str(raw.relative_to(out))}
        try:
            status = run.read_json(raw / "dispatch.json")
            row["dispatch"] = status
            run.require(status.get("exit_code") == 0, "native arm did not exit cleanly")
            row.update(arm(raw, config["lanes"][label], config, "diagnostic", mode))
        except (ValueError, OSError, KeyError, TypeError, tarfile.TarError) as error:
            row["error"] = str(error)
            if (raw / "raw.tar.gz").exists():
                row["raw_sha256"] = run.digest(raw / "raw.tar.gz")
        results.append(row)
    controls = verdict(results)
    comparison = {}
    for label, check in controls.items():
        if check["drain_valid"]:
            acceptable = [row for row in results if row["lane"] == label and row["mode"] == "on"]
            drain = [row["budgets"]["drain"] for row in acceptable]
            comparison[label] = {
                "two_drain_probes": [b["duration_ms"] for b in drain],
                "median_drain_ms": median(b["duration_ms"] for b in drain),
                "host_busy_ms_per_job_envelope": [
                    min(b["host_cpu_ms_per_job"]["busy"][0] for b in drain),
                    max(b["host_cpu_ms_per_job"]["busy"][1] for b in drain),
                ],
            }
            if "role_cpu_ms_per_job" in drain[0]:
                comparison[label]["role_ms_per_job_envelope"] = {
                    role: [min(b["role_cpu_ms_per_job"][role]["total"][0] for b in drain),
                           max(b["role_cpu_ms_per_job"][role]["total"][1] for b in drain)]
                    for role in drain[0]["role_cpu_ms_per_job"]
                }
    return {"scope": "diagnostic only", "controls": controls, "comparison": comparison, "arms": results}


def dispatch(out, config):
    request = check_inputs()
    grant = assignment(360)
    out.mkdir()
    run.write_json(out / "intake.json", {"request": request, "assignment": grant})
    # Never create or replace a shared lock. Individual canonical workers own
    # active-arm.lock and delegate native.lock to the native wrapper for Mesh/Oban.
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        assignment(360)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        salt = uuid4().hex[:12]
        stages = {}
        for label in LANES:
            for mode in ("off", "on"):
                stage = out / "staging" / label / mode
                stage.mkdir(parents=True)
                run.write_json(stage / "inventory.json", config["lanes"][label]["inventory"])
                shutil.copyfile(HERE / ("collector.py" if mode == "on" else "canonical_worker.py"), stage / "worker.py")
                if mode == "on":
                    shutil.copyfile(HERE / "canonical_worker.py", stage / "canonical_worker.py")
                remote = run.REMOTE / f"{ID}-{salt}-{label}-{mode}"
                with (stage / "stage.log").open("xb") as log:
                    for name in ("inventory.json", "worker.py", "canonical_worker.py"):
                        if (stage / name).exists():
                            run.stage(stage / name, remote, log)
                stages[label, mode] = remote
        for _, label, mode, parent, raw in layout(out, config):
            assignment(45)
            parent.mkdir(parents=True)
            lane = config["lanes"][label]
            run.dispatch_arm(parent, stages[label, mode], BLOCK, "diagnostic", lane["engine"])
            status = run.read_json(raw / "dispatch.json")
            if status.get("interrupted"):
                raise RuntimeError("remote dispatch return is ambiguous. Controller must confirm release.")
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()


def history(out, config):
    blocks, = sorted((out / "blocks").glob("01-noop.*"))
    labels = ["ordinary", "protected-mesh", "oban", "river", "apalis"]
    data = {}
    for label in labels:
        lane = config["lanes"][label]
        folder = blocks / f"measured-{label}" / f"measured-{lane['engine']}"
        row = arm(folder, lane, config, "measured", "archived")
        row["archive"] = str(folder.relative_to(ROOT))
        data[label] = row
    ordinary = data["ordinary"]
    return {
        "scope": "one archival pinned native block, no new run",
        "arms": data,
        "drain_reduction_to_match_peer_ms": {
            label: ordinary["phase_ms"]["drain"] - data[label]["phase_ms"]["drain"]
            for label in ("oban", "river", "apalis")
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("dispatch", "replay", "history"))
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    config = config_for_replay()
    directory = args.directory.resolve()
    dispatch_error = None
    if args.action == "dispatch":
        directory.relative_to(HERE)
        if directory.exists():
            parser.error("dispatch requires a fresh output directory; use replay for retained attempts")
        # This precheck guarantees an ungranted invocation has no side effects.
        assignment(360)
        check_inputs()
        try:
            dispatch(directory, config)
        except (OSError, RuntimeError, ValueError) as error:
            dispatch_error = f"{type(error).__name__}: {error}"
    result = history(directory, config) if args.action == "history" else replay(directory, config)
    if dispatch_error:
        if not directory.exists():
            raise RuntimeError(dispatch_error)
        result["dispatch_error"] = dispatch_error
    if args.action == "dispatch":
        run.write_json(directory / "summary.json", result)
    elif args.output:
        run.write_json(args.output, result)
    else:
        print(json.dumps(result, indent=2))
    if dispatch_error or (args.action == "dispatch" and not all(row["drain_valid"] for row in result["controls"].values())):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
