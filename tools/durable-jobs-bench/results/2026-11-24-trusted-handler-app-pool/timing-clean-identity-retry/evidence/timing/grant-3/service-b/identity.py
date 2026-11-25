"""Observe a pinned, directly launched child on Linux without ptrace privileges."""
import hashlib
import os
from pathlib import Path
import stat


def file_stamp(info):
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns


def pinned_file(path, expected_sha256):
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise RuntimeError(f"noncanonical executable path: {path}")
    if any(os.access(entry, os.W_OK) for entry in (path, *path.parents)):
        raise RuntimeError(f"benchmark user can replace executable: {path}")
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW), "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or stream.read(4) != b"\x7fELF":
            raise RuntimeError(f"not a regular ELF executable: {path}")
        stream.seek(0)
        if hashlib.file_digest(stream, "sha256").hexdigest() != expected_sha256:
            raise RuntimeError(f"executable digest mismatch: {path}")
        stamp = file_stamp(before)
        if file_stamp(os.fstat(stream.fileno())) != stamp or file_stamp(path.lstat()) != stamp:
            raise RuntimeError(f"executable changed during digest: {path}")
    return stamp


def process_stamp(process):
    pid = process.pid
    if process.poll() is not None:
        raise RuntimeError(f"{pid}: owned child is no longer alive")
    proc = Path("/proc") / str(pid)
    head, tail = (proc / "stat").read_text().rsplit(")", 1)
    fields = tail.split()
    parent, start = int(fields[1]), int(fields[19])
    if (head.split(" ", 1)[0] != str(pid) or fields[0] in {"Z", "X", "x"}
            or parent != os.getpid() or start <= 0):
        raise RuntimeError(f"{pid}: expected a live direct child with a start stamp")
    return parent, start


def observe(process, path, verb, sha256, executable_stamp, launch_stamp, cpus):
    pid = process.pid
    argv = [str(path), verb]
    before = process_stamp(process)
    if before != launch_stamp or process.args != argv:
        raise RuntimeError(f"{pid}: owned launch identity changed")
    proc = Path("/proc") / str(pid)
    if (proc / "cmdline").read_bytes() != b"".join(os.fsencode(arg) + b"\0" for arg in argv):
        raise RuntimeError(f"{pid}: live argv differs from the direct launch")
    denied = None
    try:
        executable = os.readlink(proc / "exe")
    except PermissionError as error:
        executable, denied = None, error.errno
    if executable is not None and executable != str(path):
        raise RuntimeError(f"{pid}: kernel executable differs from the pinned launch")
    mask = sorted(os.sched_getaffinity(pid))
    if mask != cpus or len(cpus) != 2:
        raise RuntimeError(f"{pid}: CPU affinity differs from the two-CPU trial")
    if pinned_file(path, sha256) != executable_stamp:
        raise RuntimeError(f"{pid}: executable replaced since launch")
    after = process_stamp(process)
    if before != after or process.poll() is not None:
        raise RuntimeError(f"{pid}: owned child exited or changed during readiness")
    return dict(pid=pid, argv=argv, parent_pid=before[0], launch_start_ticks=launch_stamp[1],
                start_ticks_before=before[1], start_ticks_after=after[1], cpus=mask,
                executable_sha256=sha256, observed_executable=executable, exe_errno=denied,
                method="owned-launch" if executable is None else "proc-exe")
