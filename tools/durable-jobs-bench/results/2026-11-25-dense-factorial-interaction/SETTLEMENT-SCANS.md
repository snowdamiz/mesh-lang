# Grouped settlement on dense jobs

In the two released Linode diagnostic arms, the first lock lookup in `complete_batch_v1` examined 2,259 executing rows to settle 167 jobs. Each of the 62 sampled statements had both its start and end inside the drain. In every plan, PostgreSQL read the namespace from the `mesh_jobs_expiring` index, then matched individual jobs and lease tokens against the supplied batch. The subsequent UPDATE already used the primary key and returned one target row per settled job.

| Claim configuration during diagnostic | Sampled settlements | Jobs settled | Rows returned by the lock-side index scan | Shared hits at that scan | Rows returned by UPDATE-side lookup |
|---|---:|---:|---:|---:|---:|
| Stock | 32 | 81 | 980 | 531 | 81 |
| Logical key | 30 | 86 | 1,279 | 1,083 | 86 |

These plans mostly cover widths two and three. The six later fixed8 runs used larger batches. Their ledgers reconcile 159–226 grouped settlements and zero to seven scalar settlements per 1,024 jobs. PostgreSQL recorded 268–327 ms of cumulative statement execution for grouped settlement, including locks and updates across two sessions. Isolating the lock-side lookup requires a matched experiment on fixed8. The saved plans establish neither its cost on that workload nor a critical-path saving.

Across the six paired fixed8 blocks, tying prewarmed River on drain through settlement alone would require saving 0.64–2.81 ms per grouped call if every saved microsecond advanced final settlement and the call counts held. In the fifth block River led by 636 ms, almost twice the entire recorded 327 ms for those SQL calls. The distribution of batch widths and original paired clocks are in `settlement-scans.json`.

From this directory, run `python3 -B settlement_scans.py | diff settlement-scans.json -` to verify the two plan captures and twelve measured workload archives against the released digests and reproduce the counts. All timing data comes from existing Linode runs.

## Individual settlement on the public runner

The same two scalar statements had different lookup costs on the released DENSE 8×8 and SLOW 1×64 diagnostics. Each row below accounts for every scalar settlement in that instrumented arm. The d388 change applied to grant, and both arms kept the stock scalar settlement function.

| Profile | Grant | Scalar settlements | Lock lookup rows filtered | UPDATE lookup rows filtered | Shared scan hits, lock / UPDATE |
|---|---|---:|---:|---:|---:|
| DENSE | stock | 1,024 | 0 | 0 | 4,091 / 4,089 |
| DENSE | d388 | 1,024 | 0 | 0 | 4,064 / 4,042 |
| SLOW | stock | 512 | 130,711 | 261,632 | 15,370 / 69,220 |
| SLOW | d388 | 512 | 130,614 | 261,632 | 10,510 / 40,305 |

All 4,096 DENSE lookups used both namespace and job ID in the primary-key condition. All 2,048 SLOW lookups used only namespace in the `mesh_jobs_unique_candidates` index condition, then filtered the heap rows. Improving the grant lookup left this repeated settlement search in place on SLOW. A separate partial lease-token index has a concrete target there. These DENSE captures already used a point lookup, so they supply no corresponding case for an added index. Filtered rows and shared hits measure work. Inferring a drain saving requires uninstrumented matched timings.

Run `python3 -B scalar_lookup_profiles.py | cmp - scalar-lookup-profiles.json` here. The replay uses the pinned local Git objects in `4d8c05545` and `7df5b961c`, verifies the four archive hashes, checks each query ID against the closed statement counter, and reconciles distinct attempt, effect and terminal transactions for every job.

### When the SLOW settlement scans ran

In both diagnostic arms, over 92% of the filtered rows belonged to transactions that **committed before the last claim timestamp**. A narrower settlement lookup could therefore free capacity while later jobs still needed a grant. The captures do not price how much earlier those jobs could start.

| SLOW grant | Terminals committed before last claim | Filtered rows in those terminals / total | Nested lock and UPDATE elapsed before last claim / total |
|---|---:|---:|---:|
| Stock | 454 of 512 | 361,063 / 392,343 | 180.004 / 201.917 ms |
| d388 | 457 of 512 | 362,675 / 392,246 | 236.315 / 254.095 ms |

The lock filtered roughly 57,000 rows across the first 128 settlements and about 8,100 across the last 128. The UPDATE filtered exactly 65,408 in **each** quarter of both arms. The elapsed figures include the actual tuple lock or durable UPDATE and observer overhead. Neither capture isolates search time. The d388 treatment changed the grant statement only.

Run `python3 -B slow_settlement_timeline.py | cmp - slow-settlement-timeline.json` from this directory. The replay pairs all 512 lock, UPDATE and outer calls per arm with their 512 distinct terminal commits in order on one lifecycle backend. It checks log sequence and commit brackets and puts the entire final claim millisecond in a separate bucket. Neither arm had a terminal commit in that bucket. The claim stamp comes from the jobs ledger and precedes confirmation on the client. Counting transactions that had already committed at that stamp gives a conservative boundary.

The same captures distinguish the PostgreSQL log for the outer grant SELECT from the first handler entry for that grant. Every productive nested grant CTE matched one outer SELECT on the claim backend and one ledger group of exactly the same width in the same order. A settlement qualifies below only if its *preceding* terminal transaction had already committed after the latest possible SELECT log timestamp and its own terminal transaction committed before the first new handler entered. Counting each settlement once across windows gives a conservative overlap.

| SLOW grant | Productive grants with a whole older settlement in that window | Distinct older settlements | Rows filtered in those settlements | Diagnostic lock + UPDATE elapsed |
|---|---:|---:|---:|---:|
| Stock | 17 / 48 | 228 | 182,329 | 88.484 ms |
| d388 | 6 / 37 | 103 | 64,771 | 43.873 ms |

Weighted by batch width and divided by 64 seats, the post-SELECT-log interval before first entry was 39.685–47.266 ms on stock and 28.864–36.712 ms on d388. The full ledger stamp-to-first-entry intervals were 168.188 and 95.181 normalized ms. The ranges cover the millisecond precision of the log. They measure seat occupancy on these two diagnostic timelines. The client could still have been receiving rows or waiting for COMMIT while older jobs settled. Neither the SQL elapsed nor the occupancy establishes how much wall time faster settlement would recover.
