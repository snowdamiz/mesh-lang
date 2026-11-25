# Dense queue heap layout on PostgreSQL 17.11

Lowering the jobs table's TOAST target to 512 bytes and selecting EXTERNAL argument storage left the dense job arguments inline. It also disabled the stock set-based enqueue writer's layout check. Keep the default layout for this profile. The two SQL settings produced no physical change that would justify a Linode timing reservation.

## Why look at the heap

The two native 2×32 claim diagnostics each recorded 1,024 inserts, 2,048 updates, and an estimate of 2,048 dead job tuples. One nested statement processed 1,024 rows and generated 1,896,311 and 1,900,681 WAL bytes across its calls. Its text was redacted. The counters establish the amount of WAL observed for the statement and leave the share due to arguments, indexes, and full-page images unresolved. [Native counter replay](native-claims.json) · [Original diagnostics](../2026-11-24-public-dense-set-claim/NATIVE-RESULTS.md)

## Physical layout witness

The untimed PostgreSQL 17.11 proof used the first 16 actual jobs from the checksummed dense input and the same scalar writer in three disposable databases. It checked the stock bulk layout decision separately, then claimed all 16 in order with the production set-based function and settled them with their lease tokens. Both the returned contexts and terminal arguments matched the input. PostgreSQL pageinspect returned the following sizes for every current row, using raw pages with detoasting disabled.

| Jobs table layout | Bulk layout accepted | Available / executing / completed tuple bytes | Physical args bytes in each state |
| --- | --- | --- | --- |
| Default | Yes | 1,285 / 1,317 / 1,301 | 1,110 |
| EXTERNAL args | Yes | 1,285 / 1,317 / 1,301 | 1,110 |
| EXTERNAL args and 512-byte TOAST target | No | 1,285 / 1,317 / 1,301 | 1,110 |

A rolled-back positive control expanded one payload from 1,024 to 5,120 bytes. In both EXTERNAL layouts the raw argument field then held an 18-byte external pointer and the completed heap tuple occupied 213 bytes. This witness covers storage and correctness only. Higher-level compression or separating immutable arguments into a separate table would be a different design. [Physical receipt](layout.json)

From this directory, replay the native counters offline and run the untimed geometry check with an installed PostgreSQL 17.11 binary directory and pageinspect.

```sh
python3 -B native_claim_wal.py | cmp - native-claims.json
python3 -B inspect_layout.py /opt/homebrew/opt/postgresql@17/bin | cmp - layout.json
```
