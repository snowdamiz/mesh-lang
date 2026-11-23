# Ordinary Mesh direct-file producer with one encode per spec

The candidate removed 63 redundant encodes and 33,041,484 encoded bytes from each 64 × 512 KiB submission. The compiled collector was faster than both controls in all six rotations. The real local PostgreSQL submission showed **no repeatable submit-to-commit improvement**. This experiment changes the benchmark broker only.

The collector retains the first encoded spec that would overflow a chunk and its UTF-8 byte length. The next chunk starts with that spec after the prior chunk is written. The regular collector retains its original evaluation order and checks the same inclusive 1 MiB limit, including commas and brackets. The outer loop still caps each call at 128 specs. A pending spec too large to stand alone fails inside the same outer transaction after the preceding chunk, with the same error and rollback. The broker's direct-file checks, request validation, probe receipts, connection, PostgreSQL transaction, and submit clock come from the frozen control. [Candidate](broker-once.mpl) · [Exact control](broker-control.mpl)

## Compiled component

The diagnostic build recorded each successful `job_spec` encode and the UTF-8 byte count from the real collector. It wrote each joined argument for independent validation. The control and candidate produced the same byte count, SHA-256, and ordered keys for every call. A separate uninstrumented compiled probe timed collection and joining after both fixture reading and input parsing.

| Input | Jobs | Enqueue calls | Encodes A/B → candidate | Encoded bytes A/B → candidate | Collector median A / candidate / B, ms |
|---|---:|---:|---:|---:|---:|
| Frozen large | 64 | 64 | 127 → 64 | 66,607,436 → 33,565,952 | 34.49 / 23.48 / 36.20 |
| General mixed sizes and UTF-8 | 139 | 4 | 141 → 139 | 3,395,502 → 1,955,146 | 3.82 / 2.62 / 3.79 |
| Frozen slow | 512 | 9 | 520 → 512 | 8,614,320 → 8,481,792 | 9.85 / 10.43 / 9.69 |
| Frozen dense | 1,024 | 8 | 1,024 → 1,024 | 1,232,896 → 1,232,896 | 4.11 / 3.91 / 4.55 |
| Frozen sparse | 24 | 24 | 24 → 24 | 28,896 → 28,896 | 0.441 / 0.510 / 0.443 |

The large collector saved a paired median of 11.60 ms against A and 11.00 ms against B. Its elapsed range was 18.72–28.01 ms. The mixed case beat both controls in all six rotations. The sparse figures sum 24 single-job calls. They show about 0.07 ms more collector time in the candidate. The first candidate and its raw component results are preserved. That candidate constructed an encoded-spec record on every iteration. The measured candidate constructs one only when a spec crosses the byte boundary. [First candidate](broker-first-pass.mpl) · [All component ranges and deltas](summary.json)

## Local PostgreSQL 17.11

Each arm used the unmodified direct-file adapter, the exact baseline Darwin ARM64 compiler and runtime, a warmed producer session with `sslmode=verify-full`, synchronous commit, EXTERNAL args, and 16 MiB work_mem. Each timed trial had a fresh cluster. The adapter recorded submit before sending the path. PostgreSQL's commit timestamp closed the clock. Acceptance required 64 matching hashes and IDs in durable order, 64 scalar SQL calls, 64 admission-fence calls and distinct chunk subtransaction XIDs, and one common commit timestamp matching the product XID. CPU counters bracket the adapter call and include the small post-commit timestamp read. Other queue processes were absent from this local producer comparison.

| Measurement, ms | A median [range] | Candidate median [range] | B median [range] | Paired median saving vs A / B |
|---|---:|---:|---:|---:|
| Submit-to-commit | 1,632.91 [1,148.90–2,360.26] | 1,658.60 [1,420.80–2,091.34] | 1,344.71 [1,165.95–1,709.57] | +23.63 / −365.74 |
| Broker CPU | 271.48 [252.47–301.80] | 258.67 [253.72–280.41] | 260.01 [247.65–282.91] | +16.89 / −1.82 |
| PostgreSQL backend CPU | 260.22 [226.16–283.75] | 235.58 [221.92–272.20] | 244.89 [226.32–306.23] | +20.10 / +19.10 |

The candidate beat A in three of six wall-time comparisons and B in zero. The identical controls differed by a median 368.94 ms and as much as 913.30 ms. One-minute host load ran from 56.55 to 71.36 on 15 logical CPUs. Both broker CPU and backend CPU were much smaller than elapsed time. The earlier instrumented baseline attributed 929 ms of a 981 ms warm submission to `Jobs.enqueue_in`, including PostgreSQL and transport wait. The changed collector accounts for roughly 11 ms of component time here. The lower backend CPU is descriptive since the scalar database work was identical. [Earlier stage attribution](../2026-11-23-direct-producer-v3-setup/README.md) · [Paired raw receipts and original proof-harness failures](local-raw.tar.gz)

The control and candidate passed the mixed, exact-limit, count-limit, slow, dense, and sparse receipt proofs. The real database XID groups matched the traced membership and ordering. Both rejected malformed input, a mismatched request ID, out-of-scope paths, invalid lookahead values, and oversize specs. A late invalid field rolled back 128 writes. A late oversize spec rolled back one prior scalar write. Explicit rollback left no durable records. The one-batch probe accepted the exact byte limit and rejected an overfull batch.

## Controller handoff

The existing Linode controller accepted the sealed request into its queue after priority work. It asks for one broker-only link using the stock resident compiler and runtime, a diagnostic SETUP, then five matched native large rotations. Both ordinary controls use the original broker. All six lanes inherit the just-completed 16 MiB EXTERNAL inventory, original ordinary runner, three original peers, CPU caps, data, clocks, and scorer. The controller owns the host grant, logs, and results. [Request and commands](native-request.json) · [Controller queue receipt](verification/controller-ready.txt) · [Freeze](native-freeze/freeze.json)

The sealed link script relinked the same source with the matched Darwin compiler and runtime and returned the same artifact on a second install. The relinked binary passed the TLS producer proof. Offline inventory validation preserved both identical controls, all three original peer definitions, and the warmup and measured phases of all five rotations. [Offline receipt](verification/native-offline.json)

## Workspace gates

The workspace build, formatting, core tests, runtime tests, language proofs, and the PostgreSQL 17.11 compatibility and parity-driver proof all passed. The tests compiled the archived source from `6c802a19`. The PostgreSQL wrapper's checkout banner came from the enclosing live worktree. [Commands](verification/gate-commands.txt) · [Source verification](verification/source-blobs.json) · [Exit status](verification/gate-status.txt) · [Raw logs](verification/gate-raw.tar.gz)

## Replay

Run from the repository root. The retained local toolchain and adapter are identified in [the exact build pins](pins/direct-local.json). The archived calibration and per-job database receipts are in `local-raw.tar.gz`.

```sh
d=tools/durable-jobs-bench/results/2026-11-23-ordinary-broker-once
python3 -B "$d/replay.py" | cmp - "$d/summary.json"
(cd "$d" && shasum -a 256 -c SHA256SUMS)
```

To rerun the compiled and PostgreSQL work, use fresh paths inside the owned target directory.

```sh
d=tools/durable-jobs-bench/results/2026-11-23-ordinary-broker-once
w=target/ordinary-broker-once
mkdir -p "$w/tmp"
export PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin
export TMPDIR="$PWD/$w/tmp"
python3 -B "$d/build.py" "$w/build-replay"
python3 -B "$d/component.py" "$w/component-replay" "$w/build-replay"
python3 -B "$d/local.py" proof "$w/proof-replay" "$w/build-replay" "$w/component-replay"
python3 -B "$d/local.py" time "$w/timing-replay" "$w/build-replay" "$w/component-replay"
```
