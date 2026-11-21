# Mesh clustering

## Application bootstrap and starters

`meshc init --clustered <name>` generates a route-free project. It puts the declaration in `work.mpl` and calls `Node.start_from_env()` once in `main.mpl`. The runtime derives the handler name from the module and function, such as `Work.add`. The runtime submits declared startup work automatically and records completion when the handler returns. Standby nodes skip startup submission. [Clustered scaffold and two-node walkthrough](https://meshlang.dev/docs/getting-started/clustered-example/)

```mesh
# work.mpl
@cluster pub fn add() -> Int do
	1 + 1
end
```

```mesh
# main.mpl
fn main() do
	case Node.start_from_env() do
		Ok(status) -> println("mode=#{status.mode} node=#{status.node_name}")
		Err(reason) -> println("bootstrap failed: #{reason}")
	end
end
```

`Node.start_from_env()` returns `Result<BootstrapStatus, String>`. The status has `mode`, `node_name`, `cluster_port`, and `discovery_seed` fields. With no cluster hints or cookie, startup selects standalone mode. Clustered startup reads node identity, discovery, and authentication settings from the environment. [Bootstrap fields](https://meshlang.dev/docs/distributed/#recommended-environment-bootstrap) · [Bootstrap environment](https://meshlang.dev/docs/environment-variables/#node-bootstrap)

A declaration is a public zero-argument function. `@cluster(N)` takes a positive literal replica count including the executing owner. An omitted count uses `[cluster].default_replicas` or the runtime default of 2. [Cluster declarations](https://meshlang.dev/docs/reference/#cluster-declarations) · [Handler and replica semantics](https://meshlang.dev/docs/autonomous-clusters/#handler-and-replica-semantics)

| Starter | What it includes |
| --- | --- |
| `meshc init --clustered <name>` | Source-declared startup work and runtime bootstrap. |
| `meshc init --template todo-api --db postgres <name>` | Shared PostgreSQL storage, `work.mpl` declaring `Work.sync_todos`, and a mix of clustered and local HTTP routes. See `examples/todo-postgres` in the checkout. |
| `meshc init --template todo-api --db sqlite <name>` | Single-node HTTP, local SQLite, and actor-backed write rate limiting. See `examples/todo-sqlite` in the checkout. |

## Distributed actors and names

- `Node.start(name, cookie)` and `Node.connect(name)` are manual startup and connection operations for explicitly managed protocol-one clusters. They return an integer status with `0` for success. Names use `name@host:port`. Protocol one uses TLS and cookie authentication. Autonomous protocol two requires mutual TLS and signed cluster-scoped identities. `Node.self()` gives the local name and `Node.list()` gives authenticated peers. [Manual startup, connections, and status codes](https://meshlang.dev/docs/distributed/#manual-node-startup)
- `Node.spawn(node, actor, args...)` and `Node.spawn_link(node, actor, args...)` start a named actor on a peer and return its typed PID, or PID `0` on failure. The linked form propagates abnormal exits. Remote messages copy transferable values and can carry a reply PID. Functions and local runtime handles cannot cross the connection. [Remote spawn and messaging](https://meshlang.dev/docs/distributed/#remote-actors)
- `Process.register` and `Process.whereis` are local. `Global.register`, `Global.whereis`, and `Global.unregister` use a replicated name registry. Lookups are local and return PID `0` for an absent name. The runtime cleans up names on process exit or node disconnect. [Global registry](https://meshlang.dev/docs/distributed/#global-registry)
- `Node.monitor(name, message)` sends the calling actor a typed message once when the peer disconnects, or immediately if already disconnected. It returns `0` on success or `1` before node startup. `Process.monitor(pid, message)` also works for remote PIDs and fires on process exit or node loss. [Node monitoring](https://meshlang.dev/docs/distributed/#node-monitoring)

## Clustered routes and continuity

`HTTP.clustered(handler)` and `HTTP.clustered(N, handler)` wrap a named public function in route-handler position. Mesh selects one executor and retains request continuity records. A count of 1 has the owner record only. A count of 2 requests an owner and one replica. The PostgreSQL Todo example wraps its read routes with count 1 and its idempotent create route and pressure probe with count 2. Health, toggle, and delete routes are local. The SQLite starter uses local routes throughout. [Clustered route contract](http.md#clustered-routes)

Strict durability requires replica preparation before execution. A request ID correlates traffic. A validated caller `Idempotency-Key` allows replay for a matching request, scoped by application, handler, and tenant. Reusing a key with different request semantics conflicts. Following indeterminate delivery, automatic replacement execution requires a safe HTTP method or a validated caller key. [Request identity and replay](https://meshlang.dev/docs/autonomous-clusters/#request-identity-and-safe-replay)

`Continuity.submit_declared_work` submits custom work for a registered `@cluster` function under its declared replica policy. `Continuity.status` reads a record and `Continuity.authority_status` reports role, promotion epoch, and replication health. Advanced callers can use `Continuity.submit` for explicitly placed work, `Continuity.acknowledge_replica` to report replica preparation, or `Continuity.mark_completed` to complete the current attempt. Use the returned attempt ID and execution owner when completing work. [Continuity functions and record fields](https://meshlang.dev/docs/autonomous-clusters/#advanced-continuity-api)

## Configuration, inspection, and operation

The `[cluster]` manifest configures roles, admission, routing, continuity, and scaling. Mesh keeps per-node continuity state and replicates it across nodes. Use PostgreSQL for shared application state. Capacity changes pass through the authenticated operator and fenced capacity-driver interfaces. [Manifest configuration](https://meshlang.dev/docs/autonomous-clusters/#manifest-configuration) · [Capacity drivers](https://meshlang.dev/docs/capacity-drivers/)

Application code reads local runtime state through `Cluster.capacity()`, `Cluster.pressure()`, `Cluster.telemetry()`, `Cluster.role()`, and `Cluster.state()`. The capacity map includes desired, observed, and ready counts. The pressure map reports the dominant signal and telemetry completeness. [Application APIs and fields](https://meshlang.dev/docs/autonomous-clusters/#application-apis)

Use the runtime CLI to inspect a node. List continuity records first to discover request keys.

```bash
meshc cluster status <name@host:port> --json
meshc cluster continuity <name@host:port> --json
meshc cluster continuity <name@host:port> <request-key> --json
meshc cluster diagnostics <name@host:port> --json
```

The operator runbook covers capacity, pressure, routing, scaling, events, and authenticated pause, resume, and drain. `meshc proof docker-autoscaling` is the Docker and PostgreSQL release gate for autonomous clustering. Mesh also provides separate chaos, performance, continuity-soak, and driver-conformance checks. [Cluster operations](https://meshlang.dev/docs/cluster-operations/) · [Distributed proof and prerequisites](https://meshlang.dev/docs/distributed-proof/) · [Proof commands](https://meshlang.dev/docs/tooling/#proof-commands) · [Cluster migration](https://meshlang.dev/docs/cluster-migration/)
