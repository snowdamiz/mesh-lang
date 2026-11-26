"""Run compiled actor probes with wait4 accounting. Interposition is count-only."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import time

MAIN_CASES = [(hold, width, 2) for hold in (0, 1) for width in (1, 16, 64)]
CASES = MAIN_CASES + [(1, 16, 1), (1, 16, 4)]
ORDERS = [
    ("stock-a", "reuse", "stock-b"),
    ("reuse", "stock-b", "stock-a"),
    ("stock-b", "stock-a", "reuse"),
    ("stock-b", "reuse", "stock-a"),
    ("reuse", "stock-a", "stock-b"),
    ("stock-a", "stock-b", "reuse"),
]


def parse_lines(data, prefix):
    return [json.loads(line[len(prefix):]) for line in data.splitlines() if line.startswith(prefix)]


def run(binary, output, *, total, hold, width, workers, memory, counters):
    env = dict(os.environ, MESH_SCHEDULER_MIN_WORKERS=str(workers),
               MESH_SCHEDULER_MAX_WORKERS=str(workers), PROBE_TOTAL=str(total),
               PROBE_WIDTH=str(width), PROBE_PARK=str(hold), PROBE_MEMORY=str(int(memory)))
    env.pop("DYLD_INSERT_LIBRARIES", None)
    if counters:
        env["DYLD_INSERT_LIBRARIES"] = str(counters)
    load = os.getloadavg()
    start = time.monotonic_ns()
    with output.with_suffix(".stdout").open("xb") as stdout, output.with_suffix(".stderr").open("xb") as stderr:
        child = subprocess.Popen([binary], env=env, stdout=stdout, stderr=stderr, start_new_session=True)
        while True:
            # Block in wait4 during normal work. The alarm is parent-only.
            def timeout_handler(_sig, _frame):
                os.killpg(child.pid, signal.SIGKILL)
            signal.signal(signal.SIGALRM, timeout_handler)
            signal.alarm(90)
            try:
                _, status, usage = os.wait4(child.pid, 0)
                break
            finally:
                signal.alarm(0)
    wall = (time.monotonic_ns() - start) / 1e9
    child.returncode = os.waitstatus_to_exitcode(status)
    out = output.with_suffix(".stdout").read_text()
    err = output.with_suffix(".stderr").read_text()
    work = parse_lines(out, "WORK ")
    phases = parse_lines(out, "PHASE ")
    calls = parse_lines(err, "STACK_CALLS ")
    assert child.returncode == 0 and len(work) == 1, (child.returncode, out, err)
    assert work[0]["completed"] == total and work[0]["ready"] == total * hold
    assert len(calls) == bool(counters)
    if memory:
        assert [p["id"] for p in phases] == [0, 1, 2, 3, 4]
        assert [p["ready"] - p["finished"] for p in phases] == [0, width, 0, width, 0]
    row = dict(total=total, parked=bool(hold), width=width, workers=workers,
               pid=child.pid, exit_code=child.returncode, full_wall_s=wall,
               full_user_s=usage.ru_utime, full_system_s=usage.ru_stime,
               full_minor_faults=usage.ru_minflt, full_major_faults=usage.ru_majflt,
               max_rss_bytes=usage.ru_maxrss * (1 if platform.system() == "Darwin" else 1024),
               load_before=load, load_after=os.getloadavg(), work=work[0], phases=phases, calls=calls)
    output.with_suffix(".json").write_text(json.dumps(row, sort_keys=True) + "\n")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("mode", choices=["baseline", "timing", "memory", "counts"])
    p.add_argument("out", type=Path)
    p.add_argument("stock", type=Path)
    p.add_argument("--reuse", type=Path)
    p.add_argument("--interposer", type=Path)
    p.add_argument("--rounds", type=int, default=6)
    p.add_argument("--total", type=int, default=16384)
    a = p.parse_args()
    a.out.mkdir()
    counters = a.interposer if a.mode == "counts" else None
    memory = a.mode == "memory"
    assert a.total > 64 and a.total % 64 == 0
    assert a.mode == "baseline" or a.reuse
    assert a.mode != "counts" or counters
    totals = a.total if a.mode in ("timing", "baseline") else (4096 if memory else 1024)
    count = a.rounds if a.mode in ("timing", "baseline", "memory") else 1
    cases = [c for c in CASES if c[0]] if memory else CASES
    binaries = {"stock-a": a.stock, "stock-b": a.stock, "reuse": a.reuse}
    plan = []
    for index in range(count):
        cases = cases[1:] + cases[:1]
        order = ORDERS[index % len(ORDERS)]
        if a.mode == "baseline":
            order = tuple(label for label in order if label != "reuse")
        if counters:
            order = ("stock-a", "reuse")
        plan += [(index + 1, *case, label) for case in cases for label in order]
    metadata = dict(started=datetime.now(timezone.utc).isoformat(), host=platform.platform(),
                    logical_cpus=os.cpu_count(), mode=a.mode, total=totals, plan=plan,
                    sha256={str(path.name): hashlib.sha256(path.read_bytes()).hexdigest()
                            for path in {a.stock, a.reuse, counters} if path})
    (a.out / "plan.json").write_text(json.dumps(metadata, indent=2) + "\n")
    for index, hold, width, workers, label in plan:
        stem = f"{index:02d}-park{hold}-live{width:02d}-threads{workers}-{label}"
        run(binaries[label], a.out / stem, total=totals, hold=hold, width=width,
            workers=workers, memory=memory, counters=counters)

if __name__ == "__main__":
    main()
