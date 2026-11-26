# Set-based Jobs enqueue

The integration uses the real `Jobs.enqueue` and `Jobs.enqueue_in` APIs on
PostgreSQL 17. The producer opt-in is `MESH_JOBS_SET_BASED_ENQUEUE=1`.
Qualified 16–128 spec batches use PostgreSQL's ordered MERGE source and
return receipts indexed by the source ordinal. PostgreSQL generates the
job ID, position, per-row enqueue timestamp, and effect key. The existing
namespace fence, request fingerprints, savepoint, and outer transaction
remain in force.

Capability is detected on the active connection before the writer runs.
The server checks table, storage, constraints, indexes, functions, and
trigger layout with table locks. Custom callbacks can observe each stock
statement and can suppress rows or receipts. Those schemas use the
original per-spec statements. A server fallback response is emitted before
batch DML. Any transport, PostgreSQL, or response-shape error goes through
rollback. A caller can resolve an ambiguous COMMIT by replaying the same
request keys.

## Native producer screen

Six rotated A/A/B blocks per workload on PostgreSQL 17.11 with TLS, stock
durability, scoped producer grants, and wake triggers active. The control
was rebuilt at `3828f20d` with the same retained compiler and identical
fixture. Wall and CPU were sampled around the compiled session's transaction
and receipt. Every arm reconciled independent canonical fingerprints,
stored payload hashes, order, IDs, commit timestamps, and SQL call counts.
Load varied across arms on the development machine.

| Workload | Stock backend CPU median [range] | Candidate median [range] | Median paired stock/candidate CPU |
|---|---:|---:|---:|
| 1,024 noop | 228.7 [165.6, 364.9] ms | 61.1 [55.7, 93.9] ms | 3.93× [2.58, 4.51], 6/6 |
| 128 short | 26.6 [23.2, 33.7] ms | 14.0 [10.9, 17.1] ms | 2.01× [1.36, 2.45], 6/6 |
| 64 large | 679.9 [648.4, 794.8] ms | 680.6 [641.7, 796.2] ms | 1.01× [0.85, 1.06], 3/6 |
| 2 small | 4.86 [4.15, 5.09] ms | 4.38 [3.57, 6.04] ms | 1.11× [0.74, 1.33], 4/6 |
| 24 sparse | 16.68 [12.29, 21.11] ms | 15.95 [11.51, 26.26] ms | 1.01× [0.74, 1.36], 3/6 |

Each stock sample is the mean of the two controls in its rotation. The
1,024-job transaction replaced 1,024 scalar SQL calls with eight batch
calls. The short case replaced 128 with one. The 512 KiB input hit the
existing 1 MiB batch cap and issued singleton statements in every arm.
Two-job and sparse transactions also kept stock SQL. These results cover the
producer. Settlement and stock-peer comparison require the pending
controller-owned native series.

- [Final producer series](evidence/screen-final/series.json) and
  [per-arm summary](evidence/screen-final/summary.json)
- [Compiled PostgreSQL 17.11 proof](evidence/proof-final/PASS)
- [Compiled PostgreSQL 16 fallback proof](evidence/proof-pg16-final/PASS)
- [Original prototype and attempts](../native-bulk-job-enqueue)

The fault proof uses the original repository COMMIT relay before forwarding
COMMIT, after PostgreSQL confirmed it, and after forwarding CommandComplete
while withholding ReadyForQuery. The set-based response truncation sends one
complete DataRow and part of the next. The real compiled producer returns no
receipt in each fault. Typed application writes and receipt ledgers
reconcile after replay, including an actual compiled worker drain. Earlier
failed harness runs remain in `evidence`.

The native package keeps strict, ordinary trusted v1, and ordinary trusted
v2 execution contracts in separate short/noop timing configurations. The
stock peer inventory keeps River, Oban, and Apalis pinned. The existing
large/direct/external matrix has its own qualification.
