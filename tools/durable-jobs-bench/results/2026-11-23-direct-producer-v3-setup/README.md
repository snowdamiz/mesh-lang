# Mesh direct-file producer

The direct-file arm sends an absolute fixture filename and request ID over the existing warm broker connection. Mesh checks both against the private trial directory. The broker calls the frozen `read_batch` and `transact_batch` functions on its existing PostgreSQL session. The three Mesh lanes share the same stock migrator, runner, and per-attempt worker executable.

The stock Mesh Python adapter reads, decodes, and encodes the batch before sending a 33,558,675-byte command across the pipe for the large profile. The Oban and Apalis adapters pass the path to their warm producers. River reads the batch in Go. The measured submit window begins before the producer gets the path, so it includes the file operation, decode, IPC where applicable, and the enqueue transaction. The harness generates and hashes each fixture before starting the trial, which can warm the filesystem cache. Both timers end at the last durable terminal commit. The post-commit timer starts at the first durable enqueue commit.

The large fixture has 64 jobs with 512 KiB per payload. The serialized input is 33,561,352 bytes. The trial ledger binds the input to the PostgreSQL enqueue XID, each durable queue ID and payload hash, and two independently committed effects per job. The archived four-engine baseline uses its original artifacts. This six-lane series measures a producer integration change using current stock Mesh controls and the original peer artifacts. [Four-engine baseline](../2026-11-23-baseline/README.md)

## Proof and replay

The macOS producer proof created the large fixture after `SESSION_READY`. It compared all 64 persisted hashes and queue IDs, exercised explicit rollback, and forced a fault after the first 128 writes. Malformed JSON, the wrong request ID, a relative path, a symlink, and a path outside the private fixture directory were rejected. This proof uses a warm PostgreSQL 17.11 TLS session and the stock migrator. The named-effect runner requires GNU/Linux. [Local producer proof](local-producer-proof.json) · [Local build manifest](local-manifest.json)

Run native commands during the matrix controller's assigned window. The setup command uses the retained compiler and runtime and performs one fixture link. Use a fresh evidence directory for a new setup attempt. The canonical worker pins the adapter subtree to CPUs 0 and 1 and places its own host sampler on a spare CPU. Large and dense allow 64 slots and 96 PostgreSQL connections. Short allows 16 slots and 20 connections. Sparse allows one slot and five connections.

```sh
helper=tools/durable-jobs-bench/native/direct-producer
setup=tools/durable-jobs-bench/results/2026-11-23-direct-producer-v3-setup
python3 -B "$helper/controller.py" setup "$setup"
python3 -B "$helper/audit_run.py" "$setup/large-executable-pins.json" "$setup/smoke" 33561352 --lane mesh-direct-file
python3 -B "$helper/audit_run.py" "$setup/large-executable-pins.json" "$setup/input-proof" 33561352 --trace
```

The controller added exact PostgreSQL and peer executable hashes to the
generated inventories during SETUP. The native large smoke used those pins
and reconciled all 64 jobs and 128 application commits. [Native smoke](large-smoke/reconciled.json)
· [Native build manifest](direct-manifest.json) · [Broker diff](broker-source.diff) · [Controller reservation](reservation.json)

The first default-policy rotation reconciled every job argument and queue
ID in all six ledgers. The stock Mesh broker receives a decoded
and re-encoded file from Python. The direct Mesh, Oban, and Apalis
processes first receive their path after the submit stamp. They read and
decode it before the existing enqueue call. The River receipt recorded
33,561,352 bytes, read completion at 28.161 ms, and decode completion at
277.369 ms after submit. [First rotation receipts](../2026-11-23-direct-producer-v3-large/blocks/01-large.1795463957064636000/reconciled.json)

The syscall run checks that the complete file was opened and read between the submit marker and the matching durable enqueue. It checks the same ledger as the trace-off run. Keep those diagnostic arms out of the timing series.

```sh
export PYTHONPATH=tools/durable-jobs-bench
driver="$setup/replay/interleave.py"
python3 -B "$driver" dispatch 2026-11-23-direct-producer-v3-large --config "$setup/large-executable-pins.json"
python3 -B "$driver" dispatch 2026-11-23-direct-producer-v3-controls --config "$setup/controls-executable-pins.json"
```

Replay a completed series from the raw attempts under its results directory.

```sh
set -o pipefail
python3 -B "$driver" summarize 2026-11-23-direct-producer-v3-large |
  python3 -B "$helper/summary.py"
```

## Producer attribution on macOS

The separate instrumented broker measured one large warm-path enqueue to a local
PostgreSQL database. In that 981 ms trace, the 64 calls to `Jobs.enqueue_in`
accounted for 929 ms of wall time, including PostgreSQL and transport wait.
`Json.encode` took 20.8 ms over 127 calls. Encoding runs 127 times because
`collect_specs` tries the next job to check whether it fits under the 1 MiB
limit. `File.read` and `Json.parse` took 7.2 ms and 6.1 ms. This run identifies
what to check on AMD64. [Stage data](local-attribution.json) · [Raw stage durations](local-attribution.log)

The native attribution command links an instrumented diagnostic broker with
the same compiler and runtime. It also repeats the warmed PostgreSQL rollback
and input checks. Run it in a separate SETUP window.

```sh
python3 -B "$helper/attribute_native.py" "$setup/native-attribution"
```
