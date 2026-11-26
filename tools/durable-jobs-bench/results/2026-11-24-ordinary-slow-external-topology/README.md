# Slow EXTERNAL runner topology

This series tests whether giving the same compiled V1 Mesh worker two 32-slot runners shortens late refill and improves submit-to-final-settlement. The controls are two independent instances of the same 1×64 Mesh deployment. River, Oban, and Apalis keep their qualified stock 1×64 launchers.

All six arms use owner-approved EXTERNAL payload storage, the same 512 canonical 16 KiB jobs, 100 ms holds, 64 slots, CPUs 0 and 1, and the original PostgreSQL 17.11 physical cap of 68. The three Mesh arms share the exact `ce5e551d` producer, worker and handler ELF and the same layout guard. The only Mesh treatment is moving eight warmed application connections from one runner to two runners with four each. The adapter attests their owners through Linux TCP socket inodes correlated with TLS PostgreSQL client ports. The retained auditors verify one attempt commit and one independent effect commit per job, 64 overlapping committed holds, 504 true-batch rows in eight calls, the eight-row scalar remainder, and per-runner ownership and occupancy. Every arm captures its original native raw archive, including errors and warmups.

The previous six-order DEFAULT topology manifest and the cap-71 equal-peer sensitivity are separate. The stock peers here retain their original EXTERNAL inventories byte-for-byte. [Qualified EXTERNAL series](../2026-11-24-slow-external-compat-scorer-input/README.md) · [DEFAULT topology series](../2026-11-23-ordinary-slow-runner-concentration/README.md) · [Cap-71 sensitivity](../2026-11-24-ordinary-slow-equal-topology-cap71/README.md)

## Timing and continuation

Every rotation pairs a warmup and a measurement for each lane. The first two orders are exact reverses. All six orders are frozen before launch. The controller first runs those two rotations under its own TIMING grant. The remaining four require a fresh controller grant and both fully reconciled pilot blocks.

The primary interval starts at the 64th first durable application write and ends at the 512th. Each block also reports submit-to-commit, commit-to-64th first write, last first write-to-settlement, and the total submit and drain clocks. The continuation gate requires 2×32 to beat the faster 1×64 duplicate on late refill and full submit in both pilot rotations. Median late-refill savings must be at least the larger of 100 ms and the median duplicate spread. Median full-submit savings must clear its duplicate spread. Peer win counts and paired deltas stay separate from the continuation rule.

[Sealed lane inventory and orders](frozen/series.json) · [Controller request](request.json)

## Verification

The offline check scores the prior EXTERNAL archive and the DEFAULT topology Mesh archive through the retained auditors. It also checks the composition profile, loopback socket parser and gate failures. The new kernel socket attribution is validated on the native warmups before any composed measurement can reconcile.

Run from the checkout root.

```sh
packet=tools/durable-jobs-bench/results/2026-11-24-ordinary-slow-external-topology
python3 -B "$packet/controller.py" check
python3 -B "$packet/offline.py" | diff "$packet/evidence/offline.json" -
python3 -B "$packet/controller.py" replay
```

The replay exits nonzero until both pilot rotations reconcile. It reports per-group deltas, medians, and win counts for the Mesh controls and each peer. The two dispatch commands require the controller’s active lease and use its existing timing and host locks. [Offline evidence](evidence/offline.json)
