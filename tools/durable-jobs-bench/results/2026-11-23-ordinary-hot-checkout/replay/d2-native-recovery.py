"""Recover the sealed D2-only native proof from its already linked artifact."""

import argparse
import fcntl
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
from subprocess import SubprocessError
import sys
from tempfile import TemporaryDirectory
from uuid import uuid4


STATE = Path(__file__).resolve().parent
LANE = STATE.parent / "ordinary-hot-checkout"
spec = spec_from_file_location("d2_sealed", LANE / "native/dispatch.py")
d2 = module_from_spec(spec)
spec.loader.exec_module(d2)
run = d2.run
ID = "ordinary-hot-checkout-buildfree"
FREEZE = "41cc580d7da06d106f24e7da8eaa8ba71d44f95589fc8725a83ef694114b9e67"
ATTEMPT = "ordinary-hot-checkout-1393b4a009b8"
INITIAL_IDENTITY = LANE / "evidence" / ATTEMPT / "native-identity.txt"
IDENTITY_SHA = "0af1c1104644b740e30ced0f97c950c42764fd928be2a80efe68ad7369349ff9"


def normalize(records):
    boot, kernel, disk, ticks, *files = records.splitlines()
    return [boot, kernel, disk.strip(), ticks, *files]


def retained_configuration(frozen):
    run.require(run.digest(INITIAL_IDENTITY) == IDENTITY_SHA, "original build receipt changed")
    records = normalize(INITIAL_IDENTITY.read_text())
    original = run.read_json(d2.NATIVE / "original-short-series.json")["config"]
    with TemporaryDirectory(dir=LANE / "tmp", prefix="d2-buildfree-config-") as work:
        path = Path(work) / "identity.txt"
        path.write_text("\n".join(records) + "\n")
        config = d2.qualified_config(original, path, ATTEMPT, frozen["profiles"])
    return config, records


def qualify(config, records):
    d2.grant("SETUP", ID, 120)
    nonce = uuid4().hex[:12]
    attempt = f"{ID}-{nonce}"
    output = LANE / "evidence" / attempt
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        d2.grant("SETUP", ID, 120)
        output.mkdir()
        print(output, flush=True)
        try:
            command = d2.interleave.HOST_IDLE + '''
cat /proc/sys/kernel/random/boot_id
uname -r
findmnt -no MAJ:MIN --target /var/lib/durable-jobs-bench/native
getconf CLK_TCK
sha256sum "$@"
'''
            files = [record.split()[1] for record in records[4:]]
            identity = output / "native-identity.txt"
            with identity.open("xb") as stdout, (output / "identity.log").open("xb") as stderr:
                run.ssh(["bash", "-c", command, ID, *files], stdout=stdout, stderr=stderr,
                        timeout=60).check_returncode()
            run.require(normalize(identity.read_text()) == records,
                        "cached D2 host, toolchain or launch files changed")
            run.write_json(output / "config.json", config)
            for label in d2.LABELS:
                d2.grant("SETUP", ID, 45)
                lane = config["lanes"][label]
                folder = output / "short" / label
                folder.mkdir(parents=True)
                pin = folder / "inventory.json"
                run.write_json(pin, lane["inventory"])
                remote = run.REMOTE / f"{attempt}-{label}"
                with (folder / "stage.log").open("xb") as log:
                    run.stage(run.SUITE / "worker.py", remote, log)
                    run.stage(pin, remote, log)
                engine = lane["engine"]
                run.dispatch_arm(folder, remote, {"profile": "short"}, "diagnostic", engine)
                score = d2.proof_score(folder / f"diagnostic-{engine}", config, "diagnostic", label, "short")
                run.write_json(folder / "reconciled.json", score)
        except (OSError, ValueError, KeyError, RuntimeError, SubprocessError) as error:
            run.write_json(output / "status.json", dict(status=1, freeze=FREEZE, error=str(error)))
            raise
        run.write_json(output / "status.json", dict(status=0, freeze=FREEZE,
                       config_sha256=run.digest(output / "config.json"), original_identity=IDENTITY_SHA))


def accepted_configuration():
    release = run.read_json(STATE / "requests" / f"{ID}.released.json")
    run.require(release["exit_status"] == 0, "D2 cached proof has no successful release")
    for receipt in sorted((LANE / "evidence").glob(f"{ID}-*/status.json")):
        status = run.read_json(receipt)
        path = receipt.with_name("config.json")
        if status["status"] == 0 and status["freeze"] == FREEZE and status["config_sha256"] == run.digest(path):
            return run.read_json(path)
    raise ValueError("the exact cached D2 image has no four-lane qualification")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "setup", "timing", "summarize"))
    parser.add_argument("--group", choices=range(1, 6), type=int)
    args = parser.parse_args()
    freeze = d2.seal(FREEZE)
    d2.interleave.plan = d2.plan
    d2.interleave.replay = d2.replay
    if args.action in {"check", "setup"}:
        config, files = retained_configuration(freeze)
        if args.action == "setup":
            qualify(config, files)
        else:
            print("D2 cached image and exact four-lane config verified offline")
    elif args.action == "summarize":
        sys.exit(d2.interleave.summarize(LANE / "evidence" / d2.SERIES))
    else:
        run.require(args.group is not None, "specify a granted timing group")
        d2.grant("TIMING", d2.SERIES, 60)
        sys.exit(d2.interleave.dispatch(LANE / "evidence" / d2.SERIES, accepted_configuration(), [args.group]))
