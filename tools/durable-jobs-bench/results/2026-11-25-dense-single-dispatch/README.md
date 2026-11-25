# One dispatcher with four synchronous application workers

This build-free pilot concentrates the public logger-off dense `1e70f18d` Mesh deployment on one dispatcher with 64 slots. The two-runner controls have four application scheduler workers across two processes. The candidate keeps four in one process. A one-runner fixed-two arm isolates the effect of restoring the workers. Each arm keeps 32 warmed application TLS sessions, a fixed-two broker, two independent application commits per job, and one independent terminal transaction per job.

| Lane | Dispatchers × slots | Scheduler min and max per runner | App TLS sessions per runner | Queue TLS sessions total |
|---|---:|---:|---:|---:|
| Stock A and B | 2 × 32 | 2 | 16 | 8 |
| Single four | 1 × 64 | 4 | 32 | 4 |
| Single two | 1 × 64 | 2 | 32 | 4 |

PostgreSQL 17.11, CPUs 0 and 1, its 96-connection limit, the worker-role limit of 8, shared queue capacity of 64, default storage, and enabled synchronous durability are fixed. The topology also changes the queue session count and the distribution of the app pool. Any advantage is specific to that deployment change on the same executable. Both blocks use the same 1,024-job input and run full warmups and measured arms. The second block reverses the first.

## Prior evidence

The archived native V1 1×64 fixed-two arm lowered claims from 156.5 to 89.5 per batch and runner CPU from 0.644 to 0.505 ms/job versus 2×32 fixed-two. It was slower on both clocks. The native R2 fixed-four experiment kept two dispatchers, reached eight writers, and missed its advancement gate. The macOS V1 single-runner sensitivity tested fixed 16. The repository records and thread search inspected for this packet contain no exact prior native comparison of 1×64 fixed-four against 2×32 fixed-two on the current hot individual-terminal fixture. [Native V1](../2026-11-23-ordinary-runner-topology/README.md) · [Native R2](../2026-11-24-dense-hot-worker-supply/NATIVE-RESULTS.md) · [Apple sensitivity](../2026-11-23-ordinary-topology-scheduler-supply/README.md)

The historical V1 runner-only difference is about 142 CPU-ms per 1,024 jobs, or 28–42% of the newer 337–512 CPU-ms submit budget. That gives scale across two different experiments. The public hot matched records used 544–557 runner lifetime CPU-ms per batch and 112–122 set-claim calls. Their host CPU floor requires 337–512 CPU-ms to reach 20% higher submit throughput than River. The pilot reports observed runner savings and PostgreSQL CPU together so lower dispatcher CPU can be checked for a corresponding increase in database work. PostgreSQL CPU is a matched procfs bracket with tick quantization, unmatched PIDs, and endpoint slack reported. Runner CPU is lifetime wait4 CPU. Statement elapsed time includes waits and provides no direct CPU attribution. [Historical CPU budget](../2026-11-25-dense-host-cpu-budget/README.md) · [Public hot results](../2026-11-24-dense-individual-hot-pool/logger-off-retry/NATIVE-RESULTS.md)

## Qualification and advancement

The stock setup replay qualifies the unchanged public ELF, broker, input, source, runtime, lock function, and grants. The pilot independently checks the invocation, host, original semantic ledger, two independent writes and individual terminal XIDs, protected grants, real queue namespace and 64-slot cap, and the installed set-claim function body and lock configuration. It reports call and marker evidence, contention hints, claim-to-terminal occupancy and per-runner caps, D2 health probes, readiness and final application TLS rosters, and the lower bound on writers inferred from each TLS backend's last SQL statement after the final settlement. That readback gives observed participation across the workload and cannot establish simultaneous active writer concurrency. The pinned lock function proves the shared queue row and namespace locking contract. Contention hints are direct runtime evidence. The harness does not sample lock wait time.

The scheduler check binds the selected environment to the exact direct child PID, verifies PID, start ticks and two-CPU affinity again before teardown, and checks the thread count on each runner. When the Linux executable denies a procfs environment read, the receipt records the error and uses the exact launch environment and live thread counts. It retains the broker's fixed-two launch environment and stable live thread count. The [launcher receipt](evidence/launcher.json) is a build-free, no-workload Linux ARM64 preflight with the real staged Bash entry, pinned harness and manifest, root-owned 0711 directories, and UID 65534.

In both measured blocks, single four must beat the faster stock duplicate by strictly more than both the duplicate spread and 5% on each of submit-to-final-terminal and enqueue-commit-to-final-terminal. Its last second write relative to enqueue commit must also precede both controls. Runner lifetime CPU and combined runner plus sampled PostgreSQL CPU must each fall against both controls in both blocks. Claim counts, CPU changes, and middle-write clocks remain in the scores. A miss retains 2×32 fixed-two.

An exit-0 pilot release and passing replay are required before booking peers. Run a separately labeled stock-deployment comparison and 1×64 equal-topology comparison under new controller grants with the same CPU and PostgreSQL budgets, warmups, committed workload, and clocks. Stock peers retain their normal deployment. The equal arms require a real one-process 64-slot observer for each peer. Report any stock-row win as deployment-dependent if it disappears at equal 1×64.

## Controller and replay

Run these from the checkout root. Only the sole controller registers the packet and invokes intake at the tail of the existing queue. The separate pilot request preserves prior grants, stage locations, and raw evidence.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-single-dispatch
python3 -B "$d/driver.py" check
python3 -B "$d/test_offline.py"
python3 -B "$d/stage-check.py"

# Sole controller after the existing queue releases
python3 -B "$d/driver.py" intake "$(shasum -a 256 "$d/timing.json" | cut -d ' ' -f 1)"

# After native archives arrive
python3 -B "$d/driver.py" replay
```
