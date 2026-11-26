# Native producer setup recovery

The Linux setup built the observation broker and completed the short workload. Its native exit was zero. The original controller marked SETUP failed while reading the exported PostgreSQL utility statement. The raw smoke contains the top-level spelling `RELEASE SAVEPOINT $1`, called once, and the scorer expected the literal savepoint name. PostgreSQL's normalized statistics omit that name.

The replay recognizes that exact normalized spelling on top-level rows. It retains the original cardinality, call count, wire-stage, executable and durable ledger checks. Removing or duplicating the row, changing its count, setting it to nested, or substituting another literal savepoint name each failed in offline controls. The original release and binary identities remain available. The five timing groups use the original pilot order and workload.

From the checkout root, run the following to verify the archived smoke.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-24-r2-d2-producer-boundary/recover_savepoint.py verify
```

All five native rotations reconciled, covering 20 arms, 2,560 completed short jobs and 5,120 independent application writes. Both observer modes missed the frozen perturbation test. The instrumented client recorded a median 33.69 ms inside the bulk wire exchange. The nested observer recorded median inclusive times of 8.36 ms for layout verification and 11.36 ms for the writable SQL statement. These timings describe the instrumented runs. No component qualified as a removable cost under the original rule. The next booked comparison measures cold and prewarmed River.
