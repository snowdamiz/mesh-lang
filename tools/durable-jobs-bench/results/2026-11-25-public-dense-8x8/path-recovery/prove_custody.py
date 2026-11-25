"""Exercise the exact stager and worker custody in an isolated Linux filesystem."""
from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import platform
import pwd
import runpy
import stat
import subprocess
import sys
import tempfile
from unittest.mock import patch

from recovery import CASES, HERE, original, remote, run

ROOT = Path(run.REMOTE)
SUITE = original.SUITE
LAUNCHER = original.HERE / "stage/public-native-run"
ATTEMPT = "0123456789abcdef0123456789abcdef"
PROBE = r"""
import os
from pathlib import Path
import sys

out = Path(sys.argv[1])
assert (os.getuid(), os.getgid()) == (65534, 65534)
try:
    assert out.is_absolute() and not out.is_symlink()
except PermissionError:
    print("lstat denied")
    raise SystemExit(13)
assert os.getcwd() == str(out) and out.is_dir()
for path in [out.parent, out.parent.parent, out.parent.parent.parent, out.parent.parent.parent.parent]:
    try:
        list(path.iterdir())
    except PermissionError:
        pass
    else:
        raise AssertionError(f"directory listing allowed: {path}")
for path in [out.parent / "invocation.json", out.parent.parent.parent / "worker.py"]:
    try:
        path.read_bytes()
    except PermissionError:
        pass
    else:
        raise AssertionError(f"read allowed: {path}")
try:
    (out.parent / "forbidden").touch()
except PermissionError:
    pass
else:
    raise AssertionError("write escaped adapter")
with (out / "proof.txt").open("x") as f:
    f.write("uid 65534\n")
print("private write succeeded")
"""


def metadata(path):
    info = path.stat()
    return [info.st_uid, info.st_gid, f"{stat.S_IMODE(info.st_mode):04o}"]


def invoke_worker(lane, block_traversal):
    original_stdout = io.StringIO()
    worker = runpy.run_path(str(lane / "worker.py"))
    entry = worker["main"]

    def custody_only(raw, config, args):
        assert config["engines"][args.engine].get("run_as") is None
        adapter = raw / "adapter"
        assert metadata(adapter) == [0, 0, "0700"]
        assert not any(adapter.iterdir()) and not adapter.is_symlink()
        os.chown(adapter, 65534, 65534)
        adapter.chmod(0o700)
        if block_traversal:
            lane.chmod(0o700)
        with tempfile.TemporaryDirectory(prefix="public-dense-transfer.", dir="/var/tmp") as work:
            stage = Path(work)
            stage.chmod(0o711)
            probe = stage / "public_arm.py"
            probe.write_text(PROBE)
            probe.chmod(0o644)
            command = ["runuser", "-u", "nobody", "--", "env", "-i",
                       "PATH=/usr/sbin:/usr/bin:/bin", "LC_ALL=C.UTF-8",
                       sys.executable, "-B", str(probe), str(adapter)]
            result = subprocess.run(command, cwd=adapter, text=True, capture_output=True)
        assert not result.stderr, result.stderr
        print(result.stdout.strip())
        return result.returncode

    entry.__globals__["run"] = custody_only
    args = [str(lane / "worker.py"), "run", ATTEMPT, "--engine", "mesh", "--profile", "noop",
            "--phase", "diagnostic"]
    with redirect_stdout(original_stdout), patch.object(sys, "argv", args):
        try:
            entry()
        except SystemExit as exit:
            code = exit.code
        else:
            raise AssertionError("worker returned without an exit")
    raw = lane / "raw" / ATTEMPT
    assert run.read_json(raw / "status.json")["exit_code"] == code
    return code, original_stdout.getvalue().strip(), raw


def local_shell(script, **kwargs):
    return subprocess.run(["/bin/bash", "-c", script], **kwargs)


def exercise(lane, config, blocked=False):
    assert not lane.exists()
    with tempfile.TemporaryDirectory(dir="/var/tmp") as work:
        source = Path(work) / "inventory.json"
        run.write_json(source, config)
        with (Path(work) / "stage.log").open("wb") as log, patch.object(run, "ssh", local_shell):
            run.stage(SUITE / "worker.py", lane, log)
            run.stage(source, lane, log)
    before = {str(path): metadata(path) for path in (lane, *lane.parents) if path == ROOT or ROOT in path.parents}
    assert all(mode == [0, 0, "0700"] for path, mode in before.items() if path != str(ROOT))
    assert metadata(lane / "worker.py") == metadata(lane / "inventory.json") == [0, 0, "0600"]
    code, outcome, raw = invoke_worker(lane, blocked)
    adapter = raw / "adapter"
    paths = (adapter, raw, raw.parent, lane, *lane.parents)
    after = {str(path): metadata(path) for path in paths if path == ROOT or ROOT in path.parents}
    assert metadata(adapter) == [65534, 65534, "0700"]
    assert metadata(raw / "invocation.json") == [0, 0, "0600"]
    assert all(metadata(path) == [0, 0, "0711"] for path in (raw, raw.parent, lane.parent))
    if code == 0:
        assert metadata(adapter / "proof.txt") == [65534, 65534, "0600"]
        outsider = subprocess.run(["runuser", "-u", "daemon", "--", "/bin/cat", adapter / "proof.txt"],
                                  text=True, capture_output=True)
        assert outsider.returncode == 1 and "Permission denied" in outsider.stderr
    else:
        assert code == 13 and not list(adapter.iterdir())
    return dict(outcome=outcome, exit_status=code, before=before, after=after)


def main():
    assert platform.system() == "Linux" and os.getuid() == 0
    assert not ROOT.exists(), "run only inside a disposable Linux sandbox"
    runner = pwd.getpwnam("nobody")
    assert (runner.pw_uid, runner.pw_gid) == (65534, 65534)
    launcher = LAUNCHER.read_text()
    assert 'chown nobody:nogroup "$3"' in launcher and 'chmod 0700 "$3"' in launcher
    assert 'runuser -u nobody -- "${command[@]}"' in launcher
    ROOT.mkdir(mode=0o711)
    original_lane = Path(original.remote("activation", "A1"))
    cases = {
        "original": (original_lane, False),
        "released_layout": (ROOT / "2026-11-25-public-dense-3c4032d-transfer/checked-individual", False),
        "recovery_activation": (Path(remote("activation", "A1")), False),
        "recovery_pilot": (Path(remote("pilot", "B")), False),
        "blocked_traversal": (ROOT / "path-recovery-negative/checked-individual", True),
    }
    inv = original.inventory("activation", "A1")
    results = {name: exercise(lane, inv, blocked) for name, (lane, blocked) in cases.items()}
    assert {name: value["outcome"] for name, value in results.items()} == CASES, results
    assert results["original"]["after"][str(original_lane.parent.parent)] == [0, 0, "0700"]
    print(json.dumps(dict(system=platform.system(), machine=platform.machine(),
                          python=platform.python_version(), runner=[runner.pw_uid, runner.pw_gid],
                          worker_sha256=run.digest(SUITE / "worker.py"),
                          launcher_sha256=run.digest(LAUNCHER), cases=results), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
