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
