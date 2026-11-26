# Ordinary V1 dense topology and scheduler supply

This experiment varies scheduler supply within the concentrated deployments and compares runner topologies at equal total scheduler supply. Two 32-slot runners at eight scheduler workers each are the preselected primary arm. The one-runner comparisons are secondary. [Protocol](PROTOCOL.md)

| Arm | Runners × slots × app sessions | Scheduler per runner | Total scheduler workers | Physical Jobs / app / broker sessions |
|---|---:|---:|---:|---:|
| Stock A and B | 8 × 8 × 4 | 2 | 16 | 32 / 32 / 1 |
| Two at one | 2 × 32 × 16 | 1 | 2 | 8 / 32 / 1 |
| Two at two | 2 × 32 × 16 | 2 | 4 | 8 / 32 / 1 |
| Two at eight | 2 × 32 × 16 | 8 | 16 | 8 / 32 / 1 |
| One at two | 1 × 64 × 32 | 2 | 2 | 4 / 32 / 1 |
| One at 16 | 1 × 64 × 32 | 16 | 16 | 4 / 32 / 1 |

Each arm uses the sealed V1 Mach-O and warm broker, 1,024 dense `noop` jobs, 64 slots, 32 warmed application sessions, and PostgreSQL 17.11 cap 96. Runner scheduler minimum and maximum are equal. The five timing groups alternate reversed and rotated orders under the existing local topology lock. The test uses the normal compiled handler and existing V1 admission. The earlier 8 × 8 scheduler sweep used a fixed topology. [Original topology experiment](../2026-11-23-ordinary-runner-topology/README.md) · [Earlier scheduler sweep](../2026-11-23-ordinary-scheduler-supply/README.md)

## Results

Two runners at eight workers used 16 write backends, up from four at two workers. The primary arm increased runner-tree CPU/job in every rotation and improved durable throughput in one of five. At the same total scheduler supply of 16, both concentrated layouts reduced runner CPU in every rotation versus stock. The CPU savings from concentrating the deployment persist when total scheduler supply is held fixed. [Timing ledger reduction](evidence/timing-02-summary.json)

| Matched contrast | Submit jobs/s | Commit jobs/s | Runner-tree CPU/job | Bracketed runner CPU/job |
|---|---:|---:|---:|---:|
| Two at eight vs two at two | −21.5% | −9.4% | +30.8% | +24.6% |
| One at 16 vs one at two | −2.0% | −18.1% | +53.5% | +42.5% |
| Fixed 16, two at eight vs stock | +19.0% | +18.1% | −35.9% | −30.0% |
| Fixed 16, one at 16 vs stock | +40.2% | +17.0% | −39.4% | −36.5% |
| Fixed 16, one at 16 vs two at eight | +24.8% | −2.9% | −2.5% | −13.9% |
| Fixed two, two at one vs one at two | +19.1% | +24.5% | +15.5% | +17.6% |

Values are paired medians over five matched rotations. The primary arm's commit change ranged from −39.5% to +8.1%. Fixed-16 two-runner commit throughput improved in four of five, with a range of −0.6% to +86.1%. The median absolute gap between duplicate stock arms was 60.6% from submit and 32.2% from commit. The 15-logical-CPU macOS host ran the pinned dev-linked executable without affinity. One-minute host load at arm start ranged from 18.2 to 102.3. Treat elapsed effects whose direction changed across blocks as local observations. [Raw arm ranges](evidence/timing-02-summary.json)

| Timed arm | Submit jobs/s, median (range) | Commit jobs/s, median (range) | Claim calls, median | Non-grant hints, median | App backends used, range | Occupied slots, mean then median |
|---|---:|---:|---:|---:|---:|---:|
| Stock A and B | 787 (382–1,596) | 2,111 (1,203–3,227) | 486 | 236 | 14–16 | 13.0 |
| Two at one | 1,239 (347–1,802) | 3,144 (815–3,838) | 259 | 104 | 2 | 49.1 |
| Two at two | 1,179 (767–1,911) | 3,333 (2,405–4,738) | 136 | 46 | 4 | 32.5 |
| Two at eight | 1,172 (558–1,439) | 2,737 (2,599–3,147) | 106 | 30 | 16 | 25.8 |
| One at two | 1,289 (634–1,439) | 2,759 (1,338–3,083) | 70 | 0 | 2 | 23.5 |
| One at 16 | 1,263 (261–1,644) | 2,524 (873–3,242) | 68 | 0 | 12–15 | 19.9 |

At fixed two-worker supply, both layouts used two app backends. Adding a second runner raised claim calls from 70 to 259 and hints from zero to 104. The concentrated layout with one runner used less CPU in every rotation. Two runners finished faster in three of five, with commit-rate changes from −64.6% to +135.0%. This is evidence that a second claim lane and the redistribution of slots change coordination even when scheduler supply and actual app-backend use match.

At fixed two-runner topology, the higher scheduler setting reduced claim calls and hints while increasing the PostgreSQL statement execution time for application SQL from a median of 32.6 to 72.7 ms summed across the batch. Median effect-commit-to-terminal time rose from 0.79 to 2.16 ms. That latter interval combines scheduling, settlement delivery and PostgreSQL waits. Occupancy uses claim-stamp to terminal-commit intervals and excludes reserved and unreleased slots.

The primary failed its local gate with one durable throughput win in five. The secondary single-runner high-supply arm won two of five at its fixed topology. The original build-free native request remains the next comparison under the controller's priorities. [Original request](../2026-11-23-ordinary-runner-topology/request.json)

## Proof and measurement

The replay pairs each treatment with its same-group comparator. For a comparison against stock, use the mean of its two same-group stock arms. Submit and enqueue commit each have a throughput window ending at the exact final terminal XID commit. The process exit `wait4` receipt covers full runner-tree user and system CPU. A separate calibrated `proc_pid_rusage` snapshot brackets first submit through the first observed 1,024th terminal. It includes the recorded observation lag after the last terminal commit. The Mach absolute counters use the 125/3 tick-to-nanosecond conversion checked independently against the Python process CPU clock. PostgreSQL and broker CPU are outside both runner metrics. [CPU calibration](evidence/cpu-calibration.json)

At ready and terminal, the OS confirms the configured scheduler workers plus six auxiliary threads per runner. The lifecycle replay reconciled 35,840 jobs across 35 timing arms and 7,168 jobs across seven diagnostic arms. It checks frozen payload identities and grants, two independent app writes per job, unique terminal XIDs, clean runner drains, exact SQL and local slot caps, and the stable physical TLS backend inventory. Each job executes four application SQL calls, two synchronous checkout health reads and two writes with synchronous responses. The live session snapshots show which physical app backends actually issued writes. Jobs counts and hints come from PostgreSQL and the compiled runner's own reports. V1 has one private claim lane per runner. [Source archive and fixture identity](../2026-11-23-ordinary-trusted-v1/README.md) · [Replay](replay.py)

In the activity group, the warm broker samples active V1 claim queries, app queries and app inserts in the same PostgreSQL observation. A 5 ms requested sleep and a cached ready process inventory limit observer overhead. Actual samples are slower when the broker or host is busy. Counts are sampled lower bounds. A session can be in use on the client while PostgreSQL reports it idle. This group supplies concurrency evidence and is separate from throughput and CPU comparisons.

| Activity arm | Samples | Peak active claim queries | Peak active app backends | Samples with a claim and app insert active |
|---|---:|---:|---:|---:|
| Stock A | 56 | 3 | 5 | 7 |
| Stock B | 55 | 3 | 3 | 11 |
| Two at one | 46 | 2 | 2 | 15 |
| Two at two | 67 | 2 | 4 | 10 |
| Two at eight | 55 | 1 | 10 | 11 |
| One at two | 69 | 1 | 2 | 7 |
| One at 16 | 84 | 1 | 4 | 3 |

All seven lanes showed a claim and app insert active in the same snapshot. The primary raised peak server-observed app concurrency from four to ten in this capture. It sampled one active claim at its peak with two available claim lanes. [Activity replay](evidence/activity-01-summary.json)

## Replay

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-ordinary-topology-scheduler-supply
(cd "$packet" && shasum -a 256 -c SHA256SUMS)
mkdir -p target/ordinary-topology-scheduler-supply
decoded=$(mktemp -d "$PWD/target/ordinary-topology-scheduler-supply/replay.XXXXXX")
for study in timing-02 activity-01; do
    tar -xzf "$packet/evidence/$study.tar.gz" -C "$decoded"
    python3 -B "$packet/replay.py" "$decoded/$study" > "$decoded/$study-summary.json"
    cmp "$decoded/$study-summary.json" "$packet/evidence/$study-summary.json"
done
```

For a new local run with the same pinned executable and PostgreSQL, run `python3 -B "$packet/local.py" timing-repeat`. Use a fresh name and `--mode activity` for the separate high-rate diagnostic. Recreate the CPU calibration on that host with `calibrate.py` and pass it to `replay.py --calibration` for the new evidence. The first launcher attempt failed before running an arm because it imported the earlier driver's sibling module by its unqualified name. The retained failure is separate from the timed series. [Initial launcher exit](evidence/timing-01.log)
