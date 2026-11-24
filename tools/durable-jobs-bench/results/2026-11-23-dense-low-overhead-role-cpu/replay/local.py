"""Boot the retained ARM64 PostgreSQL fixture with a private replay overlay."""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = next(p for p in HERE.parents if (p / "tools/durable-jobs-bench/verify.py").is_file())
RECOVERY = ROOT / "tools/durable-jobs-bench/results/2026-11-23-ordinary-native-cpu-recovery/replay"
sys.path.insert(0, str(RECOVERY))
spec = importlib.util.spec_from_file_location("prior_boot", RECOVERY / "local-linux.py")
prior = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prior)


def run(out, executable, lock_seconds):
    out.mkdir()
    receipt = {"host_start_load": os.getloadavg(), "kernel": str(prior.VZ / "kernel.Image")}
    with tempfile.TemporaryDirectory(prefix="guest-", dir=out) as temporary:
        work = Path(temporary)
        overlay = work / "root"
        overlay.mkdir()
        for name in ("usr", "lib"):
            shutil.copytree(prior.PYTHON / name, overlay / name, symlinks=True)
        shutil.copyfile(prior.FFI, overlay / "lib/aarch64-linux-gnu/libffi.so.8")
        payload = overlay / "roster"
        payload.mkdir()
        for path in (HERE / "fixture.py", HERE / "observe.py", RECOVERY / "linux-roster-smoke.py",
                     RECOVERY / "dynamic_roster.py", prior.FROZEN / "collector.py",
                     prior.FROZEN / "canonical_worker.py"):
            shutil.copyfile(path, payload / path.name)
        shutil.copyfile(executable, payload / "clock")
        (payload / "clock").chmod(0o755)
        (payload / "max-frame-ns").write_text(str(prior.MAX_FRAME_NS))
        subprocess.run(["openssl", "req", "-new", "-x509", "-newkey", "rsa:2048", "-nodes",
                        "-days", "1", "-subj", "/CN=127.0.0.1",
                        "-addext", "subjectAltName=IP:127.0.0.1",
                        "-keyout", "server.key", "-out", "server.crt"],
                       cwd=payload, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        init = overlay / "init"
        init.write_text(prior.INIT.replace("/roster/linux-roster-smoke.py", "/roster/fixture.py"))
        init.chmod(0o755)
        image = work / "guest.cpio"
        shutil.copyfile(prior.BASE, image)
        with image.open("ab") as bundle, (out / "pack.log").open("xb") as notes:
            paths = "".join(f"{p.relative_to(overlay)}\n" for p in sorted(overlay.rglob("*")))
            subprocess.run(["cpio", "-o", "-H", "newc"], cwd=overlay, input=paths.encode(),
                           stdout=bundle, stderr=notes, check=True)
        source = (prior.VZ / "boot.swift").read_text()
        for old, new in (("config.cpuCount = 2", "config.cpuCount = 4"),
                         (".now() + 90", ".now() + 150")):
            if source.count(old) != 1:
                raise ValueError(f"local VM anchor changed {old}")
            source = source.replace(old, new)
        program = work / "boot.swift"
        program.write_text(source)
        boot = work / "boot"
        subprocess.run(["swiftc", "-module-cache-path", work / "swift-cache", "-o", boot, program],
                       check=True)
        subprocess.run(["codesign", "--force", "--sign", "-", "--entitlements",
                        ROOT / "proof/jobs-parity/linux-regression/virtualization.entitlements", boot],
                       check=True)
        files = [boot, program, prior.VZ / "kernel.Image", prior.BASE, prior.FFI, HERE / "clock.c",
                 *(p for p in payload.iterdir() if p.name != "server.key")]
        with (out / "inputs.sha256").open("x") as hashes:
            for path in files:
                with path.open("rb") as stream:
                    print(hashlib.file_digest(stream, "sha256").hexdigest(), path.name, file=hashes)
        with (ROOT / "target/queue-performance.lock").open("rb") as lock:
            start = time.monotonic()
            deadline = start + lock_seconds
            while True:
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        receipt.update(lock_acquired=False, lock_wait_s=time.monotonic() - start)
                        (out / "host.json").write_text(json.dumps(receipt, indent=2) + "\n")
                        raise TimeoutError("local performance lock stayed occupied") from None
                    time.sleep(1)
            receipt.update(lock_acquired=True, lock_wait_s=time.monotonic() - start,
                           host_boot_load=os.getloadavg())
            try:
                with (out / "serial.log").open("xb") as serial, (out / "vm.log").open("xb") as vm:
                    subprocess.run([boot, prior.VZ / "kernel.Image", image], stdout=serial,
                                   stderr=vm, check=True, timeout=160)
            finally:
                receipt["host_end_load"] = os.getloadavg()
                (out / "host.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if b"ROSTER_SMOKE_EXIT 0" not in (out / "serial.log").read_bytes():
        raise SystemExit("Linux fixture did not pass. See serial.log")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("collector", type=Path)
    parser.add_argument("--lock-seconds", type=int, default=60)
    args = parser.parse_args()
    run(args.output, args.collector, args.lock_seconds)
