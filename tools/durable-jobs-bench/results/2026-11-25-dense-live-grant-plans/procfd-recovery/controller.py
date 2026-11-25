"""Sole-controller recovery with a new grant and the original qualified SETUP."""

from datetime import datetime, timezone
import fcntl
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile

from adapt import rewrite_observer
from roster import BOTH, CAPS, FIRST, verify_frames

HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.parent
ID = f"{ORIGINAL.name}-procfd-recovery"
PACKET = HERE / "controller-packet.json"
OUT = HERE / "evidence/native"
PREVIOUS = ORIGINAL / "evidence/native"
MODULE_SHA = "0f454e61b3f17831fb93fea58c882573b51eedf04fb2d48640a99be25b365e01"
ORIGINAL_SHA = "cff901a29c24f6fe2477a033c5c24f6b8024e7124c3fdc4bd0a739316af2281f"

sys.path.insert(1, str(ORIGINAL))
spec = importlib.util.spec_from_file_location("sealed_controller", ORIGINAL / "controller.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
run, need = base.run, base.need
base.ID = ID
FILES = [ORIGINAL / name for name in ("launcher.py", "observe.py", "plans.py", "settings.py")]
FILES += [HERE / name for name in ("launch.sh", "adapt.py", "roster.py")]


def packet():
    sealed = run.read_json(PACKET)
    need(sealed["id"] == ID and sealed["original_sha256"] == ORIGINAL_SHA
         and sealed["module_sha256"] == MODULE_SHA and sealed["schedule"] == base.BLOCKS
         and sealed["native_host"] == base.HOST and sealed["input_sha256"] == base.INPUT,
         "recovery packet binding drift")
    for path, pin in sealed["local_sha256"].items():
        need(run.digest(base.ROOT / path) == pin, f"sealed recovery input changed: {path}")


def prior():
    receipt = run.read_json(PREVIOUS / "setup.json")
    setup = run.read_json(PREVIOUS / "setup-controller-release.json")
    timing = run.read_json(PREVIOUS / "controller-release.json")
    failed = run.read_json(PREVIOUS / "failure.json")
    need(run.digest(ORIGINAL / "controller-packet.json") == ORIGINAL_SHA
         and all(r["request_sha256"] == ORIGINAL_SHA for r in (setup, timing))
         and setup["id"] == f"{ORIGINAL.name}-setup" and setup["exit_status"] == 0
         and timing["id"] == f"{ORIGINAL.name}-timing" and timing["exit_status"] == 1
         and receipt["remote"] == base.MODULE and receipt["sha256"] == MODULE_SHA
         and receipt["origin"] == "official-17.11-source"
         and receipt["canary"]["unrelated_sql_ran_without_plan"]
         and receipt["canary"]["postgres"] == "17.11"
         and receipt["canary"]["scoped_role"] == "parity_subject"
         and {r["toplevel"] for r in receipt["canary"]["nested_query_keys"]} == {False, True}
         and failed["type"] == "PermissionError" and "[Errno 13]" in failed["message"]
         and re.fullmatch(r".*'/proc/\d+/fd'", failed["message"]),
         "original positive SETUP or failed witness changed")
    return receipt


def configuration():
    config = base.configuration(MODULE_SHA)
    for lane, entry in config["lanes"].items():
        stage = run.REMOTE / f"{ID}-{lane}"
        spec = entry["inventory"]["engines"]["mesh"]
        spec["verified_files"].update({str(stage / path.name): run.digest(path) for path in FILES})
        run.check_inventory(entry["inventory"])
    return config


def grant():
    registered = base.QUEUE / f"{ID}.json"
    need(registered.is_file() and registered.read_bytes() == PACKET.read_bytes(),
         "requires the exact recovery packet registered by the sole controller")
    changes = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                         (base.QUEUE / "STATUS.md").read_text(), re.M)
    mode, name, start, end = changes[-1]
    need((mode, name) == ("TIMING", f"{ID}-timing")
         and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
         "requires the active recovery TIMING grant")
    return dict(id=name, start=start, end=end, request_sha256=run.digest(PACKET))


stock_score = base.score_arm


def score_arm(folder, config, lane, receipt):
    score = stock_score(folder, config, lane, receipt)
    with tarfile.open(folder / "raw.tar.gz") as archive:
        def get(name):
            return run.file_json(archive, f"adapter/{name}.json")

        witness, frames, launches = [get(n) for n in ("grant-roster", "grant-frames", "process-identity")]
        pinned = verify_frames(witness, frames)
        pids = [row["pid"] for row in launches if row["argv"][-1] == "run"]
        need(pids == witness["runners_by_index"], "SQL launch order disagrees with pinned processes")
        for sample in score["observation"]["samples"]:
            row = pinned.get(sample["pid"])
            need(row is not None and row[0] == "parity_subject" and row[2] <= sample["end_us"] + 999,
                 "sample came from an unpinned or replaced participant backend")
        score["connection_roster"] = witness
    return score


base.score_arm = score_arm


def timing(receipt, config):
    active = grant()
    OUT.mkdir(parents=True, exist_ok=True)
    run.write_json(OUT / "timing-grant.json", active)
    run.write_json(OUT / "series.json", dict(config=config, schedule=base.BLOCKS))
    preflight = f"""
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
test "$(cat /proc/sys/kernel/random/boot_id)" = {base.HOST[0]}
test "$(uname -smr)" = "Linux {base.HOST[1]} x86_64"
test "$(findmnt -nro MAJ:MIN --target /var/lib/durable-jobs-bench/native)" = {base.HOST[2]}
test "$(nproc --all)" = 4
test "$(taskset -c 0,1 nproc)" = 2
test "$(getconf CLK_TCK)" = 100
test "$(id -u)" = 0
test "$(runuser -u nobody -- id -u)" != 0
test "$(stat -c '%a:%u' {base.MODULE})" = 444:0
test "$(stat -c '%a:%u' $(dirname {base.MODULE}))" = 555:0
runuser -u nobody -- sha256sum {base.MODULE} | (read -r sum path; test "$sum" = {MODULE_SHA})
printf '%s\n' 'Pinned SETUP module verified as the unprivileged caller.'
"""
    with (base.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with (OUT / "timing-preflight.log").open("xb") as log:
            run.ssh(["bash", "-c", preflight], timeout=12, stdout=log, stderr=log).check_returncode()
        for lane in base.LANES:
            remote = run.REMOTE / f"{ID}-{lane}"
            folder = OUT / "staging" / lane
            folder.mkdir(parents=True)
            inventory = folder / "inventory.json"
            run.write_json(inventory, config["lanes"][lane]["inventory"])
            with (folder / "stage.log").open("xb") as log:
                for path in [*FILES, base.SUITE / "worker.py", inventory]:
                    need(base.left(grant()) >= 35, "too little granted time to stage")
                    run.stage(path, remote, log)
        ssh = run.ssh

        def within_grant(args, *, timeout, **options):
            remaining = base.left(grant()) - 3
            need(remaining > 0, "recovery TIMING window ended")
            return ssh(args, timeout=min(timeout, remaining), **options)

        run.ssh = within_grant
        try:
            for block in base.BLOCKS:
                group = OUT / "blocks" / run.block_prefix(block)
                group.mkdir(parents=True)
                run.write_json(group / "order.json", block)
                for lane in block["order"]:
                    need(base.left(grant()) >= 130, "insufficient booked time for next arm")
                    folder = group / lane
                    folder.mkdir()
                    run.dispatch_arm(folder, run.REMOTE / f"{ID}-{lane}", block, "measured", "mesh")
                    need(run.read_json(folder / "measured-mesh/dispatch.json").get("exit_code") == 0,
                         f"{lane} failed; original and new raw attempts are retained")
                    score_arm(folder / "measured-mesh", config, lane, receipt)
                run.write_json(group / "reconciled.json", base.replay_block(group, block, config, receipt))
        finally:
            run.ssh = ssh


def check(config):
    subprocess.run(["bash", "-n", HERE / "launch.sh"], check=True)
    source = rewrite_observer((ORIGINAL / "observe.py").read_text())
    compile(source, "observe.py", "exec")
    need("/proc/" not in source and 'grant-roster.json' in source, "observer did not remove FD dependency")
    for path in HERE.glob("*.py"):
        compile(path.read_text(), str(path), "exec")
    canary = run.read_json(HERE / "evidence/nonroot-tls.json")
    need(canary["euid"] != 0 and canary["postgres_version_num"] == "170011"
         and canary["tls_rosters"] == [sum(FIRST.values()), sum(BOTH.values())]
         and canary["caps"] == CAPS and canary["matched_direct_libpq_backend_pids"]
         and canary["participant_connections_by_owned_runner"] == [4, 4]
         and len(canary["negative_controls"]) == 4 and all(canary["negative_controls"].values()),
         "untimed nonroot TLS proof changed")
    archived = run.read_json(HERE / "evidence/qualified-roster.json")
    need(archived["first_roster"] == FIRST and archived["both_roster"] == BOTH
         and archived["runner_methods"] == ["owned-launch", "owned-launch"],
         "prior qualified Linux roster proof changed")
    stock = base.historical_score(config)
    need(stock["completed"] == 1024 and stock["checkout_probes"] == 4,
         "public grouped hot arm no longer reconciles")
    print(json.dumps(dict(original_setup_module_sha256=MODULE_SHA, historical_hot=stock["raw_sha256"],
                          qualified_linux=archived, untimed_nonroot=canary), indent=2))


def replay(receipt, config):
    release = run.read_json(OUT / "controller-release.json")
    need(release["id"] == f"{ID}-timing" and release["exit_status"] == 0
         and release["request_sha256"] == run.digest(PACKET), "recovery TIMING has no clean release")
    need(run.read_json(OUT / "series.json") == dict(config=config, schedule=base.BLOCKS),
         "retained series differs from sealed configuration")
    print(json.dumps([base.replay_block(OUT / "blocks" / run.block_prefix(block), block, config, receipt)
                      for block in base.BLOCKS], indent=2))


if __name__ == "__main__":
    if len(sys.argv) != 2 or sys.argv[1] not in ("check", "timing", "replay"):
        raise SystemExit("usage: controller.py check|timing|replay")
    packet()
    receipt, config = prior(), configuration()
    if sys.argv[1] == "check":
        check(config)
    elif sys.argv[1] == "timing":
        timing(receipt, config)
    else:
        replay(receipt, config)
