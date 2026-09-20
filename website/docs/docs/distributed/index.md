---
title: Distributed Actors
description: Environment bootstrap, node connections, remote actors, monitoring, and local and global process registries in Mesh
---

# Distributed Actors

> **Autonomous clusters:** This page covers the actor primitives. Use [Autonomous Clusters](/docs/autonomous-clusters/) for manifest-driven routing, admission, continuity, and scaling; [Cluster Operations](/docs/cluster-operations/) for operator controls; and [Distributed Proof](/docs/distributed-proof/) for the mandatory PostgreSQL-backed Docker release proof.

Mesh's actor model extends across machines. Once nodes authenticate and connect, remote PIDs use the same typed `send` and `receive` model as local actors.

There are two startup paths:

- `Node.start_from_env()` is the recommended application bootstrap. It chooses standalone or clustered mode from the public environment contract and starts autonomous runtime components when configured.
- `Node.start(name, cookie)` and `Node.connect(name)` are the manual compatibility primitives for local experiments and explicitly managed protocol-one clusters.

Manual protocol-one connections use TLS encryption plus an HMAC-SHA256 cookie challenge. Autonomous protocol-two peers additionally require mutual TLS and a signed, cluster-scoped node identity. Do not treat the cluster cookie as the complete autonomous trust model.

## Recommended Environment Bootstrap

Call `Node.start_from_env` once during application startup:

```mesh
fn main() do
  case Node.start_from_env() do
    Ok(status) ->
      println(
        "mode=#{status.mode} node=#{status.node_name} port=#{status.cluster_port}"
      )
    Err(error) -> println("runtime bootstrap failed: #{error}")
  end
end
```

`BootstrapStatus` exposes:

| Field | Description |
|-------|-------------|
| `mode` | `"standalone"` or `"cluster"` |
| `node_name` | Advertised node name, empty in standalone mode |
| `cluster_port` | Bound/discovered cluster port; the default is `4370` |
| `discovery_seed` | Configured discovery seed, empty in standalone mode |

When no cluster hints and no `MESH_CLUSTER_COOKIE` are present, bootstrap succeeds in standalone mode. Cluster mode reads `MESH_NODE_NAME`/`MESH_NODE_HOST`, `MESH_CLUSTER_PORT`, `MESH_CLUSTER_COOKIE`, and `MESH_DISCOVERY_SEED`; supported platform metadata can supply the node identity. Manifest-driven autonomous applications also embed their validated controller, routing, continuity, and capacity policy during the build.

## Manual Node Startup

A Mesh runtime becomes a named, addressable node by calling `Node.start`. This binds a TCP listener and makes the process ready to accept connections from other nodes:

```mesh
fn main() do
  let status = Node.start("app@localhost:4000", "a-development-cookie")
  if status == 0 do
    println("node started")
  else
    println("node start failed: #{status}")
  end
end
```

The first argument is the node name in `"name@host:port"` format. The second argument is the shared secret cookie. `Node.start` returns `0` on success, `-1` when the runtime has already started, `-2` for a listener bind failure, and `-3` for invalid identity or authentication configuration.

Behind the scenes, `Node.start`:

1. Parses the node address and binds a TCP listener on the given port
2. Builds the transport configuration for the selected protocol
3. Starts an accept loop to handle incoming connections from other nodes

## Connecting Nodes

Once a node is started, it can connect to other nodes with `Node.connect`:

```mesh
fn main() do
  let _ = Node.start("app@localhost:4000", "my_cookie")
  let status = Node.connect("worker@localhost:4001")
  if status == 0 do
    println("connected to worker")
  else
    println("connection failed: #{status}")
  end
end
```

`Node.connect` returns `0` after an authenticated session is established, `-1` if the local node has not started, `-2` for a TCP connection failure, and `-3` for invalid input or handshake failure. After authentication, nodes exchange their global registry state.

### Querying the Cluster

You can inspect the cluster state with `Node.self` and `Node.list`:

```mesh
fn main() do
  Node.start("app@localhost:4000", "my_cookie")
  Node.connect("worker@localhost:4001")

  let me = Node.self()
  println("I am: ${me}")

  let nodes = Node.list()
  println("Connected nodes: ${nodes}")
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Node.self()` | `String` | Current node name, or `""` before clustered startup |
| `Node.list()` | `List<String>` | Authenticated connected node names |

## Remote Actors

Once nodes are connected, you can spawn actors on remote nodes and communicate with them using the same `send` and `receive` primitives you use locally.

### Spawning on a Remote Node

Use `Node.spawn` to start an actor on a specific remote node:

```mesh
actor worker(prefix :: String) do
  receive do
    msg -> println("#{prefix}: #{msg}")
  end
end

actor coordinator() do
  let pid = Node.spawn("worker@localhost:4001", worker, "remote")
  send(pid, "hello from app node")
end
```

`Node.spawn` accepts the actor's normal arguments and returns a PID that is valid across nodes. Remote arguments may currently be `Int`, `Float`, `Bool`, `String`, `Pid`, or `Unit`. The target executable must contain the referenced actor so its runtime entry is registered there. Call remote spawn from an actor: the caller waits cooperatively for the spawn reply. PID `0` reports a failed spawn, such as a missing connection, unsupported argument, or unknown actor entry.

### Spawning with Links

Use `Node.spawn_link` to spawn a remote actor and establish a bidirectional link in one step. If either the local or remote actor crashes, the other receives an exit signal:

```mesh
actor task() do
  receive do
    msg -> println("task completed")
  end
end

actor coordinator() do
  let pid = Node.spawn_link("worker@localhost:4001", task)
  send(pid, "start")
end
```

This is the distributed equivalent of `spawn_link`: the remote-spawn request asks the target to create the actor with a bidirectional link to the caller.

## Local Process Registry

Use `Process` names for services within one runtime:

```mesh
actor cache() do
  receive do
    message -> println("cache: #{message}")
  end
end

fn main() do
  let pid = spawn(cache)
  if Process.register("cache", pid) == 0 do
    let found = Process.whereis("cache")
    send(found, "warm")
  end
end
```

`Process.register` returns `0` on success and `1` on failure. `Process.whereis` returns PID `0` when the name is absent. Local names are not replicated; use `Global` only when another node must resolve the actor.

## Global Registry

The global registry provides cluster-wide process name registration. Unlike local process names (which are scoped to a single node), global names are replicated across all connected nodes.

### Registering a Name

Use `Global.register` to assign a name to a process globally:

```mesh
actor db_service() do
  receive do
    message -> println("query: #{message}")
  end
end

fn main() do
  Node.start("app@localhost:4000", "my_cookie")

  let pid = spawn(db_service)
  Global.register("db_service", pid)
  println("Registered as db_service")
end
```

When a name is registered, it is broadcast to all connected nodes. Every node holds a complete replica of the name table, so lookups are always local (no network round-trip).

### Looking Up a Name

Use `Global.whereis` to find a process by its global name:

```mesh
fn main() do
  Node.start("app@localhost:4000", "my_cookie")
  Node.connect("db@localhost:4001")

  let pid = Global.whereis("db_service")
  send(pid, "query")
end
```

Since every node has a full replica of the global registry, `Global.whereis` returns immediately without a network call. PID `0` means no live registration is known.

### Unregistering a Name

Use `Global.unregister` to remove a global registration:

```mesh
actor temp_worker() do
  receive do
    _ -> nil
  end
end

fn main() do
  Node.start("app@localhost:4000", "my_cookie")

  let pid = spawn(temp_worker)
  Global.register("temp_worker", pid)
  # ... do some work ...
  Global.unregister("temp_worker")
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Global.register(name, pid)` | `Int` | Register globally; `0` is success and `1` is failure |
| `Global.whereis(name)` | `Pid` | Resolve locally from the replicated registry; `0` means absent |
| `Global.unregister(name)` | `Int` | Remove a name; `0` is success and `1` means absent |

### Automatic Cleanup

The global registry automatically cleans up registrations when:

- A **process exits** -- all global names registered by that process are removed
- A **node disconnects** -- all global names owned by that node are removed

This means you do not need to manually unregister names in crash or disconnect scenarios. The cleanup is broadcast to all remaining nodes in the cluster.

## Node Monitoring

`Node.monitor(name)` persistently registers the calling actor for node-up and node-down signals. It returns `0` on success and `1` when called outside an actor, before node startup, or with an invalid name. Register from the actor that will receive the events, after local node startup. Unlike `Process.monitor`, the current node-monitor surface does not return a removable monitor reference.

## API Reference

| Function | Returns | Description |
|----------|---------|-------------|
| `Node.start_from_env()` | `Result<BootstrapStatus, String>` | Bootstrap standalone or clustered runtime state |
| `Node.start(name, cookie)` | `Int` | Manually start a named node; `0` is success |
| `Node.connect(name)` | `Int` | Connect and authenticate; `0` is success |
| `Node.self()` | `String` | Current node name |
| `Node.list()` | `List<String>` | Connected node names |
| `Node.spawn(node, actor, args...)` | `Pid<T>` | Spawn remotely; `0` signals failure |
| `Node.spawn_link(node, actor, args...)` | `Pid<T>` | Spawn remotely and link |
| `Node.monitor(name)` | `Int` | Monitor a remote node; `0` is success and `1` is failure |
| `Process.register(name, pid)` | `Int` | Register a local process |
| `Process.whereis(name)` | `Pid` | Resolve a local process |
| `Global.register(name, pid)` | `Int` | Register a process name cluster-wide |
| `Global.whereis(name)` | `Pid` | Resolve a global process name |
| `Global.unregister(name)` | `Int` | Remove a global registration |

## Next Steps

- [Autonomous Clusters](/docs/autonomous-clusters/) -- configure runtime-owned adaptive routing and capacity management
- [Distributed Proof](/docs/distributed-proof/) -- run the autonomous Docker/PostgreSQL release gate
- [Concurrency](/docs/concurrency/) -- actors, supervision, and services on a single node
- [Developer Tools](/docs/tooling/) -- formatter, REPL, package manager, and editor support
