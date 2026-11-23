"""Test the staged Linux worker import and default process CPU clock."""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

import worker as collector

HERE = Path(__file__).resolve().parent
CHILD = """
import sys, time
print("ready", flush=True)
sys.stdin.readline()
stop = time.process_time_ns() + 25_000_000
while time.process_time_ns() < stop:
    pass
print("worked", flush=True)
sys.stdin.readline()
"""


def main():
    frozen = (HERE / "canonical_worker.py").resolve(strict=True)
    assert Path(collector.worker.__file__).resolve() == frozen
    assert Path(collector.worker.run.__code__.co_filename).resolve() == frozen
    with frozen.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    assert digest == sys.argv[1], digest
    print(json.dumps({"canonical_worker": str(frozen), "sha256": digest, "python": platform.python_version(),
                      "kernel": platform.release(), "machine": platform.machine()}), flush=True)
    assert isinstance(collector.process_clock(os.getpid()), int)
    assert collector.frozen_worker_sha256() == digest
    help_result = subprocess.run([sys.executable, "-B", str(HERE / "worker.py"), "--help"],
                                 check=True, capture_output=True, text=True)
    assert "export" in help_result.stdout and "--phase" in help_result.stdout
    credentials = {"user": 65534, "group": 65534, "extra_groups": ()} if os.getuid() == 0 else {}
    with subprocess.Popen([sys.executable, "-B", "-c", CHILD], stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, text=True, **credentials) as child:
        try:
            assert child.stdout.readline() == "ready\n"
            status = Path(f"/proc/{child.pid}/status").read_text().splitlines()
            child_uid = int(next(line for line in status if line.startswith("Uid:")).split()[1])
            assert child_uid == credentials.get("user", os.getuid())
            before = collector.sample_process(child.pid)
            assert child.stdin.write("\n") == 1
            child.stdin.flush()
            assert child.stdout.readline() == "worked\n"
            after = collector.sample_process(child.pid)
            assert after["start"] == before["start"]
            assert after["cpu_ns"] - before["cpu_ns"] >= 20_000_000

            def exit_during_clock(pid):
                absolute = collector.process_clock(pid)
                child.stdin.close()
                assert child.wait(timeout=5) == 0
                return absolute

            try:
                collector.sample_process(child.pid, cpu_clock=exit_during_clock)
            except ProcessLookupError as exc:
                assert "exited during CPU sample" in str(exc), str(exc)
            else:
                raise AssertionError("charged a process that exited inside the clock sample")
            try:
                collector.sample_process(child.pid)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError("charged a reaped child")
            try:
                collector.process_clock(child.pid)
            except OSError as exc:
                assert exc.errno, exc
            else:
                raise AssertionError("libc returned a valid clock for a reaped child")
            print(json.dumps({"cpu_progress_ns": after["cpu_ns"] - before["cpu_ns"],
                              "stable_starttime": before["start"], "child_uid": child_uid,
                              "reporter_uid": os.getuid(),
                              "exit_during_sample": "rejected", "after_reap": "rejected"}),
                  flush=True)
        finally:
            if child.poll() is None:
                child.kill()


if __name__ == "__main__":
    main()
