# Native ordered enqueue producer results

On the dedicated AMD64 host, both stock controls and the ordered-enqueue
candidate ran in each of six groups on CPUs 0 and 1. The candidate committed
sooner than both controls in every group for 128 and 1,024 jobs. The median
dense producer commit fell from 453.38 and 453.74 ms to 403.82 ms.
The submit-to-acknowledgment medians were 455.55 and 455.30 ms for stock
and 405.19 ms for the candidate. These windows cover the producer.
[Reconciled receipts](native-replay.json)

| Producer | Stock A median [min–max] | Stock B median [min–max] | Candidate median [min–max] |
| --- | ---: | ---: | ---: |
| 128 jobs | 61.72 [60.81–63.19] ms | 62.01 [61.39–62.18] ms | 55.76 [55.11–56.51] ms |
| 1,024 jobs | 453.38 [450.05–538.56] ms | 453.74 [451.77–482.64] ms | 403.82 [398.79–451.48] ms |
| Large | 1,661.65 [1,621.31–1,773.11] ms | 1,643.95 [1,625.50–1,662.93] ms | 1,653.15 [1,611.66–1,695.29] ms |
| Sparse | 1.380 [1.342–1.416] ms | 1.345 [1.322–1.394] ms | 1.383 [1.349–1.397] ms |

Sparse cells summarize each trial's median singleton latency. Large and
sparse have overlapping ranges and the same client TLS record counts as
stock. In the separate wire runs, the 128-job batch sent 136 records in
each stock arm and 34 in the candidate. The dense batch sent 1,053 in
each stock arm and 237 in the candidate. The database executed the same
number of enqueue statements in every matched arm. The dense median
PostgreSQL enqueue execution time stayed between 266 and 269 ms across
arms. No sampled arm recorded CPU steal. [Raw attempts](replay/native-runs/)

The official native stock dense diagnostic and the five candidate
semantic profiles reconciled every frozen receipt, payload, and XID.
The candidate's exact executable also passed the injected SQL error,
backend disconnect, and same-session rollback controls. The AMD64
wire-test compilation remains open. The release archive omitted five
public test vectors, so Rust emitted six compile errors. The first retry
supplied those vectors and hit its compilation deadline. Its shell
returned zero after rechecking the executable, but the log has no test
result. The final retry timed out before running the test. The local
Darwin wire proof passed. [Native proof replay](NATIVE-WIRE-PROOF.md)

The matrix owner admitted this exact executable for the official
five-group matched repeat after worker attribution. Its SHA-256 is
`33ebb48f27c4e1623b5019400847812deb48ca6a4dba26b594efb0587a301954`.
The inventory uses the same Oban, River, and Apalis entries as the
baseline. The matrix owner controls the repeat and scores the
submit-to-settlement and commit-to-settlement windows.
[Repeat inventory](inventory.json) · [Mesh build manifest](mesh.json)
· [Reservation release](lease-release.json)
