# Autonomous release gates

Run deterministic performance gates and retain their timestamped result:

```bash
cargo run -p meshc -- proof autonomous-performance
```

The command measures routing decisions, protocol envelope round trips, load-report size/encoding/one-Hz bandwidth, scheduler resize reaction, durable continuity p50/p95/p99, disk use and write amplification, compaction throughput, snapshot size/chunks/join/apply throughput, controller-commit p50/p95/p99, and capacity-driver reconciliation against `performance-budget.json`. The Docker proof separately records end-to-end p50, p95, p99, and maximum application latency for a synchronized 1,000-request burst and while failures are injected.

Run the repeated deterministic chaos gate with:

```bash
cargo run -p meshc -- proof autonomous-chaos
```

The default runs five complete rounds, rotating fault order and alternating one and four Rust test threads. It covers model invariants, retry amplification, disk limits, delayed tombstones, interrupted snapshots, minority/old-leader fencing, real three-voter leader loss, owner-loss recovery, and command deduplication. Every execution log and a machine-readable summary are retained under `target/proof/autonomous-chaos/`.

Run the release bounded-retention soak with:

```bash
cargo run -p meshc -- proof continuity-soak
```

The default is a 24-hour wall-clock run. It continuously creates active and terminal records, completes work across rotating owners, performs reads and duplicate retries, compacts retention and replication logs, resumes interrupted snapshots, records disk and logical-state samples, and requires at least one million terminal records. A short harness check must opt in explicitly:

```bash
cargo run -p meshc -- proof continuity-soak \
  --duration-seconds 10 --cycle-millis 10 --allow-short
```

A short result says `SMOKE PASS` and records `release_24h_pass: false`; it is never a release soak artifact.

Chaos coverage is split deliberately. Deterministic unit/model tests cover reordering, duplication, fencing, bounds, and idempotency. `meshc proof docker-autoscaling` injects a slow worker, abrupt worker loss, controller leader loss, Docker timeout, create-response loss, unhealthy capacity, driver restart, orphan reconciliation, snapshot interruption, and high-load-to-idle transition against real containers and PostgreSQL.
