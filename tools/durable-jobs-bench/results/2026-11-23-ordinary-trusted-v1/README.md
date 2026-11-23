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
The short profile used 128 jobs per arm and the slow profile used 512.
Both ran five matched blocks. The dense checkpoint uses 1,024 tiny jobs
and one matched block recorded as `noop` in the inventory.

| Lane | Dense jobs/s from submit | Short jobs/s from submit | Slow jobs/s from submit | Slow jobs/s from first commit |
| --- | ---: | ---: | ---: | ---: |
| Ordinary V1 | 358.24 | 442.10 | 243.52 | 364.41 |
| Protected Mesh | 174.69 | 200.58 | 163.20 | 206.13 |
| Oban | 641.43 | 280.80 | 245.29 | 321.21 |
| River | 912.49 | 719.93 | 286.93 | 391.57 |
| Apalis | 1,139.36 | 843.41 | 344.19 | 445.15 |

Short and slow values are five-block medians. V1's short submit range was
278.76–466.03 jobs/s. It finished ahead of protected Mesh in five blocks
and Oban in four. Median CPU on the two pinned CPUs, including PostgreSQL,
was 4.38 ms/job for V1 and 9.06 ms/job for protected Mesh. On slow, the
V1 range was 227.33–253.18 jobs/s from submit. The slow lane finished ahead
of protected Mesh in five blocks and Oban in one. The separate large
comparison combines the V1 worker with a direct-file producer and tuned
PostgreSQL storage. [Dense checkpoint](evidence/ordinary-trusted-v1-dense/summary-partial.json) · [Short results](evidence/ordinary-trusted-v1-short/summary.json) · [Slow results](evidence/ordinary-trusted-v1-slow/summary.json)

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
