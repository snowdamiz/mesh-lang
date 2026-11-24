# Resume the Linode durable Jobs comparison

Open the existing Mesh worktree on branch `t3/9a80aabe` on the workstation that owns the benchmark. Its path is `/Users/andreyyurlov/.t3/worktrees/mesh-lang/t3-9a80aabe`. The Git history, local controller journal, staged binaries, and SSH configuration are available there. Other owners have changes in the Git index and working tree.

**Paused November 24, 2026 at 05:51 UTC.** The final symmetric large-payload rotation released at 05:50:33 UTC. On the original Linode boot the controller acquired all three locks together, checked the pinned PostgreSQL and Mesh binaries, and found the host idle. The Linode remains powered on and the nine later bookings are parked. The public score update is local Git commit `2957ea5a`, and the final raw-series archive is `5074b78d`. [Controller receipt](../../target/linode-jobs-stepchange/requests/public-campaign-pause-20261124T055129Z.json) · [Host readback](../../target/linode-jobs-stepchange/requests/public-campaign-pause-host-20261124T055129Z.txt)

**Prompt for a new session**

> Resume the Linode durable Jobs comparison from `tools/durable-jobs-bench/RESUME.md` in this worktree. Continue the booked work through the existing controller. Run all performance measurements on Linode and keep the separate slow cap-71 fused-refill pilot on hold.

## What has been measured

The public comparison uses Mesh source `b3557273`, Linux AMD64 executable SHA-256 `af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355`. The short-job selection chose individual terminal commits. All five canonical workloads use that same binary, trusted shared-process execution, confirmed actor teardown, checked pool checkout, and two independently committed application writes per job. PostgreSQL 17.11 and each engine share CPUs 0 and 1. The canonical comparison covers 35,040 measured jobs and 70,080 application commits across Mesh, Go River, Elixir Oban, and Rust Apalis. The scorecard includes paired wins, both timing windows, and separate rows for alternate deployments. [Four-engine scorecard](results/2026-11-23-iteration-comparison/four-engine-scorecard.md) · [Interactive results](results/2026-11-23-iteration-comparison/chart.html) · [Raw public series](results/2026-11-24-trusted-jobs-product-composition/evidence/native/timing-closure-retry)

- **Short and sparse.** In short jobs Mesh beat Oban in all five pairs and River in three, on both clocks. Apalis won all five. In sparse jobs Mesh won all paired throughput comparisons. Apalis had a slightly lower median submit-to-settlement p99, 15.98 ms versus Mesh's 16.43 ms. [Sparse latency](results/2026-11-23-iteration-comparison/sparse-latency.md)
- **Dense.** With eight runners per engine, Mesh drained at 512 jobs/s, Oban at 698, River at 905, and Apalis at 1,162. Two 32-slot Mesh runners beat eight Oban runners in all five pairs. Giving every engine two 32-slot runners put all three peers ahead again. Even with free enqueue, Mesh still needed 244–400 ms of drain savings to catch River in the equal-deployment series. The repeated claim statement and WAL flushes have separate diagnostic bookings. [Equal-topology budgets](results/2026-11-23-iteration-comparison/dense-equal-budgets.md) · [Claim diagnostic](results/2026-11-24-public-dense-set-claim/controller-packet.json) · [WAL diagnostic](results/2026-11-24-dense-wal-cost/controller-packet.json)
- **Slow.** Under the canonical 68-session cap, Mesh beat Oban in all five pairs, River in two, and Apalis in zero on both clocks. An earlier 2×32 run with a 71-session cap and the V1 runner beat River in six of eight pairs from submit and seven from commit. Keep those distinct comparisons when interpreting deployment gains. [Cap-71 results](results/2026-11-24-ordinary-slow-equal-topology-cap71/NATIVE-RESULTS.md)
- **Large.** With default storage, Mesh beat River in all five pairs on both clocks. It beat Oban four times from submit, while Oban won all five drains. Apalis led both windows. When every engine used EXTERNAL storage, Mesh reached 36.24 jobs/s from submit versus 27.13 for River, 36.76 for Oban, and 72.61 for Apalis. Mesh finished ahead of Oban in three of five individual submit pairs. [Producer budgets](results/2026-11-23-iteration-comparison/producer-budgets.md)

## Replay on the workstation

Run this from the checkout root to reconstruct the public results from saved Linode receipts. It needs no SSH connection. The comparison directory also contains the standalone chart and the per-rotation machine-readable scores.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-iteration-comparison/refresh.py \
  --series public-short-selection public-dense-stock public-dense-mixed \
  public-dense-equal public-sparse public-slow public-large public-large-external
```

## Returning to the queue

The [sole Linode controller](t3-thread://v1/thread:delegated-task:command%3Amcp%3A824787c5-935c-433b-8c9b-93389c4f3694%3Adelegate-task%3Alinode-native-stepchange-controller-v1) owns native access, SETUP, and timing. Use the [controller journal](../../target/linode-jobs-stepchange/requests/STATUS.md) in this worktree. On explicit resumption the controller verifies the pinned host, boot, CPU mask, PostgreSQL and disk identity, and all three locks before issuing the next grant. Workstation work covers builds, untimed correctness, and offline replay.

After the user resumes, the controller runs these commands from the checkout root. The first two verify readiness. The last opens the stored queue, beginning with the short producer diagnostic.

```sh
python3 -B target/linode-jobs-stepchange/pause-after-public-campaign.py --probe
python3 -B target/linode-jobs-stepchange/r2-d2-producer-boundary-slots.py check
python3 -B target/linode-jobs-stepchange/continue-after-public-product.py --resume
```

The [booked order](../../target/linode-jobs-stepchange/continue-after-public-product.py) begins with the short R2/D2 producer boundary, then River prewarming, cold PostgreSQL permits, short fused refill, and the public dense D2 transfer. The remaining dense bookings cover the bulk producer comparison, fixed-four scheduler supply, WAL costs, and the actual set-based claim plan. Read each release and apply its original advancement rule before proceeding. The cold permits slot uses the controller-acknowledged sibling amendment `c4707e1e43974c0332651f5cfefcc81aa03fa1f1ffa33a215830a850bc3c890b`. [Producer request](results/2026-11-24-r2-d2-producer-boundary/request.json) · [Permits amendment](../../target/linode-jobs-stepchange/requests/2026-11-24-ordinary-pg-permits-import-amendment.json) · [Short fused-refill amendment](results/2026-11-24-short16-fused-refill/closure-retry/request.json) · [Dense D2 amendment](results/2026-11-24-dense-product-d2-transfer/closure-retry/request.json)

**Explicit hold.** The separate slow cap-71 fused-refill pilot requires user authorization even after a general resume. An intermittent shared-broker enqueue-receipt timeout remains unresolved. Its proof packet rejects native dispatch. [Transport evidence](results/2026-11-24-slow-cap71-confirmed-refill/TRANSPORT.md)

Product commit `e4156740` enables explicit set-based batch claims on the ordinary trusted runner. The compiled guide exercises the real claim function, separate terminal XIDs, and actor recovery. Its exact Linux artifact still needs correctness verification. Attribute the measured rates above to `b3557273`. [Product proof](../../proof/jobs-trusted-shared-process/README.md)

Two large-payload investigations also have local restart notes. The file-path producer needs its inline trace checked under the original broker watchdog. The consumer investigation has scoped the speculative set-claim and checked cursor calls. Both are parked outside the nine controller bookings. [File-path producer](results/2026-11-24-public-large-file-session/README.md) · [Checked cursor](results/2026-11-24-public-large-checked-cursor/README.md)
