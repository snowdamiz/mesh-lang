# Performance suite contract

All four programs run as standalone processes. The runner selects the mode with
`PERF_MODE` and supplies the environment variables below. Numeric variables
contain base-ten integers. The runner gives each trial a fresh process.

## Workloads

- `actor` starts `PERF_PAIRS` independent caller and worker pairs before the
  timer. Each caller sends the integers 1 through `PERF_ITERS`, one at a time,
  and waits for the reply before sending the next. The worker owns a counter
  and replies with its updated value. Check each reply. The checksum is the
  sum of the final counters. Measure wall time through the final reply. Sample
  round-trip latency every 64 iterations, starting at iteration 1.
- `cpu` reads a JSON array of records from `PERF_FIXTURES` and decodes it before
  starting the timer. Visit `PERF_ITERS` records in array order, wrapping at the
  end. The checksum is the sum of their scores. Each record contains a
  nonnegative integer `id` and an array of bounded integers `values`.
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
  failed worker in Go and Rust. The replacement starts with empty state.

`PERF_WARMUP` specifies an optional number of untimed operations before the
actor and CPU trials. Keep the timed actor counters separate from warm-up
counters. `PERF_WORKERS` gives the runtime's scheduler worker count where
applicable. The runner applies CPU affinity independently.

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
