"""Pin known Linux lifetimes before sampling, authenticate PostgreSQL outside the loop."""
import os
from pathlib import Path
import select
import subprocess
import time


LOGIN = {"parity_subject": "pg_jobs", "compiled_caller": "pg_application",
         "parity_producer": "pg_producer_shared"}


def stamp():
    before = time.monotonic_ns()
    epoch = time.time_ns()
    return [before, epoch, time.monotonic_ns()]


def stat(pid):
    raw = Path(f"/proc/{pid}/stat").read_text()
    head, fields = raw.rsplit(") ", 1)
    reported, comm = head.split(" (", 1)
    values = fields.split()
    if int(reported) != pid or values[0] in {"Z", "X"}:
        raise ProcessLookupError(pid)
    return {"start": int(values[19]), "parent": int(values[1]), "comm": comm,
            "threads": int(values[17]), "mask": sorted(os.sched_getaffinity(pid))}


def live(fd):
    watch = select.poll()
    watch.register(fd, select.POLLIN)
    return not watch.poll(0)


class Snapshot:
    def __init__(self, stack, postmaster, cpus, census, roots):
        start = stamp()
        paths = Path(f"/proc/{postmaster}/task/{postmaster}/children")
        children = [int(pid) for pid in paths.read_text().split()]
        pins = {}
        errors = {}
        for pid in sorted(set(children) | set(roots)):
            fd = None
            try:
                before = stat(pid)
                fd = os.pidfd_open(pid)
                after = stat(pid)
                if before != after or not live(fd):
                    raise ProcessLookupError("identity changed while opening pidfd")
                if after["mask"] != cpus or (pid in children and after["parent"] != postmaster):
                    raise ValueError("unexpected affinity or parent")
            except (OSError, ValueError) as error:
                errors[pid] = f"{type(error).__name__}: {error}"
                if fd is not None:
                    os.close(fd)
                continue
            stack.callback(os.close, fd)
            pins[pid] = (fd, after)
        sql_begin = stamp()
        db = census()
        sql_end = stamp()
        rows = db["rows"]
        by_pid = {row["pid"]: row for row in rows}
        if len(rows) != len(by_pid):
            raise ValueError("duplicate PostgreSQL session PID")
        tracked = {}
        unknown = {}
        for pid, row in by_pid.items():
            label = LOGIN.get(row["login"]) if row["tls"] is True else None
            if not label:
                unknown[pid] = "unclassified TLS login" if row["tls"] is True else "no TLS witness"
                continue
            if (pid not in pins or not live(pins[pid][0]) or pid not in children
                    or pins[pid][1]["comm"] != "postgres" or pins[pid][1]["threads"] != 1):
                unknown[pid] = "no stable postmaster child"
                continue
            tracked[pid] = label
        for pid, label in roots.items():
            if pid not in pins or not live(pins[pid][0]):
                errors[pid] = errors.get(pid, "no live root")
            elif pid in children:
                raise ValueError("root is also a PostgreSQL backend")
            else:
                tracked[pid] = label
        self.fds = {pid: pins[pid][0] for pid in tracked}
        self.record = dict(begin=start, sql_begin=sql_begin, sql_end=sql_end, end=stamp(),
                           postgres=db, unknown=unknown, errors=errors, children=children,
                           roster=[dict(pid=pid, role=label, **pins[pid][1])
                                   for pid, label in sorted(tracked.items())])


def start_sampler(executable, destination, cpus, spare, snapshot):
    # Keep the child order identical to the exported roster even if the SQL
    # snapshot used a different order.
    args = [executable, *map(str, [*cpus, spare])]
    args.extend(str(value) for row in snapshot.record["roster"]
                for value in (row["pid"], snapshot.fds[row["pid"]]))
    with destination.open("xb") as out:
        child = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=out, stderr=subprocess.PIPE,
                                 pass_fds=tuple(snapshot.fds.values()))
    return child


def stop_sampler(child):
    if child.stdin and not child.stdin.closed:
        child.stdin.close()
    try:
        code = child.wait(timeout=8)
        return {"exit": code, "stderr": child.stderr.read().decode()}
    finally:
        child.stderr.close()
        if child.poll() is None:
            child.kill()
            child.wait(timeout=5)
