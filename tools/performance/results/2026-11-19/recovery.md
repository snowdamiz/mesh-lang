# Supervisor recovery with reliable crash reporting

Mesh source commit `25a991af7184d88ee7c00b3ad1572dbbaadddd21`. Previous Mesh source commit `dcfdcd43d89b204a0dde935f0b180f507d8f14f5`. The same x86-64 workload object was linked against both release runtimes. The public Mesh workload panics and prints a diagnostic. Elixir requests a supervised shutdown. All trials require a fresh replacement, empty replacement state and the surviving sibling’s state. Mesh stderr is checked through process exit.

## Public recovery

Median / p95 / p99 in µs. The one-worker column pools three independent batches of 101 new processes per subject. The four-worker column covers 101 processes per subject. Samples run in randomized blocks, pinned to CPU 2 or CPUs 0–3 on one dedicated 4-vCPU AMD EPYC 7713 Linode. Each batch uses a separate fixed seed. Percentiles use nearest rank.

| Runtime | 1 worker | 4 workers |
| --- | ---: | ---: |
| New Mesh | 23.8 / 39.4 / 99.9 | 31.8 / 43.2 / 56.2 |
| Previous Mesh | 29.6 / 45.6 / 221.9 | 36.3 / 79.2 / 236.5 |
| Elixir | 47.7 / 71.6 / 234.8 | 46.8 / 79.8 / 122.4 |

### One-worker batches

The first batch also included the public Go and Rust programs. The next two compared Mesh and Elixir. Each cell is median / p95 / p99 in µs.

| Runtime | Batch A | Batch B | Batch C |
| --- | ---: | ---: | ---: |
| New Mesh | 23.7 / 37.1 / 99.9 | 23.7 / 35.5 / 60.4 | 23.9 / 46.6 / 70.1 |
| Previous Mesh | 30.2 / 45.6 / 221.9 | 29.1 / 45.1 / 195.8 | 30.6 / 44.9 / 218.5 |
| Elixir | 47.7 / 62.2 / 84.1 | 47.9 / 84.3 / 234.8 | 47.7 / 72.3 / 266.9 |

### Public Go and Rust fixtures

Go and Rust use explicit failure signals. Each comparison uses batch A only.

| Runtime | 1 worker | 4 workers |
| --- | ---: | ---: |
| New Mesh | 23.7 / 37.1 / 99.9 | 31.8 / 43.2 / 56.2 |
| Elixir | 47.7 / 62.2 / 84.1 | 46.8 / 79.8 / 122.4 |
| Go | 2.7 / 3.2 / 4.1 | 3.3 / 47.9 / 58.7 |
| Rust | 29.6 / 34.2 / 89.9 | 62.8 / 87.9 / 261.3 |

### Uncertainty of the p95 difference

Microseconds, new Mesh minus the comparison runtime. The interval is a 95% percentile bootstrap, resampling whole paired blocks within each batch. It reflects sampling on this one host.

| Compared with | 1 worker | 4 workers |
| --- | ---: | ---: |
| Previous Mesh | -6.2 [-19.4, +5.0] | -36.0 [-180.9, -7.9] |
| Elixir | -32.2 [-48.3, -18.1] | -36.7 [-56.7, -17.8] |

### High-sample Mesh A/B

A separate two-build experiment used three more seeds and 501 paired fresh processes per seed and worker count (1,503 per build and worker count). The driver randomized old and new Mesh inside each block. Median / p95 / p99 in µs.

| Runtime | 1 worker | 4 workers |
| --- | ---: | ---: |
| Previous Mesh | 26.4 / 39.7 / 210.5 | 33.7 / 55.4 / 214.1 |
| New Mesh | 18.6 / 24.9 / 76.5 | 29.0 / 46.8 / 169.9 |

Paired-block bootstrap of new minus previous p95 in µs, with 95% intervals. One worker -14.8 [-19.9, -13.3]. Four workers -8.6 [-15.8, -3.4].

### Whole-process time

Median milliseconds, from spawning the pinned executable until exit, including startup, polling, I/O and diagnostic drain.

| Runtime | 1 worker | 4 workers |
| --- | ---: | ---: |
| Previous Mesh | 5.448 | 6.488 |
| New Mesh | 5.536 | 6.719 |

## Pipe reader and subject on the same CPU

Median / p95 / p99 in µs.

| Runtime | 1 worker |
| --- | ---: |
| New Mesh | 23.7 / 32.0 / 48.5 |
| Previous Mesh | 100.2 / 134.4 / 144.4 |
| Elixir | 47.6 / 77.0 / 235.6 |

## Stderr redirected to a regular file

Median / p95 / p99 in µs.

| Runtime | 1 worker |
| --- | ---: |
| New Mesh | 18.4 / 32.7 / 195.1 |
| Previous Mesh | 29.2 / 48.8 / 58.1 |

## Real repeated crashes

30 fresh processes per subject and worker count, 64 real crashes per supervisor. The table includes the 63 restarts after the first in each process. All 64 crash messages were counted. Median / p95 / p99 in µs.

| Runtime | 1 worker | 4 workers |
| --- | ---: | ---: |
| New Mesh | 7.3 / 45.9 / 137.4 | 14.6 / 31.4 / 52.7 |
| Previous Mesh | 8.9 / 21.6 / 44.9 | 12.3 / 27.2 / 46.4 |
| Elixir | 188.5 / 338.3 / 386.7 | 212.6 / 386.8 / 443.7 |

## Blocked diagnostic reader

The driver fills stderr to the kernel pipe capacity, holds the reader for up to 150 ms, and checks output from a successful replacement before draining it. The following counts are single failures that recovered while the pipe was full. In a separate 64-crash test the driver keeps stderr full for 150 ms, checks that bounded backpressure limits progress, drains it, and verifies every diagnostic and state check.

| Runtime | Workers | Single recovery before drain | Burst restarts before drain | Burst diagnostics delivered |
| --- | ---: | ---: | ---: | ---: |
| New Mesh | 1 | 12/12 | 17–17 | 768/768 |
| New Mesh | 4 | 12/12 | 17–17 | 768/768 |
| Previous Mesh | 1 | 0/12 | 0–0 | 768/768 |
| Previous Mesh | 4 | 0/12 | 0–0 | 768/768 |

## Verification and scope

The campaign measures supervisor recovery. Cloud smoke covers actor, CPU, recovery and HTTP with k6. Linux benchmark binaries, measured objects and host metadata are identified in the evidence archive. End-to-end tests on macOS fill stderr, require two successive child launches and supervisor exit, and verify exact crash lines after both ordinary shutdown and explicit process exit.

## Panic attribution

600 additional fresh processes used linker wrappers to timestamp entry and return from the failure function. These instrumented observations describe where time went. The public results above come from the uninstrumented binary. Median / p95 / p99 in µs.

| Phase | 1 worker |
| --- | ---: |
| Sending the crash through entry to the failure handler | 5.4 / 5.8 / 24.7 |
| Submitting the diagnostic and recording abnormal exit | 9.7 / 10.3 / 30.5 |
| Notifying the supervisor through replacement state read | 13.8 / 20.9 / 188.6 |

## Burst tradeoff

In the cross-language run, the new build's one-worker p95 across repeated crashes increased from 21.6 to 45.9 µs. The four-worker p95 increased from 27.2 to 31.4 µs. The one-worker regular-file test also had a higher p99. Two follow-up Linux prototypes tested whether more throughput could be recovered. Batching nearby reports reduced one-worker burst p95 from 40.6 to 32.8 µs in its paired experiment, but four-worker cold median rose from 29.4 to 34.5 µs and four-worker burst p99 rose from 54.8 to 74.8 µs. Giving the writer Linux batch scheduling also increased the four-worker cold p95. The confirmed commit keeps the portable writer. Both prototype comparisons are included with the raw evidence.

## Final verification

Local verification at `25a991af` passed formatting, the locked workspace build and workspace tests (4,424 passed, 26 ignored). All five chaos rounds (50 executions), the performance proof, short continuity soak and Docker autoscaling proof passed. All 36 Docker assertions passed.

I rebuilt the unchanged public workload object from the final compiler and verified a byte-for-byte match against the measured Linux object. All 103 runtime source files on Linux matched the final commit. The final writer uses the ordinary thread scheduling policy. A Linux-specific scheduling experiment was discarded. Each supervisor shares a single diagnostic writer with a bounded queue. Saturated queues backpressure producers, and orderly exit waits for previously accepted supervised output. The native unwind path still handles failures outside the optimized supervised panic entry.
