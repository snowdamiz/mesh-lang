# Transaction-scoped batch preparation

Reserve the ordered PostgreSQL SKIP LOCKED prefix with job-row locks. Render
and validate all selected contexts over the existing claim connection while
other runners can lock and render different rows. Keep reservations inside
the original READ COMMITTED transaction and leave every job logically ready
until activation. Rollback and connection teardown release reservations
without a lease or attempt. Idle transaction timeout bounds abandoned
reservations. The existing five-second Jobs I/O timeout bounds runner waits.

Activate the prepared prefix in the same transaction. Acquire the namespace
and queue admission fences, then read current capacity. Verify the transaction,
row versions, ready state, worker registrations, and exact ordered manifest.
Write oversized markers and advance attempts only for the admitted prefix.
Return skinny confirmations and use the existing idle COMMIT gate before
launch. A lost commit acknowledgement retains stock recovery semantics.

A committed reservation table needs a lease, an expiry sweep and a shared
visibility rule for legacy claimants. An optimistic version-addressed cache
avoids row locks during transfer but can miss the true ordered prefix when
locks or values change. The transaction-scoped path uses existing PostgreSQL
rollback and SKIP LOCKED semantics for both old and new claimants. Reservation
order is the eligible order under SKIP LOCKED. Activation can complete in
a different order across runners, as with independent consumer delivery.

Run the stock and candidate on the exact same batch handler and transport.
Measure acquisition, transfer, and activation separately from setup and cold
production. Compare with measured baseline and rotate peer arms with fresh DBs.
