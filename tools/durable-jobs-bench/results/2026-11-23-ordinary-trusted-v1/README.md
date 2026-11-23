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
