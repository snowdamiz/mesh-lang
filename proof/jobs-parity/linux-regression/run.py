"""Pack, run and collect one paired regression screen on the macOS host."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import time

from pack import ROOT, SOURCE, pack
from proof_transport import read_serial, unpack


def read_phases(source, record):
    while True:
        offset = source.tell()
        line = source.readline()
        if not line.endswith(b"\n"):
            source.seek(offset)
            return
        if line.startswith(b"FROZEN_DIAGNOSTIC_ARM "):
            record(line.decode().strip())


def run(args):
    args.output.mkdir()
    image = args.output / "guest.cpio"
    serial = args.output / "guest.serial"
    with (ROOT / "target/queue-performance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        launcher = args.output.parent / "boot"
        if not launcher.is_file():
            subprocess.run(["swiftc", "-O", SOURCE / "boot.swift", "-o", launcher], check=True)
            subprocess.run(["codesign", "--force", "--sign", "-", "--entitlements",
                            SOURCE / "virtualization.entitlements", launcher], check=True)
        pack(image, args.profile, args.arms, args.mesh, args.mesh_sha256, args.assets)
        with (serial.open("xb") as output, serial.open("rb") as observer,
              (args.output / "vm.log").open("xb") as vm_log,
              (args.output / "host-load.jsonl").open("x") as load):
            def record(event):
                receipt = {"event": event, "time": time.time(), "load": os.getloadavg(), "cores": os.cpu_count()}
                print(json.dumps(receipt), file=load, flush=True)

            record("boot_start")
            memory = "4" if args.profile == "large" else "2"
            with subprocess.Popen([launcher, args.assets / "kernel.Image", image, memory],
                                  stdout=output, stderr=vm_log) as guest:
                deadline = time.monotonic() + 150
                while guest.poll() is None and time.monotonic() < deadline:
                    read_phases(observer, record)
                    time.sleep(0.05)
                if guest.poll() is None:
                    guest.kill()
                guest_status = guest.wait()
                read_phases(observer, record)
            record("boot_end")
    with serial.open("rb") as stream:
        data, proof_status = read_serial(stream)
    evidence = args.output / "evidence"
    evidence.mkdir()
    unpack(data, evidence)
    return guest_status or proof_status


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", choices=["noop", "slow", "sparse", "short", "large"])
    parser.add_argument("output", type=Path, help="fresh evidence directory")
    parser.add_argument("--mesh", type=Path, required=True, help="release ELF built with the frozen worker fixture")
    parser.add_argument("--mesh-sha256", required=True)
    parser.add_argument("--assets", type=Path, default=ROOT / "target/linux-queue-perf")
    parser.add_argument("--oban-first", action="store_true")
    args = parser.parse_args()
    args.arms = ["oban", "mesh"] if args.oban_first else ["mesh", "oban"]
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
