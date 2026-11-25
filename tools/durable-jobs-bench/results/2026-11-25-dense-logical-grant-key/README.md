# Dense grant by logical primary key

In the first complete instrumented Linode rotation, the public grouped-hot runner selected 320 jobs in 25 sampled grants. The UPDATE visited all 1,024 rows in the namespace once per grant. PostgreSQL applied the job ID at the join, which rejected 327,360 combinations. The target index scan recorded 27,487 shared-buffer hits. Each sampled statement fell entirely inside the drain window, including a 1 ms allowance for timestamp precision on each side. The second instrumented rotation stopped at readiness. The first sampled arm took 168 ms longer to drain than its duplicate-control mean. These counts identify repeated work. Its contribution to uninstrumented wall time is open.

The candidate carries the locked row’s namespace alongside its job ID through the materialized selection. The UPDATE retains both scalar equalities and specifies a closed range on the same non-null composite primary key. Both range endpoints refer to the same selected row. The query still picks, locks and updates in one statement. The physical tuple address can change during a concurrent update, so the lookup uses the logical key.

## PostgreSQL 17.11 proof

The cold-start check prepares each statement, executes six empty picks, inserts the backlog, and explains the next execution in the same session. The stock plan uses only the namespace column of the primary key. Carrying the namespace without the composite bounds leaves the ID as a filter. The final candidate has index conditions on both columns of the primary key. The 15 analyzed cases cover five queue shapes under automatic, forced custom, and forced generic planning.

In the concurrent-update cases, both implementations returned the updated context for the first row. With two eligible jobs, both returned both in order. The receipt includes the checks for namespace isolation, capacity, oversized and empty picks, and commit visibility.

From the directory containing this file, use a checkout containing the pinned source revision and the sibling fixture directories. The second command starts a temporary PostgreSQL 17.11 server for untimed SQL correctness and ordinary EXPLAIN.

```sh
python3 -B live.py | cmp - live.json
python3 -B proof.py | cmp - proof.json
shasum -a 256 candidate.sql
```

The canonical SQL SHA-256 is `d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45`. Native plan validation and duplicate-control timing belong to the sole Linode controller. The two-command fresh-snapshot implementation is held as a fallback. The earlier single-statement physical-TID implementation failed the concurrent-update proof.
