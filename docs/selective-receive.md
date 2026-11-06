# Selective receive

`receive matching` selects the first application message whose pattern and guard
succeed. Its arms may omit variants of the actor's message type. Ordinary
`receive` and `case` still require exhaustive arms.

```mesh
receive matching do
  Ready(value) when value > 0 -> value
  after 1000 -> 0
end
```

Messages that do not match remain available in their original arrival order.
The next ordinary receive takes the oldest retained application message. Another
matching receive starts again at the oldest retained message. Internal job and
service replies remain reserved for the operation that requested them.

Patterns and guards have the same meaning as in `case`. Each applicable guard
runs once for a candidate during one receive invocation. Waking or continuing
the scan does not repeat that guard. A later receive can run it again. Guards
may allocate, send messages, and call functions. A guard must not receive,
directly or through a function such as `Job.await` or a service call. The runtime
rejects that reentrancy with an actor panic. The selected arm body can receive.

`after 0` tests the application messages present when the receive begins and
runs the timeout body if none matches. A positive timeout sets one monotonic
deadline for the entire scan, including guards and wakeups. Guard execution is
cooperative. A guard already running at the deadline may finish and select its
candidate. Omitting `after` waits until a matching message arrives or the actor
stops.

Retained messages occupy mailbox item and byte capacity. Scanning does not free
capacity. Consuming a message does. `send` can return status `2` while unmatched
messages fill the mailbox. Matching receive never discards them to admit a new
message.

The compiler uses the case pattern decision tree and commits consumption before
the selected arm body. The runtime copies each scanned message into the actor
heap once, roots retained messages for GC, and retains the mailbox reservation.
A scan cursor prevents repeated guards and idle spinning. Guards run with no
mailbox or process lock held. Nonlocal guard exits abandon the scan, and actor
finalization clears scan state and retained reservations after panic or cancellation.

The compiled example is `examples/selective-receive/main.mpl`. It prints:

```text
ready 42
kept for later
```

Focused verification uses `cargo test -p meshc --test e2e_selective_receive` and
`cargo test -p mesh-rt --lib actor::receive::tests`. The compiled tests run with
GC stress enabled. Queue tests use barriers and injected expiry for race and
deadline checks. These checks qualify this unit locally. Integrated workspace,
proof workflow, and independent review gates belong to the parent effort.
