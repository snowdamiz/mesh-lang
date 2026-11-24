# Native durable Jobs comparisons

[Four-engine scorecard](four-engine-scorecard.md) shows each workload's native medians and counts wins against Oban, River and Apalis across all measured rotations.

[Open the interactive results](chart.html). Choose a series and workload to compare the engines within the same rotations. For either throughput measure, expand **Head-to-head throughput** to count paired wins and see the median percentage change against the chosen engine. The individual rotations include execution order and CPU steal. The standalone page includes its own downloadable copy of the score data.

Submit-to-settlement includes producer work and the final durable terminal commit. Commit-to-settlement starts after enqueue. Changing the producer can improve the first measure even when workers drain the queue at the same speed.

For each trusted Mesh rotation, the paired time budget shows how much end-to-end time Mesh would have to save to match the peer, and whether the gap fits inside the observed producer interval. [Producer time budgets](producer-budgets.md)

Protected Mesh creates a separate process for each attempt. The smaller-worker experiment preserves fresh exec and fenced writes for its eligible handler. The trusted shared-process experiment runs ordinary application SQL in the runner's trust domain. Both write transactions must commit independently in every scored job. The symmetric storage series configures PostgreSQL EXTERNAL payload storage on all four engines. The separate Mesh-only storage series retains the default storage on its peers.

The paired handler-progress tables follow the short and large batches through both application writes. They count how many Mesh jobs were still waiting for each step at the instant the peer finished, relative to each arm’s own enqueue commit. [Handler progress and backlog](handler-progress.md) The slow storage comparison counts outstanding jobs at each peer’s finish on both clocks. [Slow-job cutoffs](slow-peer-cutoffs.md)

In a separate dense native diagnostic, reducing V1 from eight runners to two at 64 total slots and 32 warm application sessions raised submit throughput from 350–353 to 556 jobs/s and drain throughput from 437–442 to 764 jobs/s across two rotations. This measurement compares Mesh deployments. The five-rotation canonical scorecard uses eight runners. [Topology receipts and resources](../2026-11-23-ordinary-runner-topology/README.md)

The sparse response-time replay follows each job from its own submit through durable settlement. A separate ledger decomposition shows how much of each run elapsed before the next submission. Across five rotations the median sums of those intervals range from 7.01 to 9.41 seconds per 24 jobs. [Response times](sparse-latency.md) · [Time between submissions](sparse-windows.md)

The public shared-process build at `db5937e7` has its own native correctness checks and one direct timing pair for each of short and dense. The V1 and V2 throughput series use their archived experimental executables. [Product verification and matched pairs](../2026-11-23-ordinary-trusted-product-db5937e7/README.md)

The five dense rotations with stock peer deployments put trusted Mesh at
547.01 jobs/s from submit and 751.82 from enqueue commit with two runners.
Against eight Mesh runners, the paired median gains were 50.8% and 66.2%.
Mesh finished ahead of Oban from enqueue commit in all five rotations.
Even with instantaneous Mesh enqueue, matching River from submit would
require a further 97–193 ms of drain savings per 1,024 jobs at these observed
drain times. [Dense budgets by rotation](dense-stock-budgets.md) ·
[Deployment, ledgers and native evidence](../2026-11-23-dense-matched-topology/README.md)

With two 32-slot runners per engine, trusted Mesh finished ahead of its
eight-runner control in all five dense Linode rotations. Oban, River and
Apalis each beat two-runner Mesh in all five rotations on both throughput
measures. Mesh used four of its 32 warmed application connections in each
rotation. Even if enqueue took zero time and drain stayed fixed, Mesh would
still need a median 312 ms off the 1,024-job drain to match River from submit.
[Equal-topology scorecard](../2026-11-23-dense-matched-topology/evidence/equal/scorecard.md) ·
[Time budgets by rotation](dense-equal-budgets.md)

PostgreSQL recorded 74–84 ms in total executing the Mesh handler's two INSERTs per job and its pool checkout probes across each dense run. Even crediting all of that time as a drain saving leaves Mesh 296–439 ms behind River in the five equal-topology rotations. Client waits, planning and commit acknowledgements remain outside that calculation. [PostgreSQL statement times and matched peer gaps](application-intervals.md#postgresql-execution-time-for-the-dense-mesh-handler)

On short jobs, experimental V2 Mesh with bulk enqueue and hot pool checkout
finished ahead of Go River and Elixir Oban in all five six-lane rotations
on both clocks. Rust Apalis finished ahead of Mesh in all five. The earlier
three-lane series used the same Mesh and River executables. Mesh won two of
five submit comparisons there. Across the ten rotations, Mesh enqueued
later than River every time and reached its last application commit sooner
from enqueue commit twice. River's interval from last application commit
to final settlement ranged from 11 to 102 ms. Both Mesh series used the
experimental runner that can settle before actor teardown.
[Per-rotation application and settlement intervals](application-intervals.md) ·
[Six-lane score and executable identities](../2026-11-24-bulk-d2-short-peers/README.md)

In the fresh short series that waits for actor teardown, Mesh with hot checkout finished ahead of Oban in five of five rotations and River in three of five, on both clocks. Apalis led in every rotation. Against its own exit-confirmed control, Mesh gained a paired median 3.58% from submit and 1.14% from enqueue commit, with two losses on both clocks. Mesh spent 19–33 ms longer than River from submit to enqueue commit in every pair. In the three Mesh wins, River spent 39–90 ms longer between its last application write and its last durable settlement. [Matched short scores](four-engine-scorecard.md#short-jobs-with-hot-checkout-and-confirmed-actor-teardown) · [All five interval comparisons](application-intervals.md#exit-confirmed-short-jobs-against-stock-peers)

In the exit-confirmed short series, River started with four physical database connections and reached 18 during every measured run. Mesh stayed at 13. Mesh reached the 16th second write in 21 ms at the median, compared with River's 76 ms. The next 112 second writes took Mesh 146 ms and River 79 ms. River completed those later writes faster in every rotation. They leave the contribution of opening River's connections unresolved. [Sample counts and application-write intervals](application-intervals.md#short-job-database-connections-and-write-progress)

In the five equal-topology dense rotations, the last application write committed 373–489 ms later on Mesh than on River. A local six-rotation experiment reduced terminal transaction counts in every rotation and failed to advance drain time beyond the identical controls. [Native stage chart and local completion experiment](../2026-11-24-dense-two-runner-completion/README.md)

## Slow-job storage and refill

All five Linode rotations compared default and EXTERNAL argument storage on each engine. Mesh kept the same bulk producer and one 64-slot runner. EXTERNAL saved a paired median 159 ms to enqueue commit and 179 ms from submit to final settlement against default Mesh, with gains in every group. Mesh beat EXTERNAL River in 5/5 drains and 3/5 full submissions, and beat EXTERNAL Oban in 5/5 on both clocks. Apalis led Mesh in every pair under both policies.

From the 64th to the 512th first committed write, EXTERNAL Mesh took a paired median 385 ms longer than River and 298 ms longer than Apalis. Mesh reached the 64th write 271 ms earlier than River and 36 ms earlier than Apalis at the paired medians. The next two-runner comparison can measure whether faster refill preserves the early lead through settlement. [Job counts and elapsed intervals](slow-peer-cutoffs.md) · [Matched rates](four-engine-scorecard.md#slow-jobs-with-external-payload-storage)

## Replay

Reproduce the fixed-drain calculation with
`python3 -B tools/durable-jobs-bench/results/2026-11-23-iteration-comparison/dense-topology-budgets.py equal`.
Use `stock` for the series with eight-runner peers.

Reproduce the short and dense application intervals with
`python3 -B tools/durable-jobs-bench/results/2026-11-23-iteration-comparison/application-intervals.py`.

From the checkout root, rebuild the standalone page from the committed score data.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-iteration-comparison/refresh.py
```

Rebuild the sparse time decomposition from the original 25 arms with `python3 -B tools/durable-jobs-bench/results/2026-11-23-iteration-comparison/sparse-windows.py`.

Add `--collect` to replay all locally available measured and warmup arm archives in [the input list](series-list.json). Acquisition uses the shared matrix ledger verifier, checks the frozen rotation order, two-CPU affinity, independent write counts, the displayed metrics, and the raw archive digests. A partially written block enters the viewer after the matrix owner records its reconciliation. A changed previously captured group stops the refresh. The collector reads archived groups first and can pick up newly reconciled groups from their live directories. When both copies exist, it checks their manifest and receipt digests.

[Per-arm scores, executable identities and archive digests](scores.json) · [Original four-engine matrix](../2026-11-23-baseline/README.md)
