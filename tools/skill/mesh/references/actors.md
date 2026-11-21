# Mesh actors

## Spawn, send, and receive

An actor has its own heap and mailbox. `spawn(actor_name, args...)` runs a named actor concurrently and returns a `Pid<T>`, where `T` is the mailbox's message type. Give the spawned PID an explicit type if the receive arms and typed send sites leave the type ambiguous. Inside an actor, `self()` returns its own PID. Pass it in a message when the receiver needs a reply address. [Spawning actors](https://meshlang.dev/docs/concurrency/#spawning-actors)

`receive` waits for the next message. Its arms support patterns and `when` guards and must cover the message type. An optional `after milliseconds -> value` arm runs if the wait times out. The actor exits when its body returns. Tail-call the actor with updated arguments to keep receiving.

```mesh
type CountCommand do
	Add(amount :: Int)
	Stop
end

actor counter(total :: Int) do
	receive do
		Add(amount) -> counter(total + amount)
		Stop -> println("total=#{total}")
		after 1_000 -> println("idle=#{total}")
	end
end

fn main() do
	let pid = spawn(counter, 0)
	send(pid, Add(5))
	send(pid, Stop)
end
```

`send(pid, message)` is asynchronous and returns an `Int`. A status of `0` means the message was enqueued locally or written to the remote connection. Codes `1`, `2`, and `3` mean missing PID, full mailbox, and message too large. Codes `4`, `5`, `6`, and `7` mean unavailable remote node, failed remote write, nontransferable value, and a remote send from a grouped actor. [Message passing](https://meshlang.dev/docs/concurrency/#message-passing)

The scheduler yields at waits and reduction checkpoints. Use `spawn grouped(actor_name, args...)` for actors that exchange frequent messages on the same node. An actor spawned from inside a group joins its worker. Each actor keeps its own mailbox and heap. Grouped code must use operations the compiler can verify will yield or return without blocking that worker. [Grouped actors](https://meshlang.dev/docs/concurrency/#keep-communicating-actors-on-one-worker)

## Lifecycle and process names

- `link(pid)` propagates abnormal exits in both directions. Normal and shutdown exits remove the link. Link exit signals are internal to the runtime and supervisors.
- `Process.monitor(pid, message)` sends the calling actor a message of its own mailbox type when the target exits for any reason. A missing target fires the monitor immediately. `Process.demonitor(ref)` removes an outstanding monitor owned by the caller. It returns `0` on success or `1` if the caller no longer holds the reference.
- A `terminate do ... end` block in an actor declaration provides a cleanup callback for spawned actors. See [child specifications](supervisors.md#child-specifications) for the current supervised-child limitation. [Links, monitors, and termination](https://meshlang.dev/docs/concurrency/#linking-and-monitoring)
- `Process.register(name, pid)` registers a local name and returns `0` on success or `1` on failure. `Process.whereis(name)` returns an untyped PID and uses PID `0` for an absent name. Keep a typed spawn site for the registered actor.
- `Process.install_shutdown_signals()` turns `SIGINT` and `SIGTERM` into a process-wide shutdown request. `Process.request_shutdown()` sets the same flag, and `Process.shutdown_requested()` reads it. `Process.exit(status)` exits the native process. [Process names and shutdown](https://meshlang.dev/docs/concurrency/#process-names-and-shutdown)

## Stateful services

A `service` is an actor with generated methods. `init` returns its initial state. A `call` handler returns `(new_state, reply)` and the caller waits for that reply. A `cast` handler returns the new state and its caller continues without waiting for it to run. Both kinds of handler generate snake-case methods.

```mesh
service Counter do
	fn init(initial :: Int) -> Int do
		initial
	end

	call GetCount() :: Int do |count|
		(count, count)
	end

	cast ResetCount() do |_count|
		0
	end
end

fn main() do
	let pid = Counter.start(10)
	println("#{Counter.get_count(pid)}")
	Counter.reset_count(pid)
	println("#{Counter.get_count(pid)}")
end
```

A call waits for its own reply and leaves ordinary mailbox messages queued. The caller panics if the service is gone, stops before replying, or cannot accept the call because its mailbox is full. Keep synchronous service dependencies acyclic. [Services and generated methods](https://meshlang.dev/docs/concurrency/#services-genserver)

## Jobs, timers, and bounded channels

- `Job.async(fn)` runs a zero-argument function in an actor and returns a job PID. The starting actor reads the result once through `Job.await(job)` or `Job.await_timeout(job, milliseconds)`. Both return `Result<T, String>`. A panic becomes `Err`. A timeout leaves the job running. `Job.map(values, fn)` returns one result per input in input order. The runtime bounds uncollected replies. [In-process jobs and reply limits](https://meshlang.dev/docs/concurrency/#jobs)
- `Jobs` persists queued work in PostgreSQL. It supports scheduled and recurring work, retries, and leased attempts with at-least-once delivery. See [database access](database.md#durable-jobs) for transaction boundaries. [Durable jobs](https://meshlang.dev/docs/concurrency/#durable-jobs)
- `Timer.sleep(ms)` suspends the caller cooperatively. `Timer.send_after(pid, ms, message)` sends a typed actor message. To invoke a service cast later, use `Timer.apply_after(ms, fn)` with a zero-argument function that calls the generated cast method. Timers use monotonic deadlines. [Timers](https://meshlang.dev/docs/concurrency/#timers)
- `Channel.bounded(items, policy)` or `Channel.bounded_bytes(items, bytes, policy)` returns a typed in-process queue in a `Result`. Policies are `:reject_newest`, `:drop_oldest`, and `:latest_only`. `Channel.try_send` returns immediately. `Channel.recv(channel, timeout_nanos)` yields while waiting and polls with `0`. `Duration.millis` and `Duration.seconds` construct the timeout. Inspect occupancy and drops with `Channel.depth`, `Channel.byte_depth`, and `Channel.dropped`. [Bounded channels](https://meshlang.dev/docs/concurrency/#bounded-channels)

See [supervisors](supervisors.md) for child restarts and [clustering](clustering.md#distributed-actors-and-names) for remote messaging.
