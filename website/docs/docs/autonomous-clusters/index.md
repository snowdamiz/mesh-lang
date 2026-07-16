---
title: Autonomous Clusters
description: Configure Mesh-owned routing, admission, continuity, and capacity scaling
---

# Autonomous Clusters

Autonomous mode lets Mesh choose an execution worker after a request reaches a Mesh ingress and lets a fenced controller leader reconcile worker capacity through a configured driver. Prometheus, an external autoscaling policy engine, and an external execution load balancer are not part of the control path.

Use this guide to configure an application. Use [Cluster Operations](/docs/cluster-operations/) to operate one, [Capacity Drivers](/docs/capacity-drivers/) to provision capacity, and [Distributed Proof](/docs/distributed-proof/) to run the release proof.

## What Mesh owns

Mesh owns:

- bounded admission and application queues;
- persistent authenticated peer sessions;
- load reports and adaptive owner selection;
- request, operation, attempt, and ownership identities;
- node-local durable continuity records and their cluster replication;
- local scheduler elasticity;
- consensus-backed desired worker capacity;
- drain, ownership transfer, re-replication, and termination gates;
- operator explanations and audit events.

The public address or platform still selects an ingress process. Mesh then selects the worker that executes the clustered handler.

## PostgreSQL and SQLite have different jobs

Use PostgreSQL for shared application data. Every gateway and worker must observe the same committed application rows, and multiple processes may write concurrently.

Mesh separately keeps one private SQLite continuity database per node. It stores request hashes, attempts, ownership generations, replica state, retained responses, snapshot progress, and tombstones. Nodes never share that SQLite file. Mesh replicates continuity records over its cluster protocol and synchronizes a replacement node before it becomes Ready.

PostgreSQL does not elect the Mesh leader, choose a worker, scale capacity, or drain a node. SQLite does not replace the application's PostgreSQL database.

## Manifest configuration

Autonomous policy belongs in `mesh.toml`:

```toml
[cluster]
mode = "autonomous"
default_replicas = 2
durability = "strict"

[cluster.controllers]
voters = 3
autoscale = false

[cluster.roles]
gateway = true
worker = true

[cluster.features]
protocol_two = true
durable_continuity = true
telemetry = true
local_scheduler_autoscaling = true
adaptive_routing = true
controller_quorum = true
horizontal_observe_only = false
automatic_scale_up = true
automatic_scale_down = true

[cluster.scheduler]
min_workers = 2
max_workers = 16
target_runnable_per_worker = 1.0
scale_up_window = "10s"
scale_down_window = "5m"

[cluster.autoscaling]
enabled = true
managed_roles = ["worker"]
min_nodes = 3
max_nodes = 20
target_inflight_per_node = 128
target_queue_wait = "25ms"
scale_up_window = "30s"
scale_down_window = "10m"
cooldown = "2m"
max_scale_up_step = 4
max_scale_down_step = 1
max_unavailable = 1

[cluster.routing]
algorithm = "adaptive"
load_report_interval = "500ms"
load_report_ttl = "2s"
max_inflight_per_node = 256
max_queued_per_node = 512
max_queued_bytes_per_node = "64MiB"
retry_budget_percent = 10

[cluster.continuity]
terminal_retention = "24h"
tombstone_retention = "48h"
max_terminal_records = 1000000
max_disk_bytes = "8GiB"
snapshot_chunk_bytes = "1MiB"

[cluster.capacity]
driver = "docker"
startup_timeout = "2m"
drain_timeout = "5m"
termination_timeout = "2m"
forced_termination = "never"

[cluster.capacity.docker]
image = "registry.example.com/todos@sha256:..."
pool = "workers"
template_revision = "2026-07-15"
network = "todos-private"
env = ["DATABASE_URL", "PORT=8080", "MESH_ROLES=worker"]
```

Mesh rejects invalid bounds, unsafe controller autoscaling, even multi-voter quorums, missing driver templates, secrets embedded in provider configuration, replica policies that cannot fit, scale-down windows no longer than scale-up windows, and snapshot chunks above transport bounds.

Controllers are a fixed, explicitly administered voter set. Worker autoscaling never adds or removes a voter.

The feature gates support a reversible rollout. `horizontal_observe_only = true` still evaluates and explains policy and observes the driver, but it cannot commit a policy-driven desired change or create, drain, or terminate capacity. `automatic_scale_up` and `automatic_scale_down` independently fence policy actions; reconciliation still enforces an authenticated manual desired target. Horizontal autoscaling is rejected unless protocol two, durable continuity, telemetry, and controller quorum are enabled. An autonomous data plane may stage transport, continuity, admission, local scheduling, and routing with `autoscaling.enabled = false` and no provider credentials.

## Handler and replica semantics

Replica counts include the owner:

```mesh
@cluster(3)
pub fn recompute_account(account_id :: String) -> Result<Account, String> do
  # one owner executes; two other nodes retain the continuity record
end

let router =
  HTTP.router()
  |> HTTP.on_get("/accounts/:id", HTTP.clustered(2, handle_get_account))
```

Omitting the count uses `cluster.default_replicas`. Strict durability prepares the required record replicas before execution. If the acknowledgement threshold cannot be reached, Mesh rejects the request instead of silently running with weaker durability.

## Request identity and safe replay

Every HTTP request receives a globally unique request ID. That ID is transport correlation; it does not make an unsafe mutation replayable.

Mutating routes should require `Idempotency-Key`. Mesh scopes it by application, stable handler identity, and authenticated tenant, then stores a canonical request hash. Reusing a key with the same request replays the retained success. Reusing it with different request semantics returns a conflict.

After an indeterminate transport failure, Mesh automatically starts a replacement execution only for an HTTP safe method or a request carrying a validated caller idempotency key.

## Readiness and routing

A worker is eligible only after identity, authentication, protocol negotiation, handler registration, continuity storage and synchronization, peer connectivity, admission, scheduler initialization, and application readiness succeed.

For new work, adaptive routing filters stale, overloaded, unready, draining, incompatible, and circuit-open nodes. It then uses deterministic power-of-two choices with capacity, pressure, reservation, locality, and failure penalties. The selected worker performs the authoritative reservation check before receiving the request payload.

Existing operations remain pinned to their current owner. Ownership moves only through a newer fenced attempt.

## Scaling behavior

Each process can activate or retire scheduler workers within its configured bounds. The controller quorum separately commits horizontal desired capacity. Only the current consensus leader may invoke the capacity driver, and every mutation carries a control term, desired revision, template revision, and idempotent operation ID.

Scale-up follows sustained pressure in bounded steps. Scale-down requires a complete healthy telemetry window and freezes on missing reports, continuity risk, controller instability, driver failure, or an incomplete drain.

Scale-down proceeds through Ready, Draining, Terminating, and Removed. Draining removes the node from routing, completes or transfers work, re-replicates records, and only then permits termination.

`forced_termination = "never"` is the safe default: an expired drain remains visibly blocked for manual intervention. `"after_drain_timeout"` permits abandonment of active work only after required continuity replicas exist and the draining membership generation is acknowledged. Mesh first marks the old owner lost under the existing fence, records the forced outcome and possible active-work loss, then terminates. It never force-terminates the only active copy or presents the result as a clean drain.

## Application APIs

The read-only `Cluster` APIs expose capacity, pressure, role, and lifecycle state. HTTP request accessors expose the request ID and optional idempotency key. Application request code cannot mutate desired capacity; administrative changes require the authenticated operator channel.

## Operational safety

- Losing controller quorum freezes scaling and destructive operations while ready data-plane nodes continue bounded service.
- Missing load reports make a worker routing-ineligible and prevent scale-down.
- Saturation returns an explicit temporary failure without consuming the reserved control budget.
- A late draining node cannot resume an old attempt because ownership generations are fenced.
- Autonomous protocol features require mutually authenticated protocol-two sessions. Protocol-one peers remain a manual-mode compatibility path.

## Next steps

- [Cluster Operations](/docs/cluster-operations/) — inspect, explain, pause, override, and drain safely
- [Capacity Drivers](/docs/capacity-drivers/) — configure Process, Docker, and Fly drivers
- [Cluster Migration](/docs/cluster-migration/) — move from manual or primary/standby deployments
- [Distributed Proof](/docs/distributed-proof/) — run the PostgreSQL-backed Docker release proof
