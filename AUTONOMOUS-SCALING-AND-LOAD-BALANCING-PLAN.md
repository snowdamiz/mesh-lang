# Autonomous Scaling and Built-In Load Balancing Plan

Status: Runtime implementation and mandatory local proof complete; release certification pending the time-bound and credentialed gates below

Audience: Mesh compiler, runtime, tooling, and documentation maintainers

Primary outcome: A maintainer can select a milestone, implement it without reconstructing the audit context, and prove it against explicit safety and acceptance criteria.

## 0. Implementation and release evidence

Implementation audit date: 2026-07-16

The runtime, compiler, tooling, documentation, Docker driver, and Fly driver are implemented. The mandatory local Docker/PostgreSQL proof passes with live policy-driven scale-up and scale-down, two ingress gateways, embedded-controller failover, real Docker worker creation/removal, continuity transfer, injected driver failures, and exact PostgreSQL mutation integrity.

| Area | Status | Current evidence |
| --- | --- | --- |
| Identity, replica semantics, and idempotency | Implemented | Cross-ingress identity, conflict/replay, arbitrary replica, recovery, and fencing suites |
| Persistent transport and admission | Implemented | Protocol-two multiplexing, bounded per-class queues, reservations, circuit breakers, and the 1,000-request Docker burst |
| Durable continuity | Implemented | Per-node SQLite lifecycle, batching, tombstones, compaction, incremental log, checksummed resumable snapshot, and replica-ack tests |
| Telemetry, local elasticity, and routing | Implemented | Runtime-owned signal, scheduler model, admission, heterogeneous-capacity routing, stale/draining exclusion, and performance gates |
| Consensus and horizontal reconciliation | Implemented | Three-voter OpenRaft proof, term fencing, idempotent driver operations, failover, and orphan reconciliation |
| Capacity drivers | Implemented | Process, Docker, and Fly drivers; 14-test Fly fake-API conformance suite, including signed dynamic-worker identity enforcement |
| Graceful scale-down | Implemented | Ready → Draining → Terminating → Removed proof with zero new drain assignments and continuity transfer |
| Security and operations | Implemented | Remediated threat-model review, signed node identities, role authorization, audit rejection retention, redaction, pause/override, and versioned operator state |
| Mandatory local proof | **Pass** | `target/proof/docker-autoscaling/1784238321222` (36/36 assertions; 1,000/1,000 remote requests; p99 5.307s; PostgreSQL 12 acknowledged mutations = 12 rows; cleanup complete) |
| Deterministic chaos | **Pass** | `target/proof/autonomous-chaos/1784239422659` (5 rounds) |
| Performance | **Pass** | `target/proof/autonomous-performance/1784239476871` (all 13 schema-two budgets) |
| Fly driver conformance | **Pass** | `target/proof/fly-driver-conformance/1784239637301` retained the original 13-test bundle; the current 14-test suite also passes locally after adding the signed Fly worker identity regression |
| Continuity smoke | **Pass, non-release** | `target/proof/continuity-soak/1784231611198` (30-second harness validation) |
| Full 24-hour continuity soak | **Canceled; no release evidence** | Stopped at user direction on 2026-07-16 before completion; the Fly Machine, encrypted volume, app, and hourly monitor were deleted |
| Full Fly autoscaling soak | **Deferred at user direction** | The reproducible three-controller/two-gateway/two-to-five-worker/PostgreSQL/runner harness is implemented, but the one-hour provider run was stopped during provisioning; every run app and Managed Postgres cluster was deleted, so this is not certification evidence |
| Credentialed Fly staging lifecycle | **Pending future execution** | Requires a future staging run with an immutable worker image and bearer-token environment; no Fly test resources remain allocated |

The last three rows do not represent missing runtime code. They are deliberately non-simulated release gates that consume wall-clock time or mutate external Fly resources. Autonomous mode must not be described as fully release-certified until the required retained artifacts exist.

## 1. Executive summary

Mesh currently provides actor scheduling, node discovery, clustered handler placement, continuity records, and primary/standby recovery. It does not yet provide autonomous capacity management or end-to-end, load-aware request balancing. DNS discovery observes nodes that something else created. Clustered HTTP placement starts after a proxy or platform has already selected an ingress node. Placement hashes a request key over live membership without considering capacity. Several correctness and scalability defects must also be fixed before automatic scaling is safe.

This plan delivers two related capabilities:

1. Local elasticity: a Mesh process adjusts its scheduler worker count and admission limits using runtime-native signals.
2. Horizontal elasticity: a Mesh control plane computes desired node capacity and invokes a configured capacity driver to create, drain, and terminate nodes.

Prometheus is not required. Mesh owns signal collection, aggregation, policy evaluation, scaling decisions, routing, and operator explanations. Metrics exporters may be added later as optional observability integrations.

Mesh also owns execution load balancing. Every Mesh HTTP ingress can route eligible work to a ready node using current load, capacity, health, locality, and continuity constraints. A first-party gateway role can provide the same behavior when applications prefer separate ingress processes.

One boundary must remain explicit: software cannot create machines without authority over an execution substrate. Autonomous horizontal scaling therefore requires a capacity driver with credentials for a process supervisor, container platform, or cloud API. That driver is part of the Mesh runtime contract; an external metrics system or autoscaling policy engine is not.

Local Docker proof is a release-blocking requirement. Mesh must create and remove Docker worker containers through its own capacity policy and Docker driver while traffic enters through multiple Mesh gateways. A static Compose topology, manual replica change, cloud-only demonstration, or mocked driver does not count.

## 2. Goals

The finished system must:

- Scale scheduler workers up and down inside a running Mesh process.
- Scale application nodes up and down when a capacity driver is configured.
- Prove horizontal scale-up, load-aware routing, graceful scale-down, and recovery locally with Docker Engine and Docker Compose.
- Make scaling decisions from runtime-owned signals rather than Prometheus queries.
- Keep a configurable minimum capacity and never exceed a configured maximum.
- Avoid rapid oscillation through stabilization windows, cooldowns, bounded step sizes, and explicit hysteresis.
- Admit, queue, shed, and route work using bounded resources.
- Balance new clustered requests using live load and capacity rather than membership-only hashing.
- Preserve request affinity and idempotency across retries and failover.
- Drain a node before scale-down and prove that no newly admitted work is assigned to it.
- Recover safely from node, network, controller, ingress, and capacity-driver failures.
- Bound continuity memory, disk, snapshot, and synchronization costs.
- Support multiple record replicas consistently with the public replication count.
- Keep operator state inspectable through Mesh-owned CLI commands and structured events.
- Support safe rolling upgrades and a reversible migration from the existing cluster protocol.
- Provide deterministic unit, model, integration, concurrency, chaos, soak, and performance tests.

## 3. Non-goals for the first production release

- Scaling an application to zero. At least one controller and the configured minimum worker capacity remain running.
- Predictive or machine-learning scaling. The first policy is explainable, deterministic, and reactive.
- Global anycast, authoritative DNS hosting, or cloud networking abstraction. Mesh balances execution after traffic reaches a Mesh gateway or application node.
- Exactly-once side effects across arbitrary external systems. Mesh provides deduplicated execution and replay within the configured retention window; applications must still use transactional or idempotent downstream operations.
- Unbounded queues, unbounded retries, or unbounded state retention.
- A custom cloud abstraction that attempts to hide every provider feature. The core defines a narrow idempotent capacity-driver contract.
- Replacing optional monitoring products. Prometheus, OpenTelemetry, or hosted monitoring may consume Mesh signals later, but none is needed for control decisions.
- Treating a cloud deployment or hosted CI run as a substitute for the mandatory local Docker proof.

## 4. Terms and semantic decisions

These definitions must be used consistently in code, CLI output, documentation, and tests.

- Ingress node: the Mesh process that accepts a client connection.
- Owner node: the single node selected to execute a request attempt.
- Record replica: a node that stores continuity state but does not execute the request.
- Total replicas: the owner plus record replicas. A value of one means no redundant copy.
- Controller: a Mesh role participating in strongly consistent control-plane decisions.
- Worker: a Mesh role eligible to execute clustered application work.
- Gateway: a Mesh role eligible to accept client traffic and forward it to workers.
- Capacity driver: an idempotent adapter that observes, creates, drains, and terminates capacity.
- Desired state: the controller-approved target node count and role allocation.
- Observed state: the nodes and capacity reported by the driver and live membership.
- Ready: eligible for new work.
- Draining: alive and allowed to finish existing work, but ineligible for new work.
- Request ID: a globally unique identifier for one transport request.
- Idempotency key: a stable, caller- or runtime-derived operation identity used to recognize retries.
- Attempt ID: a unique identity for one execution attempt under an idempotency key.
- Control term: a monotonically increasing fencing value issued by the consensus leader.

The existing HTTP.clustered(N, handler) and @cluster(N) argument is defined as total replicas. The owner counts as one. The compiler and runtime must stop using language that ambiguously treats N as both backups and total copies.

The first production release keeps controller voters as a fixed, explicitly administered quorum. Autoscaling changes gateway and worker capacity only. Adding or removing a controller is a separate quorum-reconfiguration operation and is never an incidental result of application scale-up or scale-down.

## 5. Hard invariants

No implementation milestone may weaken these invariants.

### 5.1 Execution and idempotency

- At most one unfenced owner may execute a given idempotency key at a time.
- A retry with the same idempotency key and the same canonical request hash returns the stored terminal result when retained.
- A retry with the same idempotency key but a different canonical request hash is rejected as a conflict.
- A generated request ID is globally unique across nodes, process restarts, and rolling replacements.
- Retries are bounded and classified. Validation, authorization, and deterministic application failures are not retried.
- A request is not automatically replayed after execution may have started unless the route is idempotent or supplies an idempotency key.

### 5.2 Routing and admission

- A node that is unready, stale, overloaded beyond its hard limit, or draining receives no new assignments.
- Every queue has a configured item and byte bound.
- Overload produces a fast, explicit response with retry guidance rather than blocking an unrelated control or scheduler thread.
- Cluster-control traffic has a separate resource budget from application traffic.
- Load reports are time-bounded. A stale report makes a node ineligible rather than optimistically healthy.

### 5.3 Scaling

- Only a fenced control-plane leader may change desired capacity.
- Capacity-driver calls are idempotent and include operation IDs and the current control term.
- Scale-down always transitions Ready to Draining to Terminating to Removed.
- The controller never terminates a node that still owns non-transferable work.
- The minimum, maximum, disruption budget, cooldown, and rate limits are enforced even after leader failover.
- A temporary metrics gap freezes scale-down. It must never be interpreted as zero load.

### 5.4 State and replication

- Active continuity records are never evicted solely because a size limit was reached.
- Terminal records have explicit retention, compaction, and tombstone rules.
- Snapshot and synchronization protocols are chunked, resumable, checksummed, and bounded.
- A replication policy either reaches its documented acknowledgement threshold or reports degraded durability before execution.
- Ownership changes are fenced by a term or generation so an old owner cannot resume after failover.

### 5.5 Compatibility and operations

- Protocol capability negotiation prevents an old node from silently misinterpreting a new message.
- Operator status distinguishes desired, observed, ready, draining, and failed capacity.
- Every automatic scaling decision records inputs, policy evaluation, outcome, and failure reason.
- All automatic behavior has an emergency pause or disable control that does not require restarting the cluster.

## 6. Current gaps to close

The implementation must explicitly close each audited gap.

| Gap | Current effect | Required destination |
| --- | --- | --- |
| DNS-only discovery | Mesh connects to externally created nodes | Reconciled desired capacity plus capacity drivers |
| Fixed scheduler size | Local capacity cannot adapt | Dynamically resizable worker pool with safe lower and upper bounds |
| External ingress selection | Mesh only places work after another system picks ingress | Any ingress performs built-in load-aware forwarding; optional first-party gateway |
| Membership-only hashing | Hot and slow nodes receive work | Capacity-aware selection with affinity and failure-domain constraints |
| Process-local request counter | Cross-node and restart collisions | Separate globally unique request IDs and deterministic idempotency keys |
| Per-request cluster connection | TLS and handshake cost for every dispatch | Persistent multiplexed authenticated peer sessions |
| Inline blocking accept path | Head-of-line blocking and control-plane starvation | Dedicated transport reactor and asynchronous completion |
| Unbounded continuity map | Memory and sync costs grow forever | Durable bounded lifecycle, retention, compaction, and incremental sync |
| Full snapshot in one message | Join eventually exceeds the message limit | Chunked snapshot plus incremental log catch-up |
| One standby limit | Language accepts counts the runtime rejects | General owner plus N minus one record replicas |
| Primary/standby authority | Cannot safely coordinate active ingress and autoscaling | Strongly consistent controller quorum with fenced leadership |
| Missing scale-down drain | Requests can race termination | Explicit drain state machine and ownership transfer |
| Order-dependent runtime test | Full suite can fail despite isolated success | Resettable test state or process-isolated tests |

## 7. Target architecture

~~~mermaid
flowchart LR
    C[Clients] --> A[Public address or DNS]
    A --> G1[Mesh gateway or app ingress]
    A --> G2[Mesh gateway or app ingress]

    G1 --> R[Built-in routing and admission]
    G2 --> R
    R --> W1[Ready worker]
    R --> W2[Ready worker]
    R --> W3[Ready worker]

    W1 --> T[Runtime telemetry]
    W2 --> T
    W3 --> T
    G1 --> T
    G2 --> T

    T --> Q[Controller quorum]
    Q --> D[Desired capacity]
    D --> P[Capacity driver]
    P --> X[Process, container, or cloud substrate]

    R --> K[Continuity and idempotency]
    K --> W1
    K --> W2
    K --> W3
~~~

The data plane and control plane are separate:

- The data plane accepts, admits, places, forwards, executes, and returns requests.
- The control plane owns membership intent, load summaries, scaling policy, capacity reconciliation, drain orchestration, and audit events.
- A controller outage may freeze scaling, but it must not stop already-ready nodes from serving safe traffic.
- A data-plane overload must not prevent heartbeats, controller messages, drain commands, or operator inspection.

### 7.1 Database and storage boundary

PostgreSQL is the shared application database for the production topology and the mandatory Docker proof. Application records that must be visible from every gateway and worker, including the PostgreSQL starter's todo records and proof mutations, must never be placed in a container-local SQLite file.

SQLite remains an intentional implementation detail of each node's embedded `ContinuityStore`. That store contains node-local request-continuity, replay, replication-log, snapshot, and tombstone state; Mesh replicates those records through the cluster protocol. It is not a shared application database and correctness must not depend on multiple processes opening the same SQLite file. Each node owns a distinct durable file or volume, and a replacement node synchronizes continuity state before becoming Ready.

This split is deliberate:

- PostgreSQL provides multi-writer shared application state, transactional mutations, and the final data-integrity oracle.
- Embedded SQLite provides low-latency local control-path durability without making continuity availability depend on PostgreSQL.
- Losing PostgreSQL makes database-backed application routes unready or failed; it does not erase already-replicated Mesh control or continuity state.
- Losing one node-local SQLite file is tolerated only to the configured continuity replica and acknowledgement policy.
- PostgreSQL credentials and migrations belong to the application deployment; continuity schema migrations remain runtime-owned and local to each node.

## 8. Public language and configuration contract

### 8.1 Manifest configuration

Cluster-wide deployment policy belongs in the package manifest rather than on individual handler declarations. The proposed contract is:

~~~toml
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
managed_roles = ["gateway", "worker"]
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
retry_budget_percent = 10

[cluster.continuity]
terminal_retention = "24h"
tombstone_retention = "48h"
max_terminal_records = 1000000
max_disk_bytes = "8GiB"
snapshot_chunk_bytes = "1MiB"

[cluster.capacity]
driver = "process"
startup_timeout = "2m"
drain_timeout = "5m"
termination_timeout = "2m"
forced_termination = "never"

[cluster.capacity.process]
command = ["./output"]
working_directory = "."
~~~

The first production-provider profile uses Fly Machines and keeps its bearer token outside the
manifest:

~~~toml
[cluster.capacity]
driver = "fly"

[cluster.capacity.fly]
app_name = "mesh-production"
token_env = "FLY_API_TOKEN"
image = "registry.fly.io/mesh-production@sha256:..."
region = "lax"
pool = "workers"
template_revision = "v1"
cpu_kind = "shared"
cpus = 1
memory_mb = 256
~~~

`token_env` names the environment variable read by the controller/driver boundary; a provider
token is never accepted inline in the manifest or written to the control log. Application database
configuration such as `DATABASE_URL` is worker-template environment and does not grant PostgreSQL
any control-plane authority.

All durations and byte sizes require strict parsing, useful diagnostics, minimum and maximum validation, and canonical formatting. Invalid combinations fail before the HTTP listener reports ready.

The manifest loader must validate at least:

- Minimum nodes are not greater than maximum nodes.
- Minimum scheduler workers are not greater than maximum workers.
- A controller role has a safe quorum configuration for autonomous horizontal scaling.
- Controller voters are excluded from autoscaled roles.
- Strict durability does not request more replicas than the maximum eligible nodes can provide.
- Scale-down windows are longer than scale-up windows unless explicitly overridden.
- Queue and in-flight bounds are positive and fit platform integer limits.
- Snapshot chunks fit below the negotiated transport frame limit.
- Autonomous mode names a usable capacity driver.
- A process driver supplies a command and working directory through typed configuration rather than shell interpolation.

### 8.2 Handler-level contract

Keep source-first declarations:

~~~mesh
@cluster(3)
pub fn recompute_account(account_id :: String) -> Result<Account, String> do
  # handler body
end

let router =
  HTTP.router()
  |> HTTP.on_get("/accounts/:id", HTTP.clustered(2, handle_get_account))
~~~

Required semantics:

- N is total replicas, including the owner.
- Omitting N uses cluster.default_replicas.
- N equal to one is valid but emits a warning in autonomous production mode because it has no record redundancy.
- A count above the implementation maximum is a compile-time error.
- A count above current live capacity is a runtime durability decision, not a parser ambiguity.
- Strict durability rejects the request before execution when the acknowledgement threshold cannot be reached.
- Degraded durability is opt-in and returns explicit continuity metadata.
- Handler metadata contains a stable route or function identity so idempotency keys are scoped correctly.

### 8.3 HTTP idempotency contract

For mutating HTTP routes:

- Accept an Idempotency-Key header.
- Scope the key by application identity, stable route identity, and authenticated tenant when applicable.
- Hash a canonical request representation and store the hash with the key.
- Return a replay marker when serving a retained successful result.
- Do not cache generic validation, authorization, or server-error responses as successful completion.
- Reject a reused key with a different canonical request hash.

For requests without a caller key:

- Generate a unique request ID from stable node identity, random boot identity, and a monotonic counter.
- Treat the generated ID as transport correlation, not as proof that an unsafe mutation can be replayed.

### 8.4 Runtime API additions

Add typed, read-only application APIs:

- Cluster.capacity() returns local and cluster capacity summaries.
- Cluster.pressure() returns the current normalized pressure and dominant signal.
- Cluster.role() returns controller, gateway, worker, or combined roles.
- Cluster.state() returns joining, ready, draining, or terminating.
- HTTP.request_id(request) returns the unique request ID.
- HTTP.idempotency_key(request) returns an optional validated operation key.

Applications must not directly mutate desired capacity through ordinary request code. Administrative changes go through authenticated operator commands.

### 8.5 Backward compatibility

- Existing Node.start and Node.start_from_env flows continue to work in manual mode.
- Existing environment variables remain aliases during the migration period.
- Existing HTTP.clustered(N, handler) source remains valid after the total-replica semantics are documented.
- Protocol version one remains available for a defined compatibility window, but autonomous mode requires protocol version two across all controllers.
- Mixed-version clusters operate in manual placement mode until every controller and eligible worker advertises the required capabilities.
- The compiler warns on configurations whose behavior changes, especially ambiguous replica assumptions and HTTP.clustered(1).

## 9. Runtime-native telemetry

Prometheus is not part of the control path. Each process maintains a bounded in-memory telemetry window and reports compact summaries to controllers.

### 9.1 Required local signals

Collect:

- Active and configured scheduler workers.
- Runnable actor count.
- Per-worker run queue depth.
- Actor mailbox depth distribution.
- Scheduler busy time and idle time.
- HTTP connections, in-flight requests, admitted queue depth, and rejected requests.
- Request queue-wait, service-time, and end-to-end latency histograms.
- Remote-dispatch queue depth, bytes, timeouts, retries, and circuit state.
- Per-peer session health and send-buffer utilization.
- Continuity active records, terminal records, disk bytes, compaction lag, and replication lag.
- Process memory and CPU availability when the platform exposes them.
- Capacity-driver operation counts, latency, and errors.

Signals used by policy decisions must be available without logs, scraping, or an external database.

### 9.2 Collection design

- Use monotonic clocks for windows and deadlines.
- Use bounded histograms or sketches rather than retaining individual observations.
- Keep cluster labels bounded; never create a metric dimension from raw request IDs, URLs, or actor IDs.
- Publish one compact load report per node at the configured interval.
- Include node identity, boot identity, role, state, capacity, sequence number, controller term, timestamp, and protocol version.
- Reject out-of-order reports from an earlier boot identity or control term.
- Expire a report after load_report_ttl.

### 9.3 Pressure score

The initial policy uses an explainable normalized score:

~~~text
inflight_pressure = inflight / target_inflight
queue_pressure = p95_queue_wait / target_queue_wait
runnable_pressure = runnable_actors / max(active_workers, 1)
memory_pressure = used_memory / memory_soft_limit

node_pressure = max(
  inflight_pressure,
  queue_pressure,
  runnable_pressure,
  memory_pressure
)
~~~

Apply an exponentially weighted moving average for decisions, while retaining the instantaneous value for overload protection. Every decision records the dominant component.

CPU utilization may inform the score but must not be the sole input. An application can be saturated on I/O, a downstream dependency, mailboxes, or queue wait while CPU remains low.

### 9.4 Telemetry failure behavior

- Missing local telemetry disables local scale-down.
- Missing reports make a remote node ineligible for new assignments after TTL.
- Loss of a controller quorum freezes desired-capacity changes.
- Load reports are advisory for routing; hard admission limits are enforced locally.
- A malformed or oversized report is rejected and recorded without taking down the peer session.

## 10. Local scheduler elasticity

The scheduler must support changing active worker count without rebuilding process-global state.

### 10.1 Design

- Initialize up to max_workers worker contexts.
- Mark a configurable subset active.
- Scale up by activating parked workers.
- Scale down by marking selected workers retiring, moving stealable jobs away, and parking them only after their local queue and current coroutine are safe.
- Never destroy actor ownership or heap state merely because a worker retires.
- Keep at least min_workers active.
- Use a coordinator separate from ordinary actor execution.

### 10.2 Local policy

- Scale up when runnable pressure remains above target for the scale-up window.
- Scale up quickly in bounded steps.
- Scale down only after the longer scale-down window.
- Scale down one worker at a time initially.
- Cancel scale-down immediately when queue wait or runnable pressure rises.
- Respect cgroup, affinity, and detected CPU limits where available.

### 10.3 Acceptance criteria

- Worker count changes under synthetic load without process restart.
- No actor, message, timer, link, monitor, or registry entry is lost while a worker retires.
- A blocked actor cannot indefinitely prevent unrelated workers from retiring.
- Repeated load bursts do not cause rapid worker-count oscillation.
- The scheduler test suite can create and destroy isolated scheduler instances without global-state leakage.

## 11. Membership and controller quorum

DNS remains a bootstrap mechanism, not the source of desired state.

### 11.1 Stable identity

Persist:

- Cluster ID.
- Stable node ID.
- Per-process boot ID.
- Assigned roles.
- Controller voting identity when applicable.

Node display names and addresses may change. Stable identity must not be derived solely from hostname, IP address, or process-local counters.

### 11.2 Controller consistency

Use a maintained embedded consensus implementation after a focused dependency and storage review. Do not build scaling leadership from DNS ordering, last-write-wins broadcasts, or an unfenced primary environment variable.

The replicated control log contains:

- Membership intent.
- Controller configuration.
- Desired capacity revisions.
- Scaling policy revisions.
- Drain and termination intents.
- Capacity-driver operation IDs and outcomes.
- Manual overrides and autoscaler pause state.
- Current fencing term.

Controllers should normally run as an odd-sized quorum. One-controller development mode is supported but clearly reported as non-resilient. Controller membership changes use joint-consensus or the equivalent safe reconfiguration mechanism supplied by the selected implementation.

For the first production release, the controller voter count is fixed after bootstrap unless an authenticated operator performs an explicit quorum reconfiguration. A worker template must not inherit the controller role. Controller voters may also serve traffic in a small deployment, but the autoscaler treats their voting role as fixed capacity and never terminates them.

### 11.3 Membership states

~~~mermaid
stateDiagram-v2
    [*] --> Provisioning
    Provisioning --> Joining: process reachable
    Joining --> Warming: authenticated and synchronized
    Warming --> Ready: readiness gates pass
    Ready --> Draining: scale-down or maintenance
    Draining --> Ready: drain cancelled
    Draining --> Terminating: active work transferred or completed
    Terminating --> Removed: driver confirms absence
    Provisioning --> Failed: startup deadline
    Joining --> Failed: auth or sync failure
    Warming --> Failed: readiness deadline
    Failed --> Provisioning: bounded retry
~~~

Only Ready workers are placement candidates.

### 11.4 Discovery reconciliation

- Resolve seeds to find initial peers.
- Authenticate and learn the cluster ID before accepting replicated state.
- Compare live sessions against controller membership.
- Disconnect or quarantine a node removed from desired membership even if its TCP session remains healthy.
- Use randomized heartbeat timing and suspicion windows to avoid synchronized storms.
- Distinguish unreachable, draining, terminated, and administratively removed states.

### 11.5 Bootstrap

- Bootstrap controller voters through an explicit initial cluster configuration.
- Require a majority of the configured voters before autonomous horizontal scaling becomes active.
- Allow a one-controller development cluster, but label it non-resilient in status and diagnostics.
- Keep application service available at fixed capacity when a controller quorum cannot form.
- Do not allow a capacity driver to create an unbounded sequence of replacement controllers.
- Require explicit operator approval and safe consensus reconfiguration to add, replace, or remove a voter.

## 12. Built-in ingress, routing, and admission

### 12.1 Ingress model

Every application node may run the Mesh HTTP ingress. A separate gateway role runs the same routing engine without application workers. When a request reaches either role:

1. Parse and validate within bounded limits.
2. Resolve route metadata.
3. Resolve or create request and idempotency identities.
4. Check continuity for an existing owner or terminal result.
5. Apply local admission control.
6. Select an eligible owner.
7. Reserve capacity on the owner.
8. Dispatch over a persistent peer session or execute locally.
9. Record outcome and return continuity headers.

External DNS or a platform address may still decide which Mesh ingress receives the TCP connection. It does not decide which worker executes the request.

### 12.2 Candidate filtering

Exclude nodes that:

- Do not advertise the required handler or protocol capability.
- Are not Ready.
- Are draining or terminating.
- Have stale load reports.
- Have an open route or peer circuit breaker.
- Exceed hard in-flight, queue, memory, or transport-buffer limits.
- Cannot satisfy the required replica and failure-domain policy.
- Are incompatible with the request's pinned continuity generation.

### 12.3 Selection algorithm

Use power-of-two choices with capacity weighting for new stateless work:

1. Deterministically sample two eligible nodes from request ID and membership generation.
2. Compare normalized pressure, outstanding reservations, locality cost, and recent failure penalty.
3. Select the lower effective score.
4. Create a short-lived reservation before dispatch.
5. Release or convert the reservation when the owner accepts or rejects.

This produces low selection overhead and avoids sending every request to a single apparently least-loaded node.

For existing idempotency keys:

- Use the continuity owner while it remains eligible.
- If ownership must move, commit a fenced ownership generation before retry.
- Do not rehash an in-flight operation merely because membership changed.

For record replicas:

- Prefer distinct nodes and failure domains.
- Select replicas independently from the execution owner.
- Never place two logical copies on the same node.

### 12.4 Admission control

Implement hierarchical bulkheads:

- A reserved control-plane budget.
- A gateway-wide application budget.
- Per-peer transport budgets.
- Per-handler or handler-class budgets where configured.
- A bounded waiting queue with item, byte, and deadline limits.

On rejection:

- Return 503 for temporary saturation.
- Include Retry-After where a useful estimate exists.
- Record whether rejection came from ingress admission, owner reservation, circuit state, or durability failure.
- Do not execute on the ingress as an unbounded fallback when the chosen owner is full.

### 12.5 Load report race handling

Load reports are not reservations. Two gateways can choose the same worker simultaneously. The owner therefore performs the authoritative admission check. A rejected reservation causes at most one bounded reselection when the request is safe to retry and the cluster retry budget allows it.

## 13. Peer transport redesign

The existing per-request TCP/TLS/HMAC connection and inline wait path must be replaced before enabling adaptive routing by default.

### 13.1 Session manager

Maintain one or a small bounded number of authenticated sessions per peer:

- Perform TLS and cluster authentication once per session.
- Negotiate protocol version, maximum frame size, compression support, roles, and feature capabilities.
- Use independent reader and writer loops.
- Multiplex requests with correlation IDs.
- Use bounded outbound queues by message class.
- Reserve control frames so application traffic cannot starve heartbeats or drain commands.
- Reconnect with exponential backoff and full jitter.
- Apply a retry budget so a cluster recovery does not create a retry storm.

### 13.2 Message lifecycle

For a remote HTTP dispatch:

1. Ingress sends Reserve with request metadata and deadline.
2. Owner replies Accepted or Rejected.
3. Ingress sends payload only after acceptance when payload size makes two-phase transfer worthwhile; small payloads may be coalesced.
4. Owner records Started before invoking the handler when continuity is required.
5. Owner returns Completed, Failed, or Indeterminate with the correlation ID.
6. Ingress completes the client response without blocking the accept loop or scheduler worker.

### 13.3 Framing and backpressure

- Use a length-delimited versioned envelope.
- Enforce maximum frame and decompressed sizes before allocation.
- Chunk large state transfer messages.
- Track queued bytes as well as message count.
- Close or quarantine a peer that repeatedly violates negotiated bounds.
- Never hold a global membership or continuity lock during network I/O.

### 13.4 Error classification

Retryable before handler start:

- Connection reset.
- Session replacement.
- Owner reservation rejection caused by transient capacity.
- Deadline-safe transport timeout before acceptance.

Not automatically retryable:

- Authentication or authorization failure.
- Protocol mismatch.
- Invalid request.
- Handler panic after Started without a valid idempotency policy.
- Idempotency hash conflict.
- Expired client deadline.

### 13.5 Circuit breakers

Keep independent sliding-window breakers for:

- Each peer session.
- Capacity-driver operations.
- Optional downstream control integrations.

Use Closed, Open, and Half-Open states with limited probes. Circuit transitions are part of Mesh telemetry and operator events.

## 14. Request identity and idempotency repair

### 14.1 Identity types

Introduce explicit types:

- RequestId: globally unique correlation identity.
- OperationKey: deterministic, scoped idempotency identity.
- AttemptId: unique attempt under an operation.
- OwnershipGeneration: fenced version of the selected owner.

RequestId generation uses:

- Stable node ID.
- Random boot ID generated from the operating system CSPRNG.
- Atomic monotonic counter.

The tuple may be encoded compactly and hashed for headers. Counter exhaustion must fail closed rather than wrap.

OperationKey generation uses:

- Cluster or application ID.
- Stable handler or route ID.
- Tenant or security scope where relevant.
- Caller Idempotency-Key, or an application-supplied deterministic operation identity.

Never substitute a random request ID for a caller idempotency key when deciding whether a mutation can be replayed.

### 14.2 Canonical request hash

Define canonical hashing per transport:

- HTTP method.
- Stable route ID, not the raw path template spelling.
- Normalized path parameters.
- Canonical query parameter ordering.
- Selected headers that affect semantics.
- Body bytes after any documented content normalization.
- Authenticated tenant scope.

Do not include connection-local data, arrival timestamps, gateway identity, or trace IDs.

### 14.3 Immediate regression coverage

Before wider refactoring, add tests that prove:

- Two ingress nodes can each issue their first request to the same handler without collision.
- A rolling restart does not reuse a retained key.
- Concurrent identical idempotency keys execute once.
- Concurrent key reuse with different payloads produces one conflict and no second execution.
- Requests without idempotency keys receive unique request IDs but unsafe mutations are not automatically replayed.

## 15. Continuity store and replication

### 15.1 Storage model

Replace the unbounded process-global map with a ContinuityStore interface. The production implementation uses one embedded SQLite transactional store per Mesh node. SQLite files are never shared between nodes; the Mesh continuity protocol performs replication and recovery. Shared application data uses PostgreSQL as defined in Section 7.1. The continuity store contains:

- A primary key for operation identity.
- Request hash.
- Owner node and ownership generation.
- Attempt history.
- Phase.
- Replica set.
- Creation, update, terminal, and expiry times.
- Response metadata or bounded response body where replay is enabled.
- Control term and schema version.

Keep a bounded in-memory index for active and recently accessed records. The store is authoritative.

### 15.2 Lifecycle

Recommended phases:

- Reserved
- Replicating
- Admitted
- Started
- Completed
- Failed
- Indeterminate
- Expired
- Tombstoned

Terminal success may be replayed until expiry. Failed and indeterminate outcomes use explicit retry rules. Active records do not expire without a reconciliation decision.

### 15.3 Retention and compaction

- Retain terminal records for terminal_retention.
- Convert removed terminal records to tombstones.
- Retain tombstones longer than the maximum retry and replication-lag window.
- Compact in bounded batches with time and I/O budgets.
- Apply both record-count and byte limits.
- When limits are approached, reject new durable work or reduce optional replay bodies rather than evicting active correctness state.
- Surface compaction lag and store pressure through CLI and telemetry.

### 15.4 Replication semantics

For N total replicas:

- Select one owner and N minus one record replicas.
- Write the initial record to the configured acknowledgement threshold before execution.
- Default strict acknowledgement threshold is a majority of N.
- Record every replica acknowledgement.
- Repair missing replicas in the background while respecting bounded queues.
- Re-replicate before draining a node that holds required copies.
- Report degraded durability explicitly when policy permits progress without the target.

Only the owner executes. Record replicas become eligible owners after a fenced ownership transfer.

### 15.5 Incremental synchronization

Replace whole-map single-message synchronization with:

1. Exchange store generation and high-water marks.
2. Stream a bounded snapshot in numbered chunks when required.
3. Verify per-chunk and final checksums.
4. Resume from the last acknowledged chunk after disconnect.
5. Stream log entries after the snapshot high-water mark.
6. Apply idempotently by operation key and version.
7. Compact only after all required replicas pass the safe point or the retention policy permits removal.

### 15.6 Recovery

On restart:

- Load stable identity and local continuity state.
- Join as Warming, not Ready.
- Reconcile control term and ownership generations.
- Catch up required state.
- Resolve records left in Started or Indeterminate using route policy.
- Become Ready only after storage, handler registration, transport, and replication gates pass.

## 16. Autonomous scaling control loop

### 16.1 Inputs

The leader evaluates:

- Ready and observed worker counts.
- Cluster-wide admitted rate and concurrency.
- Pressure distribution, not just the average.
- Queue wait and rejection rate.
- Per-handler hotspots.
- Pending provisioning and draining operations.
- Capacity-driver health.
- Current cooldown and stabilization windows.
- Minimum, maximum, step, and disruption constraints.

### 16.2 Desired capacity calculation

Initial algorithm:

~~~text
required_by_inflight =
  ceil(cluster_inflight / target_inflight_per_node)

required_by_pressure =
  ceil(ready_nodes * cluster_pressure_ewma)

raw_desired =
  max(min_nodes, required_by_inflight, required_by_pressure)

bounded_desired =
  clamp(raw_desired, min_nodes, max_nodes)
~~~

Then:

- For scale-up, use the maximum recommendation observed in the scale-up stabilization window.
- For scale-down, use the minimum recommendation only when every sample in the scale-down window permits it.
- Subtract neither pending nodes nor draining nodes incorrectly: pending successful capacity counts toward future desired state but not current ready capacity.
- Apply max_scale_up_step, max_scale_down_step, cooldown, and max_unavailable.
- Prefer scale-up when signals disagree.
- Freeze scale-down on missing reports, controller instability, capacity-driver errors, continuity pressure, or incomplete drain.

### 16.3 Reconciliation, not commands

The leader writes a DesiredCapacity revision. A reconciler repeatedly compares desired and observed state. It does not assume a create or terminate call succeeded merely because the API returned success.

Every operation has:

- Cluster ID.
- Operation ID.
- Control term.
- Desired revision.
- Node template revision.
- Deadline.
- Retry classification.

The capacity driver must safely return the existing result when the same operation ID is retried.

### 16.4 Scale-up state machine

~~~mermaid
sequenceDiagram
    participant T as Telemetry
    participant C as Controller leader
    participant D as Capacity driver
    participant N as New node

    T->>C: Sustained pressure reports
    C->>C: Commit desired capacity revision
    C->>D: Ensure capacity with operation ID and term
    D-->>C: Observed instance identity
    N->>C: Authenticate and join
    C->>N: Assign roles and membership generation
    N->>N: Register handlers and warm runtime
    N->>C: Synchronize continuity and report readiness
    C->>C: Commit Ready state
    C-->>T: New node becomes routing candidate
~~~

Failure handling:

- A startup deadline marks the instance Failed.
- The reconciler observes whether it exists before retrying creation.
- Retries use exponential backoff with full jitter.
- Repeated template failures open the capacity-driver circuit and pause further scale-up attempts while continuing to serve with existing nodes.

### 16.5 Scale-down state machine

1. Select a candidate with the lowest transferable load, no controller quorum risk, and acceptable failure-domain impact.
2. Commit Draining in the control log.
3. Remove the node from new routing candidates.
4. Wait for gateways to acknowledge the new membership generation or for the propagation deadline.
5. Finish or safely transfer active requests.
6. Re-replicate continuity records and global registrations.
7. Confirm no required ownership or replica responsibility remains.
8. Ask the capacity driver to terminate using an idempotent operation ID.
9. Observe actual absence.
10. Commit Removed and release tombstones after their retention window.

If pressure rises, drain may be cancelled before termination. Once termination begins, the controller provisions replacement capacity instead of attempting to resurrect an ambiguous instance.

### 16.6 Candidate selection

Never select:

- The leader when doing so would lose quorum.
- A node holding the only valid copy of active state.
- A node running a unique required capability.
- More nodes than max_unavailable permits.

Prefer:

- Already underutilized nodes.
- Nodes with the fewest active ownership transfers.
- Nodes whose removal improves packing without violating failure-domain spread.
- The oldest matching template revision during a rolling replacement.

## 17. Capacity-driver contract

### 17.1 Interface

The core driver operations are:

- ValidateConfiguration
- ObserveCapacity
- EnsureNode
- BeginDrain
- TerminateNode
- GetOperation

All mutating operations are idempotent. Drivers return typed states such as Pending, Succeeded, RetryableFailure, PermanentFailure, and Unknown.

### 17.2 Initial drivers

Deliver in this order:

1. Local scheduler driver: changes active scheduler workers.
2. Process driver: starts and stops child Mesh processes on one managed host without shell interpolation.
3. Docker driver: creates, observes, drains, and removes labeled Mesh worker containers through the Docker Engine API for the mandatory local proof.
4. Fly Machines production driver, selected because the repository already supports Fly deployment bootstrap and private networking, implemented through the Machines HTTP API behind the same interface.
5. Additional provider drivers based on demand.

Horizontal autoscaling is not declared generally available until the Docker driver passes the local proof and at least one production driver passes failure-injection and idempotency certification. The process driver proves the protocol but does not prove container or multi-host elasticity.

The Fly Machines driver must:

- Use the Machines HTTP API directly; do not shell out to `flyctl`.
- Read its bearer token only from the configured environment variable and redact credentials and worker environment by schema.
- Label Machine metadata with managed marker, cluster ID, pool, template revision, operation ID, control term, and desired revision.
- Re-observe by operation metadata before creation so controller restart and response loss adopt the existing Machine.
- Cordon a Machine before the Mesh drain gate can advance to termination, then delete only a Machine whose managed cluster and pool metadata match.
- Treat absence during deletion as success and classify timeouts, HTTP 408/429, and server failures as retryable without caching them as terminal results.
- Respect the provider action-rate limit with bounded concurrency, backoff, and full jitter.
- Pass a fake-API conformance suite without live credentials; a credentialed staging proof remains required before general availability.

The Docker driver is a real capacity driver, not a test stub. It must:

- Reconcile actual Docker containers against desired worker capacity.
- Create workers from an immutable image and typed container template.
- Label every managed container with cluster ID, pool, template revision, operation ID, and control term.
- Adopt an existing container only when all identity labels and the template revision match.
- Restrict observation and deletion to containers carrying the expected cluster and managed-pool labels.
- Join workers to the proof network without changing controller, database, load-generator, or artifact containers.
- Treat create and remove calls as idempotent.
- Detect create-success-response-loss, orphan, stopped, unhealthy, and already-removed cases.
- Preserve Docker operation evidence in Mesh scaling events.
- Refuse arbitrary shell commands and unapproved image substitutions.

For local development, Docker socket access belongs in a dedicated driver container or a narrowly scoped socket proxy. Application workers must not receive Docker Engine credentials. The proof documentation must state that unrestricted Docker socket access is host-root-equivalent and is not a recommended production control boundary.

### 17.3 Driver isolation and security

- Run provider calls outside scheduler and transport critical paths.
- Use separate bounded concurrency and circuit breakers.
- Apply least-privilege credentials.
- Never place secrets in control-log events, diagnostics, or command lines.
- Redact provider payloads by schema.
- Validate instance template revisions before committing desired capacity.
- Authenticate an external driver over a local protected channel or mTLS.
- Record actor, reason, operation ID, term, and outcome for every destructive call.

## 18. Readiness, draining, and shutdown

### 18.1 Readiness gates

A worker becomes Ready only after:

- Stable identity and cluster authentication succeed.
- Required protocol capabilities are negotiated.
- Handlers and route metadata are registered.
- Continuity storage opens and migrations finish.
- Required state synchronization catches up.
- Peer transport reaches the minimum healthy connectivity.
- Local admission and scheduler capacity initialize.
- Application-provided readiness checks pass.

Liveness and readiness remain separate. A draining node is live but not ready for new work.

### 18.2 Drain contract

- Stop accepting new client connections when the node is a gateway selected for termination.
- Return or redirect connection-level guidance where protocol permits.
- Mark all local handlers ineligible for new remote reservations.
- Allow accepted requests until their deadlines.
- Transfer fenced ownership for recoverable requests.
- Re-replicate records and registry ownership.
- Close idle sessions after control acknowledgements.
- Exit cleanly before the driver termination deadline.

### 18.3 Forced termination

If drain exceeds its deadline:

- Report the blocking request and state categories.
- Follow configured policy: abort scale-down, extend once, or force terminate.
- Never label a forced termination lossless.
- Preserve enough tombstone and ownership-generation state to fence a late returning node.

## 19. Operator experience

Extend the Mesh CLI with:

- meshc cluster capacity: desired, observed, pending, ready, and draining counts.
- meshc cluster pressure: cluster and per-node pressure with dominant signals.
- meshc cluster routing: candidate eligibility, load reports, reservations, and circuits.
- meshc cluster scaling: policy, stabilization windows, cooldown, and last decisions.
- meshc cluster events: ordered control and scaling events.
- meshc cluster explain: why a request chose or rejected a node.
- meshc cluster autoscale pause and resume.
- meshc cluster scale: authenticated manual desired-capacity override.
- meshc cluster drain and cancel-drain.

JSON output is versioned and tested as a public operator contract. Human output must distinguish:

- No load from missing telemetry.
- Desired capacity from ready capacity.
- Driver acceptance from observed creation.
- A healthy node from a routing-eligible node.
- Degraded durability from normal completion.

Each scaling decision explanation includes:

- Decision timestamp and control term.
- Policy revision.
- Input window.
- Dominant pressure signal.
- Raw and bounded desired counts.
- Constraints that changed the result.
- Driver operation IDs.
- Cooldown or pause reason.

## 20. Security model

The existing shared cluster cookie is insufficient as the only long-term identity mechanism for a control plane that can destroy capacity.

Plan:

- Introduce a cluster CA or equivalent node identity authority.
- Use mutually authenticated TLS for node, controller, gateway, operator, and external-driver channels.
- Separate node membership, operator, and capacity-driver permissions.
- Support credential rotation without full-cluster restart.
- Include stable node ID and cluster ID in certificates or signed identity claims.
- Bind control messages to protocol version, cluster ID, term, sender, sequence number, and expiry.
- Reject replayed control messages.
- Audit destructive actions and configuration changes.
- Redact request bodies, idempotency keys, cookies, tokens, and provider credentials.
- Rate-limit authentication failures and operator queries independently from application traffic.

The migration may continue accepting cookie-authenticated protocol-one data peers in manual mode, but autonomous controller and driver operations require the stronger protocol.

## 21. Failure-mode requirements

| Failure | Required behavior |
| --- | --- |
| One worker crashes | Stop routing to it after suspicion threshold; fence ownership; retry only safe work |
| Gateway crashes | Other gateways continue; in-flight clients observe ordinary connection failure |
| Controller leader crashes | Quorum elects a new leader; no duplicate driver operation; data plane continues |
| Controller quorum lost | Freeze scaling and destructive operations; continue bounded data-plane service |
| Network partition | Minority controllers cannot scale; stale workers become ineligible; old owners are fenced |
| Capacity API times out | Reconcile observed state before retry; reuse operation ID |
| Capacity API partially succeeds | Adopt or terminate observed orphan according to desired revision |
| New node never becomes ready | Mark failed after deadline; do not route; apply bounded retry |
| Load reports stop | Freeze scale-down; expire node from routing after TTL |
| Continuity store fills | Reject new durable work explicitly; preserve active records |
| Snapshot interrupted | Resume from acknowledged chunk and high-water mark |
| Slow handler | Consume only its bounded execution slot; do not block accept or control loops |
| Peer overload | Reject reservation quickly; open circuit when thresholds are met |
| Retry storm | Enforce cluster retry budget and full-jitter backoff |
| Rolling mixed versions | Negotiate capabilities; disable unsupported autonomous features |
| Draining node returns late | Fencing generation prevents it from executing or rejoining as Ready |

## 22. Implementation milestones

Milestones are ordered by safety dependency. Autonomous scaling must not be enabled before the request, transport, continuity, and admission foundations are complete.

### Milestone 0: Freeze contracts and establish baselines

Deliverables:

- Approve terminology, replica semantics, idempotency contract, and non-goals.
- Record architecture decisions for consensus, continuity storage, transport execution model, and the first production capacity driver.
- Build reproducible one-, two-, three-, and five-node test harnesses.
- Define the mandatory Docker Compose proof topology, Docker capacity-driver fixture, load profile, polling contracts, and retained evidence schema.
- Add baseline throughput, latency, memory-growth, join-time, and failure-recovery benchmarks.
- Add deterministic clock and fake capacity-driver interfaces.
- Make all existing runtime tests order-independent or process-isolated.

Acceptance:

- The full runtime suite passes repeatedly under default and randomized test ordering.
- Baseline artifacts are stored for later regression comparison.
- Each architectural decision has a named owner and reversal strategy.

Rollback:

- Documentation and test-only; no runtime behavior changes.

### Milestone 1: Repair identities and replica semantics

Deliverables:

- Introduce RequestId, OperationKey, AttemptId, and OwnershipGeneration.
- Replace the process-local HTTP continuity key.
- Implement canonical request hashing and HTTP idempotency header validation.
- Define total-replica semantics across parser, type checker, metadata, runtime, CLI, and docs.
- Remove the one-standby runtime ceiling or reject unsupported counts consistently until Milestone 3.

Acceptance:

- Cross-ingress and rolling-restart collision tests pass.
- Same key and same request executes once and replays.
- Same key and different request conflicts.
- Counts accepted by the compiler are either supported or receive an explicit compile-time diagnostic.

Rollback:

- Keep protocol-one key handling behind a compatibility decoder; generated requests use version-two identities only when all eligible nodes advertise support.

### Milestone 2: Replace per-request transport

Deliverables:

- Persistent peer session manager.
- Capability negotiation and protocol-two envelope.
- Correlation map and asynchronous completions.
- Bounded per-class send queues.
- Dedicated reader and writer loops.
- Retry classification, retry budget, jittered reconnect, and peer circuit breakers.
- Chunk-capable framing.

Acceptance:

- At least one thousand concurrent remote clustered requests complete without serial accept-loop behavior.
- A deliberately slow handler does not delay peer connection, heartbeat, operator query, or unrelated request acceptance beyond their budgets.
- Queue saturation returns bounded rejection.
- No global lock is held across network I/O.

Rollback:

- Keep protocol one available in manual mode; autonomous features remain disabled during fallback.

### Milestone 3: Replace continuity storage and replication

Deliverables:

- Durable ContinuityStore.
- Active, terminal, expiry, and tombstone lifecycle.
- Configurable retention and compaction.
- Owner plus arbitrary record-replica sets within a documented maximum.
- Majority acknowledgement for strict durability.
- Chunked resumable snapshot and incremental synchronization.
- Recovery and ownership fencing.

Acceptance:

- A long-running soak reaches steady-state storage usage under fixed retention.
- A new node joins after at least one million terminal test records without a frame-size failure.
- Interrupted snapshots resume.
- Draining a replica causes re-replication before termination.
- Old owners cannot execute after a newer ownership generation commits.

Rollback:

- Store schema supports read-only export and downgrade tooling; do not silently discard version-two records.

### Milestone 4: Add telemetry, admission, and local elasticity

Deliverables:

- Bounded runtime telemetry windows.
- Versioned node load reports.
- Hierarchical admission bulkheads.
- Reservation protocol.
- Dynamically resizable scheduler.
- Local scaling policy and operator inspection.

Acceptance:

- No Prometheus or external metrics endpoint is used by the policy.
- Worker count follows sustained load and remains stable during oscillating near-threshold load.
- Missing telemetry prevents scale-down.
- Saturation sheds work without starving control traffic.

Rollback:

- Set scheduler minimum equal to maximum and disable local policy without changing the transport.

### Milestone 5: Add adaptive execution routing

Deliverables:

- Candidate eligibility engine.
- Capacity-weighted power-of-two selection.
- Reservation-aware scores.
- Continuity pinning and fenced reassignment.
- Replica failure-domain selection.
- Route explanation data.
- First-party gateway role using the same router.

Acceptance:

- New work distribution tracks heterogeneous node capacity within an agreed tolerance.
- A slow or overloaded node receives progressively less new work.
- Draining and stale nodes receive zero new reservations.
- Existing idempotency keys remain pinned or move through fenced transfer.
- Multiple ingress nodes produce consistent, collision-free outcomes.

Rollback:

- Select static manual placement through a feature flag while retaining safe transport and continuity.

### Milestone 6: Add controller quorum, control-plane identity, and desired-state membership

Deliverables:

- Persistent stable identities.
- Embedded consensus integration.
- Mutually authenticated controller and node channels.
- Minimum authorization for membership, desired-capacity, and drain records.
- Fenced leader term.
- Replicated desired capacity, membership, policy, drain intents, and operation records.
- DNS bootstrap reconciled against desired membership.
- Mixed-version capability gates.

Acceptance:

- Leader failure during a pending operation does not duplicate capacity.
- Minority partitions cannot commit scaling changes.
- Data-plane service continues while scaling is frozen.
- Removed nodes cannot remain routing-eligible merely because their connection is healthy.

Rollback:

- Pause autoscaling and retain manual membership; committed control state remains inspectable.

### Milestone 7: Implement capacity drivers and horizontal scale-up

Deliverables:

- Driver SDK and conformance suite.
- Process driver.
- Docker driver and constrained local Docker Engine integration.
- First production provider driver.
- Desired-versus-observed reconciler.
- Idempotent create and observe flow.
- Startup, warm-up, readiness, and failure deadlines.
- Autoscaling decision engine in observe-only and active modes.

Acceptance:

- A sustained load increase commits a bounded desired-capacity increase.
- Repeated timeouts and leader failover do not create duplicate nodes.
- New nodes receive no traffic before readiness.
- Permanent template errors pause the affected action and explain the failure.
- The Docker driver creates and removes worker containers from committed desired-capacity revisions without invoking Docker Compose scaling as the policy engine.
- Repeated Docker API timeouts and controller failover do not create duplicate labeled containers.
- The production driver passes credential-redaction, retry, orphan, and partial-success tests.

Rollback:

- Pause the autoscaler; retain desired count as a manual target; driver performs no new destructive operation.

### Milestone 8: Implement graceful horizontal scale-down

Deliverables:

- Candidate selection.
- Drain propagation and acknowledgements.
- Active-work completion and ownership transfer.
- Re-replication gates.
- Idempotent termination and observed-removal reconciliation.
- Drain cancellation and forced-termination policies.

Acceptance:

- Graceful scale-down loses no admitted request in the supported idempotency model.
- A draining node receives zero new work after membership propagation.
- Scale-down aborts or cancels when load rebounds.
- Quorum, minimum capacity, replica policy, and max_unavailable remain satisfied.
- Late node return is fenced.

Rollback:

- Disable automatic scale-down independently from scale-up.

### Milestone 9: Security hardening, operations, and rolling migration

Deliverables:

- Complete node, operator, and capacity-driver identity lifecycle.
- Authorization roles.
- Credential rotation.
- Audit log and redaction.
- Complete operator CLI.
- Versioned JSON contracts.
- Rolling-upgrade capability matrix.
- Emergency pause, disable, and manual override.

Acceptance:

- Unauthorized scaling, drain, and termination calls are rejected and audited.
- Secrets do not appear in logs, events, diagnostics, or process listings.
- A rolling upgrade preserves service and does not enable features before quorum compatibility.
- Every scaling action can be explained from retained Mesh-owned state.

Rollback:

- Revoke driver credentials, pause automation, and operate at fixed desired capacity.

### Milestone 10: Production proof and default enablement

Deliverables:

- Updated clustered starter and deployment proof.
- A one-command, self-cleaning local Docker proof that exercises active scale-up and scale-down under load.
- Multi-node load, chaos, soak, and rolling-upgrade evidence.
- Capacity-driver certification artifacts.
- User guide, operator runbook, configuration reference, and migration guide.
- Release notes and compatibility timeline.

Acceptance:

- All definition-of-done criteria in this document pass.
- The mandatory local Docker proof passes from a clean checkout on a documented supported Docker environment.
- Observe-only operation runs for an agreed soak period before active scale-up.
- Automatic scale-down has a separate canary period after scale-up is stable.
- Maintainers sign off on correctness, performance, security, and operations evidence.

Rollback:

- Defaults return to observe-only while users can opt into active mode.

## 23. Test strategy

### 23.1 Unit tests

- Duration, byte-size, and policy validation.
- Request ID uniqueness and counter exhaustion.
- Canonical request hashes.
- Idempotency transitions.
- Candidate filtering and scoring.
- Pressure and desired-capacity calculation.
- Stabilization windows, cooldowns, and step bounds.
- Drain candidate constraints.
- Retention and compaction selection.
- Protocol encode, decode, bounds, and capability negotiation.
- Driver idempotency.

### 23.2 Property and model tests

- At most one valid owner per operation generation.
- Desired capacity always remains within minimum and maximum.
- Scale-down never violates quorum, replica, or disruption constraints.
- Reordered and duplicated protocol messages converge or are rejected safely.
- Arbitrary retry sequences do not create duplicate driver operations.
- Continuity merge is associative and idempotent for valid versions.
- Tombstones prevent deleted records from reappearing during delayed sync.

Use deterministic clocks, seeded randomness, and a simulated transport so failures are reproducible.

### 23.3 Integration scenarios

- Same clustered handler entered concurrently through every node.
- Node restart while peers retain continuity.
- Heterogeneous worker capacities.
- Slow, overloaded, and stale-report nodes.
- Peer connection churn.
- Handler panic before and after Started.
- Controller leader change during scale-up and scale-down.
- Network partition with majority and minority sides.
- Capacity creation succeeds but response is lost.
- Capacity termination times out after actual removal.
- New node joins during compaction.
- Snapshot interruption and resume.
- Rolling protocol-one to protocol-two upgrade.
- Autoscaler pause, manual override, and resume.

### 23.4 Performance tests

Track:

- Local versus remote request throughput.
- p50, p95, and p99 queue wait, dispatch overhead, and total latency.
- TLS handshake rate after session reuse.
- Scheduler scaling reaction and retirement time.
- Routing decision cost.
- Load-report bandwidth.
- Continuity write amplification, compaction cost, and disk growth.
- Snapshot throughput and join time by retained-state size.
- Controller decision and commit latency.
- Capacity-driver reconciliation latency separately from provider startup time.

Release gates use the Milestone 0 baseline. Any regression above the agreed budget requires an explicit exception with profiling evidence.

### 23.5 Chaos and soak tests

Run:

- Twenty-four-hour bounded-retention soak under mixed reads, writes, retries, and node churn.
- Repeated kill, partition, delay, duplication, and disk-full injection.
- Scaling oscillation workload around both thresholds.
- Retry-storm workload after a full peer outage.
- Controller failover during every capacity-driver phase.
- Graceful and forced drain with long-running handlers.

No test may rely on arbitrary sleeps for correctness. Use observable state transitions and bounded deadlines.

### 23.6 Mandatory local Docker proof

Autonomous scaling and built-in load balancing are not considered implemented until they are proven locally with Docker. A hosted cloud demonstration, mocked capacity driver, static two-node Compose file, manual docker compose scale command, or CI-only result does not satisfy this gate.

#### Required command and environment

Provide one repository-owned command:

~~~text
meshc proof docker-autoscaling
~~~

The command must:

- Require Docker Engine and Docker Compose v2.
- Work on supported Linux hosts and Docker Desktop environments.
- Build the Linux Mesh application and driver images reproducibly.
- Validate the resolved Compose configuration before startup.
- Start from a clean, uniquely named Compose project.
- Need no Kubernetes cluster, Prometheus server, external load balancer, cloud account, or hosted control plane.
- Permit an initial pull of pinned base images; once those images are present, the proof must be repeatable without network access.
- Poll Mesh and Docker state with bounded deadlines instead of sleeping for assumed convergence.
- Exit nonzero on any failed invariant.
- Collect evidence before cleanup on success or failure.
- Remove proof containers, networks, and temporary volumes by default.
- Support an explicit keep-running mode for debugging.

Local execution is mandatory before release. CI should run the same command, but a CI pass does not replace the maintainer's ability to reproduce it locally.

#### Required Compose topology

~~~mermaid
flowchart LR
    L[Load generator] --> G1[Mesh gateway 1]
    L --> G2[Mesh gateway 2]
    G1 --> W[Autoscaled worker pool]
    G2 --> W
    W --> DB[PostgreSQL]

    C1[Controller 1] --> DD[Docker capacity driver]
    C2[Controller 2] --> DD
    C3[Controller 3] --> DD
    DD --> DE[Docker Engine]
    DE --> W

    O[Proof collector] --> C1
    O --> G1
    O --> G2
    O --> DE
~~~

The release-gate topology contains:

- Three fixed controller voters so leader failover and fencing are real.
- Two fixed Mesh gateways so requests can enter through more than one process.
- A worker pool with a minimum of two, a maximum of at least five, and no manually declared fixed replica count above the minimum.
- PostgreSQL or another shared durable application dependency required by the chosen starter.
- A dedicated Docker capacity-driver container or constrained Docker socket proxy.
- A load-generator container.
- An evidence collector.

A lightweight developer profile may use one non-resilient controller, but it cannot replace the three-controller release proof.

Controllers and gateways may share a process only if the proof can still kill the active controller leader without removing every ingress. Autoscaled workers must not be controller voters. The Docker driver must create workers from the same image and template revision as the initial worker pool.

#### Required proof sequence

1. Build images and start controllers, gateways, the database, the Docker driver, the load generator, the collector, and the minimum worker pool.
2. Wait until controller quorum, gateways, workers, continuity storage, and application readiness are healthy.
3. Record baseline desired, observed, Ready, and routing-eligible capacity.
4. Send traffic through both gateways and prove request IDs do not collide.
5. Run a sustained load above the scale-up threshold without issuing any manual scaling command.
6. Observe a committed desired-capacity increase and a Docker EnsureNode operation.
7. Observe new labeled worker containers, cluster join, warm-up, continuity synchronization, and Ready transitions.
8. Prove that traffic reaches the new workers only after Ready.
9. Keep load running and verify adaptive routing reduces assignments to an intentionally constrained or slowed worker.
10. Kill one ordinary worker and prove safe removal, fenced ownership recovery, and replacement when desired capacity requires it.
11. Kill the active controller leader during a pending or recent Docker operation and prove that the new leader does not create a duplicate worker.
12. Remove sustained load and wait through the configured local proof stabilization window.
13. Observe a committed desired-capacity decrease.
14. Prove each selected worker transitions through Draining, receives no new assignments, completes or transfers admitted work, re-replicates continuity, and is then removed by the Docker driver.
15. Confirm capacity returns to the configured minimum and remains stable for the anti-flap observation window.
16. Run final continuity, routing, scaling, container-label, error-rate, and data-integrity assertions.
17. Collect the proof bundle and clean up.

The proof profile may use shorter stabilization windows than production defaults, but it must execute the same policy, state machines, reconciliation, and driver code. It must not include test-only shortcuts that set desired capacity directly.

#### Required assertions

The command fails unless all of these are true:

- Scale-up and scale-down were initiated by Mesh policy decisions from runtime-owned telemetry.
- No Prometheus query, Docker Compose scale command, manual desired-capacity mutation, or external load-balancing decision occurred.
- Desired, observed, Ready, and Docker container counts converge at every stable checkpoint.
- Every Docker mutation maps to one committed Mesh operation ID and control term.
- No operation ID creates more than one managed worker.
- Every managed worker has the expected cluster, pool, template, operation, and term labels.
- New workers receive no request before Ready.
- Draining workers receive no new reservation after the drain membership generation propagates.
- Requests enter through both gateways and execution is distributed across eligible workers.
- Load distribution follows advertised capacity within the tolerance fixed during Milestone 0.
- No cross-ingress or restart-related continuity-key collision occurs.
- No admitted idempotent request executes concurrently on two owners.
- Graceful scale-down loses no admitted request under the supported idempotency model.
- The final database state matches acknowledged mutations.
- Controller failover creates no duplicate capacity operation or split-brain desired state.
- After load ends, capacity returns to the configured minimum without oscillation.
- All queues, retries, state transfers, and deadlines stay within configured bounds.

#### Required failure injection

At minimum, inject:

- A slow worker.
- An abruptly killed worker.
- A controller leader failure.
- A lost or delayed Docker create response after container creation.
- A Docker API timeout.
- A traffic drop from high load to idle.

The full release proof should also cover an interrupted continuity snapshot, Docker driver restart, unhealthy new worker, and orphan reconciliation.

#### Required evidence bundle

Retain a timestamped local proof bundle containing:

- Resolved Compose configuration with secrets redacted.
- Source revision and dirty-worktree status.
- Docker and Compose versions.
- Image IDs and immutable digests.
- Mesh compiler and runtime versions.
- Scaling policy and node-template revisions.
- Baseline, peak, post-failure, draining, and final capacity snapshots.
- Ordered controller terms, desired-capacity revisions, scaling decisions, and driver operations.
- Docker container lifecycle and label snapshots.
- Per-gateway and per-worker routing counts.
- Load-generator request, latency, replay, conflict, and error summaries.
- Continuity records for sampled requests from both gateways.
- Database integrity assertions.
- Redacted logs for controllers, gateways, workers, and the driver.
- Cleanup outcome.
- A machine-readable summary with one pass or fail field per required assertion.

The proof command must print the evidence bundle location and a concise final summary. Evidence is part of the release gate, not optional debugging output.

## 24. Rollout strategy

Use independent feature gates:

- Protocol-two transport.
- Durable continuity store.
- Runtime telemetry.
- Local scheduler autoscaling.
- Adaptive routing.
- Controller quorum.
- Horizontal observe-only autoscaling.
- Automatic scale-up.
- Automatic scale-down.

Rollout stages:

1. Ship instrumentation and new identities with old placement.
2. Enable protocol-two transport in canary clusters.
3. Migrate continuity storage and prove bounded retention.
4. Enable admission control.
5. Run adaptive routing in shadow mode and compare decisions.
6. Enable adaptive routing for safe read routes.
7. Pass the mandatory local Docker routing and scale-up proof.
8. Run horizontal autoscaling in observe-only mode.
9. Enable scale-up with scale-down disabled.
10. Pass the mandatory local Docker graceful scale-down and leader-failover proof.
11. Enable conservative one-node-at-a-time scale-down outside the Docker proof.
12. Make autonomous mode the recommended production path only after local Docker and production-provider proof artifacts remain green.

Every gate has:

- A manifest switch.
- An authenticated runtime pause where applicable.
- Operator status.
- Structured reason for being disabled.
- A tested rollback path.

## 25. Documentation and starter updates

Update:

- Distributed runtime guide.
- Clustered HTTP guide.
- Manifest reference.
- Node and Cluster API reference.
- Operator CLI reference.
- Idempotency guide.
- Capacity-driver authoring guide.
- Production deployment guide.
- Failure and recovery runbook.
- Protocol compatibility matrix.
- Migration guide from primary/standby and external placement.

The PostgreSQL starter should eventually demonstrate:

- Multiple active ingress nodes.
- Built-in adaptive execution placement.
- Idempotent clustered mutation as well as clustered reads.
- Runtime-owned telemetry and scaling explanations.
- Observe-only and active autoscaling configuration.
- Graceful drain evidence.
- The exact local Docker proof command, prerequisites, expected state transitions, evidence layout, debugging mode, and cleanup behavior.

Do not update public claims until the corresponding production proof passes.

## 26. Work sequencing and parallel ownership

Safe parallel tracks after Milestone 0:

- Compiler track: manifest schema, diagnostics, handler metadata, API types.
- Identity track: request identity, canonical hashing, idempotency contract.
- Transport track: protocol envelope, sessions, multiplexing, backpressure.
- Storage track: ContinuityStore, retention, compaction, synchronization.
- Scheduler track: telemetry, admission, dynamic workers.
- Tooling track: CLI JSON schemas, events, explain output, test harness.

Integration gates:

- Transport and identity integrate before continuity replication.
- Continuity and admission integrate before adaptive routing.
- Telemetry and controller consensus integrate before scaling policy.
- Driver conformance and readiness integrate before active scale-up.
- Drain, fencing, and re-replication integrate before scale-down.

Assign one maintainer as the owner of cross-cutting invariants and protocol compatibility. Each track must provide tests and operator visibility with its implementation rather than deferring them to the end.

## 27. Decision log required before implementation

Record explicit decisions for:

1. Embedded consensus implementation and storage.
2. Continuity storage engine and schema migration policy: per-node SQLite with runtime-owned migrations and protocol replication; PostgreSQL is reserved for shared application state.
3. Protocol-two transport reactor model.
4. First production capacity driver: Fly Machines HTTP API, with environment-sourced credentials and metadata-fenced adoption/deletion.
5. Docker driver socket-isolation and managed-container boundary.
6. Default strict versus degraded durability.
7. Maximum supported total replica count.
8. Canonical HTTP body normalization rules.
9. Default terminal and tombstone retention.
10. Gateway deployment and public-address guidance.
11. Compatibility window for protocol one and old environment aliases.

Each decision records context, selected option, rejected alternatives, operational consequences, and reversal cost.

## 28. Definition of done

The work is complete only when all of the following are true:

- Mesh changes local scheduler capacity autonomously under sustained load.
- Mesh changes horizontal desired capacity autonomously through a certified driver.
- The same horizontal scaling behavior is proven locally by Mesh creating and removing Docker worker containers under live traffic.
- No Prometheus or external policy engine participates in scaling decisions.
- Any Mesh ingress can select an execution owner using current bounded load data.
- Overloaded, stale, unready, and draining nodes receive no new assignments.
- Cross-ingress and rolling-restart request identity collisions are impossible under tests and model checks.
- Remote dispatch uses persistent multiplexed sessions and does not block accept or control loops.
- Continuity storage reaches steady-state under bounded retention.
- Snapshot and incremental synchronization work beyond the old single-message limit.
- Public replica counts match runtime behavior.
- Controller failover cannot duplicate capacity operations.
- Graceful scale-down drains, transfers, re-replicates, terminates, and observes removal.
- Every automated decision is visible and explainable through Mesh-owned CLI state.
- Mixed-version rollout and rollback are tested.
- Security review covers node identity, operator authorization, driver credentials, replay protection, and audit redaction.
- Full unit, property, integration, chaos, soak, and performance gates pass.
- The mandatory one-command local Docker proof passes, retains its complete evidence bundle, and cleans up successfully.
- Public documentation claims only behavior backed by retained proof artifacts.

## 29. Recommended first implementation slice

Start with a vertical safety slice rather than the autoscaler UI:

1. Add globally unique RequestId and deterministic OperationKey types.
2. Route the same clustered handler through two ingress nodes.
3. Use one persistent peer session with correlation IDs.
4. Apply a bounded owner reservation.
5. Store the continuity record through a minimal ContinuityStore.
6. Return the response and replay it for a duplicate idempotency key.
7. Expose the decision through the existing cluster CLI.
8. Prove concurrent, restart, slow-handler, and saturation cases.

This slice exercises the identities, transport, admission, continuity, and operator seams that every later routing and scaling milestone depends on.
