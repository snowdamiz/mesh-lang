# PostgreSQL waits and actor cancellation

PostgreSQL queries, pool checkout and connection handshakes suspend a Mesh
actor while waiting. They do not occupy its scheduler worker. This applies to
plain TCP and to the TCP transport beneath rustls; TLS authentication and
certificate checks retain their existing behavior. Native callers outside a
Mesh coroutine block until the same operation completes.

A single Mio readiness reactor serves socket waits. Its queue and combined
pending registrations are bounded to 8192. It waits for readiness or the next
monotonic deadline, rather than polling sockets on a periodic timer. Read and
write operations retry spurious readiness and Interrupted results; their
partial byte positions are preserved by Read::read_exact and Write::write_all.
The socket mutex is held only for nonblocking I/O or registration, never while
suspending an actor. Mio performs the actual I/O as well as registration so its
Windows readiness bookkeeping is updated.

System DNS and TCP establishment run on four bounded connector threads, with
at most 64 queued requests. Each TCP address has at most ten seconds, within a
30-second overall connection-establishment deadline. Cancelled or expired
queued requests are skipped. An OS DNS resolver already in progress cannot be
interrupted, but it owns only copied host data and a reply channel; it cannot
retain a pool lock, actor stack reference or pool slot. Queue exhaustion
returns an error. Startup authentication and health-check reads retain their
30-second per-read limit. Ordinary SQL reads remain unbounded, as before;
configure PostgreSQL statement_timeout when a query needs a server-side limit.
Writes retain their ten-second per-write limit.

Pool capacity checks and waiter registration occur under the pool mutex.
Actor waiters release it before suspending; native waiters use the condition
variable. Returning, closing or discarding a connection wakes waiters after
releasing pool locks. RAII guards cover a reserved opening slot, a health
check, a scoped query/transaction lease and return-time rollback. Cancellation
discards the socket and releases its counted slot. Destructors do not send
PostgreSQL messages or suspend. Server-side connection loss rolls back any
open transaction. A partially read/written connection is marked unusable
before a wait and never re-enters the idle pool on cancellation.

Transaction callbacks retain rollback-on-error and rollback-on-user-panic
behavior. Actor stop and coroutine forced-unwind payloads propagate through
the transaction boundary. They must not be converted into callback errors or
start another yielding rollback during stack destruction. Runtime functions
that may suspend use the C-unwind ABI; their machine calling convention and
public Mesh signatures are unchanged.

Mesh query parameters needed after typed preparation are copied to bounded
Rust-owned storage before I/O. Intermediate rows also remain Rust-owned until
ReadyForQuery. Mesh maps and DbValues are allocated only after all network
waits finish, since GC handles inside a Rust Vec are not actor-heap roots.
Existing row, value and aggregate-result limits still apply.

Pool/Repo scoped calls own their cancellation cleanup. The low-level Pg API
retains its existing explicit Pg.close lifetime contract. Do not share a raw
Pg connection concurrently between actors.

Focused tests cover fragmented replies under GC pressure, partial reads,
write backpressure, read deadlines, actor-to-actor readiness, exhausted pool
wakeups and cancellation at opening, health checking, waiting, callback and
rollback boundaries. The PostgreSQL-backed TLS test checks that a second actor
runs while an authenticated TLS query waits in pg_sleep.

The readiness contract follows the [Mio Poll portability documentation](https://docs.rs/mio/1.1.1/mio/struct.Poll.html).
The TLS transport uses [rustls StreamOwned](https://docs.rs/rustls/0.23.46/rustls/struct.StreamOwned.html).
