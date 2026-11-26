# LARGE FILE producer timing on the durable path

The five-order Linode timing release at 01:42:20 UTC on November 26 completed all 40 arms and passed the sealed replay. The plain-string producer had saved 47.807 ms per 64-job submission in the component. In the durable trial it settled a paired median **78.324 ms sooner than the original public FILE broker** and **4.086 ms later than the faster relinked stock duplicate**. The median stock duplicate spread was 103.240 ms from submit and 62.039 ms to enqueue commit. This trial does not establish a durable gain over the matched relink.

The table follows the same reference through every stage. Stock B was the faster duplicate from submit in all five orders, including both mirrored orders. Rows are stock B elapsed minus candidate elapsed in ms. Each row's four stage differences sum to its submit difference, up to rounding. The latest claim stamp comes from PostgreSQL. It precedes the client acknowledgement. Different jobs can supply the last claim, last effect and last settlement.

| Order | Submit → enqueue commit | Enqueue commit → latest claim stamp | Latest claim stamp → last effect commit | Last effect → last settlement | Total submit gain |
|---:|---:|---:|---:|---:|---:|
| 1 | +72.5 | −0.3 | +20.6 | +4.4 | +97.2 |
| 2 | −1.6 | +30.1 | −28.2 | −4.4 | −4.1 |
| 3 | −14.5 | −64.6 | +7.0 | +1.5 | −70.7 |
| 4 | −32.2 | −27.8 | +2.3 | +0.9 | −56.8 |
| 5 | +24.9 | −25.4 | +20.0 | +5.9 | +25.5 |

Measured separately against the faster duplicate *for each clock*, candidate enqueue took 1.624 ms longer at the paired median, and post-commit drain took 2.462 ms longer. The controls resolve to the same producer path, SHA and argv. Stock B ran last, first, second, third and first respectively. Its persistent submit advantage cannot be explained by a fixed position alone. The producer was the sole treatment; differences after durable enqueue reflect observed downstream variation. No peer ran in these five groups.

[Exact stage differences and measured archive hashes](file-producer-landmarks.json) · [Parent arithmetic](file_producer_landmarks.py) · [Sealed durable trial](../2026-11-25-public-large-canonical-string-linux/durable/README.md)

The arithmetic takes the released timing directory and the official replay JSON as its two arguments. It binds the packet, compares each reconciled block to the replay, checks the 20 measured raw hashes, and reads the timestamps from the 64 completed job ledgers per arm. The official replay additionally covers the 20 warmups.

## The receipt boundary changes the sign of the comparison

The FILE harness records both the producer transaction's commit timestamp and the time its caller receives the result. Across all 40 verified archives, the result arrived 3.319–27.067 ms after the commit timestamp, with an 8.236 ms median. Against stock B, the faster-submit duplicate in all five measured blocks, **the candidate lost 1.624 ms at durable enqueue and gained 7.019 ms at acknowledgement** by paired medians. In block 2 alone the comparison went from a 1.624 ms loss to a 7.019 ms gain. Stock A gave gains of 33.727 and 51.835 ms on those respective clocks. The exact original gave gains of 46.457 and 48.183 ms.

A process CPU snapshot taken when the caller receives its result measures through that later boundary. For attribution report how far each snapshot lies from the transaction clock. Keep the scored enqueue from submit through the recorded commit. Reading all 64 persisted insertion timestamps after settlement can further divide time before the first scalar insert from time between successive inserts. The existing ledger contains the identities needed for this join, but omits those insertion timestamps. [Replay from the released archive hashes](file_receipt_clock.py)
