"""Check the sealed Python stage under Linux nobody without starting an arm."""
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
import fcntl
import json
import shutil
import subprocess
import tempfile

ROOT = Path.cwd()
TARGET = ROOT / "target/native-dense-wal-cost"
RESULTS = ROOT / "tools/durable-jobs-bench/results"
OLD = RESULTS / "2026-11-24-dense-hot-worker-supply"
NAMES = {
    "wal_capture.py": RESULTS / "2026-11-24-dense-wal-cost",
    "native-arm.py": RESULTS / "2026-11-24-dense-two-runner-checkout",
    "identity.py": RESULTS / "2026-11-24-dense-two-runner-checkout",
    "peer-arm.py": RESULTS / "2026-11-23-dense-matched-topology",
}
PROBE = r"""
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys

stage = Path('/relay-proof/stage')
assert stat.S_IMODE(stage.stat().st_mode) == 0o711 and stage.stat().st_uid == 0

def as_nobody(*args):
    return subprocess.run([sys.executable, '-B', *args], user=65534, group=65534,
                          extra_groups=[], cwd='/', capture_output=True, text=True, timeout=10)

probe = '''
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
import os, stat
stage = Path('/relay-proof/stage')
assert os.getuid() == os.getgid() == 65534 and not os.getgroups()
status = Path('/proc/self/status').read_text()
assert int(next(s.split()[1] for s in status.splitlines() if s.startswith('CapEff:')),16) == 0
try:
    list(stage.iterdir())
except PermissionError:
    pass
else:
    raise AssertionError('staging directory is listable')
spec = spec_from_file_location('staged_wal', stage/'wal_capture.py')
wal = module_from_spec(spec)
spec.loader.exec_module(wal)
for key, filename in [('identity','identity.py'),('checkout','native-arm.py'),('peers','peer-arm.py')]:
    file = stage / filename
    assert file.stat().st_uid == 0 and stat.S_IMODE(file.stat().st_mode) == 0o644
    assert Path(wal.load(key, file).__file__) == file
'''
checked = as_nobody('-c', probe)
if checked.returncode:
    raise RuntimeError(checked.stderr)
mask = os.sched_getaffinity(0)
os.sched_setaffinity(0, {min(mask)})
try:
    for engine in ('mesh', 'go', 'rust', 'oban'):
        rejected = as_nobody(str(stage/'wal_capture.py'), engine, '/unused')
        assert rejected.returncode == 1 and 'diagnostic requires the pinned unprivileged Linux CPU envelope' in rejected.stderr
finally:
    os.sched_setaffinity(0, mask)
print(json.dumps(dict(architecture=os.uname().machine, kernel=os.uname().release,
                      offline=True, uid=65534, gid=65534, stage_mode='0711', file_mode='0644',
                      explicit_imports=['identity.py','native-arm.py','peer-arm.py'],
                      pin_guard_rejected=['mesh','go','rust','oban'],
                      sha256={f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in stage.iterdir()})),
      flush=True)
"""
spec = spec_from_file_location("prior_linux_stage", OLD / "local-linux-stage.py")
vm = module_from_spec(spec)
spec.loader.exec_module(vm)
with tempfile.TemporaryDirectory(prefix="proof-stage-", dir=TARGET) as dirname:
    scratch = Path(dirname)
    overlay = scratch / "overlay"
    stage = overlay / "relay-proof/stage"
    stage.mkdir(parents=True)
    for name in ("usr", "lib"):
        shutil.copytree(vm.PYTHON / name, overlay / name, symlinks=True)
    shutil.copyfile(vm.LIBFFI, overlay / "lib/aarch64-linux-gnu/libffi.so.8")
    for name, directory in NAMES.items():
        shutil.copyfile(directory / name, stage / name)
    (overlay / "linux-stage-test.py").write_text(PROBE)
    init = overlay / "init"
    init.write_text(vm.INIT.replace("LINUX_STAGE_EXIT", "WAL_STAGE_EXIT"))
    init.chmod(0o755)
    cpio = scratch / "stage.cpio"
    shutil.copyfile(vm.VM / "slow-runtime-4k.cpio", cpio)
    with cpio.open("ab") as out:
        members = "\n".join(str(p.relative_to(overlay)) for p in sorted(overlay.rglob("*"))) + "\n"
        subprocess.run(["cpio", "-o", "-H", "newc", "-R", "0:0"], check=True,
                       cwd=overlay, input=members.encode(), stdout=out, stderr=subprocess.PIPE)
    with (ROOT / "target/queue-performance.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = subprocess.run([vm.VM / "boot", vm.VM / "kernel.Image", cpio],
                                capture_output=True, text=True, timeout=90)
    if result.returncode or "WAL_STAGE_EXIT 0" not in result.stdout:
        raise RuntimeError(result.stdout[-4000:] + result.stderr[-4000:])
    receipt, = [line for line in result.stdout.splitlines() if line.startswith('{"architecture"')]
    print(json.dumps(json.loads(receipt), indent=2, sort_keys=True))
