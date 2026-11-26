# Capacity for replacement claims on eight public runners

The successful 8×8 public grouped-hot pilot used 402 and 409 terminal transactions. **Neither block had a single eight-job terminal transaction.** Two- and three-job transactions predominated. Increasing the maximum cohort width alone would have released no additional seats in the observed cohorts.

| Measured block | Positive grant groups | Terminal XIDs | Minimum replacement grants for 960 seats | Minimum grant-bearing transactions including initial fill |
|---|---:|---:|---:|---:|
| Forward | 255 | 402 | 342 | 350 |
| Reverse | 254 | 409 | 346 | 354 |

The previous two-runner fusion transferred a seat only from a confirmed parent to a replacement in the same runner. On eight runners, give each runner a free initial fill of eight jobs and assume it can use its largest observed terminal groups first, irrespective of when they happened. Stop when that runner has covered all its other jobs. Even this optimistic selection requires **95 and 100 more grant-bearing transactions** than were observed in the public pilot. The comparison concerns the number of transactions performing grants. Fusion could still eliminate separate commit exchanges. Changing cohort widths or using a mixture of ordinary and replacement grants would change the bound.

This result removes a premise for transferring the two-runner fusion to 8×8. The earlier two-runner experiment replaced 960 seats in both rotations and missed its throughput gate. On eight runners the same number of replacements would be spread over smaller transactions. The two public arms also accumulated 180 and 195 ms from second application commit to terminal commit when averaged over 64 seats. This interval includes actor teardown and durable settlement. It cannot be counted as recoverable wall time.

From this directory run `python3 -B eight_runner_cohorts.py | cmp - eight-runner-cohorts.json`. The replay reads the owner's committed pilot `dad83049a5`, verifies the scored raw hashes and per-runner terminal XIDs, and cross-checks the positive group count against PostgreSQL call and row totals. [Full width distributions](eight-runner-cohorts.json) · [Earlier fusion contract](../2026-11-25-dense-fixed8-writer-fusion/replay/packet/README.md)
