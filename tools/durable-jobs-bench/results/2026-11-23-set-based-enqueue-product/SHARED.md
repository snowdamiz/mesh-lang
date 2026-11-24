# Set-based enqueue on the shared Mesh branch

Local commit `5dfca3d9` integrates the ten product files from `84e1cec9` on
`t3/9a80aabe`. PostgreSQL 17 deployments can opt in with
`MESH_JOBS_SET_BASED_ENQUEUE=1`. The ordinary configuration uses the ordered
writer. [Configuration and supported inputs](../../../../proof/jobs-set-based-enqueue/README.md)

The fresh shared-checkout PostgreSQL 17.11 run exited successfully with the
parity driver check, scoped roles on fresh and upgraded databases, and seven
enqueue relay cases. The script recorded checkout `2e4123b3`. Its differences
from `5dfca3d9` are confined to benchmark tooling and evidence. The original
gate log retains the failed forged COMMIT-tag case. The shared transaction
helper returned the committed receipts when the relay substituted a ROLLBACK
tag after the database committed. [Shared log](evidence/shared/pg-17.11.log.gz) ·
[Original gates and fault history](README.md)

Commit `3b20337f` archived the historical five-rotation V2 short comparison.
All 187 files in the original manifest replay, including 30 raw arm archives,
the CPU-steal rejection in dispatch `29083af379b0`, and the unchanged retry
`8eef0f9b8239`. The dispatcher logs reproduce their original hashes after
decompression. These timings used the experimental `ce5e551d` writer.
[Native short comparison](../2026-11-23-bulk-enqueue-v2-short/README.md)

The controller registered the pinned `84e1cec9` source bundle for an exclusive
Linux AMD64 correctness run behind the booked workload comparisons.
Its 1,200 regular files match the shared commit. The original request and
README are preserved with the controller's input hashes. The earlier README
describes the raw archive's status at submission. [Frozen request](request.json)

Run the offline check from the repository root.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-set-based-enqueue-product/replay/verify_archive.py
```
