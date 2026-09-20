---
title: Distributed Proof
description: Run the PostgreSQL-backed Docker proof for autonomous scaling, routing, failover, and drain
prev: false
next: false
---

# Distributed Proof

The autonomous clustering release gate is one repository-owned command:

```bash
cargo run -p meshc -- proof docker-autoscaling
```

With an installed compiler, run `meshc proof docker-autoscaling`.

This is a real local proof, not a static Compose example. Mesh commits desired capacity from runtime telemetry, invokes its Docker capacity driver, creates and removes workers, routes live requests through two gateways, survives a worker and controller leader loss, drains surplus capacity, and validates shared PostgreSQL state.

Use [Autonomous Clusters](/docs/autonomous-clusters/) for configuration and [Cluster Operations](/docs/cluster-operations/) for the operator commands exercised here.

## Prerequisites

- Docker Engine and Docker Compose v2
- OpenSSL
- a supported Linux host or Docker Desktop environment
- enough memory and CPU for three controllers, two gateways, PostgreSQL, two baseline workers, four proof-support containers, and up to five Ready workers
- local access to build the Mesh application and driver images

The first run may pull pinned base images. Once present, the proof does not require a cloud account, Kubernetes, Prometheus, an external load balancer, or a hosted control plane.

## Topology

The fixed topology contains three controller voters, two gateway-only processes, PostgreSQL, two baseline workers, a dedicated mTLS Docker driver service, a load generator, and an evidence collector. The driver creates additional worker-only containers on the proof network.

Application workers never receive Docker credentials. The dedicated driver service is the only container with Docker Engine access. Treat unrestricted socket access as host-root-equivalent; the proof fixture is not a recommended production credential boundary.

## What PostgreSQL proves

PostgreSQL is a shared application dependency and the final data-integrity oracle. The proof seeds acknowledged Todo mutations and verifies the final row count after routing, worker loss, leader loss, scale-up, and scale-down.

PostgreSQL does not perform Mesh consensus, scaling, routing, continuity, or drain. Each node keeps a different private SQLite continuity file. Mesh replicates those records and synchronizes replacements through its protocol.

The PostgreSQL readiness check requires continuous availability across the image entrypoint's temporary initialization server and final server restart. A node cannot pass readiness by connecting only during the initialization window.

## Proof sequence

The command:

1. records Docker, Compose, source, and dirty-worktree information;
2. generates temporary node and driver mTLS identities;
3. validates the fully resolved Compose configuration with secrets redacted;
4. reproducibly builds the application and driver images;
5. starts a uniquely named project and waits for PostgreSQL, controller quorum, gateways, baseline workers, continuity synchronization, and application readiness;
6. records baseline capacity and container labels;
7. sends sustained traffic through both gateways and checks request-key uniqueness;
8. observes a policy-committed desired-capacity increase and Docker EnsureNode operations;
9. follows the latest committed desired revision and waits for desired, provider-observed, and routing-eligible worker state to remain converged before benchmarking;
10. completes 1,000 synchronized remote requests with unique continuity keys, zero failures, and p99 at or below 6,000 ms;
11. abruptly kills a worker and verifies replacement at the committed desired count;
12. kills the active controller leader and verifies a higher-term leader without duplicate capacity;
13. removes load and waits for the complete healthy scale-down window;
14. observes Draining, routing exclusion, continuity transfer and re-replication, termination, and provider-observed absence;
15. verifies return to minimum capacity without oscillation;
16. validates operation IDs, labels, routing, error bounds, continuity, PostgreSQL state, and cleanup.

No proof step changes desired capacity manually or uses `docker compose scale`.

## Evidence

The command prints a timestamped evidence directory. `summary.json` contains 36 release assertions plus the final pass state. The bundle also retains environment and image identities, redacted resolved Compose configuration, baseline/peak/draining/final capacity, controller consensus snapshots, ordered decisions and driver operations, container lifecycle and labels, per-gateway and per-worker routing counts, the 1,000-request and injected-failure summaries, sampled continuity, database integrity, the redacted Compose log, redacted per-managed-worker logs, and cleanup outcome.

Evidence is collected before cleanup on success and failure. Cleanup removes the fixed project, driver-created workers, networks, temporary volumes, and temporary credentials by default.

## Debugging options

```bash
# Keep the failed or successful topology for live inspection
cargo run -p meshc -- proof docker-autoscaling --keep-running

# Reuse local proof images
cargo run -p meshc -- proof docker-autoscaling --no-build

# Choose the evidence location
cargo run -p meshc -- proof docker-autoscaling --evidence-dir ./proof-evidence
```

For a failure, read `summary.json` first. Then compare capacity snapshots, the load failure classification, consensus terms, driver operation IDs, managed-container labels, continuity records, database integrity, and the redacted Compose log. Use `--keep-running` only when retained evidence is insufficient.

## Pass criteria

The proof exits nonzero unless Mesh itself initiates scale-up and scale-down, desired/observed/Ready/provider counts converge, operation IDs stay unique, all 1,000 synchronized requests complete remotely with unique keys inside the p99 budget, both gateways serve traffic, execution reaches eligible workers only after Ready, draining nodes receive no new work, controller failover creates no duplicate capacity, admitted safe work retains continuity, PostgreSQL matches acknowledged mutations, capacity returns to minimum without flapping, all resource bounds remain enforced, and cleanup completes.

This command is mandatory before autonomous clustering release. A mocked driver, manual replica change, hosted-only demonstration, or CI-only result does not replace it.
