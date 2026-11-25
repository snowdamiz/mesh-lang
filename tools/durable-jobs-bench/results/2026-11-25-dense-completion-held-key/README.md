# Dense grouped completion held lookup

**Closed before native intake.** Keep the released fixed8ms + d388
completion function. The per-input primary-key variant changed the returned
ordinals for identical duplicates and increased lock-side scan buffer hits
in 240 of 256 matched plans. The smaller array predicate also changed the
ordinals. The composite range kept the observed duplicate result but failed
to produce a primary-key point lookup in every forced-generic case. No
controller booking was made.

## Baseline

[stock.sql](stock.sql) is byte-identical in the historical grouped
`b3557273d72be3802bf189b284e7caaf5f3caa0f` migration, the grouped-refill
source archive and the public checked-bounded correctness revision
`3c4032d06b10bc54a47290c673562fb063b47789`. In the latter revision it
lives in `compiler/mesh-rt/src/jobs/grouped_schema.sql`. The released
fixed8ms + d388 performance lane used grouped-refill ELF SHA-256
`d56d0e6112a94ff601bebf6f777061068ed2fe0f2f2da586963dd6e1326a220a`.
The d388 overlay replaces the grant function. Fixed8ms changes the writer
schedule. Both use the same grouped completion body.
[Released packet](../2026-11-25-dense-fixed8-logical-factorial/controller-packet-retry.json)

The historical replay verified the full redacted nested body in all 62
samples. Their expiring-index scans returned 2,259 executing rows and hit
1,614 shared buffers for 167 settled jobs. The UPDATE used full primary-key
equality and returned one target row per settled job. Stock itself also
chose full primary-key equality in 24 of 256 local plans, all custom. The
repository search found no separate grouped completion deployment with a
point-key rewrite. The scalar renew and settlement functions already use
the logical key and a lease-token guard. They retain their own contracts.
[Historical reduction](../2026-11-25-dense-factorial-interaction/SETTLEMENT-SCANS.md)

## PostgreSQL 17.11 results

The three trial bodies change only the materialized `held` CTE. Each
retains the exact `checked` and UPDATE suffix, the signature and
configuration, and the lock strength and skip policy. The replay replaces
the actual function as its owner and checks invoker execution, ACLs and
role access after every replacement. It covers wrong and null tokens,
cancelled and expired attempts, namespace isolation, cardinality errors
without retained row locks, locked-prefix progress, rollback and retry,
idempotence, lower array bounds, and scalar claim and cancellation
alongside grouped settlement.

Four READ COMMITTED races commit a concurrent renewal, expiry, token
rotation or cancellation after the completion snapshot and before the row
lock. A fixture-only RLS predicate holds the command at an advisory gate
for those tests. A fixture-only clock probe verifies that the held row
already has the no-key-update lock when completion evaluates the lease
deadline. All four variants agreed on those cases, including the exact
expiry boundary.

The caller-visible counterexample uses `[5, 4, 5, 4]` and the matching
leases in the actual function at default plan settings. Stock and the
range return `[3, 4]`. The array and lateral variants return `[1, 2]`.
Each completes the same two jobs once. PostgreSQL allows either join row
to supply `RETURNING` when multiple source rows match an UPDATE target.
This experiment required preserving the observed ordinals as well as
the durable row changes.
[PostgreSQL 17 UPDATE](https://www.postgresql.org/docs/17/sql-update.html)

## Physical access

The untimed EXPLAIN matrix uses 1,024 rows, clustered or scattered executing
rows, executing counts 0, 1, 4, 8, 16, 32, 48 and 64, widths 1 through 8,
and both forced generic and custom plans. Each analyzed update rolls back.
The counts compare the jobs scan inside `held` against the corresponding
stock plan. The plan receipt also retains UPDATE-side access.

| Held variant | Full primary-key scans out of 256 (128 generic) | Fewer / same / more shared hits | Width ≥4 and executing ≥16, fewer / same / more |
| --- | ---: | ---: | ---: |
| [Array predicate](array.sql) | 16 (0 generic) | 164 / 92 / 0 | 80 / 0 / 0 |
| [Composite range](range.sql) | 24 (0 generic) | 9 / 218 / 29 | 3 / 56 / 21 |
| [Per-input locked probe](lateral.sql) | 240 (112 generic) | 11 / 5 / 240 | 3 / 0 / 77 |

For example, at 48 scattered executing rows and width 8, the generic
per-input probe recorded 38 scan hits versus 16 for stock. The separate
array filter often saved heap hits by putting the ID restriction into the
partial expiring index. That index is ordered by lease expiry before ID.
The generic array result and the duplicate counterexample leave that
rewrite outside this point-key follow-up.

The released warmed River lead was a paired median of 272.638 ms from
submit and 158.896 ms from enqueue commit. The historical counter
replay prices tying River at 0.64–2.81 ms of critical-path savings per
grouped call if every microsecond advanced final settlement. These local
plans contain no timing and supply no estimate of a native wall-clock
saving. The prior peer results and their original archives are unchanged.
[Warmed River release](../2026-11-25-dense-fixed8-river-readiness/NATIVE-RESULTS.md)

## Replay

Use PostgreSQL 17.11 binaries. Set `PG17_BIN` to their directory if needed.
The replay starts and cleans up an isolated socket-only cluster. The
compressed schema fixture is the byte-exact b355 migration. The SQL variants
are experiment inputs under this directory. No product migration is modified.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-completion-held-key
python3 -B "$d/replay.py" | cmp - <(gzip -dc "$d/evidence/local.json.gz")
python3 -B "$d/historical.py"
```

The historical check uses committed sibling evidence. For a narrow Git
export, include this directory, the dense factorial interaction reduction,
the dense logical grant native diagnostic, the fixed8 logical factorial
packet, and the fixed8 River readiness evidence.
