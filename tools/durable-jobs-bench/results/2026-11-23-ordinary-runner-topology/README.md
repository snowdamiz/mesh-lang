# Concentrating ordinary V1 dense runner capacity

Across ten local rotations, two runners cut runner-tree CPU per job by a paired median of 51.8%. The two-runner deployment showed no repeatable throughput gain. A single runner cut CPU per job by 58.4%, eliminated contention hints, and reduced commit-to-terminal throughput in nine of ten groups. [First-series results](evidence/local-01-summary.json) · [Second-series results](evidence/local-02-summary.json)

## Design

Every arm ran the same pinned ordinary V1 runner and warm broker, frozen 1,024-job `noop` batch, 64 configured slots, 32 warmed application sessions, and PostgreSQL 17.11 with verified TLS, synchronous commit, fsync, and a physical cap of 96. Each process ran two scheduler workers. Changing the runner count also changed total scheduler workers and the number of physical queue sessions, which is part of the tested deployment. V1 uses the existing batch admission statement and separate settlement of each job. V1's two app writes use the normal compiled application handlers. [V1 source and native lineage](../2026-11-23-ordinary-trusted-v1/README.md) · [Local pins](local-pins.json)

| Label | Runners × slots × app sessions per runner | Scheduler workers in total | Physical queue / app / broker sessions |
|---|---:|---:|---:|
| Stock A and B | 8 × 8 × 4 | 16 | 32 / 32 / 1 |
| Tuned two-runner | 2 × 32 × 16 | 4 | 8 / 32 / 1 |
| Tuned single-runner | 1 × 64 × 32 | 2 | 4 / 32 / 1 |

The recovered native eight-runner control recorded 361 lifetime non-grant hints, 576 claim SQL calls, and 19–273 jobs per runner. The earlier local scheduler-supply test varied scheduler workers with the eight-runner deployment held fixed. The prior short, slow and protected deployments use different work or execution contracts. The new local comparison uses only the dense ordinary V1 executable. [Recovered native claim evidence](../2026-11-23-ordinary-claim-turns/README.md) · [Scheduler supply](../2026-11-23-ordinary-scheduler-supply/README.md)

A dedicated local lock serialized the arms. Five groups in each series include both identical stock controls and both tuned layouts in reverse and rotated orders. Each arm starts a disposable cluster, warms its sessions, and submits the batch through the existing broker. The submit window starts at the producer call. The commit window starts at the durable enqueue commit. Both end at the last job's terminal commit. CPU comes from the runner tree's shutdown rusage and includes startup, idle, and drain. PostgreSQL and broker CPU are outside that number. The local arm64 executable links the Cargo dev runtime and runs on the shared macOS host without native CPU affinity. [Local driver](local.py) · [Arm instrumentation](arm.py)

## Measured rates and CPU

| Layout | Arms | Jobs/s from submit | Jobs/s from enqueue commit | Runner-tree CPU ms/job |
|---|---:|---:|---:|---:|
| Stock A and B | 20 | 633 (141–1,663) | 1,929 (315–3,295) | 1.84 (1.39–2.82) |
| Tuned two-runner | 10 | 650 (286–1,758) | 2,002 (463–3,836) | 0.94 (0.66–1.18) |
| Tuned single-runner | 10 | 465 (47–1,256) | 1,096 (65–2,516) | 0.81 (0.58–1.41) |

Values are medians with the full range in parentheses. Pairing each tuned arm against the mean of its two same-group controls gives the following changes. The paired estimates account for drift across groups that the pooled medians do not capture.

| Tuned layout | Submit jobs/s | Commit jobs/s | Runner-tree CPU/job |
|---|---:|---:|---:|
| Two-runner | +0.3% (−25.0% to +75.9%) | −9.4% (−60.5% to +57.6%) | −51.8% (−61.4% to −45.3%) |
| Single-runner | −10.1% (−85.3% to +45.6%) | −31.9% (−94.4% to +5.8%) | −58.4% (−62.2% to −35.4%) |

Two-runner commit throughput changed by a median of +12.7% in the first series and −17.2% in the second. The identical controls' commit ratio ranged from −42.8% to +344.3% across the ten groups. Host one-minute load at startup ranged from 26.0 to 149.7 on 15 logical CPUs. Runner CPU fell in all ten pairs for both deployments. The 40 scored arms completed 40,960 jobs and committed 81,920 application writes. The sampled activity group and initial smoke are separate. [All measured arms](evidence/local-01-summary.json) · [Confirmation series](evidence/local-02-summary.json)

## Mechanism and correctness

Both series warmed all 32 application sockets for each layout and observed 14–16 distinct app backends execute a write in stock, four in the two-runner deployment, and two in the single-runner deployment. These are physical PostgreSQL backend IDs matched across the warmed and terminal snapshots. A separate sampled arm observed two active app backends in a single runner. The sample is a lower bound on instantaneous concurrency. The compiled handler's synchronous `Pool.execute` and the V1 pool's most-recently-returned checkout order explain how two scheduler workers can keep using just two warmed sessions. [Sampled run](evidence/local-activity-01-summary.json) · [Pinned handler and runtime](../2026-11-23-ordinary-trusted-v1/README.md)

| First series, median per arm | Stock A+B | Tuned two-runner | Tuned single-runner |
|---|---:|---:|---:|
| Queue-role SQL calls | 3,063 | 1,640 | 1,397 |
| Claim SQL calls | 464 | 138 | 82 |
| Non-grant contention hints | 234 | 44 | 0 |
| Application SQL calls | 4,096 | 4,096 | 4,096 |
| Mean occupied slots from claim stamp to terminal, of 64 | 14.7 | 33.0 | 32.7 |
| Peak executing from SQL samples | 14 | 36 | 60 |
| Jobs per runner, full observed range | 0–292 | 402–622 | 1,024 |

Each arm reconciles exact payloads and assignments, one producer XID, 1,024 attempt XIDs, 1,024 effect XIDs, and 1,024 terminal XIDs. All 3,073 XIDs are distinct. The replay rejects missing writes, retries, runner errors and lease recovery. It checks the configured per-runner slot limits in compiled runner reports, the aggregate SQL queue row at 64, PostgreSQL role and connection caps, and observed executing occupancy. Claim stamps precede admission commit and have millisecond resolution. The occupancy intervals approximate resident jobs and omit runtime reservations and release lag. [Ledger replay](replay.py)

The SQL counts use snapshots just before enqueue and after the observer sees the terminal count. Runner hint counters are lifetime totals. The closing snapshot may omit one hinted claim per runner, so the replay records a conservative in-window minimum. Grant assignment is matched by runner PID. The replay also bounds the number of granting calls with the archived claim-interval method, using the smaller of the runner's slots and V1's 16-job batch width. Plain empty calls can enter the upper bound. [Grant bound derivation](../2026-11-23-ordinary-claim-turns/README.md)

## Native diagnostic

The sealed intake is `ordinary-runner-topology`, request SHA-256 `b113fb34d563b2d5b46645e43938bbc8ee5eb9b6f4647cce527d59446c9b090a`. The matrix owner can dispatch eight arms in two opposite orders under a new 12-minute exclusive SETUP grant after the existing priorities. The request reuses the retained V1 ELF, broker and PostgreSQL on CPUs 0 and 1, with no build. It prices whether reduced admission work and runner CPU translate to elapsed gains when PostgreSQL and Mesh share two CPUs, and whether single-runner session supply remains the limiting condition. The control scores stay in the canonical series. Each changed topology has its own diagnostic label. The local request check validated all four inventories, and dispatch without the grant exited before staging. [Owner intake and exact command](request.json) · [Grant rejection](evidence/native-no-grant.log)

For a subsequent same-size cross-engine comparison, run Mesh, Oban, River, and Apalis with matched process and slot allocations where each engine supports them. Keep the same two CPUs, input, durability, connection cap and two committed application writes. Pin the queue, app and broker budgets and record actual backend use. An independently tuned comparison should give each engine the same aggregate resource envelope and an equivalent chance to choose its process count, worker concurrency and pool sizing. Freeze the chosen deployments before fresh matched, rotated measurements. Keep these two comparison policies in distinct result series. [Prior canonical comparison](../2026-11-23-ordinary-trusted-v1/README.md)

## Replay

Run from the repository root. Archived runs can be verified without the original executable or a database. A fresh local run needs the pinned Mach-O, exact V1 source archive contents, and PostgreSQL 17.11.

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-ordinary-runner-topology
(cd "$packet" && shasum -a 256 -c SHA256SUMS)
mkdir -p "$PWD/target/ordinary-runner-topology"
replay_dir=$(mktemp -d "$PWD/target/ordinary-runner-topology/replay.XXXXXX")
for series in local-01 local-02 local-activity-01; do
    tar -xzf "$packet/evidence/$series.tar.gz" -C "$replay_dir"
    python3 -B "$packet/replay.py" local "$replay_dir/$series" > "$replay_dir/$series-summary.json"
    cmp "$replay_dir/$series-summary.json" "$packet/evidence/$series-summary.json"
done
python3 -B "$packet/native-dispatch.py" check
```

To reproduce timing with the retained local inputs, run `python3 -B "$packet/local.py" local-repeat-01`. Supply `--source`, `--artifacts`, and `--pg-bin` to use another location for the same pins. The isolated pre-series smoke and the initial replay rejection are retained. The frozen harness writes the legacy label “one process per batch” for dense even when the trusted adapter uses a warm broker. The final replay checks the broker's readiness, database audit, and stable physical backend directly. [Initial replay rejection](evidence/initial-replay-rejection.json) · [Initial smoke](evidence/first-smoke.tar.gz) · [Matrix tests](evidence/matrix-tests.log)
