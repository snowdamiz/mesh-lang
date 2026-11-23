"""GNU AMD64 component. The standalone relay receipts are synthetic."""
import argparse
from array import array
from concurrent.futures import ThreadPoolExecutor
from hashlib import sha256
from io import BytesIO
import json
import os
from pathlib import Path
import resource
import signal
import socket
import struct
import subprocess
from threading import Barrier
import time

SELECTOR = "--mesh-internal-job-attempt-effect-v1"
ENV = {"MESH_DIAGNOSTIC_JOB_DISABLE_THP": "1",
       "MESH_SCHEDULER_MIN_WORKERS": "1", "MESH_SCHEDULER_MAX_WORKERS": "1"}
WIDTH = 64
WAVES = 4
ORDERS = [
    ("stock-a", "lean", "stock-b"),
    ("lean", "stock-b", "stock-a"),
    ("stock-b", "stock-a", "lean"),
    ("stock-b", "lean", "stock-a"),
    ("lean", "stock-a", "stock-b"),
]


def frame(obj):
    body = json.dumps(obj, separators=(",", ":"), ensure_ascii=False).encode()
    return struct.pack("<I", len(body)) + body


def receive(stream):
    raw = stream.read(4)
    if not raw:
        return None
    assert len(raw) == 4, raw
    size, = struct.unpack("<I", raw)
    assert 0 < size <= 65536, size
    body = stream.read(size)
    assert len(body) == size, (len(body), size)
    return json.loads(body)


def usage(info):
    return dict(user_us=round(info.ru_utime * 1e6), system_us=round(info.ru_stime * 1e6),
                minor_faults=info.ru_minflt, major_faults=info.ru_majflt,
                peak_rss_kib=info.ru_maxrss, voluntary_switches=info.ru_nvcsw,
                involuntary_switches=info.ru_nivcsw)


def boot_ms():
    return time.clock_gettime_ns(time.CLOCK_BOOTTIME) // 1_000_000


def attempt(image, fingerprint, key, barrier, *, fault=None, hold=0):
    control, stdin = socket.socketpair()
    effect, endpoint = socket.socketpair()
    with control, stdin, effect, endpoint:
        control.settimeout(6)
        effect.settimeout(6)
        begin = time.monotonic_ns()
        proc = subprocess.Popen([str(image), SELECTOR], stdin=stdin, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, process_group=0, env=ENV)
        status = None
        try:
            stdin.close()
            control.sendmsg([b"\1"], [(socket.SOL_SOCKET, socket.SCM_RIGHTS,
                                       array("i", [endpoint.fileno()]))])
            endpoint.close()
            if barrier is not None:
                barrier.wait(timeout=5)
            now = boot_ms()
            deadline = now + (200 if fault == "deadline" else 6000)
            lease = now + (200 if fault == "lease" else 8000)
            request = dict(handler=fingerprint, deadline=deadline, lease_deadline=lease)
            context = dict(attempt=1, args=dict(key=key, payload="m" * 1024,
                                               hold_ms=hold, fail_first=fault == "handler-error"))
            control.sendall(frame(request))
            # Context uses the original length-framed UTF-8 worker ABI.
            control.sendall(frame(context))
            effects = []
            first_ns = None
            second_ns = None
            with effect.makefile("rb") as stream:
                if fault != "handler-error":
                    first = receive(stream)
                    first_ns = time.monotonic_ns()
                    assert first["name"] == "first", first
                    values = first["values"]
                    assert len(values) == 4 and values[0] == key and values[1] > 0, first
                    assert values[2:] == [proc.pid, False], first
                    effects.append(first)
                    effect.sendall(b"\1")
                    if fault == "parent-eof":
                        control.shutdown(socket.SHUT_WR)
                    elif fault == "signal":
                        os.kill(proc.pid, signal.SIGTERM)
                    if fault in (None, "signal", "unknown-commit"):
                        second = receive(stream)
                        second_ns = time.monotonic_ns()
                        assert second["name"] == "second" and second["values"] == [key, False], second
                        effects.append(second)
                        if fault == "unknown-commit":
                            effect.shutdown(socket.SHUT_RDWR)
                        else:
                            effect.sendall(b"\1")
                # Reading to EOF (with a deadline) also detects extra calls.
                if fault != "unknown-commit":
                    assert receive(stream) is None
            pid, status, info = os.wait4(proc.pid, 0)
            end = time.monotonic_ns()
            proc.returncode = os.waitstatus_to_exitcode(status)
            assert pid == proc.pid
            output = proc.stdout.read()
            errors = proc.stderr.read().decode()
            expected = {"deadline": 171, "lease": 172, "parent-eof": 173}.get(fault, 0)
            assert proc.returncode == expected, (fault, proc.returncode, errors)
            if expected:
                assert not output and len(effects) == 1
            else:
                result = receive(BytesIO(output))
                if fault == "handler-error":
                    assert json.loads(result["Ok"]) == {
                        "action": "fail", "reason": "named effect handler failed"}
                elif fault == "unknown-commit":
                    # The second write was never acknowledged by the relay.
                    assert "Err" in result, result
                else:
                    assert json.loads(result["Ok"]) == {"action": "complete"}, result
            if hold and second_ns:
                assert second_ns - first_ns >= (hold - 2) * 1_000_000
            return dict(pid=pid, wait_status=status, start_ns=begin, first_ns=first_ns,
                        second_ns=second_ns, reaped_ns=end, effects=effects, stderr=errors,
                        **usage(info))
        finally:
            if status is None:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                proc.wait()
            proc.stdout.close()
            proc.stderr.close()


def batch(image, fingerprint, name, count, proof=False):
    before = usage(resource.getrusage(resource.RUSAGE_SELF))
    start = time.monotonic_ns()
    rows = []
    with ThreadPoolExecutor(WIDTH) as pool:
        for wave in range(count):
            ready = Barrier(WIDTH)
            work = [pool.submit(attempt, image, fingerprint, f"{name}-{wave}-{i}", ready)
                    for i in range(WIDTH)]
            wave_rows = [task.result() for task in work]
            assert all(not row["stderr"] for row in wave_rows), wave_rows
            rows.extend(dict(row, wave=wave) for row in wave_rows)
    finish = time.monotonic_ns()
    after = usage(resource.getrusage(resource.RUSAGE_SELF))
    parent = {key: after[key] - before[key] for key in before if key != "peak_rss_kib"}
    parent["lifetime_peak_rss_kib"] = after["peak_rss_kib"]
    assert len({row["pid"] for row in rows}) == len(rows)
    return dict(parent=parent, wall_ns=finish - start, rows=rows)


def witness(image, fingerprint):
    held = Barrier(2)
    with ThreadPoolExecutor(2) as pool:
        a, b = [pool.submit(attempt, image, fingerprint, f"témoin-{i}", held, hold=35)
                for i in range(2)]
        rows = [a.result(), b.result()]
    rows.append(attempt(image, fingerprint, "second-exec", None))
    addresses = set()
    for row in rows:
        pid = row["pid"]
        lines = row["stderr"].splitlines()
        ctor_a = lines[0].split()
        assert ctor_a[:3] == ["LEAN_CTOR_A", str(pid), str(pid)], row
        assert lines[1:] == [f"LEAN_CTOR_B {pid} {pid + 4}", f"LEAN_THREAD_TLS {pid} 917",
                             f"LEAN_TIMERFD {pid} BOOTTIME ABSTIME CLOEXEC",
                             f"LEAN_HANDLER {pid} {pid + 4}"], row
        addresses.add(ctor_a[3])
    assert len(addresses) == len(rows), rows
    assert max(row["start_ns"] for row in rows[:2]) < min(row["reaped_ns"] for row in rows[:2])
    return rows


def main(args):
    assert os.uname().machine == "x86_64" and os.getuid() != 0
    assert len(os.sched_getaffinity(0)) == 2
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    closure = json.loads((args.build / "closure.json").read_text())
    fingerprint = closure["fingerprint"]
    images = {"stock-a": args.stock, "stock-b": args.stock, "lean": args.build / "mesh-lean-worker"}
    hashes = {key: sha256(val.read_bytes()).hexdigest() for key, val in images.items()}
    assert hashes["lean"] == closure["worker_sha256"]
    with args.output.open("x") as output:
        def retain(**row):
            print(json.dumps(row, ensure_ascii=False), file=output, flush=True)

        retain(kind="system", boot=Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
               hashes=hashes, affinity=sorted(os.sched_getaffinity(0)), uname=list(os.uname()))
        if args.mode == "correctness":
            # The parent owns a dummy credential and CLOEXEC fd. Neither
            # is inherited by the child, including in its constructors.
            os.environ["MESH_EFFECT_APP_URL"] = "fixture-private-parent-only"
            with open("/dev/null", "rb") as fd:
                assert not os.get_inheritable(fd.fileno())
                retain(kind="constructors", rows=witness(args.build / "mesh-lean-witness", fingerprint))
            for arm in ("stock-a", "lean"):
                for fault in ("deadline", "lease", "parent-eof", "signal", "handler-error", "unknown-commit"):
                    row = attempt(images[arm], fingerprint, f"contrôle-{fault}", None, fault=fault,
                                  hold=35 if fault == "signal" else
                                  800 if fault in ("deadline", "lease", "parent-eof") else 0)
                    retain(kind="fault", arm=arm, fault=fault, **row)
            return
        for arm in ("stock-a", "lean"):
            retain(kind="warmup", arm=arm, **batch(images[arm], fingerprint, f"{arm}-warmup", 1))
        for rotation, order in enumerate(ORDERS, 1):
            for arm in order:
                retain(kind="sample", rotation=rotation, arm=arm, load=os.getloadavg(),
                       **batch(images[arm], fingerprint, f"r{rotation}-{arm}", WAVES))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["correctness", "timing"])
    parser.add_argument("stock", type=Path)
    parser.add_argument("build", type=Path)
    parser.add_argument("output", type=Path)
    main(parser.parse_args())
