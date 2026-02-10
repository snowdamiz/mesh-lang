# Domain Pitfalls: Database, Serialization & HTTP Enhancements

**Domain:** Adding JSON serde (deriving(Json)), SQLite driver (C FFI), PostgreSQL driver (pure wire protocol), parameterized queries, HTTP path parameters, and HTTP middleware to an actor-based compiled language with per-actor GC
**Researched:** 2026-02-10
**Confidence:** HIGH (based on direct Snow codebase analysis of gc.rs, heap.rs, scheduler.rs, json.rs, server.rs, router.rs, codegen/types.rs, mir/types.rs; SQLite official C/C++ interface docs; PostgreSQL wire protocol v3.2 documentation; OWASP SQL injection prevention guides; conservative GC literature; cooperative scheduling research)

**Scope:** This document covers pitfalls specific to adding 6 feature areas to the Snow compiler and runtime for the next milestone. Each pitfall is analyzed against Snow's existing architecture: corosensei coroutines on M:N scheduler, per-actor mark-sweep GC with conservative stack scanning, uniform u64 value representation, extern "C" runtime ABI, LLVM codegen with no runtime reflection, and the existing opaque JSON/HTTP subsystems.

---

## Critical Pitfalls

Mistakes that cause crashes, security vulnerabilities, data corruption, or require architectural rewrites.

---

### Pitfall 1: Conservative GC Follows sqlite3* Pointers Into C Heap, Corrupting SQLite State

**What goes wrong:**
Snow's conservative GC (heap.rs lines 390-448) scans every 8-byte-aligned word on the actor's coroutine stack and in GC-managed object bodies, treating any value that falls within a page range as a potential pointer. SQLite C FFI returns opaque pointers (`sqlite3*`, `sqlite3_stmt*`) that the Snow program stores in variables. These pointers point into C-heap memory (malloc'd by SQLite), NOT into the actor's GC-managed pages.

The GC's `find_object_containing()` (heap.rs lines 460-490) only checks against the actor's own pages, so SQLite pointers will NOT be mistakenly traced -- they fall outside page ranges. This is safe. However, the real danger is the inverse: if a GC-managed Snow value (e.g., a SnowString containing a SQL query) is passed to SQLite via FFI, and the only remaining reference to that string is held inside SQLite's internal structures (not on the Snow stack or in a GC-managed object), the GC will collect it. SQLite then reads freed memory.

Concretely: you call `sqlite3_prepare_v2(db, sql_string_ptr, ...)`. The Snow GC may run between prepare and step. If `sql_string_ptr` was a temporary SnowString that's no longer on the stack, the GC frees it. SQLite's prepared statement holds a dangling pointer to the SQL text.

**Why it happens:**
Snow's GC is conservative and scans only the coroutine stack and GC heap objects. It has no knowledge of references held by C libraries. The `find_object_containing` check in heap.rs only scans the actor's own page list, so it correctly ignores C-heap pointers -- but it also means C-held references to GC objects are invisible roots.

**Consequences:**
- Use-after-free crashes deep in SQLite (SIGSEGV in `sqlite3_step`)
- Intermittent failures that only occur under GC pressure (hard to reproduce)
- Data corruption if SQLite reads partially-overwritten GC-freed memory

**Prevention:**
1. **Copy SQL strings to C heap before FFI calls.** When calling `sqlite3_prepare_v2`, copy the SnowString bytes into a Rust `Vec<u8>` or `CString` (system-heap allocated), pass that to SQLite, then free the C copy after SQLite no longer needs it (after `sqlite3_finalize`). This is the pattern used by Erlang NIFs.
2. **Pin GC objects during FFI calls.** Add a root-pinning mechanism: before a blocking C call, register the Snow value as an explicit GC root in a side table on the Process struct. The GC mark phase checks this table in addition to the stack. Unpin after the C call returns.
3. **Never store SnowString pointers in SQLite bind parameters.** For `sqlite3_bind_text`, always pass `SQLITE_TRANSIENT` so SQLite makes its own copy of the string data immediately.
4. **Wrap sqlite3* and sqlite3_stmt* as opaque u64 values.** Store them as immediate integers (cast pointer-to-u64), not as GC-allocated objects. Since they don't fall within GC page ranges, the GC ignores them entirely. This is the simplest approach and matches how Snow already handles Router (Box::into_raw as *mut u8 in router.rs line 59).

**Detection:**
- ASAN/MSAN builds catching use-after-free in SQLite calls
- Failures only under high allocation pressure (allocate many objects between prepare and step)
- Intermittent SIGSEGV with stack traces inside `sqlite3_step` or `sqlite3_column_text`

**Phase:** SQLite driver implementation. Must be addressed in the foundational design, not retrofitted.

---

### Pitfall 2: Blocking SQLite/PostgreSQL Calls Starve the Actor Scheduler

**What goes wrong:**
Snow uses cooperative scheduling with corosensei coroutines on an M:N thread pool (scheduler.rs). Actors yield at explicit yield points: `snow_reduction_check()` calls inserted by the compiler, and `snow_actor_receive()` which sets the process to Waiting state. SQLite's `sqlite3_step()` and `sqlite3_exec()` are blocking C calls that can take milliseconds to seconds. PostgreSQL network I/O (connect, send query, read response) blocks on TCP sockets.

When an actor calls `sqlite3_step()`, the entire OS worker thread blocks. Since coroutines are `!Send` and thread-pinned (scheduler.rs line 2: "yielded coroutines stay in the worker's local suspended list"), ALL other actors suspended on that same worker thread are starved. With the default thread pool size (one per CPU core), a handful of concurrent database queries can block the entire scheduler.

The existing HTTP server already has this problem (server.rs line 15: "Blocking I/O in tiny-http is accepted (similar to BEAM NIFs)"), but database queries are typically much longer-running than HTTP request/response cycles.

**Why it happens:**
Snow's scheduler has no mechanism to detect that an actor is about to make a blocking syscall and preemptively move it or its siblings. There is no `spawn_link` to a separate OS thread for blocking work. The existing pattern (HTTP server) accepts blocking because each HTTP handler is short-lived. Database queries are not.

**Consequences:**
- A single slow SQL query (table scan, complex join) blocks an entire worker thread
- With N worker threads and N concurrent queries, the entire scheduler deadlocks
- Service actors (registered processes doing `receive` loops) become unresponsive
- No timeout mechanism to cancel stuck queries

**Prevention:**
1. **Dedicate a separate OS thread (or thread pool) for database operations.** The SQLite driver should spawn a dedicated Rust `std::thread` that owns the `sqlite3*` connection. Snow actors communicate with it via message passing (actor send/receive). The database thread runs queries synchronously and sends results back. This is exactly the BEAM NIF pattern for "dirty schedulers."
2. **For PostgreSQL, use non-blocking socket I/O.** Since the PG wire protocol is pure Snow-managed TCP (no C FFI), implement it with non-blocking sockets. Register the socket FD with the scheduler and yield the actor while waiting for socket readability/writability. Resume when data arrives. This avoids blocking entirely.
3. **SQLite WAL mode + busy timeout.** Configure SQLite with `PRAGMA journal_mode=WAL` and `sqlite3_busy_timeout()` to reduce contention. Even with a dedicated thread, concurrent writers need WAL to avoid "database is locked" errors.
4. **Limit concurrent database actors.** Use a connection pool pattern: a single actor owns the database connection and serializes queries. Other actors send query requests and receive results asynchronously. This is the "gen_server" pattern from Erlang/OTP.

**Detection:**
- Application becomes unresponsive under load (actors stop processing messages)
- `Timer.sleep` in other actors takes much longer than expected
- Worker threads show 100% CPU in blocking syscalls (strace shows futex/read)

**Phase:** Must be designed at the architecture level before implementing either driver. The blocking strategy decision affects the entire API surface.

---

### Pitfall 3: SQL Injection Through String Interpolation in Query Construction

**What goes wrong:**
Snow has string interpolation and concatenation. Without parameterized queries as the default and ONLY path, users will construct SQL like:
```
let query = "SELECT * FROM users WHERE name = '" ++ user_input ++ "'"
Db.exec(conn, query)
```
This is textbook SQL injection. Even WITH parameterized queries available, if the API also exposes raw `exec(conn, sql_string)` without parameters, users will take the path of least resistance and concatenate.

The deeper language-level issue: Snow has no way to distinguish a "SQL query string" from a regular string at the type level. Both are `String`. There's no `SqlQuery` newtype that enforces parameterization.

**Why it happens:**
- Raw query execution is simpler to implement than parameterized queries
- String concatenation is the first thing every programmer reaches for
- Without a distinct type, the compiler cannot enforce safe query construction
- SQLite's `sqlite3_exec()` (one-shot convenience function) encourages raw strings

**Consequences:**
- SQL injection vulnerabilities in every Snow application using the database
- Data exfiltration, data destruction, authentication bypass
- Snow gets a reputation as an insecure language for web development

**Prevention:**
1. **Make parameterized queries the primary API.** The user-facing API should be `Db.query(conn, "SELECT * FROM users WHERE name = ?", [name])`. The raw `Db.exec(conn, sql)` function should exist only for DDL (CREATE TABLE, etc.) where parameters are not applicable.
2. **Use sqlite3_prepare_v2 + sqlite3_bind_* for all queries with parameters.** Never pass user data through `sqlite3_exec()`.
3. **For PostgreSQL extended protocol, always use parameterized queries.** The PG wire protocol's Parse/Bind/Execute flow naturally separates query text from parameters. Simple protocol (`Query` message) should only be used for DDL or administrative commands.
4. **Consider a `Query` type at the type system level** (future enhancement). A `Query` type that can only be constructed from string literals (not runtime strings) would provide compile-time SQL injection prevention. This is aspirational but worth noting in the design.
5. **Document prominently** that string concatenation for SQL is a security vulnerability. Make the safe path the easy path.

**Detection:**
- Code review for string concatenation with SQL keywords
- SQL injection in applications built with Snow
- No automated detection possible without a Query type -- this is a design decision, not a bug

**Phase:** Parameterized query API design. Must be decided before exposing any database API to users.

---

### Pitfall 4: deriving(Json) Codegen Emits Wrong Field Order or Misses Nested Types

**What goes wrong:**
Snow's existing JSON is opaque (json.rs: `SnowJson { tag: u8, value: u64 }` with tags for Null/Bool/Number/Str/Array/Object). The new `deriving(Json)` must generate `to_json()` and `from_json()` methods that convert between typed Snow structs and this SnowJson representation. The codegen runs at MIR/LLVM level where struct field metadata is available.

The pitfall: struct field iteration order in the compiler must match the runtime's SnowMap key order. Snow structs are defined with named fields, and `to_json()` must emit a JSON object where keys match field names and values are recursively serialized. If the codegen iterates fields in a different order than the parser/typechecker stored them (e.g., HashMap iteration order vs. definition order), the JSON output has scrambled keys.

For `from_json()`, the inverse is worse: the codegen must look up each struct field by name in the JSON object. If a field is missing, it must produce a type error (Result::Err), not silently default to zero. If a JSON field has the wrong type (expecting Int but got String), the generated code must handle this at runtime since JSON is dynamically typed.

Nested types compound the problem: `deriving(Json)` on a struct `User { name: String, address: Address }` requires that `Address` also derives `Json`. If it doesn't, the codegen must either emit a compile error or fall back to opaque JSON representation. The compiler needs to track which types have `Json` implementations.

**Why it happens:**
- Struct field order is an implementation detail that varies between HashMaps and Vecs in different compiler phases
- The typechecker uses `StructDefInfo` with `fields: Vec<...>` which preserves definition order, but downstream code might re-sort or lose ordering
- Recursive type traversal for nested `deriving(Json)` requires the trait resolution system to verify Json impl existence at compile time
- Snow's HM type inference means the concrete type might not be known until after inference -- `deriving(Json)` on generic types requires monomorphization

**Consequences:**
- JSON output has wrong field names or wrong field order (breaks API contracts)
- `from_json()` silently produces zero-valued fields for missing data (logic bugs)
- Nested structs without `deriving(Json)` cause runtime crashes instead of compile errors
- Generic structs like `Wrapper<T>` fail to derive Json because T's Json impl isn't resolved

**Prevention:**
1. **Use definition order (Vec, not HashMap) for struct fields throughout the pipeline.** Verify that AST, typechecker StructDefInfo, MIR StructDef, and codegen all preserve insertion order. Snow's `StructDefInfo` already uses `Vec<FieldInfo>`, so this should be maintained through MIR lowering.
2. **Generate from_json() with explicit field-by-field lookup.** For each field, generate: (a) look up key in JSON object, (b) if missing, return Err("missing field: name"), (c) if wrong type, return Err("expected Int for field: name, got String"), (d) recursively deserialize nested types.
3. **Require explicit deriving(Json) on all nested types.** At compile time (during trait resolution), check that every field type of a `deriving(Json)` struct has a ToJson/FromJson implementation. Emit a compile error otherwise: "type Address does not implement Json, required by deriving(Json) on User."
4. **Handle primitive types built-in.** Int, Float, Bool, String, List<T>, Map<String, T>, and Option<T> should have built-in Json implementations. Only user-defined structs/enums need explicit deriving.
5. **Add round-trip tests.** For every struct with deriving(Json), test that `from_json(to_json(value)) == value`. Field order, nesting, and missing-field errors must all be covered.

**Detection:**
- JSON output with wrong field names in integration tests
- `from_json()` returning unexpected Ok values for malformed input
- Compile errors about missing trait impls when using nested types

**Phase:** deriving(Json) codegen implementation. The trait resolution check is the critical gate.

---

### Pitfall 5: PostgreSQL Wire Protocol State Machine Mishandles Async Messages

**What goes wrong:**
The PostgreSQL wire protocol is stateful. The server can send asynchronous messages (NoticeResponse, ParameterStatus, NotificationResponse) at ANY time, even in the middle of processing query results. A naive implementation that expects a fixed message sequence (e.g., "after sending Query, expect RowDescription then DataRow then CommandComplete then ReadyForQuery") will crash or hang when it receives an unexpected NoticeResponse between DataRow messages.

Additionally, the very first message (StartupMessage) has no message-type byte, unlike all subsequent messages. Many implementations get the initial handshake wrong because they apply the "read type byte, read length, read payload" pattern universally.

The ErrorResponse handling in the extended protocol is another trap: after an error, the server discards ALL subsequent messages until it receives a Sync. If the client has pipelined multiple commands, it must be prepared for the server to skip them all and respond with ReadyForQuery after the Sync.

**Why it happens:**
- The PostgreSQL protocol documentation is thorough but dense; implementers often read only the "happy path"
- Testing with `psql` (which uses simple protocol) masks extended protocol bugs
- Async messages like ParameterStatus are sent when server config changes, which is rare in development but common in production (e.g., after SET commands, or when a DBA changes settings)
- The startup message format exception is easy to miss

**Consequences:**
- Client hangs waiting for a message type that never arrives (desync)
- Connection becomes unusable after any error (failed error recovery)
- Production failures when server sends NoticeResponse or ParameterStatus at unexpected times
- Authentication failures because the startup handshake is wrong

**Prevention:**
1. **Implement the client as a state machine with a message dispatch loop.** Every state must handle NoticeResponse, ParameterStatus, and ErrorResponse in addition to expected messages. Use a `match` on message type, not a sequential read sequence.
2. **Special-case the startup sequence.** The initial exchange (StartupMessage -> AuthenticationXxx -> ParameterStatus* -> BackendKeyData -> ReadyForQuery) should be handled as a distinct phase with its own message reading logic that doesn't expect a type byte on the first response.
3. **Always check the ReadyForQuery transaction status byte.** The byte is 'I' (idle), 'T' (in transaction), or 'E' (failed transaction requiring ROLLBACK). Ignoring this leads to silent transaction state corruption.
4. **Buffer extended protocol messages until Sync.** When pipelining, buffer all outgoing messages (Parse, Bind, Describe, Execute) and only send them as a batch terminated by Sync. This makes error recovery predictable: on error, the server discards everything up to the Sync.
5. **Test with pgbouncer and connection pooling.** Connection poolers add their own ParameterStatus and NoticeResponse messages, exposing async message handling bugs.

**Detection:**
- Connection hangs after certain query patterns
- "unexpected message type" errors in logs
- Works fine with simple queries but breaks with prepared statements
- Works in development but fails in production (where async messages are more common)

**Phase:** PostgreSQL wire protocol implementation. The state machine design must be correct from the start; retrofitting is essentially a rewrite.

---

### Pitfall 6: sqlite3_stmt* Lifetime Leaks When Actor Crashes

**What goes wrong:**
SQLite requires that `sqlite3_finalize()` is called on every `sqlite3_stmt*` before `sqlite3_close()` is called on the connection. Snow actors can crash (panic in handler, supervisor kills them, linked actor dies). If an actor holding a prepared statement crashes, the `sqlite3_stmt*` is leaked -- `sqlite3_finalize()` is never called. Subsequently, `sqlite3_close()` on the connection returns `SQLITE_BUSY` because unfinalized statements remain.

Snow's actor termination path (scheduler.rs `handle_process_exit`, line 607) invokes an optional `terminate_callback` and then marks the process as Exited. The actor's heap is eventually reclaimed. But SQLite's C-allocated `sqlite3_stmt*` is on the system heap, not the GC heap -- heap reset does not free it. And even if it did, freeing the raw memory without calling `sqlite3_finalize` corrupts SQLite's internal state.

**Why it happens:**
- C resource cleanup is not automatic -- there is no RAII in the Snow runtime
- Snow actors are designed to crash and restart (Erlang "let it crash" philosophy), but C resources are not crash-safe
- The terminate_callback mechanism exists but is optional and must be explicitly set

**Consequences:**
- SQLite connections become permanently unusable after actor crashes
- Memory leaks from unfinalized prepared statements accumulate
- `sqlite3_close` fails silently or returns SQLITE_BUSY, leaving file locks held
- Database file remains locked, blocking other processes

**Prevention:**
1. **Wrap the sqlite3* connection in a "resource" abstraction that registers a terminate callback.** When an actor opens a database, the runtime should automatically register a cleanup function that calls `sqlite3_finalize` on all open statements and `sqlite3_close_v2` on the connection. `sqlite3_close_v2` is specifically designed for this: it defers closing until all statements are finalized, and it marks the connection as unusable immediately.
2. **Use `sqlite3_close_v2()` instead of `sqlite3_close()`.** The v2 variant is "zombie-safe" -- it marks the connection as unusable and defers actual resource cleanup until all prepared statements are finalized. This prevents SQLITE_BUSY errors.
3. **Track all sqlite3_stmt* per connection.** Maintain a list of active prepared statements in the connection wrapper. On cleanup (normal or crash), iterate and finalize all of them before closing the connection.
4. **Connection-per-actor model.** Each actor that uses the database should own its own connection. When the actor exits (normally or via crash), its terminate callback handles cleanup. This avoids shared-connection lifetime issues.

**Detection:**
- Database file locks not released after actor crash
- "database is locked" errors from other actors/processes after a crash
- Growing memory usage from leaked prepared statements
- `sqlite3_close` returning SQLITE_BUSY in terminate callbacks

**Phase:** SQLite driver architecture. The resource cleanup pattern must be established before any statement caching or connection pooling.

---

## Moderate Pitfalls

Mistakes that cause incorrect behavior, performance problems, or developer confusion, but are fixable without architectural changes.

---

### Pitfall 7: HTTP Path Parameter Routing Ambiguity With Existing Exact/Wildcard Matcher

**What goes wrong:**
Snow's current router (router.rs) supports exact match (`/api/health`) and wildcard (`/api/*`). Adding path parameters (`/users/:id`) introduces ambiguity: does `/users/profile` match the static route `/users/profile` or the parameterized route `/users/:id`? The current `matches_pattern` function (router.rs lines 42-49) uses a simple linear scan with first-match-wins semantics. Adding `:param` patterns to this system without a clear priority order creates unpredictable routing.

The existing wildcard semantics add further confusion: `/api/*` matches `/api/users/123` (multi-segment). Should `/api/:resource` match only `/api/users` (single segment) or also `/api/users/123` (multi-segment)?

**Why it happens:**
- The current router was designed for exact + wildcard only; path parameters were not part of the initial design
- First-match-wins ordering means route registration order determines behavior, which is fragile
- No compile-time or startup-time check for conflicting routes

**Prevention:**
1. **Define a clear priority order: exact > parameterized > wildcard.** When multiple routes match, always prefer the most specific one. This matches the behavior of Express.js, Actix-web, and most mature web frameworks.
2. **Path parameters match exactly one segment.** `/users/:id` matches `/users/123` but NOT `/users/123/posts`. Multi-segment capture requires explicit wildcard (`/api/*path`).
3. **Detect conflicting routes at registration time.** When adding a route, check if an existing route would create ambiguity. Emit a warning or error: "route /users/:id conflicts with /users/profile -- static route takes priority."
4. **Store routes in a trie/radix tree instead of a linear list.** This makes lookup O(path length) instead of O(routes), and naturally handles priority (static children checked before parameterized children).

**Detection:**
- Routes that work in isolation but break when other routes are added
- 404 errors for paths that should match a parameterized route
- Wrong handler called for a path that matches both static and parameterized routes

**Phase:** HTTP path parameter implementation. Design the router upgrade before implementing parameter extraction.

---

### Pitfall 8: HTTP Middleware Execution Order and Short-Circuit Semantics

**What goes wrong:**
Middleware chains have two phases: the "inbound" phase (before the handler) and the "outbound" phase (after the handler). A logging middleware should wrap the entire request lifecycle; an authentication middleware should short-circuit before the handler if the request is unauthorized. If middleware is modeled as a simple list of functions that run sequentially before the handler, there's no way to implement "after handler" behavior (e.g., response logging, timing, CORS header injection).

The existing `handle_request` function (server.rs lines 218-313) directly calls the handler and then writes the response. There's no hook point for post-handler processing.

**Why it happens:**
- The simplest middleware model is "list of functions that run before the handler," which misses outbound processing
- Snow's function-pointer-based handler dispatch (transmute to fn(*mut u8) -> *mut u8) doesn't naturally compose with middleware wrapping
- Without closures that capture middleware state, middleware can't maintain per-request context (e.g., request start time for logging)

**Prevention:**
1. **Model middleware as `fn(Request, Next) -> Response` where `Next` is a closure/function that calls the next middleware or handler.** This "onion" model (used by Express, Koa, Tower, Plug) naturally supports both pre-handler and post-handler logic. The middleware calls `Next(request)` to continue the chain and can modify the response afterward.
2. **For Snow's runtime, implement middleware as a chain of function pointers.** Each middleware receives the request and a "next" function pointer. It can: (a) modify the request and pass to next, (b) short-circuit by returning a response directly, or (c) call next, then modify the response before returning.
3. **Define middleware ordering at the router level, not per-route.** Global middleware (logging, CORS) applies to all routes. Route-specific middleware (auth) applies to individual routes. The router stores both lists and chains them appropriately.
4. **Handle short-circuiting explicitly.** Authentication middleware returns a 401 response without calling next. Error-handling middleware wraps next in a try/catch equivalent (Result handling in Snow).

**Detection:**
- CORS headers missing from error responses (middleware didn't run on the outbound path)
- Logging middleware doesn't log response times (no outbound hook)
- Authentication middleware runs but handler is still called (short-circuit not working)

**Phase:** HTTP middleware implementation. The middleware model choice affects the entire API.

---

### Pitfall 9: JSON Number Precision Loss Between Snow's Int/Float and JSON

**What goes wrong:**
Snow's existing JSON representation (json.rs lines 82-91) stores numbers as `u64` in the SnowJson value field. For integers, the raw i64 is stored as-is. For floats, `f64::to_bits()` stores the IEEE 754 bit pattern. The problem: there's no tag to distinguish integer-stored-as-u64 from float-stored-as-u64. Both use tag `JSON_NUMBER` (tag 2). The `snow_json_to_serde_value` function (json.rs lines 129-135) always interprets the value as i64, losing all float information.

When `deriving(Json)` generates `to_json()` for a Float field, it calls `snow_json_from_float` (json.rs line 288) which stores `f64::to_bits()`. When `from_json()` reads it back, the current code interprets it as i64, producing a garbage integer instead of the original float.

More subtly: JSON numbers have no integer/float distinction. The number `1.0` and `1` are the same in JSON. But Snow has distinct Int and Float types. `from_json()` must decide: is the JSON number `42` an Int or a Float? The answer depends on the target type, which is known at compile time (from the struct field type) but NOT available at the runtime JSON parsing level.

**Why it happens:**
- The original JSON implementation was opaque (parse JSON, access by key, get values) -- there was no need to distinguish number types because users would explicitly call get_int/get_float
- `deriving(Json)` requires automatic type-directed deserialization, which the existing representation doesn't support
- The SnowJson tagged union has one NUMBER tag, not separate INT and FLOAT tags

**Prevention:**
1. **Add separate NUMBER_INT (tag 2) and NUMBER_FLOAT (tag 6) tags.** This is a breaking change to the SnowJson representation but is necessary for round-trip fidelity. The existing tag numbering (0-5) leaves room for tag 6.
2. **In deriving(Json) from_json(), use the target type to coerce.** When deserializing into an Int field, accept both integer and float JSON numbers (truncating the float). When deserializing into a Float field, accept both (promoting the integer). This type-directed approach works because `from_json` is generated per-struct.
3. **In the runtime JSON parser (serde_value_to_snow_json), use `as_i64()` vs `as_f64()` to choose the tag.** If `serde_json::Number::as_i64()` succeeds, store as NUMBER_INT. Otherwise use `as_f64()` and store as NUMBER_FLOAT.
4. **Update `snow_json_to_serde_value` to check the new tags** and produce the correct serde_json::Number variant.

**Detection:**
- Float fields deserialize as garbage integers
- Round-trip test: `from_json(to_json({ x: 3.14 }))` produces `{ x: some_large_integer }`
- JSON encoding of floats produces integer strings

**Phase:** JSON serde implementation. The SnowJson tag split should happen before deriving(Json) codegen.

---

### Pitfall 10: PostgreSQL Connection Lifecycle in Actor-Per-Connection HTTP Model

**What goes wrong:**
Snow's HTTP server creates one actor per HTTP connection (server.rs lines 197-213). Each actor handles a single request and exits. If each HTTP handler actor opens a PostgreSQL connection, the overhead is enormous: the PostgreSQL wire protocol startup sequence involves 18+ message exchanges including authentication, parameter status exchange, and potentially TLS negotiation. At 100 requests/second, that's 100 TCP connections opened and closed per second, each with full handshake overhead.

Even with SQLite (in-process), opening the database file on every request means repeated file I/O and page cache cold starts.

**Why it happens:**
- The actor-per-connection model is natural for HTTP but terrible for database connections
- Without connection pooling, every request pays full connection setup cost
- Snow has no built-in connection pool primitive

**Prevention:**
1. **Implement a database connection pool as a long-lived service actor.** The pool actor owns N connections and distributes them to requesting actors. HTTP handler actors send a "checkout" message, receive a connection handle, execute queries, and send a "checkin" message to return the connection.
2. **For SQLite, use a single connection actor.** SQLite performs best with a single writer. A single actor owning the connection serializes all writes naturally. Read-only queries can use WAL mode with a separate read connection.
3. **For PostgreSQL, maintain a small pool of persistent connections.** Pool size should be configurable (default: 5-10). Connections are validated with a simple query before checkout (to detect stale/broken connections).
4. **The pool actor should handle connection recovery.** If a connection is broken (network error, server restart), the pool should detect it, discard the bad connection, and create a new one.

**Detection:**
- Extremely slow database operations in HTTP handlers
- "too many connections" errors from PostgreSQL
- High CPU/memory from connection churn
- Latency spikes on first request after idle (connection not cached)

**Phase:** Database driver architecture. Pool design should happen before HTTP+database integration.

---

### Pitfall 11: Type System Integration for Database Row Types

**What goes wrong:**
Snow uses Hindley-Milner type inference (snow-typeck). Database queries return rows with typed columns, but the column types are known at the database level, not at the Snow type level. A query like `Db.query(conn, "SELECT id, name FROM users", [])` returns... what type? Options include:

1. `List<Map<String, Json>>` -- fully dynamic, loses type safety
2. `List<(Int, String)>` -- correct but how does the compiler know the column types?
3. `List<User>` -- requires mapping column names to struct fields

Option 1 is what most dynamic languages do but defeats Snow's static typing. Option 2 requires the programmer to specify the return type, and the compiler trusts them (unsafe). Option 3 requires a derive mechanism similar to `deriving(Json)`.

The current type system resolves collection types as `MirType::Ptr` (mir/types.rs line 77), which is correct for runtime but means the type checker cannot verify column type mismatches at compile time.

**Why it happens:**
- SQL query results have schema-dependent types that are not known at compile time
- Snow has no dependent types or type-level strings to express "this query returns these columns"
- The gap between SQL's type system and Snow's type system is fundamental

**Prevention:**
1. **Start with explicit type annotation: `Db.query<User>(conn, sql, params)`.** The programmer specifies the expected result type. The runtime checks at runtime that column count and types match the struct fields (using the same metadata from `deriving(Json)`). This is the approach used by Diesel (Rust), sqlx (Rust, with compile-time verification), and most typed database libraries.
2. **Reuse the deriving(Json) infrastructure for row mapping.** A struct with `deriving(Json)` already has field-name-to-type metadata. Extend this to support `deriving(Row)` or reuse the same serialization infrastructure. Column names map to field names; column types are coerced to Snow types (INTEGER -> Int, TEXT -> String, REAL -> Float, NULL -> Option<T>).
3. **For the initial implementation, return `List<Map<String, String>>` (all text).** SQLite returns all values as text by default. PostgreSQL returns text in simple protocol mode. This is safe, correct, and type-system-compatible, but forces users to manually convert types. It's a good MVP that can be improved later.
4. **Do NOT try to make the compiler verify SQL queries at compile time.** This is extremely complex (requires an SQL parser, schema awareness, and type-level computation). Save it for a much later milestone if ever.

**Detection:**
- Runtime type mismatch errors when query returns unexpected column types
- Boilerplate type conversion code in every database handler
- Users requesting compile-time SQL checking (feature request, not a bug)

**Phase:** Database query result API design. The return type decision affects the entire user experience.

---

### Pitfall 12: Existing Opaque JSON to Struct-Aware JSON Migration

**What goes wrong:**
Snow currently has `Json` as an opaque type (MirType::Ptr, resolved in mir/types.rs line 77). Users write:
```
let json = Json.parse(text)?
let name = Json.get(json, "name")
```
The new `deriving(Json)` provides:
```
let user = User.from_json(text)?
let name = user.name
```
Both systems must coexist. If `deriving(Json)` replaces the opaque `Json` type entirely, existing code breaks. If both exist but with confusing overlap, users don't know which to use.

The deeper issue: `Json.parse` returns a `Json` value (opaque), while `User.from_json` returns a `Result<User, String>`. These are different types. The opaque `Json` type is still useful for dynamic JSON (unknown schema, configuration files, API responses with varying structure). The struct-aware path is for known schemas.

**Why it happens:**
- Two JSON systems serving different purposes but with overlapping names and use cases
- Users expect `from_json` to work on a `Json` value, not a `String` -- but the impl takes a String because it parses from text

**Prevention:**
1. **Keep both systems.** The opaque `Json` type (parse, get, encode) serves dynamic JSON. The `deriving(Json)` system (to_json, from_json) serves typed JSON. Make the naming clear: `Json.parse` returns `Json`, `User.from_json` returns `Result<User, String>`.
2. **Add a bridge: `User.from_json_value(json: Json) -> Result<User, String>`.** This allows parsing JSON once with `Json.parse` and then converting to a typed struct. Useful when the top-level structure is dynamic but inner values are typed.
3. **Document clearly** when to use each system: opaque for unknown/dynamic JSON, deriving for known schemas.
4. **Consider naming the derive `deriving(Serialize)` or `deriving(Encode)` instead of `deriving(Json)`.** This avoids name collision with the `Json` type. However, `deriving(Json)` is more discoverable. The milestone context says `deriving(Json)`, so use that, but be prepared for naming confusion.

**Detection:**
- User confusion about which JSON system to use
- Type errors when trying to pass opaque Json to a typed from_json
- Existing code using Json.parse breaks (should not happen if both coexist)

**Phase:** deriving(Json) design phase. The migration/coexistence strategy should be documented before implementation.

---

## Minor Pitfalls

Issues that cause developer friction or minor bugs, but have straightforward fixes.

---

### Pitfall 13: SQLite C Library Linking Across Platforms

**What goes wrong:**
Snow compiles to native binaries via LLVM and links against the Rust runtime (`-lsnow_rt`). Adding SQLite requires linking against `libsqlite3`. On macOS, SQLite is bundled in `/usr/lib/libsqlite3.dylib`. On Linux, it may or may not be installed (`apt install libsqlite3-dev`). On Windows, there's no system SQLite at all.

The alternative is to bundle SQLite source code (the "amalgamation" -- a single `sqlite3.c` file) and compile it into the runtime. This is the approach used by rusqlite's `bundled` feature. But this means the Snow runtime crate needs a `build.rs` that compiles C code, adding complexity.

**Prevention:**
1. **Bundle the SQLite amalgamation in the Snow runtime.** Compile `sqlite3.c` via a `cc::Build` in the snow-rt build script. This eliminates the system dependency entirely. The amalgamation is ~240KB of C code and compiles in seconds.
2. **Link statically.** The bundled SQLite is compiled into `libsnow_rt.a`, so Snow programs don't need SQLite installed at runtime.
3. **Set appropriate compile flags:** `SQLITE_THREADSAFE=1` (serialized mode for safety), `SQLITE_ENABLE_FTS5` (full-text search), `SQLITE_ENABLE_JSON1` (JSON functions).

**Detection:**
- "undefined symbol: sqlite3_open" at link time
- Snow programs fail to run on systems without SQLite installed
- Different SQLite versions causing behavioral differences

**Phase:** SQLite driver build system setup. Do this first, before any runtime code.

---

### Pitfall 14: PostgreSQL Password Authentication Hash Mismatch

**What goes wrong:**
PostgreSQL supports multiple authentication methods: trust, password (cleartext), md5, and scram-sha-256. Most production setups use scram-sha-256 (the default since PostgreSQL 14). A wire protocol client that only implements cleartext password authentication will fail to connect to most PostgreSQL servers.

The md5 authentication requires computing `md5(md5(password + username) + salt)` -- getting this wrong produces "password authentication failed" errors that look like wrong credentials, not wrong implementation.

scram-sha-256 is a multi-step challenge-response protocol (SASLInitialResponse, SASLContinue, SASLFinal) that requires HMAC-SHA-256 and PBKDF2. It's significantly more complex than md5.

**Prevention:**
1. **Implement scram-sha-256 from the start.** It's the default auth method and will be required for nearly all connections. Use a well-tested HMAC-SHA-256 library (ring, sha2) rather than implementing crypto from scratch.
2. **Support md5 as a fallback.** Some older PostgreSQL installations still use md5. The implementation is simple: `md5(concat(md5(concat(password, username)), salt))`.
3. **Support trust for development.** Trust authentication requires no password exchange -- the server just sends AuthenticationOk after the startup message.
4. **Test against a real PostgreSQL instance.** A local Docker container with different auth configurations exercises the entire authentication flow.

**Detection:**
- "password authentication failed for user" errors that are actually auth protocol bugs
- Connection works with `trust` but fails with `md5` or `scram-sha-256`
- Hanging during SASL exchange (wrong message sequence)

**Phase:** PostgreSQL driver authentication implementation. Must be complete before any query functionality.

---

### Pitfall 15: Path Parameter Extraction From URL Needs Percent-Decoding

**What goes wrong:**
HTTP URLs can contain percent-encoded characters: `/users/John%20Doe` should extract the parameter `id = "John Doe"`. If the router extracts path parameters by simple string splitting without percent-decoding, the parameter value will be `"John%20Doe"` (with literal percent signs).

Similarly, path segments can contain characters that look like delimiters: `/files/a%2Fb` should be a single segment with value `a/b`, not two segments `a` and `b`.

**Prevention:**
1. **Percent-decode path parameters after extraction.** Use a standard URL decoding function (or implement the trivial `%XX` -> byte mapping).
2. **Decode AFTER splitting on `/`.** The URL should be split into segments first (on literal `/`), then each segment should be percent-decoded. This ensures that `%2F` within a segment doesn't create a false segment boundary.
3. **Handle invalid percent-encoding gracefully.** `%ZZ` is not valid. Return a 400 Bad Request or pass the literal string through.

**Detection:**
- Path parameters with spaces, unicode characters, or special characters are mangled
- Routes with `/` in parameter values match wrong handlers
- Tests using only ASCII alphanumeric paths pass but real-world URLs fail

**Phase:** HTTP path parameter extraction. Simple to implement correctly, easy to get wrong if not considered.

---

### Pitfall 16: SnowString Lifetime During PostgreSQL Wire Protocol Send

**What goes wrong:**
The PostgreSQL wire protocol client needs to serialize query strings and parameter values into TCP messages. If the query string is a SnowString (GC-managed on the actor heap), and the TCP send involves a blocking syscall or an async yield point, the GC might run between constructing the message and completing the send. If the SnowString is the only reference and it's not on the stack during the GC scan, it gets collected and the TCP send reads freed memory.

This is the same class of problem as Pitfall 1 (GC + C FFI) but for network I/O instead of SQLite. The difference is that PostgreSQL uses no C FFI -- the wire protocol is pure Rust/Snow. But the data still crosses a boundary (user space -> kernel space via TCP send).

**Prevention:**
1. **Copy SnowString data into a Rust Vec<u8> (system-heap) message buffer before sending.** The wire protocol serializer should build the complete message in a system-heap buffer, copy all SnowString data into it, and then send the buffer. The SnowString can be collected after the copy without affecting the send.
2. **This is the natural implementation anyway.** Building a wire protocol message means concatenating bytes into a buffer, which inherently copies the data. Just make sure the intermediate SnowString reference isn't the only one at any yield point.

**Detection:**
- Corrupted query text in PostgreSQL server logs
- "invalid message format" errors from PostgreSQL
- Intermittent under high GC pressure; hard to reproduce

**Phase:** PostgreSQL wire protocol serializer. Follow the copy-to-buffer pattern from the start.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Severity | Mitigation |
|-------------|---------------|----------|------------|
| SQLite C FFI foundation | GC follows C pointers (Pitfall 1) | CRITICAL | Store sqlite3* as opaque u64, use SQLITE_TRANSIENT for binds |
| SQLite C FFI foundation | Blocking calls starve scheduler (Pitfall 2) | CRITICAL | Dedicated DB thread or actor, never block worker threads |
| SQLite C FFI foundation | Statement leak on crash (Pitfall 6) | CRITICAL | sqlite3_close_v2, terminate callback, per-connection stmt tracking |
| SQLite C FFI foundation | Cross-platform linking (Pitfall 13) | MODERATE | Bundle amalgamation, static link via cc::Build |
| PostgreSQL wire protocol | State machine async messages (Pitfall 5) | CRITICAL | Flexible message dispatch, handle async messages everywhere |
| PostgreSQL wire protocol | Auth hash mismatch (Pitfall 14) | MODERATE | Implement scram-sha-256 from day one |
| PostgreSQL wire protocol | SnowString lifetime in send (Pitfall 16) | MODERATE | Copy to system-heap buffer before TCP send |
| PostgreSQL wire protocol | Connection lifecycle (Pitfall 10) | MODERATE | Connection pool actor, not per-request connections |
| Parameterized queries | SQL injection (Pitfall 3) | CRITICAL | Params-first API, sqlite3_bind_*/PG extended protocol |
| Parameterized queries | Row type integration (Pitfall 11) | MODERATE | Start with Map<String,String>, add typed results later |
| deriving(Json) | Field order / nested types (Pitfall 4) | CRITICAL | Definition-order iteration, compile-time trait check for nested types |
| deriving(Json) | Number precision (Pitfall 9) | MODERATE | Separate INT/FLOAT tags in SnowJson representation |
| deriving(Json) | Migration from opaque Json (Pitfall 12) | LOW | Keep both systems, add bridge function |
| HTTP path parameters | Routing ambiguity (Pitfall 7) | MODERATE | Priority order: exact > param > wildcard; trie-based router |
| HTTP path parameters | Percent-decoding (Pitfall 15) | LOW | Decode after split on /, handle invalid encoding |
| HTTP middleware | Ordering / short-circuit (Pitfall 8) | MODERATE | Onion model: fn(Request, Next) -> Response |

---

## Sources

### Official Documentation
- [SQLite C/C++ Interface Introduction](https://sqlite.org/cintro.html) -- lifecycle rules for sqlite3/sqlite3_stmt objects
- [SQLite Quirks, Caveats, and Gotchas](https://sqlite.org/quirks.html) -- SQLite-specific behavioral surprises
- [SQLite Threading Modes](https://www.sqlite.org/threadsafe.html) -- multi-threaded SQLite configuration
- [PostgreSQL Wire Protocol v3.2 (Frontend/Backend Protocol)](https://www.postgresql.org/docs/current/protocol.html) -- complete protocol specification
- [PostgreSQL Message Flow](https://www.postgresql.org/docs/current/protocol-flow.html) -- detailed message exchange sequences
- [OWASP SQL Injection Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/SQL_Injection_Prevention_Cheat_Sheet.html) -- parameterized query best practices
- [OWASP Query Parameterization Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Query_Parameterization_Cheat_Sheet.html) -- language-specific parameterization patterns

### Domain Research
- [Haskell FFI Safety and Garbage Collection](https://frasertweedale.github.io/blog-fp/posts/2022-09-23-ffi-safety-and-gc.html) -- GC + FFI interaction analysis (HIGH confidence)
- [Boehm Conservative GC](https://www.hboehm.info/gc/conservative.html) -- why conservative GC requires pointer discipline (HIGH confidence)
- [Hacking the Postgres Wire Protocol (PgDog)](https://pgdog.dev/blog/hacking-postgres-wire-protocol) -- practical protocol implementation experience (MEDIUM confidence)
- [pgwire Rust Library](https://github.com/sunng87/pgwire) -- reference implementation of PG wire protocol in Rust (MEDIUM confidence)
- [Threading Models in Coroutines and Android SQLite API](https://medium.com/androiddevelopers/threading-models-in-coroutines-and-android-sqlite-api-6cab11f7eb90) -- SQLite + coroutine interaction (MEDIUM confidence)
- [SQLite Concurrent Writes](https://tenthousandmeters.com/blog/sqlite-concurrent-writes-and-database-is-locked-errors/) -- concurrency pitfalls analysis (MEDIUM confidence)
- [Middleware Order in ASP.NET Core](https://bytecrafted.dev/posts/aspnet-core/middleware-order-best-practices/) -- middleware ordering patterns (MEDIUM confidence)
- [Managing Path Parameters in Express.js](https://medium.com/@gilbertandanje/managing-path-parameters-in-express-js-avoiding-route-conflicts-d9f5eefe8e68) -- route conflict analysis (MEDIUM confidence)

### Codebase Analysis (PRIMARY SOURCE)
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/gc.rs` -- GC allocation entry points, conservative scanning model
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/heap.rs` -- per-actor heap, mark-sweep GC, free list, find_object_containing
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/scheduler.rs` -- M:N work-stealing scheduler, coroutine lifecycle
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/stack.rs` -- corosensei coroutine management, thread-local context
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/json.rs` -- existing opaque SnowJson representation
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/server.rs` -- actor-per-connection HTTP server
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/router.rs` -- exact + wildcard router
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/codegen/types.rs` -- MirType to LLVM type mapping
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/types.rs` -- Ty to MirType resolution, type registry
