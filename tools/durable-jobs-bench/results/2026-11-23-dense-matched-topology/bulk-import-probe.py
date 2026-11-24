"""Exercise the exact bulk import entry points as Linux nobody with no workload."""

import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys

STAGE = Path("/bulk-import/staged")
FILES = ("native-bulk.sh", "native-bulk-import-retry.sh", "bulk-arm.py", "arm.py")


def probe(*args):
    return subprocess.run([sys.executable, "-B", *args], user=65534, group=65534,
                          extra_groups=[], cwd="/", env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
                          capture_output=True, text=True, timeout=10)


def main():
    for path in (STAGE.parent, STAGE):
        assert path.stat().st_uid == 0 and stat.S_IMODE(path.stat().st_mode) == 0o711
    for name in FILES:
        path = STAGE / name
        assert path.stat().st_uid == 0 and stat.S_IMODE(path.stat().st_mode) == 0o644
    original, retry = [(STAGE / name).read_text() for name in FILES[:2]]
    interpreter = "/usr/bin/python3 -B"
    assert original.split(interpreter, 1)[0] == retry.split(interpreter, 1)[0]
    arguments = '"$here/bulk-arm.py" "$source" "$out" "$producer" "$mode"'
    assert original.split(interpreter, 1)[1].strip() == arguments
    code, = re.findall(r"-c '([^']+)' " + re.escape(arguments), retry)
    witness = probe("-c", """
import os, sys
from pathlib import Path
p = Path(sys.argv[1])
assert os.getuid() == os.getgid() == 65534 and os.getgroups() == []
assert next(row.split()[1] for row in Path('/proc/self/status').read_text().splitlines()
            if row.startswith('CapEff:')) == '0000000000000000'
try:
    list(p.iterdir())
except PermissionError:
    assert all((p / name).read_bytes() for name in ('arm.py', 'bulk-arm.py'))
else:
    raise AssertionError('staging directory was listable')
""", str(STAGE))
    assert witness.returncode == 0, witness.stderr
    entry = str(STAGE / "bulk-arm.py")
    old = probe(entry, "--help")
    repaired = probe("-c", code, entry, "--help")
    assert old.returncode == 1 and "ModuleNotFoundError: No module named 'arm'" in old.stderr, old
    assert repaired.returncode == 0 and "{scalar,bulk}" in repaired.stdout and not repaired.stderr, repaired
    print(json.dumps(dict(kernel=os.uname().release, architecture=os.uname().machine,
                          uid=65534, gid=65534, capabilities=0, stage_mode="0711", file_mode="0644",
                          directory_listing_denied=True, explicit_reads_pass=True,
                          original_exit=old.returncode, original_error=old.stderr.splitlines()[-1],
                          retry_exit=repaired.returncode, retry_help=repaired.stdout,
                          before_workload=True, sha256={
                              name: hashlib.sha256((STAGE / name).read_bytes()).hexdigest()
                              for name in FILES}), sort_keys=True))


if __name__ == "__main__":
    main()
