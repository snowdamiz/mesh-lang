"""Measure the Mach CPU tick conversion against Python's process CPU clock."""

import ctypes
import json
import os
import platform
import sys
import time

from arm import cpu_reader


class Timebase(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


if __name__ == "__main__":
    if sys.platform != "darwin":
        raise SystemExit("calibration requires Darwin")
    ratio = Timebase()
    if ctypes.CDLL(None).mach_timebase_info(ctypes.byref(ratio)) != 0 or not ratio.denom:
        raise SystemExit("mach_timebase_info failed")
    read = cpu_reader()
    pid = os.getpid()
    rows = []
    for _ in range(3):
        before = read({pid})["pids"][pid]
        start = time.process_time_ns()
        while time.process_time_ns() - start < 100_000_000:
            sum(range(100))
        clock_ns = time.process_time_ns() - start
        after = read({pid})["pids"][pid]
        ticks = sum(after[key] - before[key] for key in ("user_ns", "system_ns"))
        estimate_ns = ticks * ratio.numer / ratio.denom
        if abs(estimate_ns / clock_ns - 1) > 0.02:
            raise SystemExit("Mach and process CPU clocks did not agree after scaling")
        rows.append({"raw_ticks": ticks, "clock_ns": clock_ns, "scaled_ns": estimate_ns})
    print(json.dumps({"system": platform.system(), "kernel": platform.release(),
                      "architecture": platform.machine(), "numer": ratio.numer,
                      "denom": ratio.denom, "checks": rows}, indent=2, sort_keys=True))
