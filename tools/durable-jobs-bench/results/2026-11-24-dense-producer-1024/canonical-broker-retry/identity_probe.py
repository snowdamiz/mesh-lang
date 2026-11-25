"""Untimed nobody controls for the canonical broker binding and file/launch proof."""
import hashlib
import json
import os
from pathlib import Path
import runpy
import select
import shutil
import subprocess
import sys

STAGE = Path("/var/lib/durable-jobs-bench/native/source-dense-producer-1024-09b9395cf5c6")
BROKER = STAGE / "broker/mesh"
PACKET = STAGE / "packet-canonical-broker"


def expect_error(label, call, match):
    try:
        call()
    except RuntimeError as exc:
        assert match in str(exc), (label, str(exc))
        return str(exc)
    raise AssertionError(f"{label} passed")


def nobody():
    os.setgroups([])
    os.setgid(65534)
    os.setuid(65534)
    assert os.getuid() == os.getgid() == 65534
    assert sorted(os.sched_getaffinity(0)) == [0, 1]


def main():
    sys.path.insert(0, str(PACKET))
    scope = runpy.run_path(str(PACKET / "adapter.py"))
    path = scope["broker_path"]
    pin = scope["pinned_file"]
    observe = scope["observe"]
    stamp = scope["process_stamp"]
    BROKER.parent.mkdir()
    shutil.copyfile("/bin/sleep", BROKER)
    BROKER.chmod(0o555)
    digest = hashlib.sha256(BROKER.read_bytes()).hexdigest()
    alternate = BROKER.with_name("same-bytes")
    shutil.copyfile(BROKER, alternate)
    alternate.chmod(0o555)

    # The actual native failure kept this spelling in the child argv.
    os.environ["DENSE_BROKER"] = str(PACKET / "../broker/mesh")
    assert str(path.__globals__["HERE"]) == str(PACKET)
    common = expect_error("dotdot", path, "must name the installed stage executable")
    os.environ["DENSE_BROKER"] = str(BROKER)

    def child(stage):
        nobody()
        if stage == "symlink":
            result = expect_error("target symlink", lambda: pin(path(), digest), "noncanonical executable path")
            print(json.dumps(dict(symlink=result), sort_keys=True), flush=True)
            return
        if stage == "ancestor":
            result = expect_error("ancestor symlink", lambda: pin(path(), digest), "noncanonical executable path")
            print(json.dumps(dict(ancestor=result), sort_keys=True), flush=True)
            return
        before = pin(path(), digest)
        with subprocess.Popen([str(path()), "30"]) as proc:
            try:
                start = stamp(proc)
                good = observe(proc, path(), "30", digest, before, start, [0, 1])
                assert good["pid"] == proc.pid and good["argv"] == [str(BROKER), "30"]
                require_hash = expect_error("wrong hash", lambda: pin(path(), "0" * 64), "digest mismatch")
                wrong_argv = expect_error("wrong argv",
                    lambda: observe(proc, path(), "31", digest, before, start, [0, 1]), "launch identity changed")
                wrong_launch = expect_error("wrong launch",
                    lambda: observe(proc, path(), "30", digest, before, (start[0], start[1] + 1), [0, 1]),
                    "launch identity changed")
                print(json.dumps(dict(canonical=True, uid=os.getuid(), gid=os.getgid(), dotdot=common,
                                      wrong_hash=require_hash, wrong_argv=wrong_argv,
                                      wrong_launch=wrong_launch), sort_keys=True), flush=True)
                os.write(started_w, b"x")
                assert os.read(replaced_r, 1) == b"x"
                replaced = expect_error(
                    "same bytes, new inode", lambda: observe(proc, path(), "30", digest, before, start, [0, 1]),
                    "executable")
                assert "kernel executable differs" in replaced or "replaced since launch" in replaced
                print(json.dumps(dict(replaced=replaced), sort_keys=True), flush=True)
            finally:
                proc.terminate()

    def fork(test):
        pid = os.fork()
        if pid == 0:
            try:
                child(test)
            except BaseException:
                import traceback
                traceback.print_exc()
                os._exit(1)
            os._exit(0)
        return pid

    def success(pid):
        assert os.waitpid(pid, 0)[1] == 0

    started_r, started_w = os.pipe()
    replaced_r, replaced_w = os.pipe()
    pid = fork("launch")
    assert select.select([started_r], [], [], 10)[0] and os.read(started_r, 1) == b"x"
    os.replace(alternate, BROKER)
    os.write(replaced_w, b"x")
    success(pid)
    BROKER.rename(alternate)
    BROKER.symlink_to(alternate)
    success(fork("symlink"))
    BROKER.unlink()
    alternate.rename(BROKER)
    backing = STAGE / "other-broker"
    BROKER.parent.rename(backing)
    BROKER.parent.symlink_to(backing)
    success(fork("ancestor"))


if __name__ == "__main__":
    main()
