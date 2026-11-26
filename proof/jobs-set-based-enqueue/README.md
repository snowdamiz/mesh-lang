# Set-based Jobs enqueue on PostgreSQL 17

A producer opts in with `MESH_JOBS_SET_BASED_ENQUEUE=1`. The default stays the
ordered scalar writer. `Jobs.migrate` adds `mesh_jobs.enqueue_batch_v1` and
`mesh_jobs.enqueue_batch_layout_v1` on PostgreSQL 17 only, beside the
unchanged `mesh_jobs.enqueue`. Jobs schema version 1 is unchanged. The
[role grants](../../scripts/fixtures/jobs-role-grants.sql) give `EXECUTE` to
the producer when the functions exist and skip them otherwise.

## Choosing the writer

The client chooses before any batch write. It needs 16 to 128 specs without
`unique` or `replace` and without an encoded NUL. It runs one capability
query on the same connection. That query requires PostgreSQL 17, READ
COMMITTED, `EXECUTE` on both functions, receipt `SELECT` and `INSERT`, and a
shared Jobs migration lock.

The namespace fence is then taken exactly as for the ordered writer. The
server function locks the four Jobs tables and fingerprints them: storage,
columns, constraints, indexes, triggers, rules, row security, and the
fingerprinted function definitions. `EXTERNAL` and the default `EXTENDED`
storage on `mesh_jobs.jobs.args` fingerprint alike, from the qualified
[EXTERNAL compatibility hunk](../../tools/durable-jobs-bench/results/2026-11-23-slow-external-compat/frozen/ce5e551d-external-layout.patch).
The fingerprint excludes ACLs. It answers
`stock` before any write for any other layout and for existing or repeated
request keys. The client then runs the ordered statements in the same
savepoint.

A qualified batch inserts queues and jobs with one ordered `MERGE` and
writes receipts. It returns them by source ordinal. PostgreSQL generates
each ID, position, clock sample, and effect key. The client accepts the
reply only with one valid insert receipt per input, in ordinal order.
Otherwise it rolls back to the savepoint and returns an error. It never
retries through the ordered writer after a batch was sent.

## Checks

| Check | Where |
|---|---|
| Field-by-field equivalence with the ordered writer, unusual JSON, schedules, queues, receipt fingerprints, replay | `set_based_enqueue_matches_the_ordered_writer_for_every_independent_field` |
| Each fallback: under 16, opt-in off, unique, duplicate and existing keys, REPEATABLE READ, migration lock, NUL error parity | `set_based_enqueue_keeps_the_ordered_writer_for_unqualified_batches` |
| Default and claim-opt-in schema after repeated migration. LZ4 session setting and EXTERNAL argument storage keep the writer. MAIN argument storage, EXTERNAL metadata storage, column compression, a trigger, and row security select the ordered writer until restored | `set_based_enqueue_layout_guard_tracks_catalog_changes_not_session_settings` |
| Old and new producers on one namespace fence, outer rollback of a batch and its queue | `set_based_and_ordered_writers_share_the_namespace_fence_and_roll_back` |
| Real TLS relay: partial row, missing row, wrong ordinal, non-insert receipt, COMMIT dropped, COMMIT reply lost, ReadyForQuery withheld; replay of committed batches by request key | [`prove.py`](../../scripts/fixtures/jobs-set-based-enqueue/prove.py) with `set_based_enqueue_never_reports_an_uncertain_reply` |
| Scoped producer runs a real batch on fresh and upgraded databases. Other roles are denied. Grants survive migration. Functions are absent off PostgreSQL 17 | [`verify-jobs-roles.sh`](../../scripts/verify-jobs-roles.sh) |

```bash
cargo test --workspace --locked
PG_BIN_DIR=/path/to/postgres/bin bash scripts/verify-postgres-jobs.sh \
  --compatibility 17.11 --parity-driver-check --j6-roles
```

Repeat the database command with PostgreSQL 18.7 and 19.0. Those versions
keep the ordered writer, and the relay proof reports that it skipped.

## Native evidence

These runs used the pre-integration source `ce5e551d079e83bfab5e6ba1d52b47892bf35319`
(local branch `t3/set-based-enqueue-v2`) from
[bulk-enqueue-ce5e551d079e.tar.gz](../../tools/durable-jobs-bench/results/2026-11-23-set-based-enqueue/native/bulk-enqueue-ce5e551d079e.tar.gz)
(SHA-256 `542925005370032c71db1a2aa488c2d35d16f14f9fdc8f54ee0afbb81f80fc82`).
That source was gated locally on PostgreSQL 17.11 and 16, then qualified in
native SETUP `3b09dc3d0e23`. The integrated product source adds tests,
proofs, docs, and an internal parameter. It has not run on the Linode host.
Its native ELF identity is pending a new correctness run.

| Series, Linode, PostgreSQL 17.11 | Bulk/scalar submit, same ELF | Bulk/scalar commit | Medians, submit jobs/s |
|---|---|---|---|
| Ordinary V2 short, 5 groups | 1.212× median, 5/5 | 1.033×, 4/5 | bulk 596.54, scalar 498.16, River 697.01, Apalis 742.12, Oban 308.13 |
| Ordinary V1 slow, 5 groups | 1.045× median, 5/5 | 0.970×, 1/5 | bulk 261.68, scalar 250.39, River 288.66, Apalis 355.45, Oban 247.27 |

- Setup qualification and V1 slow raw ledgers:
  `tools/durable-jobs-bench/results/2026-11-23-set-based-enqueue/evidence/native-setup-3b09dc3d0e23`
  and `…/bulk-enqueue-v1-slow`. The controller committed them in `03dfb958` and `bad1a384`.
- V2 short: inventory, orders, and replay pinned by the controller in `119493e5`.
  The raw packet `tools/durable-jobs-bench/results/2026-11-23-bulk-enqueue-v2-short/evidence/bulk-enqueue-v2-short`
  holds the first dispatch `29083af379b0`, exit 1. Its group 2
  (`02-short.1795473812802464000`) failed the 1% CPU-steal limit at 1.06%.
  It also holds the unchanged retry dispatch `8eef0f9b8239`, exit 0, with
  the same config and driver hashes. The summary uses the retry's group 2
  (`02-short.1795473978600006000`). The packet SHA256SUMS is `4a0a46aa…0e35`.
- Sealed source packets: `75bbb906`.
