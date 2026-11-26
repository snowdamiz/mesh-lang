# Five fresh 8×8 peer rotations

The sole Linode controller released the series with exit 0 at 07:07:56 UTC.
Independent replay matched all 70 fresh arms and all five rotations.
**The original gate returned inconclusive for River, Oban and Apalis.**
River led submit in all five measured pairs. Oban trailed B on both clocks
in four pairs. Apalis led B on both clocks in four pairs. The original
gate requires every rotation to clear the duplicate-spread margin and the
candidate CPU envelope.

[Release](evidence/controller-release.json) ·
[Compact paired data](evidence/summary.json) ·
[Independent replay](evidence/independent-replay.json)

## Measured clocks

All entries are five-run medians with minimum and maximum in milliseconds.
Last effect is the final durable application write measured from the
producer commit. The replay also reports the same event from submit.

| Lane | Submit through settlement | Commit through settlement | Commit through last effect |
| --- | ---: | ---: | ---: |
| Late capacity B | 1,400 (1,309–2,429) | 1,185 (1,133–2,249) | 1,181 (1,127–2,243) |
| Same image off A1 | 1,469 (1,464–1,489) | 1,294 (1,277–1,314) | 1,281 (1,271–1,305) |
| Same image off A2 | 1,532 (1,487–1,823) | 1,351 (1,310–1,639) | 1,335 (1,296–1,625) |
| Public 4924b588 bridge | 1,506 (1,463–1,527) | 1,321 (1,291–1,343) | 1,312 (1,282–1,336) |
| River | 1,206 (1,144–1,232) | 1,129 (1,070–1,153) | 891 (874–906) |
| Oban | 1,651 (1,586–1,688) | 1,499 (1,435–1,546) | 1,498 (1,431–1,543) |
| Apalis | 933 (910–26,837) | 893 (872–26,797) | 884 (851–910) |

The paired deltas subtract B from the named lane. Positive values mean B
finished earlier in the same rotation.

| Lane paired against B | Submit delta | Drain delta | Last effect delta |
| --- | ---: | ---: | ---: |
| Public bridge | +115 (−967 to +197) | +148 (−959 to +188) | +152 (−961 to +186) |
| River | −217 (−1,197 to −94) | −56 (−1,096 to +5) | −297 (−1,337 to −230) |
| Oban | +228 (−758 to +334) | +314 (−730 to +407) | +317 (−727 to +411) |
| Apalis | −467 (−1,502 to +25,484) | −302 (−1,359 to +25,658) | −299 (−1,356 to −216) |

The absolute off-duplicate spread had medians of 58 ms on both full
clocks and maxima of 356 ms submit and 344 ms drain. B passed the original
two-pilot advancement criteria in rotations 1 and 3. Rotation 2 missed
both full clocks, last effect and the CPU envelope. Duplicate noise
prevented a submit pass in rotation 4 and prevented both clock passes
in rotation 5.

## SQL, CPU and retained tails

B took 134–159 productive grants and 263–289 checked bounded calls.
The bridge took 253–256 productive grants and 383–387 bounded calls.
Across the five B measurements, 125–139 pending requests gained
confirmed seats. All Mesh lanes peaked at 65 PostgreSQL sessions.
River peaked at 78–81 and Oban and Apalis each reached 81. Mesh B median
runner, PostgreSQL and pinned-host CPU were 1,283, 1,520 and 2,590 ms.
The bridge medians were 1,331, 1,620 and 2,800 ms. Median pinned-host
CPU was 1,940 ms for River, 3,200 ms for Oban and 1,860 ms for Apalis.
The replay retains resource ranges, Mesh queue statement counters and
River's 2,656–2,692 total SQL calls.

B's rotation 2 recorded 22 contention hints and took 2,249 ms to
drain. The other B measurements recorded zero hints and drained in
1,133–1,195 ms. PostgreSQL accumulated 8.86 seconds in the prelock
statement across sessions in rotation 2, versus 4.62–5.17 seconds in
the other B measurements. This aggregate includes wait time. The final
B effect in that rotation preceded final settlement by 6.5 ms.

Apalis's rotation 3 reached its last durable effect at 851 ms and
finished settlement at 26,797 ms. One job's final settlement followed
its own effect by 25,949 ms. Both observations remain in the release
and paired ranges. Each arm completed its frozen workload.
