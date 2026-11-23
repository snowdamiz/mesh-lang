"""Boot the existing local ARM64 Linux sandbox with the exact staged worker."""
import argparse
import fcntl
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from diagnose import stage_worker

HERE = Path(__file__).resolve().parent
ROOT = next(parent for parent in HERE.parents if (parent / "tools/durable-jobs-bench/verify.py").is_file())
VZ = ROOT / "target/linux-queue-perf"
PYTHON = ROOT / "target/jobs-perf-work/linux-proof-tuned/overlay"
BASE = VZ / "slow-runtime-4k.cpio"
INIT = '''#!/usr/bin/python3 -B
import ctypes
import subprocess
from pathlib import Path
libc = ctypes.CDLL(None, use_errno=True)
try:
    if libc.mount(None, b"/proc", b"proc", 0, None):
        raise OSError(ctypes.get_errno(), "mount proc")
    sha = Path("/clock/canonical.sha256").read_text().strip()
    code = subprocess.call(["/usr/bin/python3", "-B", "/clock/clock-smoke.py", sha])
    print(f"CLOCK_SMOKE_EXIT {code}", flush=True)
finally:
    libc.reboot(0x4321FEDC)
'''


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run_guest(out, ffi, collector):
    out.mkdir()
    with tempfile.TemporaryDirectory(prefix="vz-clock-", dir=out) as location:
        temp = Path(location)
        payload = temp / "overlay"
        payload.mkdir()
        shutil.copytree(PYTHON / "usr", payload / "usr", symlinks=True)
        shutil.copytree(PYTHON / "lib", payload / "lib", symlinks=True)
        library = payload / "lib/aarch64-linux-gnu/libffi.so.8"
        shutil.copyfile(ffi, library)
        sample = payload / "clock"
        sample.mkdir()
        stage_worker(sample, "on")
        if collector != HERE / "collector.py":
            shutil.copyfile(collector, sample / "worker.py")
        shutil.copyfile(HERE / "clock-smoke.py", sample / "clock-smoke.py")
        (sample / "canonical.sha256").write_text(digest(ROOT / "tools/durable-jobs-bench/worker.py") + "\n")
        init = payload / "init"
        init.write_text(INIT)
        init.chmod(0o755)
        image = temp / "guest.cpio"
        with image.open("wb") as bundle, BASE.open("rb") as base:
            shutil.copyfileobj(base, bundle)
            bundle.flush()
        with image.open("ab") as bundle:
            paths = "".join(f"{path.relative_to(payload)}\n" for path in sorted(payload.rglob("*")))
            subprocess.run(["cpio", "-o", "-H", "newc"], cwd=payload,
                           input=paths.encode(), stdout=bundle, check=True)
        with (out / "inputs.sha256").open("x") as receipt:
            for path in (VZ / "boot", VZ / "kernel.Image", BASE, PYTHON / "usr/bin/python3",
                         ffi, collector, sample / "canonical_worker.py", image):
                print(digest(path), path.name, file=receipt)
        with (ROOT / "target/queue-performance.lock").open("rb") as lock:
            deadline = time.monotonic() + 60
            while True:
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        raise TimeoutError("local sandbox stayed locked")
                    time.sleep(1)
            with (out / "serial.log").open("xb") as serial, (out / "vm.log").open("xb") as vm:
                subprocess.run([VZ / "boot", VZ / "kernel.Image", image],
                               stdout=serial, stderr=vm, check=True, timeout=100)
    data = (out / "serial.log").read_bytes()
    sys.stdout.buffer.write(data)
    if b"CLOCK_SMOKE_EXIT 0" not in data:
        raise SystemExit(1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("libffi", type=Path)
    parser.add_argument("--collector", type=Path, default=HERE / "collector.py")
    args = parser.parse_args()
    run_guest(args.output.resolve(), args.libffi.resolve(), args.collector.resolve())
