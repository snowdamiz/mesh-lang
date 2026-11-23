"""Spare-CPU procfs recorder, based on the matrix role-worker."""
from collections import Counter
import json
import os
from pathlib import Path
import threading
import time

import canonical_worker as worker

PROC = Path("/proc")
ROLES = {"parity_subject": "pg_jobs", "compiled_caller": "pg_application",
         "parity_producer": "pg_producer"}
PERIOD_NS = 20_000_000


def stamp():
    first = time.monotonic_ns()
    epoch = time.time_ns()
    return [first, epoch, time.monotonic_ns()]


def pg_cpu(pid, proc=PROC):
    """Read thread-group utime/stime. A comm field can itself contain ')'."""
    raw = (proc / str(pid) / "stat").read_text()
    head, fields = raw.rsplit(") ", 1)
    values = fields.split()
    pid_text, comm = head.split(" (", 1)
    if int(pid_text) != int(pid) or values[0] in {"X", "Z"}:
        raise ValueError(f"process {pid} is unavailable")
    return {"comm": comm, "state": values[0], "parent": int(values[1]),
            "start": int(values[19]), "user": int(values[11]),
            "system": int(values[12]), "threads": int(values[17])}


def children(pid, proc):
    return set((proc / str(pid) / "task" / str(pid) / "children").read_text().split())


def cmdline(pid, proc):
    return (proc / str(pid) / "cmdline").read_bytes().rstrip(b"\0").split(b"\0")


def publish_registry(adapter, engine, cpus, executable, proc):
    masks = json.loads((adapter / "process-affinity.json").read_text())
    published = json.loads((adapter / "postgres-tls-sessions.json").read_text())
    if len(masks) != 9 or any(mask != cpus for mask in masks.values()):
        raise ValueError("expected a postmaster and eight published runner PIDs on the trial mask")
    postmaster, = [pid for pid in masks if pg_cpu(pid, proc)["comm"] == "postgres"]
    runners = set(masks) - {postmaster}
    parent, = {pg_cpu(pid, proc)["parent"] for pid in runners}
    if engine == "mesh":
        executable = os.fsencode(executable)
        for pid in runners:
            if cmdline(pid, proc) != [executable, b"run"]:
                raise ValueError(f"published runner {pid} does not have the pinned argv")
        # V1 writes exact runner PIDs but does not log the broker's PID.
        # Select the unique sibling of the pinned runners with exact session argv.
        candidates = []
        for pid in children(parent, proc):
            try:
                if cmdline(pid, proc) == [executable, b"session"]:
                    candidates.append(pid)
            except FileNotFoundError:
                continue
        broker, = candidates
        try:
            sink = (proc / broker / "fd/1").readlink()
        except PermissionError:
            sink = None  # shared root may not have CAP_SYS_PTRACE for nobody
        if sink is not None and sink != adapter / "broker-output.log":
            raise ValueError("ordinary session stdout differs from the dedicated broker log")
        witness = {"argv": [os.fsdecode(executable), "session"], "parent_pid": parent,
                   "stdout_verified": sink is not None, "method": "exact sibling argv"}
    else:
        broker = str(json.loads((adapter / "oban-broker-emulator.json").read_text())["pid"])
        witness = {"method": "published Oban broker PID", "parent_pid": parent}
    if broker in masks or pg_cpu(broker, proc)["parent"] != parent:
        raise ValueError("broker is not a distinct child of the published runners' owner")
    registry = {str(row["backend"]): ROLES[row["role"]] for row in published}
    expected = ({"pg_jobs": 32, "pg_application": 32, "pg_producer": 1} if engine == "mesh"
                else {"pg_jobs": 80, "pg_producer": 1})
    if (len(registry) != len(published) or not all(row["tls"] is True for row in published)
            or Counter(registry.values()) != expected or set(registry) & (set(masks) | {broker})):
        raise ValueError(f"unexpected TLS role census: {dict(Counter(registry.values()))}")
    return {"postmaster": postmaster, "backend_roles": registry, "runner_pids": sorted(runners),
            "broker_pid": broker, "broker_witness": witness}


def verify_roster(registry, engine, proc):
    seen = {}
    for pid in children(registry["postmaster"], proc):
        try:
            title = cmdline(pid, proc)[0].decode(errors="replace").split()
        except FileNotFoundError:
            continue
        if len(title) >= 3 and title[0] == "postgres:" and title[2] == engine:
            seen[pid] = ROLES.get(title[1], f"unregistered:{title[1]}")
    if seen != registry["backend_roles"]:
        raise ValueError("live PostgreSQL client PIDs or role titles differ from the TLS census")


def host_ticks(cpus, proc):
    rows = {}
    begin = stamp()
    for line in (proc / "stat").read_text().splitlines():
        name, *values = line.split()
        if name in cpus:
            rows[name] = [int(value) for value in values[:8]]
    return {"begin": begin, "end": stamp(), "ticks": rows}


def process_clock(pid):
    return time.clock_gettime_ns(time.clock_getcpuclockid(int(pid)))


def capture_frame(adapter, engine, cpus, executable, registry, proc=PROC, cpu_clock=process_clock):
    begin = stamp()
    host = host_ticks({f"cpu{cpu}" for cpu in cpus}, proc)
    frame = {"begin": begin, "host": host}
    if engine != "go":
        try:
            if registry is None and (adapter / "postgres-tls-sessions.json").exists():
                registry = publish_registry(adapter, engine, cpus, executable, proc)
            if registry is not None:
                verify_roster(registry, engine, proc)
                roster = {**registry["backend_roles"],
                          **dict.fromkeys(registry["runner_pids"], "runner"),
                          registry["broker_pid"]: "broker",
                          registry["postmaster"]: "postmaster"}
                frame["processes"] = {}
                for pid, role in roster.items():
                    row = pg_cpu(pid, proc)
                    if role.startswith("pg_") and (row["comm"] != "postgres" or
                            str(row["parent"]) != registry["postmaster"] or row["threads"] != 1):
                        raise ValueError(f"backend {pid} changed PostgreSQL identity")
                    if role in {"runner", "broker"} and row["parent"] != registry["broker_witness"]["parent_pid"]:
                        raise ValueError(f"process {pid} left the runner owner")
                    try:
                        cpu_ns = cpu_clock(pid)
                    except OSError as error:
                        raise ValueError(f"process CPU clock unavailable for {pid}") from error
                    if proc == PROC and os.sched_getaffinity(int(pid)) != set(cpus):
                        raise ValueError(f"process {pid} left the pinned CPU mask")
                    frame["processes"][pid] = dict(row, role=role, cpu_ns=cpu_ns)
                verify_roster(registry, engine, proc)
        except (LookupError, OSError, ValueError) as error:
            frame["roster_error"] = f"{type(error).__name__}: {error}"
    frame["end"] = stamp()
    return frame, registry


def record(raw, cpus, engine, executable, halt, issues):
    try:
        spare = min(os.sched_getaffinity(0) - set(cpus))
        os.sched_setaffinity(0, {spare})
        with (raw / "role-frames.jsonl").open("x", buffering=1) as output:
            header = {"format": 1, "hz": os.sysconf("SC_CLK_TCK"), "spare_cpu": spare, "cpus": cpus,
                      "engine": engine, "period_ns": PERIOD_NS}
            output.write(json.dumps(header) + "\n")
            registry = None
            while not halt.is_set():
                frame, discovered = capture_frame(raw / "adapter", engine, cpus, executable, registry)
                frame["reporter_thread_cpu_ns"] = time.thread_time_ns()
                if registry is None and discovered is not None:
                    frame["registry"] = discovered
                    registry = discovered
                output.write(json.dumps(frame, separators=(",", ":")) + "\n")
                elapsed = time.monotonic_ns() - frame["begin"][0]
                halt.wait(max(0, PERIOD_NS - elapsed) / 1e9)
    except Exception as error:
        issues.append(error)


def capture(raw, config, args, run_arm):
    if args.phase != "diagnostic":
        raise ValueError("role CPU capture is diagnostic-only")
    files = config["engines"][args.engine]["verified_files"]
    executable = next((path for path in files if path.endswith("/mesh")), None) if args.engine == "mesh" else None
    halt = threading.Event()
    issues = []
    thread = threading.Thread(target=record, args=(raw, config["cpus"], args.engine, executable, halt, issues))
    thread.start()
    try:
        code = run_arm(raw, config, args)
    finally:
        halt.set()
        thread.join()
    if issues:
        worker.write_json(raw / "role-recorder-error.json", {"error": str(issues[0])})
        return code or 1
    return code


if __name__ == "__main__":
    canonical_run = worker.run

    def instrumented(raw, config, args):
        return capture(raw, config, args, canonical_run)

    worker.run = instrumented
    worker.main()
