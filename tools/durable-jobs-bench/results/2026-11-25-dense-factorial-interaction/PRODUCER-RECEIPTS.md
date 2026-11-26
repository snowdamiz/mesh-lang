# Dense producer time and the cost of returning enqueue receipts

Across the six matched Linode blocks, Mesh took a median 166.5 ms from first submit to the durable enqueue commit. Prewarmed River took 76.4 ms. The paired difference was 86.8 ms. River spent a median 10.2 ms through decoding its input and 66.1 ms from its enqueue call through the commit record. The canonical Mesh archive ends the corresponding phase at the same durable boundary but has fewer internal timestamps.

Even granting Mesh an instantaneous producer and holding both engines' measured drains fixed leaves River ahead in every block. The remaining paired deficit is 82.5 ms at the median. This is a counterfactual for the measured workload. Earlier release to the workers could also change the drain in a real implementation.

| Block | Mesh enqueue minus River enqueue, ms | Mesh drain alone minus River submit through settlement, ms |
|---|---:|---:|
| 1 | 88.1 | 50.1 |
| 2 | 111.6 | 87.9 |
| 3 | 126.2 | 77.2 |
| 4 | 74.6 | 106.5 |
| 5 | 84.5 | 550.0 |
| 6 | 85.6 | 31.0 |

## Receipts in the earlier 128-job instrumented producer

For each archived instrumented SHORT arm, sum four disjoint intervals on the broker's monotonic clock. They cover the first returned bulk row through ReadyForQuery, ReadyForQuery to sending RELEASE SAVEPOINT, the savepoint response to sending the XID query, and the XID response to sending COMMIT. The latter two include building the application ID map and final response. Database processing before the first returned row, including construction and durable storage of receipts, falls outside these windows.

The combined intervals took 0.564–0.711 ms across the five client-observer runs and 0.512–0.683 ms across the five nested-observer runs. As a scale comparison, repeating even the largest observation eight times gives 5.69 ms. The dense broker builds a growing map over eight calls and encodes it once. Its actual internal cost is unmeasured. Both SHORT observers failed their original perturbation checks. The separate client verification after the commit cannot shorten time to the enqueue commit.

Keep the current receipt processing. These captures provide little support for a receipt-parser experiment aimed at the 75–126 ms dense enqueue gap. The dense PostgreSQL diagnostic recorded 104.7–120.7 CPU-ms across the eight Mesh enqueue executors and 20.8–23.3 CPU-ms across River's eight. The statement counters include the whole executor and the observer failed its perturbation screen. They cannot price server-side receipts independently from row insertion, indexes, triggers and safeguards. [Statement CPU evidence](../2026-11-25-dense-statement-cpu/recovery/README.md)

Run `python3 -B producer_receipts.py | cmp - producer-receipts.json` in this directory. The replay checks the custody hashes of the 12 measured dense raw archives and ledgers, verifies the workload and dual application commits, and reconstructs the four receipt intervals from all ten short producer traces against their original scorer. [Machine-readable results](producer-receipts.json) · [Native dense custody](../2026-11-25-dense-fixed8-river-readiness/evidence/release-audit.json)
