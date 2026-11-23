# Ordinary trusted V1 replay base

This packet preserves the exact source and launch recipe for the November 23,
2026 native ordinary trusted experiment. Its base is
`3828f20d2d6beb683868cce33e6dd1958365914b`. The source archive used for the
native setup has SHA-256
`e742644d89b872a41b4d9d241091a2349491f3de6d393fe77c4cbdbc05f6a58c`.
The code delta is available separately for review. [Source patch](replay/source-draft.patch.gz)

## Execution contract

Ordinary V1 runs trusted handlers inside the warmed runner processes.
The fixture performs two independent committed application writes through
the database role. It preserves queue-token checks at terminal settlement,
verified TLS, the connection and occupancy budgets, and the durable
ledger. The protected stock path uses a fresh worker process and
token-fenced application effects. The V1 experiment trusts the shared
application process and its application writes. Its scores use the
`ordinary-trusted-shared-process` contract.

The native V1 runner ELF has SHA-256
`faefaf34344958650a0fbdd19556d86d9f5126c5007b9c77225447004a6781e4`.
The protected Mesh control uses
`3dc20c730bd269da86c9c98f225d5f85bdc0dd3d3a31908e0eac90aa50d8dfd2`.
The peer executable hashes match the published storage series. Each
profile's `series.json` records the exact adapter, artifact, and PostgreSQL
pins for every lane. [Peer executable hashes](../2026-11-23-payload-storage-symmetric/README.md)

## Native comparison

The November 23 arms shared CPUs 0 and 1 with PostgreSQL 17.11. Every arm
reconciled two independently committed application writes per terminal job.
All four profiles ran five matched blocks. Dense used 1,024 tiny jobs
and appears as `noop` in the inventory. Short used 128 jobs, slow used
512, and sparse submitted 24 jobs individually.

| Lane | Dense jobs/s from submit | Short jobs/s from submit | Slow jobs/s from submit | Sparse submit-to-settlement p99 (ms) |
| --- | ---: | ---: | ---: | ---: |
| Ordinary V1 | 358.77 | 442.10 | 243.52 | 23.36 |
| Protected Mesh | 174.69 | 200.58 | 163.20 | 32.22 |
| Oban | 648.15 | 280.80 | 245.29 | 232.74 |
| River | 843.91 | 719.93 | 286.93 | 242.60 |
| Apalis | 1,139.36 | 843.41 | 344.19 | 16.09 |

The table reports five-block medians. Dense V1 ranged from 355.60 to
365.64 jobs/s and beat protected Mesh in all five blocks. Short V1 ranged
from 278.76 to 466.03 jobs/s and beat Oban in four blocks and River in one.
Median CPU on the two pinned CPUs, including PostgreSQL, was 4.38 ms/job
for short V1 and 9.06 ms/job for protected Mesh. Slow V1 ranged from
227.33 to 253.18 jobs/s from submit and reached 364.41 jobs/s from first
commit. Sparse V1 used one slot and five connections. Its p99 settlement
latency was lower than Oban and River in all five blocks. [Dense results](evidence/ordinary-trusted-v1-dense/summary.json) · [Short results](evidence/ordinary-trusted-v1-short/summary.json) · [Slow results](evidence/ordinary-trusted-v1-slow/summary.json) · [Sparse results](evidence/ordinary-trusted-v1-sparse/summary.json)

The separate large comparison combines the V1 worker with a direct-file
producer and tuned PostgreSQL storage. [Large results](../2026-11-23-ordinary-trusted-v1-large/README.md)

## Rebuild and replay

Restore the exact packet to a fresh directory inside the checkout.

```sh
tools/durable-jobs-bench/results/2026-11-23-ordinary-trusted-v1/restore
```

The restored `replay/setup.py` uses the existing Linode, PostgreSQL 17.11,
the archived current Mesh AMD64 compiler and link ABI, and the three matrix
locks. Run it under an `ordinary-trusted-v1` SETUP assignment with the
`source_archive_sha256` argument in the archived pins. The live queue
request carries the corresponding source, setup, and peer inventory pins.
A successful short smoke produces `evidence/native/qualified-short.json`.
[Recipe pins](replay/pins.json)

```sh
base=target/ordinary-trusted-v1-replay
export PYTHONPATH="$PWD/tools/durable-jobs-bench"
python3 -B "$base/replay/setup.py" "$base/ordinary-trusted-v1.tar.gz" \
    e742644d89b872a41b4d9d241091a2349491f3de6d393fe77c4cbdbc05f6a58c
python3 -B "$base/replay/interleave.py" dispatch ordinary-trusted-v1-short \
    --config "$base/evidence/native/qualified-short.json"
python3 -B "$base/replay/interleave.py" summarize ordinary-trusted-v1-short
```

Run the dispatch under a separate controller timing assignment. With a
saved series under the restored `evidence/` directory, the summarize
command runs offline.
