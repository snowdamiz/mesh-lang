# Native durable Jobs comparison

Compare Mesh, Oban, River, and Apalis across all five workloads. The scorecard reports both submission through final settlement and enqueue commit through final settlement. [Four-engine scorecard](results/2026-11-23-iteration-comparison/four-engine-scorecard.md) · [Interactive chart and per-rotation results](results/2026-11-23-iteration-comparison/chart.html)

## Adapter handoff

The matrix owner runs official timing. Keep builders and functional smokes off the host during timing. While setting up, each lane should publish its host command, build identity, and ready state in its own README. For the active assignment and handoff protocol, see `target/linode-jobs-stepchange/requests/STATUS.md` and `README.md` in the same directory.

The [reversed controls](results/2026-11-23-controls/README.md) and [four-engine baseline](results/2026-11-23-baseline/README.md) are sealed. The due-query improvement has a [four-engine repeat](results/2026-11-23-due-range-repeat/README.md), [alternating dense pairs](results/2026-11-23-due-range-focused/README.md), and an [integrated product proof](results/2026-11-23-mesh-due-range/README.md). The [ordered-enqueue repeat](results/2026-11-23-ordered-enqueue-repeat/README.md) measured a short-profile regression and held the patch. The [worker attribution capture](results/2026-11-23-native-worker-cost/README.md) closed without qualifying a worker candidate.

Each adapter runs one fresh, single-engine arm per invocation. Entry point arguments are `PROFILE EVIDENCE_DIRECTORY` (plus any fixed arguments in the adapter's command prefix). The controller creates an empty mode-0700 evidence directory and pins the invocation to the shared two CPUs. Use a separate owned scratch directory for disposable data and credentials. Start a fresh capped PostgreSQL cluster or reset an equivalently isolated owned trial cluster outside the timed window. Return nonzero on any failure, retaining the safe evidence produced so far. The controller provides `DURABLE_JOBS_PHASE` (`warmup`, `measured`, or `diagnostic`). Adapters should run exactly one stand-alone arm per call and defer warmup ordering to the controller.

Profiles reproduce the input batches and hashes from `proof/jobs-parity/linux-regression/trial.py`, generated with `scripts/jobs-parity.py` and `scripts/jobs-parity-profiles.json`. The dense profiles use one transaction containing the product row and queue inserts. Sparse uses 24 singleton transactions, waits for each settlement, then delays 250 ms between arrivals. Use the same 3-second ready idle and 100 ms observer sampling interval as the frozen proof, a single warm producer/observer session where possible, and independently committed attempt and effect writes. The selected runner slots apply across the entire arm. Count all driver, worker, observer and PostgreSQL connections against the physical cap, with zero reserved connections.

| Profile | Runners | Slots | PG physical cap | Jobs | Payload bytes | Hold (ms) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| noop | 8 | 64 | 96 | 1,024 | 1,024 | 0 |
| slow | 1 | 64 | 68 | 512 | 16,384 | 100 |
| sparse | 1 | 1 | 5 | 24 | 1,024 | 1 |
| short | 1 | 16 | 20 | 128 | 1,024 | 1 |
| large | 8 | 64 | 96 | 64 | 524,288 | 1 |

Keep the adapter's original raw exports and logs. Also emit `ledger.json` with the following comparison fields (or publish your raw schema early so the controller can translate it). Use integer PostgreSQL epoch microseconds for commit times. The timestamp of the producer call is on the same host clock. Capture it at the adapter boundary before the enqueue call. Use the attempt row for handler entry, and the terminal tuple's PostgreSQL commit timestamp for settlement. Sample only outside any serialization path that artificially gates enqueues or completions.

- `input_sha256` is the frozen concatenated-batch hash. `engine`, `profile`, `version` and `artifact_sha256` identify the lane, pinned dependency/build version and executable or build manifest digest. Include the detailed build/lock/resolver evidence in separate files.
- `database` contains `version` (string, e.g. `17.11`), `tls`, `fsync`, `synchronous_commit`, `full_page_writes`, `track_commit_timestamp` (booleans), `max_connections`, `database_connection_limit`, `reserved_connections`, and `superuser_reserved_connections` (integers). Check TLS on the application/runner and producer paths.
- `runners`, `slots`, `peak_executing`, `peak_connections`, `runner_cpu_seconds`, `peak_runner_rss_bytes` describe the observed topology and resources. Include raw process, thread, and connection samples and PG settings/counters. `runner_cpu_seconds` uses process-tree user + system CPU including startup, idle, drain, and shutdown, as in the frozen proof.
- `transactions` is a list of `{request_id, submitted_us, committed_us, xid, enqueued_ids}`. `enqueued_ids` maps frozen job key to native durable queue ID as text. The committed product transaction is the producer witness.
- `jobs` is a list of `{id, job_key, payload_sha256, hold_ms, fail_first, state, attempt, attempts, effects, entered_us, attempt_xid, effect_xid, attempt_committed_us, effect_committed_us, settled_us}`. Counts and identity witnesses come from PostgreSQL. Map each library's successful terminal state to `completed` and retain its native value in raw evidence. Include `settled_xid` when available. If the adapter can supply `claimed_us` (using the actual queue claim timestamp), retain it as an optional stage metric. Use only database commit timestamps for attempt, effect and terminal. Require two distinct nonempty XIDs and strict first-commit-before-second-commit ordering. Carry durable attempt and effect counts and include any unexpected/extra rows in raw evidence even if no row joins a completed queue job.

Keep credentials and private keys out of evidence. Prefer a public/safe manifest with exact versions, flags, certificate verification mode and digests, plus raw observations without credentials. Do not serialize environment dumps or private PostgreSQL data directories. The controller stages a sanitized evidence mirror under `results/` after each call. Debug invocations and warmups belong in separate series and cannot supply missing measured arms.

## Run the matrix

The [November 23 checkpoint](results/2026-11-23-native/README.md) records the current SSH and Mesh platform blockers. Start the commands below after the native setup owner has pinned the Linode host key and all four adapters have passed native functional smokes. Set up PostgreSQL on disk with verified loopback TLS. Finish all builds before the timing owner reserves the host.

Write `tools/durable-jobs-bench/inventory.local.json` from the adapter handoffs. Set `cpus` to a list of two Linux CPU IDs. Under `engines`, add `mesh`, `oban`, `go`, and `rust`. Each entry has these fields.

| Field | Value |
| --- | --- |
| `argv` | List beginning with the absolute executable path on the Linode. Add fixed arguments such as `mesh` for `native/run`. The driver adds `PROFILE EVIDENCE_DIRECTORY`. Use credential-free arguments. |
| `postgres_storage` | Existing absolute directory on the disk where the adapter starts its PostgreSQL cluster. |
| `run_as` | Optional existing system username for an adapter that needs a non-root caller. Omit it for a wrapper that drops privileges itself. |
| `version` | Exact version string from the adapter ledger. |
| `artifact_sha256` | Exact release executable or build manifest hash from the adapter ledger. |
| `verified_files` | Map from absolute remote paths to SHA-256 digests for wrapper files. Required for the replacement River adapter. |
| `evidence_globs` | Reviewed relative globs for safe text exports. Include the adapter's build, SQL and process evidence, failure logs, and receipts. The export always includes `ledger.json` when present. |

From the checkout root, run the reversed controls, then the baseline. The dispatcher checks each arm, records failures, and continues with the other arms. The final CSV requires all four engines in each of 25 blocks. It reports the median and range across the five groups for each profile and engine.

Use the River build whose native setup receipt confirms that the submit timer starts before batch file input. The dispatcher rejects the original predecoded River binary for controls, baseline, and repeat runs. Copy the replacement wrapper digests from its superseding `READY.json` into `verified_files`.

```sh
bench=tools/durable-jobs-bench
python3 -B "$bench/run.py" check-host --series 2026-11-23-native
python3 -B "$bench/run.py" dispatch --kind controls --series 2026-11-23-controls \
    --inventory "$bench/inventory.local.json"
python3 -B "$bench/run.py" dispatch --kind baseline --series 2026-11-23-baseline \
    --controls-series 2026-11-23-controls --inventory "$bench/inventory.local.json"
python3 -B "$bench/run.py" summarize --series 2026-11-23-baseline \
    > "$bench/results/2026-11-23-baseline/comparison.csv"
```

The controller uses the same host boot, disk, and two-CPU mask for the controls and all measured arms. The adapter and the PostgreSQL process it starts inherit the mask. The host recorder runs on a spare CPU and samples `/proc/stat`, disk counters, I/O pressure, and known build processes every 200 ms. The scorer rejects windows with a gap above one second, CPU steal above 1%, or a detected build. CPU percentages cover the samples that bracket the submit window. The raw samples preserve the exact bounds. For dense no-op, slow, and large, the controller runs a full workload in an unmeasured arm before each measured block. Each invocation starts a fresh database instance or uses its adapter's equivalent isolation procedure.

The measured throughput denominators end at the latest PostgreSQL terminal commit. One starts at the earliest transaction's call boundary and the other at the earliest durable producer commit. Pickup ends at each job's handler entry. The verifier matches every terminal row to the enqueue receipt and to its frozen payload, then checks both application commits, the hold, and the final settlement. `peak_executing` is the lane's observed active Jobs occupancy. Queue rows waiting for asynchronous settlement may outlive the handler, so retain both counts when the adapter provides them.

Each attempt is written on the Linode under `/var/tmp/mesh-durable-jobs-matrix/`, with an immediate sanitized `raw.tar.gz` copy under the local `results/` series. Re-run `dispatch` with the same series and inventory to retry an incomplete block. If the SSH reply or transfer failed after the adapter started, run `recover --series SERIES` first. A retry keeps the earlier attempts and the scorer uses the first complete valid block in attempt order. Use a fresh series for any build, driver, or inventory change. Add `--group` and `--profile` to narrow a resumption. Use `--kind diagnostic` in a separate series for instrumented runs.

To exercise the scheduler, evidence rules, and archive replay without SSH, run `python3 -B tools/durable-jobs-bench/test_matrix.py -v`. The tests use the five frozen inputs with synthetic timestamps and resource counters.

After changing Mesh, use a fresh inventory and series to repeat the affected profiles. The repeat requires a complete archived baseline and reuses its CPU mask, peer artifacts, and host disk. Each selected profile runs all five rotated groups. Repeat `--profile` for each affected profile. Keep the same profile list if you resume the series.

```sh
python3 -B "$bench/run.py" dispatch --kind repeat --series 2026-11-23-mesh-repeat \
    --reference-series 2026-11-23-baseline --inventory "$bench/inventory.local.json" \
    --profile noop
python3 -B "$bench/run.py" summarize --series 2026-11-23-mesh-repeat \
    > "$bench/results/2026-11-23-mesh-repeat/comparison.csv"
```

## Interleave pinned artifacts

Use `interleave.py` to compare multiple Mesh artifacts in the same block as
Oban and River, or to run a direct pair. Build, link, and run correctness
smokes during SETUP. The standalone release builder uses the same frozen
fixture and explicit runtime archive as the native bootstrap.

```sh
native=tools/durable-jobs-bench/native
"$native/build-linux" "$source" "$artifact" "$build_state" "$rust_bin" "$llvm_prefix"
```

Supply a pinned checkout or a source bundle produced by `native/linode bundle`.
The builder expects an x86-64 Linux host with Rust 1.99.0 and LLVM 21.1.8.
Use absolute paths. `artifact` must be a new directory whose parent exists.
Set `CARGO_HOME` or `CARGO_TARGET_DIR` to a prepared cache if needed.

The interleave config pins the boot ID, kernel, and PostgreSQL storage device
major and minor number in `host`. Record Linux `getconf CLK_TCK` in
`clock_ticks`. In `lanes`, give each label an `engine` and an `inventory`
in the standard format. The first label is the reference for paired changes.
List the frozen `profiles` and record the shared CPU mask in `cpus`.
The adapter, its workers, and PostgreSQL inherit the mask. Use a fresh series
for each mask. The three-CPU Mesh and Oban launcher takes
`NATIVE_JOBS_CPU_COUNT=3`. Both CPU caps retain the standard engine scheduler
settings. The host recorder runs on a spare CPU.

Set `contract` to `strict-frozen` for the original workload and isolated
worker execution. Record the launch settings in `description`.
Use `persistent-worker` and a separate series for a candidate that reuses a
worker process. Describe the worker lifetime, fault and cleanup rules, and any
prework in the config. Report the time spent outside the submit window as well
as both throughput windows. The durable-ledger verifier runs for both contracts.

```sh
bench=tools/durable-jobs-bench
python3 -B "$bench/interleave.py" dispatch "$series" --config "$config"
python3 -B "$bench/interleave.py" summarize "$series" > "$bench/results/$series/summary.json"
```

Add `--group` to dispatch one rotation at a time. Run the same dispatch
again to recover a missing export. If recovery leaves an incomplete block,
dispatch creates a fresh block and keeps every earlier attempt. The offline
summary reports missing blocks, measured distributions, paired deltas, and
raw archive hashes for each selected block. Host CPU counters cover the
sampled interval bracketing submit and final settlement. They include the
work on the pinned CPUs, including PostgreSQL.
