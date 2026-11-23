# Native durable Jobs comparisons

[Open the interactive results](chart.html). Choose a series and workload to compare the engines within the same rotations. Each row in the expanded table identifies its execution order and CPU steal. The viewer includes the five-group baseline and completed groups from subsequent experiments.

Submit-to-settlement includes producer work and the final durable terminal commit. Commit-to-settlement starts after enqueue. Changing the producer can improve the first measure even when workers drain the queue at the same speed.

Protected Mesh creates a separate process for each attempt. The smaller-worker experiment preserves fresh exec and fenced writes for its eligible handler. The trusted shared-process experiment runs ordinary application SQL in the runner's trust domain. Both write transactions must commit independently in every scored job. The symmetric storage series configures PostgreSQL EXTERNAL payload storage on all four engines. The separate Mesh-only storage series retains the default storage on its peers.

## Replay

From the checkout root, rebuild the standalone page from the committed score data.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-iteration-comparison/refresh.py
```

Add `--collect` to replay all locally available measured and warmup arm archives in [the input list](series-list.json). Acquisition uses the shared matrix ledger verifier, checks the frozen rotation order, two-CPU affinity, independent write counts, the displayed metrics, and the raw archive digests. A partially written block enters the viewer after the matrix owner records its reconciliation. A changed previously captured group stops the refresh. Some in-progress series live under the checkout's `target/` directory.

[Per-arm scores, executable identities and archive digests](scores.json) · [Original four-engine matrix](../2026-11-23-baseline/README.md)
