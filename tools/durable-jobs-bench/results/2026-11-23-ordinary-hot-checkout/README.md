# Native ordinary V2 hot checkout

On November 23, 2026, four diagnostic arms and five matched timing
rotations exercised the D2-only runner on CPUs 0 and 1 with PostgreSQL
17.11 and verified TLS. The native D2 ELF was
`af0d1dc5151a24031f7c09020eb077c2440b36cb94793799b5c358a7e7a21c7b`.
The two controls were identical stock V2. All three Mesh lanes used
the stock broker and the V2 grouped-success policy. Each arm kept 256
independent application commits for 128 jobs. [Release and configuration](controller/)

| Lane | Submit jobs/s | Drain jobs/s | Submit-to-settlement p99 (ms) | Pinned CPU (ms/job) |
| --- | ---: | ---: | ---: | ---: |
| D2 | 485.58 | 687.68 | 262.99 | 3.594 |
| Stock V2 A | 492.77 | 689.04 | 257.64 | 3.594 |
| Stock V2 B | 506.03 | 714.41 | 252.95 | 3.516 |
| River | 563.09 | 606.95 | 227.32 | 2.734 |

D2 sent two checkout probes per arm. Each stock control sent 256.
D2 finished ahead of both controls in one submit window and two drain
windows. It beat River in three drain windows and zero submit windows.
CPU steal was zero. Full-workload improvement failed to replicate.
The hot path preserves error reporting without
automatic retries. A silently stalled connection still inherits statement
wait semantics. [Native summary](native-summary.json)

The first SETUP compiled the exact runner and exited when `findmnt`
returned padded block-device text. A second SETUP reverified the same ELF,
runtime and host, trimmed the device display field, and passed all four
smokes. The first timing invocation then exited before any arm after JSON
serialization sorted its lane keys. The recovered launcher restores the
five original sealed rotation orders. The packet retains both exits and
all successful raw arms. [Original SETUP](evidence/ordinary-hot-checkout-1393b4a009b8/status.json) · [Source freeze](sealed-inputs/target/ordinary-hot-checkout/native/freeze.json)

## Offline replay

Restore into a fresh directory and supply the existing repository tooling.
The restore reconstructs the compiled database source from the sealed
original source archive and exact D2 patch. Both replay commands run offline.

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-ordinary-hot-checkout
driver=$(python3 -B "$packet/replay/restore.py" target/d2-results-replay)
python3 -B "$driver" summarize > target/d2-results-replay/summary.json
cmp target/d2-results-replay/summary.json "$packet/native-summary.json"
```
