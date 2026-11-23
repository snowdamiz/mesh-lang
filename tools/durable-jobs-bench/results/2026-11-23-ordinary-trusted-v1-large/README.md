# Ordinary V1, direct-file producer, and EXTERNAL storage

Five native crossover blocks ran on November 23, 2026, with workers and
PostgreSQL 17.11 sharing CPUs 0 and 1. All engines used EXTERNAL storage
and received the same 64 jobs of 512 KiB each. The two Mesh lanes used the
same direct-file broker. The ordinary lane ran trusted in-process handlers.
The protected lane ran fresh attempt processes with fenced application
effects. Both kept two independent application commits and one queue
terminal transaction per job. [Frozen comparison](freeze-retry/external.json)

| Lane | Jobs/s from submit, median [range] | Jobs/s from first commit | Submit-to-settle p99 (ms) | Runner CPU (ms/job) | Pinned CPU (ms/job) | Observed arm (s) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Ordinary V1 | 48.63 [47.38–52.65] | 106.61 | 1,316 | 17.77 | 35.31 | 9.50 |
| Protected Mesh | 41.04 [40.00–41.94] | 76.80 | 1,559 | 23.28 | 42.50 | 9.77 |
| Oban | 37.19 [31.51–38.27] | 116.07 | 1,721 | 145.42 | 40.78 | 15.95 |
| River | 27.54 [27.43–27.82] | 70.62 | 2,324 | 14.73 | 52.03 | 9.02 |
| Apalis | 73.98 [69.93–79.16] | 331.07 | 865 | 5.80 | 21.72 | 8.60 |

Ordinary V1 finished ahead of protected Mesh, Oban, and River in every
block from first submit. The median was 18% higher than protected Mesh,
31% higher than Oban, and 77% higher than River. Apalis finished ahead of
V1 in every block. Pinned CPU includes PostgreSQL and covers the sampled
interval from submit through terminal settlement. The observed arm includes
database setup, pre-submit DDL, and post-settlement checks. [Native comparison](evidence/ordinary-trusted-large-assembly-matched/comparison.csv)

The policy replay covered all 25 measured arms and 25 warmups. Every
arm reconciled 64 byte-exact uncompressed payloads and 128 independently
committed application writes. The original smoke failed the launch
diagnostic when Linux denied a cross-UID `/proc/PID/exe` read. The retry
uses the owned launch, exact live argv, process lineage, and lifetime
witnesses. Both default and EXTERNAL smoke receipts and the earlier
failure are included. The timed series uses the corrected overlay and
EXTERNAL policy. [Native attempts](evidence/native) · [Exact overlay manifest](freeze-retry/freeze.json)

The ordinary runner and protected runner hashes are in the V1 comparison.
The direct broker and peer executables match the protected symmetric
EXTERNAL series. [V1 executable pins](../2026-11-23-ordinary-trusted-v1/README.md) · [Broker and peer hashes](../2026-11-23-payload-storage-symmetric/README.md)

Replay the timing from the checkout root. To rerun the per-arm storage
and launch replay, copy this packet into a fresh directory directly under
`target/` and run `replay-retry/receipt.py block external BLOCK_DIR` with
the copied block path.

```sh
suite=tools/durable-jobs-bench
archive="$suite/results/2026-11-23-ordinary-trusted-v1-large"
PYTHONPATH="$suite" python3 -B "$archive/replay/interleave.py" summarize ordinary-trusted-large-assembly-matched
(cd "$archive" && shasum -a 256 -c SHA256SUMS)
```
