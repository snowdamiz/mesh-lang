---
title: Concurrency
description: Actors, typed messages, services, supervision, jobs, timers, channels, and process lifecycle in Mesh
---

# Concurrency

Mesh uses the actor model for concurrency, inspired by Erlang/Elixir. Actors are lightweight processes that communicate by message passing. Jobs, scheduler-aware timers, and bounded channels cover common work that does not need a long-lived actor.

> **Autonomous clusters:** This guide teaches the concurrency model. Continue with [Autonomous Clusters](/docs/autonomous-clusters/) for runtime-owned elasticity and [Distributed Proof](/docs/distributed-proof/) for release verification.

## The Actor Model

In Mesh, actors are independent units of computation that do not share memory. Each actor:

- Has its own **mailbox** for receiving messages
- Communicates exclusively via **message passing**
- Runs **concurrently** with other actors
- Is **isolated by default** -- an unlinked actor crashing does not bring down other actors

This model avoids shared-memory data races and shared-state corruption. Application-level waiting cycles are still possible—for example, two services that synchronously call each other—so keep synchronous dependencies acyclic.

## Spawning Actors

Define an actor with the `actor` keyword and start it with `spawn`:

```mesh
actor greeter() do
  receive do
    msg -> println("actor received")
  end
end

fn main() do
  let pid = spawn(greeter)
  send(pid, 1)
  println("main done")
end
```

The `spawn` function returns a **PID** (process identifier) that you use to communicate with the actor. Actors run concurrently with the function that spawned them. Actor parameters become arguments to `spawn`:

```mesh
actor counter(total :: Int) do
  receive do
    amount -> counter(total + amount)
  end
end

fn main() do
  let pid = spawn(counter, 0)
  send(pid, 5)
end
```

Mesh infers `Pid<T>` from the actor's received message type. Sending a value of the wrong type is a compile-time error. Inside an actor, `self()` returns its own PID.

## Message Passing

Actors communicate by sending and receiving messages. Use `send` to deliver a message to an actor's mailbox, and `receive` to wait for the next message:

```mesh
actor worker() do
  receive do
    msg -> println("worker done")
  end
end

fn main() do
  let w1 = spawn(worker)
  let w2 = spawn(worker)
  let w3 = spawn(worker)
  send(w1, 1)
  send(w2, 2)
  send(w3, 3)
  println("main sent all")
end
```

Key points about message passing:

- Messages are processed **one at a time** from the actor's mailbox
- `receive` blocks until the next message arrives
- The current compiler executes the first receive arm; use a single variable or wildcard arm and perform any branching in its body
- You can spawn multiple actors and send messages to each independently

A receive can provide a timeout in milliseconds. The receive expression returns either the message arm's value or the timeout arm's value:

```mesh
actor worker() do
  let result = receive do
    value -> value
  after 1_000 -> 0 end
  println("#{result}")
end
```

Actors can also perform computation before responding. Here is an actor that runs a function when it receives a message:

```mesh
fn count_loop(n :: Int, target :: Int) -> Int do
  if n >= target do
    n
  else
    count_loop(n + 1, target)
  end
end

actor worker() do
  receive do
    msg -> println("${count_loop(0, 100)}")
  end
end

fn main() do
  let pid = spawn(worker)
  send(pid, 1)
end
```

## Linking and Monitoring

Actors can be linked so that failures propagate between them. If one linked actor crashes, the other is notified:

```mesh
actor linked_worker() do
  receive do
    _ -> println("linked worker done")
  end
end

actor linker() do
  let worker = spawn(linked_worker)
  link(worker)
  receive do
    message -> send(worker, message)
  end
end

fn main() do
  let linker_pid = spawn(linker)
  send(linker_pid, 1)
end
```

- **`link(pid)`** -- bidirectionally links two actors. If one dies, the other receives an exit signal.
- **`Process.monitor(pid)`** -- creates a one-way monitor and returns its reference; it returns sentinel `0` outside actor context.
- **`Process.demonitor(reference)`** -- removes a monitor and returns `0` on success or `1` outside actor context or when the reference is unknown.

Linking is the foundation for building fault-tolerant systems: supervisors use links to detect and restart failed actors.

An actor can also declare a termination callback for cleanup:

```mesh
actor worker() do
  receive do
    _ -> println("work complete")
  end
terminate do
  println("cleaning up")
end
end
```

## Supervision

Supervisors are special actors that monitor and restart child actors when they fail. Define a supervisor with the `supervisor` keyword:

```mesh
actor worker() do
  receive do
    msg -> println("worker got message")
  end
end

supervisor WorkerSup do
  strategy: one_for_one
  max_restarts: 3
  max_seconds: 5

  child w1 do
    start: fn -> spawn(worker) end
    restart: permanent
    shutdown: 5000
  end
end

fn main() do
  let sup = spawn(WorkerSup)
  println("supervisor started")
end
```

### Supervision Strategies

| Strategy | Behavior |
|----------|----------|
| `one_for_one` | Only the failed child is restarted |
| `one_for_all` | All children are restarted when one fails |
| `rest_for_one` | The failed child and all children started after it are restarted |
| `simple_one_for_one` | Accepted template strategy; dynamic child start/terminate operations are not exposed by the current Mesh source API |

### Child Specifications

Each `child` block configures how the supervisor manages that actor:

| Option | Purpose |
|--------|---------|
| `start` | Function that spawns the child actor |
| `restart` | Restart policy: `permanent` (always), `transient` (only on abnormal exit), `temporary` (never) |
| `shutdown` | Positive milliseconds to wait for graceful shutdown, or `brutal_kill` |

### Restart Limits

- **`max_restarts`** -- maximum number of restarts allowed within the time window
- **`max_seconds`** -- the time window in seconds

If a child exceeds the restart limit, the supervisor itself shuts down, escalating the failure to its parent supervisor.

When omitted, the strategy defaults to `one_for_one`, `max_restarts` to `3`, and `max_seconds` to `5`.

## Services (GenServer)

Services are stateful actors that follow the GenServer pattern. They provide a structured way to manage state with synchronous calls and asynchronous casts:

```mesh
service Counter do
  fn init(start_val :: Int) -> Int do
    start_val
  end

  call GetCount() :: Int do |count|
    (count, count)
  end

  call Increment(amount :: Int) :: Int do |count|
    (count + amount, count + amount)
  end

  cast Reset() do |_count|
    0
  end
end

fn main() do
  let pid = Counter.start(10)
  let c1 = Counter.get_count(pid)
  println("${c1}")
  let c2 = Counter.increment(pid, 5)
  println("${c2}")
  Counter.reset(pid)
  let c3 = Counter.get_count(pid)
  println("${c3}")
end
```

### Service Anatomy

- **`init`** -- called when the service starts, returns the initial state
- **`call`** -- synchronous request/response. The handler receives the current state and returns a tuple `(new_state, reply)`
- **`cast`** -- asynchronous fire-and-forget. The handler receives the current state and returns the new state

### Starting and Calling Services

The compiler auto-generates snake_case methods from your PascalCase definitions:

```mesh
service Store do
  fn init(start_val :: Int) -> Int do
    start_val
  end

  call Get() :: Int do |state|
    (state, state)
  end

  call Set(value :: Int) :: Int do |_state|
    (value, value)
  end

  cast Clear() do |_state|
    0
  end
end

fn main() do
  let pid = Store.start(100)
  let v1 = Store.get(pid)
  println("${v1}")
  let v2 = Store.set(pid, 200)
  println("${v2}")
  Store.clear(pid)
  let v3 = Store.get(pid)
  println("${v3}")
end
```

| Definition | Generated method |
|------------|-----------------|
| `Store.start(100)` | Starts the service with initial value |
| `Store.get(pid)` | Calls the `Get` handler |
| `Store.set(pid, 200)` | Calls the `Set` handler |
| `Store.clear(pid)` | Casts the `Clear` handler |

Services with no init arguments use `start()` with no parameters:

```mesh
service Accumulator do
  fn init() -> Int do
    0
  end

  call Add(n :: Int) :: Int do |state|
    (state + n, state + n)
  end
end

fn main() do
  let pid = Accumulator.start()
  let _ = Accumulator.add(pid, 1)
  let _ = Accumulator.add(pid, 2)
  let result = Accumulator.add(pid, 3)
  println("${result}")
end
```

Service calls are synchronous and wait for a reply. Casts are asynchronous. Prefer a cast, a direct actor message, or a job when the caller must remain independent of the service's response time.

## Jobs

`Job` runs finite work on lightweight actors and returns failures through `Result`.

```mesh
fn main() do
  let job = Job.async(fn() -> 21 * 2 end)
  case Job.await_timeout(job, 1_000) do
    Ok(value) -> println("#{value}")
    Err(error) -> println("job failed: #{error}")
  end
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Job.async(fn)` | `Pid<T>` | Run a zero-argument function in an actor linked to the caller |
| `Job.await(job)` | `Result<T, String>` | Wait without a timeout |
| `Job.await_timeout(job, timeout_ms)` | `Result<T, String>` | Wait up to the given number of milliseconds |
| `Job.map(values, fn)` | `List<Result<U, String>>` | Run one job per list element and collect replies as they complete |

Job replies go to the actor that started them. The current await collector does not correlate a mailbox reply with the PID argument, so await a job from its original caller and do not mix overlapping `Job.async` calls with unrelated mailbox traffic. A timed-out job is not cancelled; its eventual reply remains in the caller's mailbox. `Job.map` exposes each completion's success or failure instead of failing the whole batch at the first error, but its result order is completion order rather than input order.

## Timers

Timers use monotonic deadlines. Sleeping an actor yields its scheduler worker so other actors can run.

| Function | Returns | Description |
|----------|---------|-------------|
| `Timer.sleep(milliseconds)` | `Unit` | Suspend the current actor until the delay expires |
| `Timer.send_after(pid, milliseconds, message)` | `Unit` | Deliver a typed message after a delay |

```mesh
actor reminder() do
  receive do
    message -> println(message)
  after 5_000 -> println("no reminder received") end
end

fn main() do
  let pid = spawn(reminder)
  Timer.send_after(pid, 100, "time to stretch")
  Timer.sleep(200)
end
```

## Bounded Channels

`Channel` provides bounded, in-process queues for `Int` values. Creation and queue operations return `Result`; producers use `try_send` and never wait for space.

```mesh
fn main() do
  case Channel.bounded_bytes(128, 1_024, :reject_newest) do
    Ok(channel) ->
      let _ = Channel.try_send(channel, 42)
      case Duration.millis(10) do
        Ok(timeout) -> case Channel.recv(channel, timeout) do
          Ok(value) -> println("#{value}")
          Err(error) -> println(error)
        end
        Err(error) -> println(error)
      end
    Err(error) -> println(error)
  end
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Channel.bounded(item_capacity, policy)` | `Result<Int, String>` | Create a queue bounded by item count |
| `Channel.bounded_bytes(item_capacity, byte_capacity, policy)` | `Result<Int, String>` | Apply both item and byte bounds |
| `Channel.try_send(channel, value)` | `Result<Int, String>` | Enqueue an `Int` immediately or report backpressure |
| `Channel.recv(channel, timeout_nanos)` | `Result<Int, String>` | Dequeue, waiting for at most that many nanoseconds; `0` polls |
| `Channel.depth(channel)` | `Int` | Current item count, or `-1` for an unknown handle |
| `Channel.byte_depth(channel)` | `Int` | Current queued bytes, or `-1` for an unknown handle |
| `Channel.dropped(channel)` | `Int` | Values rejected or replaced, or `-1` for an unknown handle |

Overflow policies are:

- `:reject_newest` — keep queued values and return `Err("channel full")` for the new value.
- `:drop_oldest` — discard the oldest value and enqueue the new value.
- `:latest_only` — discard every queued value and retain only the newest.

`Channel.recv` takes nanoseconds. Use `Duration.millis` or `Duration.seconds` to make the unit explicit and to detect overflow.

Each queued `Int` consumes eight bytes, so `bounded_bytes` uses the smaller of `item_capacity` and `byte_capacity / 8`. `try_send` returns `Ok(0)` on acceptance and may return `"channel busy"` instead of waiting for the shared registry lock. `recv` polls synchronously, so keep waits short when calling it from an actor; a long wait occupies that scheduler worker.

## Process Names and Shutdown

`Process` manages actors within the current runtime and coordinates graceful shutdown.

| Function | Returns | Description |
|----------|---------|-------------|
| `Process.register(name, pid)` | `Int` | Register a local name; `0` is success and `1` is failure |
| `Process.whereis(name)` | `Pid` | Resolve a local name; PID `0` means not found |
| `Process.monitor(pid)` | `Int` | Monitor an actor; returns reference `0` outside actor context |
| `Process.demonitor(reference)` | `Int` | Remove a monitor; `0` is success and `1` is failure |
| `Process.install_shutdown_signals()` | `Unit` | Treat native `SIGINT` and `SIGTERM` as shutdown requests |
| `Process.shutdown_requested()` | `Bool` | Read the process-wide shutdown flag |
| `Process.request_shutdown()` | `Unit` | Set the shutdown flag programmatically |
| `Process.exit(status)` | `Unit` | Exit the native process with a status code |

HTTP servers observe the same shutdown flag: after shutdown is requested they stop accepting new connections and drain connections already accepted.

Create and remove monitors from the actor that owns them. Calls to
`Process.monitor` outside actor context return reference `0`;
`Process.demonitor` returns `1` outside actor context or for an unknown
reference.

## Deterministic Random Values

`Random` is a deterministic, explicitly state-threaded pseudo-random generator. It is useful for simulations, scheduling choices, and reproducible tests; it is not a cryptographic random source.

```mesh
fn main() do
  let state = Random.seed(42)
  let next = Random.next_int(state, 1, 100)
  let next_state = Tuple.first(next)
  let value = Tuple.second(next)
  println("state=#{next_state}, value=#{value}")
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Random.seed(seed)` | `Int` | Normalize a deterministic generator state |
| `Random.next_int(state, min, max)` | `Tuple` | Return `(next_state, value)` with `min <= value <= max` |
| `Random.next_unit_ppm(state)` | `Tuple` | Return `(next_state, value)` where `0 <= value < 1_000_000` |

## Next Steps

- [Type System](/docs/type-system/) -- structs, generics, traits, and deriving
- [Distributed Mesh](/docs/distributed/) -- remote actors and global process names
- [Syntax Cheatsheet](/docs/cheatsheet/) -- quick reference for all Mesh syntax
