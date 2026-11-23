"""Stage and verify the separate equal-topology peer adapters."""

import argparse
from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path
import subprocess
import sys
import tarfile

from matrix import PACKET, QUEUE, SERIES, SUITE, WORK, run, require, validate

REMOTE = Path("/var/lib/durable-jobs-bench/dense-matched-topology-v1")
SETUP = "dense-matched-topology-peer-setup"
ADAPTERS = ("peer-arm.py", "native-oban.sh")
BINARY = "river-2x32"
APALIS = "durable-jobs-apalis-bench"


def peer(original, engine, pins):
    script = REMOTE / "peer-arm.py"
    verified = dict(original["verified_files"])
    verified[str(script)] = pins["files"][script.name]
    stock = original["argv"]
    out = dict(original, verified_files=verified)
    if engine == "oban":
        launcher = Path(stock[2]).with_name("arm.py")
        wrapper = REMOTE / "native-oban.sh"
        verified[str(wrapper)] = pins["files"][wrapper.name]
        out["argv"] = [*stock[:2], "/bin/bash", str(wrapper), str(launcher)]
    else:
        launcher = Path(stock[0]).with_name("run.py")
        out["argv"] = ["/usr/bin/python3", "-B", str(script), engine, str(launcher), *stock[1:]]
        if engine == "go":
            binary = REMOTE / BINARY
            out["argv"][out["argv"].index("--binary") + 1] = str(binary)
            verified[str(binary)] = out["artifact_sha256"] = pins["files"][binary.name]
        else:
            binary = launcher.parents[3] / "target/linode-apalis/build/release" / APALIS
            out["argv"].extend(["--binary", str(binary)])
            verified[str(binary)] = original["artifact_sha256"]
    return out, []


def make_adapter_readable(_):
    """The immutable shared setup stage is already readable by the benchmark UID."""


def check_config(config):
    pins = config["river_equal"]
    receipt = run.read_json(PACKET / "river-build.json")
    require(pins["stock_sha256"] == receipt["stock_rebuild_sha256"] == config["inventory"]["engines"]["go"]["artifact_sha256"]
            and pins["files"][BINARY] == receipt["candidate_sha256"], "River build pins differ")
    require(set(pins["files"]) == {*ADAPTERS, BINARY}, "unexpected equal-topology staged files")
    for name in ADAPTERS:
        require(run.digest(PACKET / name) == pins["files"][name], f"altered equal-topology launcher: {name}")
    subprocess.run(["bash", "-n", PACKET / "native-oban.sh"], check=True)


def publish_config(build):
    config = run.read_json(PACKET / "stock.json")
    config["description"] = "Ordinary V1 and stock peers at two native runner processes with 32 slots each, plus the V1 eight-runner control."
    config["lanes"] = {label if lane["engine"] == "mesh" else label.replace("8x8", "2x32"):
                      dict(lane, runners=lane["runners"] if lane["engine"] == "mesh" else 2)
                      for label, lane in config["lanes"].items()}
    config["river_equal"] = dict(stock_sha256=build["stock_rebuild_sha256"],
                                 files={BINARY: build["candidate_sha256"],
                                        **{name: run.digest(PACKET / name) for name in ADAPTERS}})
    check_config(config)
    validate(config, "equal")
    run.write_json(PACKET / "equal.json", config)


INSTALL = """
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
test "$(cat /proc/sys/kernel/random/boot_id)" = "$1"
test "$(uname -r)" = "$2"
dest="$3"
shift 3
umask 077
mkdir "$dest"
tar -xf - -C "$dest" --no-same-owner --no-same-permissions
cd "$dest"
printf '%s  %s\n' "$@" | sha256sum -c -
chmod 0644 peer-arm.py native-oban.sh
chmod 0755 river-2x32 "$dest"
"""


def assignment():
    import re

    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (QUEUE / "STATUS.md").read_text(), flags=re.M)
    require(events and events[-1][:2] == ("SETUP", SETUP), "no active controller SETUP assignment")
    _, _, start, end = events[-1]
    require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "outside SETUP assignment")
    request = PACKET / "request-peer-setup.json"
    require(request.read_bytes() == (QUEUE / f"{SETUP}.json").read_bytes(), "SETUP request seal differs")
    for path, pin in run.read_json(request)["local_sha256"].items():
        require(run.digest(SUITE.parents[1] / path) == pin, f"altered SETUP file: {path}")
    return dict(start=start, end=end, sha256=run.digest(request))


def setup(binary, receipts):
    grant = assignment()
    config = run.read_json(PACKET / "equal.json")
    check_config(config)
    pins = config["river_equal"]["files"]
    require(run.digest(binary) == pins[BINARY], "staged Linux River differs")
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        assignment()
        receipts.mkdir()
        run.write_json(receipts / "grant.json", grant)
        with tarfile.open(receipts / "transfer.tar", "w") as archive:
            for name in ADAPTERS:
                archive.add(PACKET / name, arcname=name, recursive=False)
            archive.add(binary, arcname=BINARY, recursive=False)
        checks = [value for name, checksum in pins.items() for value in (checksum, name)]
        with (receipts / "transfer.tar").open("rb") as source, (receipts / "setup.log").open("xb") as log:
            result = run.ssh(["/bin/bash", "-c", INSTALL, "dense-setup", *config["host"][:2], str(REMOTE), *checks],
                             stdin=source, stdout=log, stderr=log, timeout=90)
        run.write_json(receipts / "setup.json", dict(exit=result.returncode, remote=str(REMOTE), files=pins,
                                                      transfer_sha256=run.digest(receipts / "transfer.tar")))
        return result.returncode


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("freeze", "setup"))
    parser.add_argument("binary", type=Path, nargs="?")
    args = parser.parse_args()
    if args.action == "freeze":
        publish_config(run.read_json(PACKET / "river-build.json"))
    else:
        if args.binary is None:
            parser.error("supply the pinned Linux River binary")
        sys.exit(setup(args.binary, WORK / "peer-setup"))
