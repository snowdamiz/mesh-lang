# Mesh supervisors

A `supervisor` declares a static set of linked children and the policy for restarting them. `spawn(SupervisorName)` starts the supervisor. Child IDs must be unique within it. [Supervision](https://meshlang.dev/docs/concurrency/#supervision)

```mesh
actor worker(name :: String) do
	println("started #{name}")
	receive do
		_ -> println("finished #{name}")
	end
end

supervisor WorkerSup do
	strategy: rest_for_one
	max_restarts: 3
	max_seconds: 5

	child reader do
		start: fn -> spawn(worker, "reader") end
		restart: transient
		shutdown: 1_000
	end

	child writer do
		start: fn -> spawn(worker, "writer") end
		restart: temporary
		shutdown: brutal_kill
	end
end

fn main() do
	let _ = spawn(WorkerSup)
end
```

## Child specifications

| Field | Behavior |
| --- | --- |
| `start` | A zero-argument closure ending in `spawn(named_actor, args...)`. The supervisor starts the child process and runs the actor there. The closure evaluates its arguments on each start. |
| `restart` | `permanent` restarts on any exit. `transient` restarts on abnormal exits and leaves a normal or shutdown exit alone. `temporary` removes the child when that child exits. The default is `permanent`. |
| `shutdown` | A positive timeout in milliseconds for graceful shutdown before a forced kill, or `brutal_kill` to skip the grace period. The default is 5,000 ms. |

Use ordinary `spawn` inside a child specification. `spawn grouped` is rejected there. The `start` closure must return a PID, and a non-spawning body such as `fn -> 42 end` is a compile error. [Child specifications](https://meshlang.dev/docs/concurrency/#child-specifications)

Use an actor declaration as the child target. In the current runtime, a nested `spawn(ChildSup)` starts a separate supervisor and the parent tracks the short-lived child entry. The current child entry also skips registration of the actor's `terminate` callback.

## Restart strategies and limits

The exiting child's policy determines whether a restart runs. For a multi-child restart, the supervisor stops the affected running children in reverse declaration order and starts the affected specifications in declaration order. The restart can include a `temporary` sibling still in the child list.

| Strategy | Restart scope |
| --- | --- |
| `one_for_one` (default) | The exiting child |
| `one_for_all` | All remaining child specifications, including the exiting child |
| `rest_for_one` | The exiting child and all specifications declared after it |
| `simple_one_for_one` | Recognized template strategy. Its static exit handling matches `one_for_one`. Dynamic child operations exist in the Rust runtime ABI and have no Mesh source API. |

`max_restarts` and `max_seconds` count restart events across the supervisor in a sliding time window. The defaults are three events in five seconds. A restart that would exceed the budget makes the supervisor stop its children and exit normally in the current runtime. Use `Process.monitor` to observe the exit. [Restart strategies](https://meshlang.dev/docs/concurrency/#supervision-strategies) · [Restart limit configuration](https://meshlang.dev/docs/concurrency/#restart-limits)

A panic or runtime fault is an abnormal exit. Returning an `Err` value is ordinary control flow and carries no custom restart policy. Use [actor links and monitors](actors.md#lifecycle-and-process-names) for application-level failure notification. [Runtime errors and panics](https://meshlang.dev/docs/concurrency/#runtime-errors-and-panics)
