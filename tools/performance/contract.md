# Performance suite contract

All four programs run as standalone processes. The runner selects the mode with
`PERF_MODE` and supplies the environment variables below. Numeric variables
contain base-ten integers. The runner gives each trial a fresh process.

## Workloads

- `actor` starts `PERF_PAIRS` independent caller and worker pairs before the
  timer. Each caller sends the integers 1 through `PERF_ITERS`, one at a time,
  and waits for the reply before sending the next. The worker adds the received
  integer to its own counter and replies with the new value. Check each reply.
  Start all callers from a barrier and time with a monotonic clock. Use Mesh
  actors, Elixir processes, Go goroutines with capacity-one channels, and Rust
  Tokio tasks with capacity-one channels. The checksum is the
  sum of the final counters. Measure wall time through the final reply. Each
  caller samples one round trip per chunk of 64 timed iterations, starting at
  iteration 1. Within zero-based chunk `block`, sample at zero-based offset
  `(block * 17) % 64`. If the offset falls past the end of a partial final
  chunk, sample its last iteration. Keep the Mesh main function alive until
  the coordinator exits so the timed workload runs before scheduler shutdown.
- `cpu` reads a JSON array of records from `PERF_FIXTURES` and decodes it before
  starting the timer. Visit `PERF_ITERS` records in array order, wrapping at the
  end. The checksum is the sum of their scores. Each record contains a
  integer `id` from 0 through 1,000,000 and an array `values` of at most 256
  integers from -1,000,000 through 1,000,000. Accept integral JSON numbers
  such as `1.0`. Use distinct keys and ignore other fields.
  The score is `id + sum((index + 1) * value)` with zero-based indices.
- `serve` listens on the TCP port in `PERF_PORT`. `GET /health` returns
  `{"ok":true}`. `POST /score` decodes one record, computes its score, and
  encodes `{"id":<input id>,"score":<score>}`. Malformed JSON and invalid
  record shapes return 400. Use HTTP/1.1, application/json, and one request per
  TCP connection. The client sends Content-Length and Connection: close.
- `recovery` starts two independent workers. Check that both respond before
  timing. Fail one worker through its own message loop. Time from sending the
  failure request to receiving a successful reply from the replacement.
  Verify that the sibling retains its pre-failure state. Use a one-for-one
  supervisor in Mesh and Elixir. Use a dedicated restart loop watching the
  failed worker in Go and Rust. The replacement starts with empty state. Keep
  the Mesh main function alive until the recovery coordinator exits.

`PERF_WARMUP` specifies an optional number of untimed operations before the
trial, per pair for `actor` and in total for `cpu`. Keep the timed actor
counters separate from warm-up counters. Use at most 100,000,000 iterations
and 64 pairs so scores and checksums fit in a signed 64-bit integer.
`PERF_WORKERS` gives the runtime's scheduler worker count where applicable.
The runner sets the startup flags and applies CPU affinity independently.

## Results

After a successful actor, CPU, or recovery trial, print one JSON object on
standard output with integer `ops`, `elapsed_ns`, and `checksum` fields. The
actor object includes an array `rtt_ns` of integer samples. Recovery returns
`ops: 1` and `checksum: 1` after checking both worker states. A result mismatch
fails the process. Send diagnostics to standard error. The runner measures
startup and process memory separately.

Use semantic JSON equality for responses. Outside the benchmark, use the same
input and oracle for every program. Measure from a second host to compare HTTP
and sustained remote request/reply. Native cluster protocols need their own
contract for authentication, encryption, delivery, and failure behavior.
