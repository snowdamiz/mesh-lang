# PostgreSQL Bind integration

Product commit `7e6c956048270de94d229841bff83d40577ea2c8` adds the isolated
`436d705463a01e2fb4ecfecc493abbd3d79e69b3` driver change to `t3/9a80aabe`.
It changes two files. Both product blobs match the isolated commit byte for
byte. The shared parent is `440d7ed2aed500df113f613e9c8ad2ea9aa6ddc7`.
[Original patch](pg-bind-single-copy.patch)

## Driver review

- Execute, text query, typed query, checked Jobs claim, and the scoped Jobs
  effect session all use the updated Bind writer. The opt-in PostgreSQL 17
  set-based enqueue path passes its namespace and JSON batch through
  `PgJobDb::query` to the same text query writer. The batch's value-limit
  fallback stays in the store. Typed queries still obtain the result formats
  via a separate Parse/Describe/Sync exchange.
- The Bind tag precedes the signed big-endian length, which includes its four
  bytes. The two unnamed C strings, signed 16-bit format and parameter counts,
  per-value signed 32-bit lengths, NULL sentinel, value bytes, and result
  formats retain their original order. Result format count is bounded by the
  signed RowDescription count and the caller emits only 0 or 1. Execute/Sync
  adds 15 bytes. Describe/Execute/Sync adds 22 bytes. The suffix is reserved
  with the Bind and excluded from the Bind's wire length.
- The writer checks the 32,767-parameter cap, then the 16 MiB per-value cap,
  then the 64 MiB frame cap, before reserving or writing. Accumulation in
  `u64` can represent the maximum sum on either pointer width. The bounded
  frame and suffix fit into 32-bit and 64-bit `usize` before reservation and
  the frame length fits the wire's `i32`. The effect session still reports
  a Bind validation error as `BeforeSubmission` before its TLS send.
- The change removes the intermediate Bind allocation and body copy in the
  driver. The historical live queue timings remain unresolved.

## Archive provenance

The [original manifest](ORIGINAL-SHA256SUMS) covers the incoming patch,
fixtures, programs, summaries, and receipts without changing their bytes.
The [source bundle](isolated-436d7054.bundle) preserves the exact isolated
commit object with stock commit `29abb9cb` as its prerequisite.
The archived replay accepted an arbitrary `--candidate` directory for the
extracted Rust and externally supplied runtime archives for its live
comparisons. Its recorded archive and binary digests identify those artifacts.
The packet contains aggregate transcripts, timing rows, and condensed gate
output. The four runtime archives, per-frame original records, raw captured
Bind values, full original gate logs, and a source-to-archive attestation
are absent. The archived transcript comparisons can be checked as receipts.

The archived PostgreSQL log shows claim-admission checks. The checked-in
gate at the isolated candidate invokes claim admission alone. The archived
README's set-based enqueue claim extends beyond its log and that source
revision. The current shared-source script runs both proofs.

The [pinned replay](audit_replay.py) recreates the 64-job input and checks its
digest, reproduces the archived summary byte for byte, applies the raw patch
to a scratch copy of the stock blobs, and compares both outputs with Git.
It reads the isolated source from the exact commit into a separate directory
and passes that snapshot to the original Rust extraction and differential.
The 257-value case models the sum that would overflow a 32-bit accumulator.
The executable replay runs on Darwin ARM64.

Run from the checkout with Rust on `PATH`.

```sh
cd tools/durable-jobs-bench/results/2026-11-23-pg-bind-single-copy
shasum -a 256 -c ORIGINAL-SHA256SUMS
python3 -B audit_replay.py
```

## Workstation stop and native handoff

Before the local timing stop, the integrated diagnostic linked the fresh
runtime and captured the same verified PG17.11 TLS transcript digests as the
archived stock run for the mixed values and the 64 ordered enqueues. It
checked committed data and the argument lengths. The launcher wrote both
diagnostic receipts and then hit zsh's read-only `status` variable when
saving the exits. The completed outputs are retained as old Darwin
diagnostics. The queued retry was cancelled before it acquired the lock.
The unrestricted workspace command was interrupted during parser tests.
Both launchers terminated and released their lock ownership.

The [native request](native/request.json) was submitted as
`pg-bind-single-copy-7e6c9560` through the sole Linode controller's existing
queue. Its source seal binds the exact integrated and stock encoder inputs,
the extracted Rust and the allocation probe. SETUP qualifies two native
release binaries. A separate TIMING grant runs eight balanced groups with
duplicate controls and preserves raw output. The decision uses driver
copying and allocation counters. The request is queued behind the active
grant, with no native result yet.

## Fresh correctness gates

The [gate receipt](integrated/fresh-gates.json) records the commands,
checkout revisions, exit codes, suite counts, and hashes of the saved logs.
The build, workspace correctness, and PostgreSQL wrappers check their
workspace inputs against `7e6c9560` before and after execution. The shared
HEAD advanced through benchmark evidence commits during the run. The
Cargo workspace and PostgreSQL proof inputs stayed identical to the
product commit.

The workspace build used the checkout's existing Cargo target. The
workspace test command excludes the performance, timed collection and actor
benchmarks, timing-distribution tests, and measurement-based soak cases.
The PostgreSQL script runs the set-based enqueue relay only on 17.11,
where the opt-in writer installs. The effect-fence SQL proof runs on all
three versions. The compiled effect gate requires GNU/Linux ARM64 or AMD64
and returned exit 2 on Darwin. The differential models a 32-bit overflow
input on the ARM64 host.
