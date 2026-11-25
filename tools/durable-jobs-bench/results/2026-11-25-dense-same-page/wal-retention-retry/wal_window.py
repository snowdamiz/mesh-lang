"""Preserve a bounded, fully flushed WAL window before a shutdown can recycle it."""
from pathlib import Path
import shutil
import subprocess
import time

WAL_STATE = """
SELECT json_build_object(
    'flush_lsn', pg_current_wal_flush_lsn(),
    'segment_bytes', pg_size_bytes(current_setting('wal_segment_size')),
    'timeline', left(pg_walfile_name(pg_current_wal_insert_lsn()), 8))
"""
WALDUMP_SHA = "924c9e685ca62bb7b59bbca3adc8c3d438396b64214d48e5c54b6489f02aeb22"
# The original is a two-segment window. Bound scratch use even if a fixture misbehaves.
MAX_COPY_BYTES = 128 * 1024 * 1024


def lsn(text):
    high, low = (int(piece, 16) for piece in text.split("/"))
    return (high << 32) + low


def window(start, stop, state):
    begin, end, flushed = map(lsn, (start, stop, state["flush_lsn"]))
    size = state["segment_bytes"]
    timeline = int(state["timeline"], 16)
    if (not 0 < begin < end <= flushed or not 0 < timeline < 2**32
            or size < 1024 * 1024 or size > 2**30 or size & (size - 1)):
        raise ValueError("invalid or unflushed WAL window")
    first, last = begin // size, (end - 1) // size
    if (last - first + 1) * size > MAX_COPY_BYTES:
        raise ValueError("WAL window exceeds disposable snapshot capacity")
    return [f"{timeline:08X}{index // (2**32 // size):08X}{index % (2**32 // size):08X}"
            for index in range(first, last + 1)]


def wait_flushed(read_state, stop):
    until = time.monotonic() + 5
    while True:
        state = read_state()
        if lsn(state["flush_lsn"]) >= lsn(stop):
            return state
        if time.monotonic() >= until:
            raise ValueError("WAL insert endpoint did not flush before snapshot")
        time.sleep(0.05)


def preserve(wal, snapshot, start, stop, state):
    names = window(start, stop, state)
    for name in names:
        target = snapshot / name
        with (wal / name).open("rb") as source, target.open("xb") as dest:
            shutil.copyfileobj(source, dest)
        if target.stat().st_size != state["segment_bytes"]:
            raise ValueError(f"incomplete WAL segment {name}")
    return dict(state, start_lsn=start, end_lsn=stop, files=names)


def decode(bindir, env, snapshot, out, receipt, relation):
    command = [Path(bindir) / "pg_waldump", "-p", snapshot, "-t", str(int(receipt["timeline"], 16)),
               "-r", "Heap", "-R", relation, "-s", receipt["start_lsn"], "-e", receipt["end_lsn"]]
    with (out / "jobs-wal.log").open("x") as stdout, (out / "layout-decoder.log").open("x") as stderr:
        result = subprocess.run(command, env=env, stdout=stdout, stderr=stderr, timeout=30, check=False)
    if result.returncode:
        raise ValueError(f"pg_waldump exit {result.returncode}; see layout-decoder.log")
    return (out / "jobs-wal.log").read_text()
