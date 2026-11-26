# PostgreSQL session closure after the final SLOW settlement

In all ten eligible measured Mesh V1 2×32 arms, PostgreSQL recorded all 17 application, participant and producer session disconnections within 154 ms of the final durable settlement. The larger interval in the lifecycle report extends through reconciliation, physical storage inspection and shutdown of the disposable database. It provides no measurement of how long a Mesh runner process took to exit. [Lifecycle clocks and releases](RESULTS.md)

The table uses milliseconds. The last column gives the summed session lifetimes of long administrative connections opened after the last product disconnect. These are wall durations. They can include client and server waits.

| Codec | Order | Settlement to final product disconnect | Final product disconnect to adapter end | Long administrative sessions, summed |
|---|---:|---:|---:|---:|
| PGLZ | 1 | 85.8 | 3125.5 | 2502 |
| PGLZ | 2 | 116.5 | 3148.2 | 2541 |
| PGLZ | 3 | 153.9 | 3158.4 | 2539 |
| PGLZ | 4 | 121.8 | 3152.4 | 2422 |
| PGLZ | 5 | 113.0 | 3126.8 | 2507 |
| LZ4 | 1 | 102.5 | 6254.6 | 5616 |
| LZ4 | 2 | 106.9 | 6209.9 | 5559 |
| LZ4 | 3 | 72.4 | 6414.6 | 5742 |
| LZ4 | 4 | 80.4 | 6219.9 | 5590 |
| LZ4 | 5 | 141.0 | 6264.4 | 5627 |

For each order use the measured `two-bulk-{pglz,lz4}` raw archive. Take the latest `jobs[].settled_us` from `adapter/ledger.json`. In `adapter/physical-postgres.log` select disconnections in database `mesh` for roles `compiled_caller`, `parity_subject` and `parity_producer`. Count records after settlement and take the latest UTC timestamp. Subtract that timestamp from `adapter_end.epoch_ns / 1000` in `adapter/startup-clock.json`. PostgreSQL log timestamps have millisecond precision. [Orders 1–2](evidence/pilot) · [Orders 3–6](evidence/full)

For the last column use `parity_owner` disconnections in database `mesh`. Infer session start from disconnect time minus logged session duration. Require that start to exceed the last product disconnect by more than 1 ms, and sum logged durations of at least 500 ms. All ten arms had 12 administrative sessions whose start met the boundary. The median summed duration of the long sessions was 2507 ms for PGLZ and 5616 ms for LZ4. This bounds the scope of the observed post-settlement delay. Individual query and process-exit timings require separate evidence. [Reconciliation and adapter end](adapter.py#L153)

Order 6 stays descriptive. Its PGLZ and LZ4 settlement-to-final-disconnect intervals were 92.3 and 88.2 ms, followed by 3304.2 and 6147.1 ms to adapter end. The original CPU-steal rejection and paired settlement clocks stand.
