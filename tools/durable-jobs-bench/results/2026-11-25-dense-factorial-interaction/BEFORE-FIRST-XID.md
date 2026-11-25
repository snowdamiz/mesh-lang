# Most Mesh handler time before the first commit preceded XID assignment

Across six native dense rotations, **at least 79.1–88.5% of Mesh's accumulated time between handler entry and its first application commit preceded assignment of the INSERT's transaction ID** during the middle 512 second writes. Even the lower bound for Mesh exceeded the upper bound for prewarmed River by 6.29–16.10 concurrent jobs. This narrows where the excess accumulates. A session can receive a statement, or wait for a lock, before PostgreSQL assigns its XID. The bounds include that possibility.

| Rotation | Mean Mesh jobs before first XID, bounds | Mean warm River jobs before first XID, bounds | Minimum share of Mesh entry-to-first-commit time before first XID |
|---|---:|---:|---:|
| 1 | 12.95–15.28 | 0.73–2.19 | 84.7% |
| 2 | 8.79–11.11 | 0.79–2.50 | 79.1% |
| 3 | 13.65–15.93 | 0.90–2.46 | 85.7% |
| 4 | 13.77–16.06 | 0.93–2.28 | 85.8% |
| 5 | 18.53–20.95 | 1.23–2.43 | 88.5% |
| 6 | 13.01–15.40 | 1.27–2.81 | 84.4% |

The older dense comparison gives a separate intervention on the lookup performed at handler entry. Passing the application pool directly reduced the **upper** bound for mean jobs awaiting a first XID to below 0.7 in every block. The service controls' lower bounds ranged from 6.4 to 10.2. Direct binding also left 1.15–6.14 more jobs between the claim stamp and handler entry than either duplicate in each block. The middle 512 writes finished 6.4–45.3 ms sooner than the faster duplicate. Removing a visible wait can move a queue earlier in execution. The current fixed8 experiment therefore needs both the handler clocks and durable settlement on its own executable. [Older direct-pool comparison](../2026-11-24-trusted-handler-app-pool/timing-clean-identity-retry/evidence/summary.json)

## How the bounds work

The benchmark executes two autocommit writes per job. PostgreSQL allocates ordinary XIDs from a cluster-wide sequence. For the first write, handler entry precedes allocation. For the second, the first recorded commit precedes allocation. Each write's allocation precedes its own recorded commit. The replay checks the small non-wrapping XID range independently in each fresh cluster. [PostgreSQL transaction identifiers](https://www.postgresql.org/docs/17/transaction-id.html)

Sort the 2,048 writes in one arm by XID. For any write, the latest starting witness among it and lower XIDs is a lower bound on its allocation time. The earliest commit among it and higher XIDs is an upper bound. This works even when a higher XID commits before a lower one. Clip each bounded interval to the time between the 256th and 768th second commits, sum across jobs, and divide by that window's duration to obtain mean occupancy. These overlapping job intervals cannot be added to predict a wall-clock saving.

Run `python3 -B test_xid_bounds.py` and `python3 -B xid_bounds.py | cmp - xid-bounds.json` in this directory. The replay validates 30 native archive hashes across the two series, covering 30,720 jobs and 61,440 application writes. [Unrounded sums in job-microseconds](xid-bounds.json)
