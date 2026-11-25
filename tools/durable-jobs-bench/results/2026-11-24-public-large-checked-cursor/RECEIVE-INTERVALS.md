# Where the LARGE checked cursor spent time after rendering

In four released 8×8 diagnostic arms, every one of the 64 jobs could be assigned to a successful cursor Execute. In the two EXTERNAL orders, the union of the intervals **after the final row's claim stamp and before Execute finished** covered 91–109 ms and 100–116 ms. This is where the stock path finishes the final UPDATE, returns rows from its tuplestore and writes replies to the socket. Deleting a client-side validation pass alone cannot recover that entire interval.

All values below are milliseconds. The ranges allow for the server log's timestamp precision. Each union counts elapsed time once where intervals overlap across the eight claim sessions.

| Storage and order | Positive cursor calls | Final row stamp → Execute end, union | Execute end → start of COMMIT, union | Execute end → start of COMMIT, last grant alone |
|---|---:|---:|---:|---:|
| Default forward | 9 | 91.0–109.0 | 28.4–45.9 | 3.0–5.0 |
| Default reverse | 8 | 96.0–112.0 | 31.6–47.6 | 8.2–10.2 |
| EXTERNAL forward | 9 | 91.0–109.0 | 22.0–40.1 | 0.8–2.8 |
| EXTERNAL reverse | 8 | 100.0–116.0 | 26.4–42.5 | 1.2–3.2 |

The source renders each claimed context before stamping and updating the row. The reader handles each PostgreSQL DataRow as it arrives and validates the assembled batch before sending COMMIT. The Execute-to-COMMIT interval can include remaining TLS data, client validation, scheduling and the return trip to PostgreSQL. Bytes can also reach the reader during Execute. These clocks do not measure any component's CPU. [Released SQL and reader revision](README.md)

The collector matched each successful cursor Execute to the immediately following COMMIT on the same PostgreSQL backend by log sequence number. The job's millisecond claim stamp must fall within Execute and handler entry must follow COMMIT. In reverse EXTERNAL, one stamp could initially match two calls. The other 63 jobs uniquely identified eight productive calls, exactly the count from the SQL and contention receipts. That left one possible call for the last job. Each identified call delivered at most eight jobs to one runner. The replay pins the two original scores and checks all four raw hashes. [Per-arm intervals and grant sizes](receive-intervals.json)

A simple fixed-history model allows earlier results to be transmitted between the first and last stamp of a grant, while holding the observed render schedule fixed. Credit within a grant is capped by both its first-to-last stamp span and its last-stamp-to-COMMIT interval. Even crediting the final UPDATE, the final row and the COMMIT request, the summed ceilings are 140.0 and 150.1 ms for EXTERNAL. The prepared consumer reserves rows concurrently and takes its queue lock during a separate activation. Its socket stalls, overlap and end-to-end saving require separate evidence. The original stock cursor decisions stand. [EXTERNAL release](evidence/native/external-4m/README.md)

Run the reproducible comparison from this directory.

```sh
python3 -B receive_intervals.py | cmp - receive-intervals.json
```
