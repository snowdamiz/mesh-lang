"""macOS per-process CPU sampler for the local producer screens.

Replaces target/large-args-domain/replay.py, which an external cleanup
deleted. Same source (proc_pid_rusage user+system ticks scaled by the
mach timebase). calibrate_cpu() checks the scale against process_time.
"""
import ctypes as C
import os
import time

_PROC_RUSAGE = C.CDLL("/usr/lib/libproc.dylib").proc_pid_rusage
_RUSAGE_INFO_V2 = 2
# rusage_info_v2: 16-byte uuid, then uint64 fields; user/system time are 2 and 3.
_FIELDS = 2 + 18

TIMEBASE = (C.c_uint32 * 2)()
assert C.CDLL(None).mach_timebase_info(C.byref(TIMEBASE)) == 0


def backend_cpu(pid):
    """Return (user, system) CPU time of pid in mach absolute-time ticks."""
    fields = (C.c_uint64 * _FIELDS)()
    if _PROC_RUSAGE(int(pid), _RUSAGE_INFO_V2, C.byref(fields)) != 0:
        raise OSError(C.get_errno(), f"proc_pid_rusage failed for {pid}")
    return fields[2], fields[3]


def _ms(pid):
    return sum(backend_cpu(pid)) * TIMEBASE[0] / TIMEBASE[1] / 1e6


def calibrate_cpu(seconds=0.25):
    """Busy-loop and compare the sampler with process_time for this process."""
    pid = os.getpid()
    before, cpu_before = _ms(pid), time.process_time()
    deadline = time.perf_counter() + seconds
    while time.perf_counter() < deadline:
        pass
    sampled, reference = _ms(pid) - before, (time.process_time() - cpu_before) * 1000
    ratio = sampled / reference
    assert 0.9 < ratio < 1.1, (sampled, reference)
    return dict(timebase=[TIMEBASE[0], TIMEBASE[1]], sampled_ms=sampled,
                process_time_ms=reference, ratio=ratio)
