"""Execute the two sealed launcher entry points inside Linux, without timing."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys

ROOT = Path("/relay-regression")
STAGE = ROOT / "staged"
ENTRY = STAGE / "native-arm.py"
NAMES = ("native-arm.py", "identity.py", "native-dense.sh", "native-dense-import-retry.sh")


def main():
    for parent in (ROOT, STAGE):
        assert parent.stat().st_uid == 0 and stat.S_IMODE(parent.stat().st_mode) == 0o711
    for name in NAMES:
        assert stat.S_IMODE((STAGE / name).stat().st_mode) == 0o644
    original = (STAGE / "native-dense.sh").read_text()
    retry = (STAGE / "native-dense-import-retry.sh").read_text()
    interpreter = "/usr/bin/python3 -B"
    assert original.split(interpreter, 1)[0] == retry.split(interpreter, 1)[0]
    assert original.split(interpreter, 1)[1].strip() == '"$here/native-arm.py" "$mode" "$source" "$runner" "$out"'
    code, = re.findall(r"/usr/bin/python3 -B -c '([^']+)' \"\$here/native-arm.py\"", retry)

    # Both paths stop before any PostgreSQL inspection, even if imports succeed.
    os.sched_setaffinity(0, {min(os.sched_getaffinity(0))})
    def probe(*args):
        return subprocess.run([sys.executable, "-B", *args], user=65534, group=65534,
                              extra_groups=[], env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
                              cwd="/", capture_output=True, text=True, timeout=10)

    witness = probe("-c", """
import os, stat, sys
from pathlib import Path
p = Path(sys.argv[1])
assert os.getuid() == os.getgid() == 65534
assert os.getgroups() == []
assert int(next(r.split()[1] for r in Path('/proc/self/status').read_text().splitlines()
                if r.startswith('CapEff:')), 16) == 0
assert stat.S_IMODE(p.stat().st_mode) == 0o711
try:
    list(p.iterdir())
except PermissionError:
    assert (p / 'identity.py').read_bytes()
    print('directory listing denied; explicit file read succeeded')
else:
    raise AssertionError('staging directory was listable')
""", str(STAGE))
    assert witness.returncode == 0, witness
    arguments = [str(ENTRY), "d2-off", "/unused", "/unused", "/unused"]
    old = probe(*arguments)
    repaired = probe("-c", code, *arguments)
    old_error = "ModuleNotFoundError: No module named 'identity'"
    new_error = "ValueError: native CPU envelope changed"
    assert old.returncode == 1 and old_error in old.stderr, old
    assert repaired.returncode == 1 and new_error in repaired.stderr and "ModuleNotFoundError" not in repaired.stderr, repaired
    print(json.dumps(dict(kernel=os.uname().release, architecture=os.uname().machine,
                          python=sys.version.split()[0], uid=65534, gid=65534, capabilities=0,
                          stage_mode="0711", file_mode="0644", witness=witness.stdout.strip(),
                          old_trace=old.stderr, repaired_trace=repaired.stderr,
                          stopped_before_benchmark=True, sha256={
                              name: hashlib.sha256((STAGE / name).read_bytes()).hexdigest()
                              for name in NAMES}), sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
