#!/usr/bin/env python3
"""Compare two toolchains built with build.sh.

usage: run.py <runs> <labelA> <labelB> [benchmark ...]

Runs alternate A/B order every round, after one discarded warm-up per binary
(macOS scans a new binary on first exec). Reports the median and minimum wall
time, peak RSS, and whether both builds printed the same output.
"""
import os
import re
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))


def once(label, name):
    path = os.path.join(HERE, "bin", label, name)
    start = time.perf_counter()
    proc = subprocess.run(
        ["/usr/bin/time", "-l", path], capture_output=True, text=True, timeout=900
    )
    wall = time.perf_counter() - start
    rss = re.search(r"(\d+)\s+maximum resident set size", proc.stderr)
    return wall, (int(rss.group(1)) / 1048576 if rss else 0.0), proc.returncode, proc.stdout.strip()


def main():
    runs, a, b = int(sys.argv[1]), sys.argv[2], sys.argv[3]
    # Skip dotfiles: macOS leaves `._name` sidecars on non-APFS volumes.
    programs = os.listdir(os.path.join(HERE, "programs"))
    names = sys.argv[4:] or sorted(n for n in programs if not n.startswith("."))
    print(
        f"{'benchmark':<18}{a + ' med':>11}{b + ' med':>11}{'med x':>8}"
        f"{a + ' min':>11}{b + ' min':>11}{'min x':>8}{a + ' MB':>10}{b + ' MB':>9}  output"
    )
    for name in names:
        wall = {a: [], b: []}
        rss, out, code = {}, {}, {}
        for label in (a, b):
            once(label, name)
        for i in range(runs):
            for label in (a, b) if i % 2 == 0 else (b, a):
                seconds, mb, rc, stdout = once(label, name)
                wall[label].append(seconds * 1e3)
                rss[label] = max(rss.get(label, 0), mb)
                out[label], code[label] = stdout, rc
        med_a, med_b = statistics.median(wall[a]), statistics.median(wall[b])
        min_a, min_b = min(wall[a]), min(wall[b])
        if code[a] != 0 and code[b] == 0:
            verdict = f"{a} CRASHED (exit {code[a]}); {b} printed {out[b][:24]!r}"
        elif code[b] != 0:
            verdict = f"{b} FAILED (exit {code[b]})"
        else:
            verdict = "same" if out[a] == out[b] else f"DIFF {out[a][:20]!r} vs {out[b][:20]!r}"
        print(
            f"{name:<18}{med_a:>11.1f}{med_b:>11.1f}{med_a / med_b:>7.2f}x"
            f"{min_a:>11.1f}{min_b:>11.1f}{min_a / min_b:>7.2f}x{rss[a]:>10.1f}{rss[b]:>9.1f}  {verdict}",
            flush=True,
        )


if __name__ == "__main__":
    main()
