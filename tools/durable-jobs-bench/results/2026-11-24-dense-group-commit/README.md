# Dense PostgreSQL group commit on Linode

The two prior native R2 hot-checkout runs recorded 1,694 and 1,650 cluster WAL syncs. River recorded 459 and 513, Oban 932 and 1,013, and Apalis 514 and 507. Mesh's cumulative sync times of 821 and 806 ms imply means of 0.484 and 0.488 ms per sync. These counters cover the whole workload and clean shutdown. They cannot assign waits to particular jobs or establish how much of the drain could be saved. [Original native exports and reduction](../2026-11-24-dense-wal-cost/evidence/native/final-replay.json)

The treatment sets PostgreSQL 17.11 `commit_delay=100` µs and `commit_siblings=1`. A WAL flush leader can briefly wait for other transactions to share a sync. The default already permits concurrent commits to share flushes. Waiting longer can slow a workload. Every engine gets the same treatment, verified with the durability settings in three off-clock snapshots. [PostgreSQL 17 WAL configuration](https://www.postgresql.org/docs/17/wal-configuration.html)

## Matched runs

Six four-engine blocks use the prior 1,024-job input and pinned binaries. Each block runs one full warmup and one measurement per engine. The deployment uses two 32-slot runners per engine on CPUs 0 and 1, shared with PostgreSQL. Every arm must verify 2,048 distinct application write XIDs, the original settlement contract, and the ext4 device identity. Mesh uses the experimental R2 hot pool. Public-product transfer would need its own evidence.

| Rotation | PostgreSQL settings, in order | Engine order in each block |
| --- | --- | --- |
| 1 | Default A, treatment, default B | Mesh, River, Oban, Apalis |
| 2 | Default B, treatment, default A | Apalis, Oban, River, Mesh |

Both defaults explicitly set zero delay and five siblings. Every block gets a fresh staging namespace. The original scorer verifies every warmup and measurement, and retains failed attempts. An orderly shutdown and same-cluster readback account for delayed PostgreSQL counter publication. The result reports every engine's submit and drain throughput, middle-write duration, and sync count for every block.

A larger comparison earns time if Mesh records fewer syncs than both duplicates in both rotations and beats the faster default on all three clocks in each rotation. Each saving in milliseconds must exceed both 5% of the faster default and the absolute gap between defaults. These two rotations screen for a repeatable effect. They cannot establish a public-product win over another engine.

## Replay and dispatch

From the checkout root, verify the archived input hashes, stage content, and scorer controls offline.

```sh
d=tools/durable-jobs-bench/results/2026-11-24-dense-group-commit
python3 -B "$d/controller.py" check
python3 -B "$d/test_packet.py"
```

The sole controller runs the following when this request reaches the head of its queue. It acquires the normal controller fences. All performance measurements run on Linode.

```sh
python3 -B "$d/controller.py" intake "$(shasum -a 256 "$d/request.json" | cut -d' ' -f1)"
```

The raw arm archives are written under `evidence/native/<rotation>-<case>/blocks`. Once both rotations qualify, regenerate the combined scores with `python3 -B "$d/controller.py" replay`.
