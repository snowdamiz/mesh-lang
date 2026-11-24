"""Run the off-clock stage check using the repository's existing Linux guest."""
import fcntl
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
VM = ROOT / "target/linux-queue-perf"
PYTHON = ROOT / "target/jobs-perf-work/linux-proof-tuned/overlay"
LIBFFI = ROOT / "target/ordinary-native-cpu/local-vz-dependency/libffi.so.8"
PRIOR = HERE.parent / "2026-11-24-dense-two-runner-checkout"
INIT = """#!/usr/bin/python3 -B
import ctypes
import subprocess
from pathlib import Path
libc = ctypes.CDLL(None, use_errno=True)
try:
    if libc.mount(None, b'/proc', b'proc', 0, None):
        raise OSError(ctypes.get_errno(), 'mount proc failed')
    for path in (Path('/relay-proof'), Path('/relay-proof/stage')):
        path.chmod(0o711)
    for path in Path('/relay-proof/stage').iterdir():
        path.chmod(0o644)
    code = subprocess.call(['/usr/bin/python3', '-B', '/linux-stage-test.py', '/relay-proof/stage'])
    print(f'LINUX_STAGE_EXIT {code}', flush=True)
finally:
    libc.reboot(0x4321FEDC)
"""


def main():
    with tempfile.TemporaryDirectory(prefix="dense-supply-stage-", dir=ROOT / "target") as scratch:
        scratch = Path(scratch)
        overlay = scratch / "overlay"
        stage = overlay / "relay-proof/stage"
        stage.mkdir(parents=True)
        for name in ("usr", "lib"):
            shutil.copytree(PYTHON / name, overlay / name, symlinks=True)
        shutil.copyfile(LIBFFI, overlay / "lib/aarch64-linux-gnu/libffi.so.8")
        for name in ("native-arm.py", "identity.py"):
            shutil.copyfile(PRIOR / name, stage / name)
        shutil.copyfile(HERE / "stage-arm.py", stage / "stage-arm.py")
        shutil.copyfile(HERE / "linux-stage-test.py", overlay / "linux-stage-test.py")
        init = overlay / "init"
        init.write_text(INIT)
        init.chmod(0o755)
        image = scratch / "stage.cpio"
        shutil.copyfile(VM / "slow-runtime-4k.cpio", image)
        with image.open("ab") as stream:
            paths = "\n".join(str(path.relative_to(overlay)) for path in sorted(overlay.rglob("*"))) + "\n"
            subprocess.run(["cpio", "-o", "-H", "newc", "-R", "0:0"], check=True,
                           cwd=overlay, input=paths.encode(), stdout=stream, capture_output=False)
        with (ROOT / "target/queue-performance.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = subprocess.run([VM / "boot", VM / "kernel.Image", image],
                                    capture_output=True, text=True, timeout=100)
        if result.returncode or "LINUX_STAGE_EXIT 0" not in result.stdout:
            raise RuntimeError(result.stdout[-4000:] + result.stderr[-4000:])
        witness, = [line for line in result.stdout.splitlines() if line.startswith('{"architecture"')]
        print(json.dumps(json.loads(witness), sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
