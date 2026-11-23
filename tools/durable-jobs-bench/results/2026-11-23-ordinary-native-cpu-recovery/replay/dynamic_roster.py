"""Linux diagnostic roster using fresh PostgreSQL session snapshots."""
from dataclasses import dataclass
import ctypes
import json
import os
import select
import subprocess
import time

from collector import PROC, ROLES, clock_resolver, stamp

SQL = """SELECT json_build_object('observer', pg_backend_pid(), 'sessions',
coalesce(json_agg(json_build_array(a.pid, a.usename,
  (extract(epoch FROM a.backend_start) * 1000000)::bigint,
  a.application_name, coalesce(s.ssl, false), a.state)), '[]'))
FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s USING (pid)
WHERE a.datname = current_database() AND a.backend_type = 'client backend';"""
PROC_ERRORS = (FileNotFoundError, ProcessLookupError)


def before_deadline(deadline):
    remaining = (deadline - time.monotonic_ns()) / 1e9
    if remaining <= 0:
        raise TimeoutError("roster snapshot exceeded the existing frame limit")
    return remaining


class SessionReader:
    """One autocommit psql connection. A timeout invalidates its protocol."""

    def __init__(self, argv, env, application):
        self.application = application
        self.process = subprocess.Popen(
            [*argv, "-P", "pager=off"], env={**env, "PGAPPNAME": application},
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.buffer = bytearray()
        self.observer_pid = None
        self.failed = False

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=2)
        self.process.stdin.close()
        self.process.stdout.close()

    def read(self, deadline, limit):
        if self.failed:
            raise ValueError("census connection has an unread response")
        self.failed = True
        begin = stamp()
        self.process.stdin.write(SQL.encode() + b"\n")
        self.process.stdin.flush()
        fd = self.process.stdout.fileno()
        while b"\n" not in self.buffer:
            readable, _, _ = select.select([fd], [], [], before_deadline(deadline))
            if not readable:
                raise TimeoutError("session census exceeded the existing frame limit")
            chunk = os.read(fd, 8192)
            if not chunk:
                raise ValueError("session census client exited")
            self.buffer.extend(chunk)
            # PostgreSQL application_name is NAMEDATALEN-bounded. Reject
            # protocol corruption and never collect beyond the server's cap.
            if len(self.buffer) > 1024 + limit * 1024:
                raise ValueError("session census exceeded its response bound")
        line, _, remainder = self.buffer.partition(b"\n")
        self.buffer = bytearray(remainder)
        if self.buffer:
            raise ValueError("unexpected additional session census output")
        result = json.loads(line)
        rows = {}
        for pid, login, started_us, application, tls, state in result["sessions"]:
            if (pid in rows or type(pid) is not int or pid <= 0
                    or type(started_us) is not int or started_us <= 0):
                raise ValueError("unusable or duplicate PostgreSQL session identity")
            rows[pid] = [login, started_us, application, tls, state]
        pid = result["observer"]
        identity = rows.get(pid)
        if (len(rows) > limit or not identity or identity[2] != self.application
                or identity[3] is not True or identity[4] != "active"
                or (self.observer_pid is not None and self.observer_pid != pid)):
            raise ValueError("session census did not prove its own live TLS backend")
        self.observer_pid = pid
        self.failed = False
        return {"begin": begin, "end": stamp(), "observer_pid": pid, "rows": rows}


@dataclass
class Participant:
    pid: int
    role: str
    fd: int
    stat_fd: int
    clock_id: int
    start: int
    parent: int
    comm: str

    @classmethod
    def open(cls, pid, role, cpus, parent=None):
        fd = os.pidfd_open(pid)
        stat_fd = None
        try:
            stat_fd = os.open(PROC / str(pid) / "stat", os.O_RDONLY | os.O_CLOEXEC)
            row = read_stat(pid, stat_fd)
            if os.fstat(stat_fd).st_uid != os.getuid():
                raise ValueError("process UID differs from the unprivileged recorder")
            if parent is not None and row["parent"] != parent:
                raise ValueError("process parent differs from its launch witness")
            if os.sched_getaffinity(pid) != cpus:
                raise ValueError("participant escaped the pinned mask")
            if role == "postmaster" and row["comm"] != "postgres":
                raise ValueError("postmaster witness is not a PostgreSQL process")
            if role.startswith("pg_") and (row["comm"] != "postgres" or row["threads"] != 1):
                raise ValueError("client session is not a PostgreSQL backend process")
            clock_id = ctypes.c_int()
            error = clock_resolver()(pid, ctypes.byref(clock_id))
            if error:
                raise OSError(error, os.strerror(error), pid)
            if select.select([fd], [], [], 0)[0]:
                raise ProcessLookupError(pid)
            return cls(pid, role, fd, stat_fd, clock_id.value,
                       row["start"], row["parent"], row["comm"])
        except BaseException:
            if stat_fd is not None:
                os.close(stat_fd)
            os.close(fd)
            raise

    def close(self):
        os.close(self.stat_fd)
        os.close(self.fd)

    def sample(self, cpus):
        before = read_stat(self.pid, self.stat_fd)
        cpu = time.clock_gettime_ns(self.clock_id)
        after = read_stat(self.pid, self.stat_fd)
        for row in (before, after):
            if (row["start"], row["parent"], row["comm"]) != (self.start, self.parent, self.comm):
                raise ValueError("process changed its kernel identity")
        if os.sched_getaffinity(self.pid) != cpus:
            raise ValueError("participant escaped the pinned mask")
        return {**before, "cpu_ns": cpu, "role": self.role}


def read_stat(pid, fd):
    # Offset zero asks procfs for fresh counters on the same open PID inode.
    data = os.pread(fd, 4096, 0)
    head, delimiter, tail = data.rpartition(b") ")
    pid_text, _, comm = head.partition(b" (")
    fields = tail.split()
    if not delimiter or len(fields) < 20 or int(pid_text) != pid or fields[0] in {b"Z", b"X"}:
        raise ProcessLookupError(pid)
    return {"comm": os.fsdecode(comm), "parent": int(fields[1]), "start": int(fields[19]),
            "user": int(fields[11]), "system": int(fields[12]), "threads": int(fields[17])}


class Roster:
    """The caller owns the runner/broker Popen witnesses and the postmaster."""

    def __init__(self, cpus, postmaster, owner, runner_pids, broker_pid, cap, reader, frame_ns):
        self.cpus = set(cpus)
        self.postmaster = postmaster
        self.reader = reader
        self.cap = cap
        self.frame_ns = frame_ns
        self.fixed = {}
        self.sessions = {}
        roles = {postmaster: ("postmaster", None)}
        if broker_pid in roles:
            raise ValueError("broker PID overlaps its postmaster")
        roles[broker_pid] = ("broker", owner)
        for pid in runner_pids:
            if pid in roles:
                raise ValueError("participant PID overlaps another launch witness")
            roles[pid] = ("runner", owner)
        if len(roles) != 10:
            raise ValueError("expected eight distinct runner witnesses")
        try:
            for pid, (role, parent) in roles.items():
                self.fixed[pid] = Participant.open(pid, role, self.cpus, parent)
        except BaseException:
            self.close()
            raise

    def close(self):
        for item in (*self.fixed.values(), *self.sessions.values()):
            item.close()
        self.fixed.clear()
        self.sessions.clear()

    @staticmethod
    def identity(row):
        # state may change between active/idle. It does not assign the login.
        login, started_us, _, tls, _ = row
        return login, started_us, tls

    def frame(self):
        begin = stamp()
        deadline = begin[0] + self.frame_ns
        left = self.reader.read(deadline, self.cap)
        left_rows = left["rows"]
        retired = set(self.sessions) - {
            (pid, self.identity(row)) for pid, row in left_rows.items()}
        for key in retired:
            self.sessions.pop(key).close()
        candidates = dict(self.fixed)
        unavailable = {}
        unclassified = {}
        for pid, row in left_rows.items():
            if pid in self.fixed:
                raise ValueError("SQL backend PID overlaps a fixed participant")
            role = "pg_observer" if pid == left["observer_pid"] else ROLES.get(row[0])
            if role is None or row[3] is not True:
                unclassified[pid] = row
                continue
            key = (pid, self.identity(row))
            try:
                if key not in self.sessions:
                    self.sessions[key] = Participant.open(pid, role, self.cpus, self.postmaster)
                candidates[pid] = self.sessions[key]
            except PROC_ERRORS:
                unavailable[pid] = "unavailable while binding pidfd"
        collected = {}
        polls = select.poll()
        by_fd = {}
        for pid, subject in candidates.items():
            before_deadline(deadline)
            polls.register(subject.fd, select.POLLIN)
            by_fd[subject.fd] = pid
            try:
                collected[pid] = subject.sample(self.cpus)
            except PROC_ERRORS:
                unavailable[pid] = "unavailable while sampling process clock"
        right = self.reader.read(deadline, self.cap)
        right_rows = right["rows"]
        for fd, events in polls.poll(0):
            if events:
                unavailable[by_fd[fd]] = "pidfd exited before the closing census"
        changed = {pid for pid in left_rows.keys() | right_rows.keys()
                   if pid not in left_rows or pid not in right_rows
                   or self.identity(left_rows[pid]) != self.identity(right_rows[pid])}
        before_deadline(deadline)
        if unavailable.keys() & self.fixed.keys():
            raise ValueError("fixed witness left before the closing census")
        processes = {pid: row for pid, row in collected.items()
                     if pid not in changed and pid not in unavailable}
        return {"begin": begin, "end": stamp(), "left": left, "right": right,
                "processes": processes, "changed_sessions": sorted(changed),
                "unavailable": unavailable, "unclassified": unclassified}
