"""Run the roster smoke in the repository's pre-existing local ARM64 VM."""
import argparse
import fcntl
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from recover import FROZEN
from reduce import MAX_FRAME_NS, ROOT

HERE = Path(__file__).resolve().parent
VZ = ROOT / "target/linux-queue-perf"
PYTHON = ROOT / "target/jobs-perf-work/linux-proof-tuned/overlay"
BASE = VZ / "slow-runtime-4k.cpio"
FFI = ROOT / "target/ordinary-native-cpu/local-vz-dependency/libffi.so.8"
INIT = '''#!/usr/bin/python3 -B
import ctypes
import fcntl
import os
from pathlib import Path
import socket
import struct
import subprocess
import traceback

libc = ctypes.CDLL(None, use_errno=True)
code = 1
try:
    if libc.mount(None, b"/proc", b"proc", 0, None):
        raise OSError(ctypes.get_errno(), "mount proc")
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        request = struct.pack("16sH22x", b"lo", 0)
        _, flags = struct.unpack("16sH22x", fcntl.ioctl(sock, 0x8913, request))
        fcntl.ioctl(sock, 0x8914, struct.pack("16sH22x", b"lo", flags | 1))
    work = Path("/fixture")
    work.mkdir()
    work.chmod(0o700)
    os.chown(work, 65534, 65534)
    key = work / "server.key"
    key.write_bytes(Path("/roster/server.key").read_bytes())
    key.chmod(0o600)
    os.chown(key, 65534, 65534)
    Path("/roster/server.key").unlink()
    code = subprocess.call(["/usr/bin/python3", "-B", "/roster/linux-roster-smoke.py"],
                           cwd=work, user=65534, group=65534, extra_groups=[],
                           env={**os.environ, "HOME": "/fixture", "USER": "nobody"})
except BaseException:
    traceback.print_exc()
finally:
    print(f"ROSTER_SMOKE_EXIT {code}", flush=True)
    libc.reboot(0x4321FEDC)
'''


def main(out, cpus):
    out.mkdir()
    with tempfile.TemporaryDirectory(prefix="guest-", dir=out) as tmp:
        work = Path(tmp)
        overlay = work / "root"
        overlay.mkdir()
        for path in ("usr", "lib"):
            shutil.copytree(PYTHON / path, overlay / path, symlinks=True)
        shutil.copyfile(FFI, overlay / "lib/aarch64-linux-gnu/libffi.so.8")
        payload = overlay / "roster"
        payload.mkdir()
        for source in (HERE / "linux-roster-smoke.py", HERE / "dynamic_roster.py",
                       FROZEN / "collector.py", FROZEN / "canonical_worker.py"):
            shutil.copyfile(source, payload / source.name)
        (payload / "max-frame-ns").write_text(str(MAX_FRAME_NS))
        subprocess.run(["openssl", "req", "-new", "-x509", "-newkey", "rsa:2048",
                        "-nodes", "-days", "1", "-subj", "/CN=127.0.0.1",
                        "-addext", "subjectAltName=IP:127.0.0.1",
                        "-keyout", "server.key", "-out", "server.crt"],
                       cwd=payload, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        init = overlay / "init"
        init.write_text(INIT)
        init.chmod(0o755)
        image = work / "guest.cpio"
        shutil.copyfile(BASE, image)
        with image.open("ab") as bundle:
            paths = "".join(f"{p.relative_to(overlay)}\n" for p in sorted(overlay.rglob("*")))
            subprocess.run(["cpio", "-o", "-H", "newc"], cwd=overlay, input=paths.encode(),
                           stdout=bundle, check=True)
        boot = VZ / "boot"
        if cpus != 2:
            source = (VZ / "boot.swift").read_text()
            anchor = "config.cpuCount = 2"
            if source.count(anchor) != 1:
                raise ValueError("local VM CPU anchor changed")
            boot = work / "boot"
            program = work / "boot.swift"
            program.write_text(source.replace(anchor, f"config.cpuCount = {cpus}"))
            subprocess.run(["swiftc", "-module-cache-path", work / "swift-cache",
                            "-o", boot, program], check=True)
            entitlements = work / "entitlements.plist"
            with entitlements.open("xb") as plist:
                subprocess.run(["codesign", "-d", "--entitlements", ":-", VZ / "boot"],
                               stdout=plist, stderr=subprocess.DEVNULL, check=True)
            subprocess.run(["codesign", "--force", "--sign", "-",
                            "--entitlements", entitlements, boot], check=True)
        with (out / "inputs.sha256").open("x") as receipt:
            for path in (boot, VZ / "boot.swift", VZ / "kernel.Image", BASE, PYTHON / "usr/bin/python3",
                         FFI, payload / "collector.py", payload / "canonical_worker.py",
                         payload / "dynamic_roster.py", payload / "linux-roster-smoke.py"):
                with path.open("rb") as stream:
                    print(hashlib.file_digest(stream, "sha256").hexdigest(), path.name, file=receipt)
        with (ROOT / "target/queue-performance.lock").open("rb") as lock:
            deadline = time.monotonic() + 60
            while True:
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        raise TimeoutError("local Linux proof lock stayed occupied") from None
                    time.sleep(1)
            with (out / "serial.log").open("xb") as serial, (out / "vm.log").open("xb") as vm:
                subprocess.run([boot, VZ / "kernel.Image", image],
                               stdout=serial, stderr=vm, check=True, timeout=120)
    result = (out / "serial.log").read_bytes()
    sys.stdout.buffer.write(result)
    if b"ROSTER_SMOKE_EXIT 0" not in result:
        raise SystemExit(1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--cpus", type=int, choices=(2, 4), default=4)
    args = parser.parse_args()
    main(args.output, args.cpus)
