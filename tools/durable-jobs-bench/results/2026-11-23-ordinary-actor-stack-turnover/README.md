# Guarded stack reuse saves CPU on short Mesh actors

Keep the isolated patch for ordinary actor work. The 64-wide immediate spawn-and-exit probe used **28% less full-process CPU per actor** on Apple Silicon. A cohort that kept 64 actors parked at once had no repeatable CPU improvement beyond the duplicate-control spread. The Jobs budget does not support a paid native queue series. [Patch](stack-reuse.patch) · [Replay](REPLAY.md)

## Ordinary actor turnover

The pinned checkout is `3b20337ff7912c96362ef4a86ff32a4623226d6d`. Its `DEFAULT_STACK_SIZE` is 512 KiB. The old 64 KiB text in `stack.rs` describes neither this checkout nor the archived V1 build. Corosensei 0.3.2 reserves 528 KiB on this 16 KiB-page host, protects the 512 KiB writable range, and unmaps the entire region at drop. The emitted LLVM IR calls `mesh_actor_spawn` for both entries. The stock and reuse arms use the same IR and release compiler. [Spawn calls](evidence/spawn-ir.txt) · [Artifact identities](evidence/artifacts.sha256)

Each timing arm starts one process and runs 16,384 actors inside it. `wait4` accounts for the child from process startup through scheduler join and thread-local cleanup. The in-program interval, CPU split, faults, wall time, and exit status are also recorded. Every actor has a unique checked ticket. Parked cohorts signal readiness and wait for a release message containing their ticket. All cohort members reach the ready barrier before any member receives its release. The width for immediate actors is the number of starts per wave. Scheduler workers usually retire those actors as they start, so that width does not imply simultaneous live stacks.

The table gives medians and run ranges in microseconds of full-process CPU per actor. Each of the six rotations includes two stock executions and one reuse execution. Paired savings subtract the reuse arm from the mean of both controls within the same rotation. [All original, timed, counted, and memory arms](evidence/actor-arms.tar.gz) · [Statistics](evidence/report-data.json)

| Cohort | Scheduler workers | Stock µs/actor | Reuse µs/actor | Paired savings µs/actor | Faster rotations |
| --- | ---: | ---: | ---: | ---: | ---: |
| Immediate 1 | 2 | 24.06 [21.63, 30.28] | 20.63 [18.55, 24.28] | 3.50 [2.84, 5.68] | 6/6 |
| Immediate 16 | 2 | 9.87 [8.80, 11.17] | 8.41 [7.66, 12.20] | 1.37 [−2.48, 2.39] | 5/6 |
| Immediate 64 | 2 | 8.85 [7.46, 9.82] | 6.35 [6.03, 6.72] | 2.44 [1.45, 2.97] | 6/6 |
| Parked 1 | 2 | 41.19 [28.33, 54.34] | 34.59 [22.16, 44.29] | 5.77 [−0.78, 11.85] | 5/6 |
| Parked 16 | 1 | 9.83 [8.14, 12.29] | 9.19 [7.78, 10.82] | 0.64 [−0.52, 2.69] | 5/6 |
| Parked 16 | 2 | 14.61 [10.87, 17.45] | 13.35 [9.31, 18.65] | 1.59 [−3.26, 2.79] | 4/6 |
| Parked 16 | 4 | 23.35 [19.88, 30.67] | 20.23 [18.20, 21.01] | 3.20 [1.18, 7.87] | 6/6 |
| Parked 64 | 2 | 11.73 [11.01, 12.62] | 11.52 [10.61, 12.81] | 0.39 [−1.00, 0.77] | 5/6 |

The strongest result is immediate width 64. Its median paired system CPU saving was 2.12 µs per actor. Median stock-to-stock CPU spread was 0.46 µs per actor. Full-process wall time was 6.36 [5.33, 17.05] µs per actor stock and 4.46 [3.55, 7.46] µs with reuse. One-minute load ranged from 31.4 to 39.1 on the shared 15-logical-CPU host. The timing records include load before and after each arm. A separate stock sample at width 16 recorded 219 scheduler samples at the three stack VM calls, including 118 in `munmap`. The sample includes waiting and is not a CPU profile. [Stock stack sample](evidence/stock-profile.txt.gz)

## Allocation counts and memory

The count-only interposer matches the anonymous inaccessible reservation and then tracks protection, reset, and unmap by address. Each stock execution of 1,024 actors called reservation, protection, and unmap 1,024 times each. Reuse changed the counts as shown below. Every reservation also had one protection and one eventual unmap. The process exited with zero tracked mappings left. None of these counters ran in a timing arm. [Counts and raw output](evidence/actor-arms.tar.gz)

| Cohort | Workers | Fresh reuse reservations | Writable resets | Peak tracked mappings, stock to reuse |
| --- | ---: | ---: | ---: | ---: |
| Immediate 1 | 2 | 2 | 1,024 | 1 to 2 |
| Immediate 16 or 64 | 2 | 2 | 1,024 | 2 to 2 |
| Parked 1 | 2 | 2 | 1,024 | 2 to 2 |
| Parked 16 | 1 | 520 | 512 | 16 to 16 |
| Parked 16 | 2 | 141 | 899 | 16 to 24 |
| Parked 16 | 4 | 25 | 1,024 | 16 to 25 |
| Parked 64 | 2 | 784 | 256 | 64 to 64 |

All timing-arm medians rounded to 1.05 minor faults per actor. Resetting the writable pages requires the next actor to fault its pages back in. With two workers, the separate resource probes measured three OS threads before work, at full width, and after drain. One and four workers produced two and five OS threads respectively. Stock CPU per actor on the 16-parked-actor probe increased with more workers on this host. The fixed-width input does not model the throughput headroom of real application work.

The table shows median changes from each process's pre-work RSS in KiB. It uses three resource rotations, a checked live cohort, and a sample 40 ms after the final release and completion. The timed process high-water mark uses the kernel's `wait4` peak. The 64-parked-actor high-water mark was 9,776 KiB stock and 9,816 KiB with reuse. [RSS, virtual size, and thread samples](evidence/report-data.json)

| Parked actors, two workers | Last live RSS delta, stock to reuse | Last drained RSS delta, stock to reuse | Last drained virtual-size delta, stock to reuse |
| --- | ---: | ---: | ---: |
| 1 | 824 to 832 KiB | 808 to 816 KiB | 0 to 1,056 KiB |
| 16 | 1,120 to 1,136 KiB | 880 to 880 KiB | 0 to 8,448 KiB |
| 64 | 2,040 to 2,064 KiB | 1,016 to 1,040 KiB | 0 to 8,448 KiB |

## Stack ownership and teardown

The patch keeps at most eight idle `DefaultStack` mappings per owning thread, or 4.125 MiB of virtual space per worker with this page size. On completion it takes the stack via corosensei's `into_stack`, which requires a completed coroutine and clears sanitizer poisoning. Before caching, it replaces the writable anonymous range on macOS or discards its anonymous pages via `MADV_DONTNEED` on Linux. The guard page stays outside the reset range. The cache drops a mapping at capacity, on reset failure, or at thread exit.

Each checkout constructs a fresh coroutine with a new stack anchor. Conservative scans use the current actor's TLS context and its own stack bounds. The reset covers the entire writable range, including previously used spill slots. The scheduler still creates a new PID, process, argument storage, heap, and mailbox. The patch leaves the stack-limit TLS transitions, scheduler placements, alias tracking, and exit hook call sites in place. Suspended coroutines take the existing forced-unwind drop path. The focused tests exercised owner-local reuse, an eight-mapping cap, stale-byte clearing, a foreign call faulting on the reused guard, suspended destructors, panic replacement, and exit-hook order. The compiled kill proofs passed all 25 ordinary and 14 grouped cases for both stock and reuse. Linux execution remains unverified. [Patch and tests](stack-reuse.patch) · [Ordinary proof](evidence/proof-reuse-ordinary.log) · [Grouped proof](evidence/proof-reuse-grouped.log)

## Local verification

The workspace build passed. The workspace sweep ran 4,823 tests successfully and left 66 ignored by default. The pinned Z3 tests, fuzz build, diagnostics checks, lint and format checks, and the disposable PostgreSQL Jobs proof passed. The PostgreSQL proof ran 75 runtime Jobs tests and three compiled durability cases. The Lean-dependent checks require a local Lean installation. This machine had no pinned Lean installation. [Exit codes and logs](evidence/gates/exits.tsv)

## The queue budget is below one percent of either submit gap

One sealed dense V1 diagnostic ran the retained runner executable and one ran the archived source with the isolated patch and a release runtime. Both used the same 1,024-job workload, eight runners with two scheduler workers each, 64 attempt slots, and PostgreSQL 17.11 with verified TLS. Each ledger has 2,048 independently committed application writes and terminal reconciliation for all 1,024 jobs. In the retained run, each runner reserved exactly one more guarded mapping than its completed-attempt count. The process-level counts were 1,032 reservations, 1,032 protections, and 1,032 unmaps. The patched arm ran 64 of each and 1,032 writable resets. Both arms peaked at 64 runner OS threads and 65 PostgreSQL connections. These diagnostic arms establish path and count. The archived executable and the release relink have different build profiles and the interposer adds work. [Reconciliation and per-runner counts](evidence/v1-count-summary.json)

The retained two-CPU Linux checkpoint has one matched native block. Inverse throughput gives these aggregate clocks in µs/job. The underlying wall clocks begin at submit or the first durable enqueue commit and end at the last terminal settlement. [Native block](evidence/native-v1-dense-checkpoint.json)

| Clock | Native V1 | Oban | River | Gap to Oban | Gap to River |
| --- | ---: | ---: | ---: | ---: | ---: |
| Submit | 2,791.4 | 1,559.0 | 1,095.9 | 1,232.4 | 1,695.5 |
| Durable commit | 2,192.4 | 1,414.9 | 1,022.2 | 777.5 | 1,170.2 |

If the 2.44 µs paired CPU saving transferred to Linux and landed entirely on the queue clock once per attempt, it would save 2.50 ms across 1,024 jobs. That covers 0.20% of the Oban submit gap and 0.14% of the River submit gap. Giving the candidate the most favorable CPU result observed in any pair, 11.85 µs per attempt, still covers only 0.96% and 0.70% of the submit gaps, or 1.52% and 1.01% of the durable-commit gaps. This sensitivity check assumes perfect transfer and serialization. No paid host time was requested or scheduled. [Budget calculation](evidence/queue-budget.json)
