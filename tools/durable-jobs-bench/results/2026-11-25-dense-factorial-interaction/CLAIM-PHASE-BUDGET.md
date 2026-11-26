# Where a productive DENSE claim waits

In two 8×8 measured observer arms the checked COMMIT reached the dispatcher a median 24 µs later for partial requests. The largest measured interval was earlier, from sending a partial request's queue prelock to receiving its response. It took a median 30.4 and 32.8 ms. Multiple PostgreSQL sessions waited concurrently. Removing a wait on one session cannot be credited as an equal saving through final settlement.

| Productive requests wholly between durable effects 256 and 768 | Forward | Reverse |
|---|---:|---:|
| Middle interval | 470.264 ms | 499.779 ms |
| Partial / full requests | 62 / 0 | 63 / 2 |
| Partial SELECT send to checked row reply, median | 3.482 ms | 2.567 ms |
| Partial COMMIT send to checked receipt, median | 2.004 ms | 2.458 ms |
| Partial COMMIT receipt to dispatcher, p95 | 1.092 ms | 1.453 ms |
| Client time between checked replies and next sends while holding the lock, all included grants | 3.603 ms | 4.659 ms |

The last row sums nonoverlapping spans from successful prelock receipt to SELECT send and from validated SELECT reply to COMMIT send. The numbers exclude the server, network, and TLS portions of the exchanges. The dispatcher marker precedes handler launch. Joining the first handler entry to the COMMIT marker across clocks gives a partial-grant p95 of 0.54–2.54 and 1.46–3.46 ms. The historical uninstrumented public B deficit to River across the middle 512 effects was 191–947 ms. Both observed arms failed a perturbation control. No matched River speed estimate follows from these traces.

## Can the follower join before its own first send?

At each first-send edge, the offline model registers the follower with the oldest outstanding partial prelock from another PID. Only a send already made, with no observed receipt or cancellation at that edge, is eligible. Each turn participates at most once. Pair selection precedes inspection of outcomes. Later checks ask if both grants were productive, consecutive in the recorded database order, and fit the observed per-owner credits by the leader's SELECT. This last check assumes those stock credits still arrive. It is a favorable retrospective condition.

| Frozen-schedule result | Forward | Reverse |
|---|---:|---:|
| All selected pairs | 67 | 71 |
| Empty or unsuccessful member | 4 | 7 |
| Both productive, original grants nonconsecutive or reversed | 22 | 15 |
| Consecutive but follower would lose rows | 6 | 11 |
| Consecutive, room for observed rows | 35 | 38 |
| Of those, join through follower receipt wholly inside middle | 20 | 18 |
| Sum of those followers’ COMMIT round trips | 44.5 ms | 57.7 ms |

A combined grant still has to update and deliver every row, recover an ambiguous COMMIT, and charge both owners until each receives its work. Even deleting the selected followers’ entire COMMIT intervals in these traces gives a small direct budget relative to the historical River gap. Reordering, earlier subsequent grants, and the cost of authenticated handoff require new evidence. A dispatcher serving multiple pending reservations on one claim connection would change the arrival process and needs its own price.

Both scripts take the released claim-timeline evidence directory containing raw.tar.gz, verified.json and controller-release.json. claim_phase_budget.py pins all three and reconciles the 2,048 independent application transactions in each arm. follower_join.py pins the verified trace reduction and retains each qualifying interior pair. Both outputs are archive arithmetic.

[Phase receipt](claim-phase-budget.json) · [Follower pairs](follower-join.json) · [Uninstrumented first-write comparisons](BEFORE-FIRST-XID.md)
