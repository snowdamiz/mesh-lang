"""Test the exact staged Python entry as Linux nobody under root-owned 0711."""
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys

FILES = ("stage-arm.py", "native-arm.py", "identity.py")


def main(stage):
    assert sys.platform == "linux" and os.getuid() == 0
    for folder in (stage, stage.parent):
        assert folder.stat().st_uid == 0 and stat.S_IMODE(folder.stat().st_mode) == 0o711
    for name in FILES:
        path = stage / name
        assert path.stat().st_uid == 0 and stat.S_IMODE(path.stat().st_mode) == 0o644

    def nobody(*args):
        return subprocess.run([sys.executable, "-B", *args], user=65534, group=65534,
                              extra_groups=[], env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
                              cwd="/", capture_output=True, text=True, timeout=10)

    evidence = nobody("-c", """
import importlib.util, os, subprocess, sys
from pathlib import Path
stage = Path(sys.argv[1])
assert os.getuid() == os.getgid() == 65534 and not os.getgroups()
status = Path('/proc/self/status').read_text().splitlines()
assert int(next(line.split()[1] for line in status if line.startswith('CapEff:')), 16) == 0
try:
    list(stage.iterdir())
except PermissionError:
    assert (stage / 'identity.py').read_bytes()
else:
    raise AssertionError('private staging directory was listable')
spec = importlib.util.spec_from_file_location('staged', stage / 'stage-arm.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
with subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']) as child:
    try:
        frame = module.pg_cpu(os.getpid())['processes']
        assert frame[str(child.pid)]['parent'] == os.getpid()
        assert frame[str(os.getpid())]['threads'] == 1
    finally:
        child.terminate()
""", str(stage))
    evidence.check_returncode()

    # Deliberately miss the two-CPU gate. This exits before any native source,
    # binary, PostgreSQL connection, or workload can be touched.
    mask = os.sched_getaffinity(0)
    try:
        os.sched_setaffinity(0, {min(mask)})
        control = nobody(str(stage / "native-arm.py"), "d2-on", "/unused", "/unused", "/unused")
        assert control.returncode == 1 and "ModuleNotFoundError: No module named 'identity'" in control.stderr
        for workers in ("2", "4"):
            selected = nobody(str(stage / "stage-arm.py"), workers, "/unused", "/unused", "/unused")
            assert selected.returncode == 1 and "ValueError: native CPU envelope changed" in selected.stderr
            assert "ModuleNotFoundError" not in selected.stderr
    finally:
        os.sched_setaffinity(0, mask)
    print(json.dumps(dict(uid=65534, gid=65534, stage_mode="0711", file_mode="0644",
                          effective_caps=0, off_clock=True, kernel=os.uname().release,
                          proc_cpu_nobody_child=True,
                          architecture=os.uname().machine, failed_plain_import=True,
                          explicit_import_reached_cpu_guard=[2, 4],
                          sha256={name: hashlib.sha256((stage / name).read_bytes()).hexdigest()
                                  for name in FILES}), sort_keys=True), flush=True)


if __name__ == "__main__":
    main(Path(sys.argv[1]))
