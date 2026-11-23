# Native durable Jobs comparisons

[Four-engine scorecard](four-engine-scorecard.md) shows each workload's native medians and counts wins against Oban, River and Apalis across all measured rotations.

[Open the interactive results](chart.html). Choose a series and workload to compare the engines within the same rotations. For either throughput measure, expand **Head-to-head throughput** to count paired wins and see the median percentage change against the chosen engine. The individual rotations include execution order and CPU steal. The standalone page includes its own downloadable copy of the score data.

Submit-to-settlement includes producer work and the final durable terminal commit. Commit-to-settlement starts after enqueue. Changing the producer can improve the first measure even when workers drain the queue at the same speed.

For each trusted Mesh rotation, the paired time budget shows how much end-to-end time Mesh would have to save to match the peer, and whether the gap fits inside the observed producer interval. [Producer time budgets](producer-budgets.md)

Protected Mesh creates a separate process for each attempt. The smaller-worker experiment preserves fresh exec and fenced writes for its eligible handler. The trusted shared-process experiment runs ordinary application SQL in the runner's trust domain. Both write transactions must commit independently in every scored job. The symmetric storage series configures PostgreSQL EXTERNAL payload storage on all four engines. The separate Mesh-only storage series retains the default storage on its peers.

The paired handler-progress tables follow the short and large batches through both application writes. They count how many Mesh jobs were still waiting for each step at the instant the peer finished, relative to each arm’s own enqueue commit. [Handler progress and backlog](handler-progress.md)

## Replay

From the checkout root, rebuild the standalone page from the committed score data.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-iteration-comparison/refresh.py
```

Add `--collect` to replay all locally available measured and warmup arm archives in [the input list](series-list.json). Acquisition uses the shared matrix ledger verifier, checks the frozen rotation order, two-CPU affinity, independent write counts, the displayed metrics, and the raw archive digests. A partially written block enters the viewer after the matrix owner records its reconciliation. A changed previously captured group stops the refresh. The collector reads archived groups first and can pick up newly reconciled groups from their live directories. When both copies exist, it checks their manifest and receipt digests.

[Per-arm scores, executable identities and archive digests](scores.json) · [Original four-engine matrix](../2026-11-23-baseline/README.md)
