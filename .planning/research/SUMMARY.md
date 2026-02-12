# Project Research Summary

**Project:** Snow v3.0 Production Backend Features
**Domain:** Production-grade database and HTTP capabilities for compiled programming language
**Researched:** 2026-02-12
**Confidence:** HIGH (codebase analysis + official PostgreSQL/rustls documentation)

## Executive Summary

Snow v3.0 adds production backend capabilities to an existing compiled language with an actor-based runtime. The research reveals a rare advantage: all three TLS dependencies required for this milestone are already compiled into the build as transitive dependencies of `ureq 2`. Adding TLS support requires zero new crate compilations and no build time increase. The recommended approach wraps Snow's existing blocking I/O model (PostgreSQL wire protocol, HTTP server) with rustls, leveraging the M:N actor scheduler to multiplex concurrent operations without async complexity.

The recommended architecture builds on Snow's existing patterns: connection pooling as an actor-based manager (following Elixir's db_connection model), transactions as scoped block-based wrappers (automatic BEGIN/COMMIT/ROLLBACK), TLS as transparent protocol upgrades (SSLRequest for PostgreSQL, rustls-wrapped streams), and struct-to-row mapping via compiler code generation (extending the proven deriving(Json) infrastructure). All four features integrate cleanly with the existing runtime without breaking changes.

Key risks center on connection pooling's interaction with Snow's cooperative scheduler. The critical pitfall is blocking worker threads during pool checkout, causing cascading starvation in the M:N scheduler. Prevention requires message-passing pool design and generation-counted handles instead of raw pointers. TLS handshakes must run on dedicated OS threads for HTTPS (server-side) but are acceptable as blocking operations for PostgreSQL (client-side, pooled connections). Transaction safety depends on pool-level cleanup: connections must be validated on checkin/checkout with automatic ROLLBACK for in-transaction state.

## Key Findings

### Recommended Stack

The stack recommendation prioritizes zero new build dependencies. All TLS crates (rustls 0.23.36, ring 0.17.14, webpki-roots 1.0.6, rustls-pki-types 1.14.0) are already transitive dependencies of ureq 2, verified via cargo tree. Adding them as direct dependencies promotes existing crates without new compilations.

**Core technologies:**
- **rustls 0.23 (ring provider):** Pure Rust TLS 1.2/1.3 for PostgreSQL and HTTPS. Use ring instead of aws-lc-rs for simpler cross-compilation. Already compiled via ureq 2.
- **webpki-roots 1.0:** Bundled Mozilla root CAs for certificate validation. Preserves single-binary philosophy (no system cert dependency). Already compiled via ureq 2.
- **crossbeam-channel 0.5 (existing):** MPMC channels for connection pool checkout/return. Already used for actor mailboxes, reused for pool.
- **parking_lot 0.12 (existing):** Fast mutexes for pool metadata. Already used for GC locks, reused for pool state.

**Dependency changes:** Remove tiny_http (uses rustls 0.20, incompatible with 0.23). Replace with minimal hand-rolled HTTP/1.1 parser + rustls 0.23 for full TLS control.

**Zero new dependencies for:** Connection pooling (uses existing crossbeam-channel + parking_lot), transactions (pure protocol-level SQL commands), struct-to-row mapping (compiler codegen infrastructure).

### Expected Features

**Must have (table stakes):**
- **Connection pooling:** Every production backend pools database connections. Opening per-request is prohibitively expensive (10-150ms setup, 1.3MB per PG connection). Actor-based pool with checkout/checkin matches Snow's runtime model.
- **PostgreSQL TLS:** Cloud databases (AWS RDS, Supabase, Neon) require TLS. Hard blocker for production deployment. Implemented via SSLRequest upgrade on existing wire protocol.
- **HTTPS server:** Production HTTP must support TLS. Implemented by replacing tiny_http + using rustls directly (tiny_http's ssl-rustls feature uses obsolete rustls 0.20).
- **Database transactions:** Block-based `Db.transaction(conn, fn)` with automatic BEGIN/COMMIT/ROLLBACK. Essential for data consistency. Prevents manual transaction management errors.
- **Struct-to-row mapping (deriving(Row)):** Automatic database row to struct hydration. Eliminates manual Map.get + String.to_int boilerplate. Follows existing deriving(Json) pattern.

**Should have (competitive):**
- **Pool.transaction(pool, fn):** Combines checkout + transaction + checkin. Most ergonomic pattern for web applications.
- **deriving(Db, Json) on same struct:** One definition, two serializers. Parse from DB row AND serialize to JSON response.
- **Health-checked pool connections:** Pool validates connections before checkout, silently replacing stale ones.

**Defer (v2+):**
- ORM/query builder DSL, migration framework, per-query timeouts, read/write splitting, compile-time SQL validation, WebSocket/HTTP/2, distributed transactions, nested struct hydration from JOINs.

### Architecture Approach

The architecture extends Snow's four-layer integration pattern (typeck builtins -> MIR lowering -> LLVM intrinsics -> runtime implementation). New features integrate at the runtime layer with minimal compiler changes, except struct-to-row which adds MIR code generation following the deriving(Json) template.

**Major components:**
1. **db/pool.rs (NEW):** Connection pool as Rust-side data structure (not Snow actor) using crossbeam-channel for checkout/return. Opaque u64 handle with generation counting to prevent use-after-free.
2. **db/pg.rs (MODIFY):** PgStream enum (Plain | Tls) wrapping TcpStream vs rustls::StreamOwned. SSLRequest flow before StartupMessage. Transaction state tracking from ReadyForQuery status byte.
3. **http/server.rs (REWRITE):** Replace tiny_http with hand-rolled HTTP/1.1 parser + rustls for HTTPS. TLS accepts run on dedicated OS thread pool (not actor scheduler) to prevent handshake starvation.
4. **mir/lower.rs (MODIFY):** Generate from_row functions for deriving(Row), following generate_from_json_struct pattern. Auto snake_case conversion for column names.

**Data flow pattern:** Snow program calls runtime function with opaque handles -> Runtime validates handle (generation check) -> Execute operation on blocking I/O -> Yield scheduler if long operation -> Return Result to Snow.

### Critical Pitfalls

1. **Pool checkout blocking worker thread causes scheduler deadlock:** Snow's !Send coroutines are thread-pinned. Blocking wait for pool exhaustion prevents other actors on same thread from running, including actors holding connections. Prevention: pool checkout must yield cooperatively, not block. Use bounded channel with timeout.

2. **TLS handshake (100-500ms) starves actor scheduler:** HTTPS accept-loop TLS handshakes block worker threads, freezing unrelated actors. Prevention: run TLS handshakes on dedicated OS threads (outside scheduler), dispatch established connections to actors. PostgreSQL TLS acceptable (one-time per connection, pooled).

3. **Transaction left open after actor crash poisons pool connection:** Actor crashes mid-transaction, connection returned to pool in-transaction state. Next checkout inherits wrong transaction context. Prevention: pool validates ReadyForQuery status on checkin, issues ROLLBACK if status is T or E.

4. **Opaque u64 handle becomes dangling pointer after pool reclaim:** Current Box::into_raw pattern incompatible with pooling (shared ownership). Prevention: replace raw pointers with generation-counted slot IDs. Stale handles return error instead of SIGSEGV.

5. **TLS session state not GC-visible during encryption:** SnowString data passed to TLS encryption buffer, GC may collect if stack reference optimized away. Prevention: maintain copy-to-Vec<u8>-buffer pattern from existing pg.rs (already safe, ensure TLS refactor follows same pattern).

## Implications for Roadmap

Based on research, recommended 4-phase structure aligned with feature dependencies and risk mitigation:

### Phase 1: TLS for PostgreSQL
**Rationale:** Hard blocker for cloud database connectivity. Scoped change to pg.rs (SSLRequest + PgStream enum). Does not affect other features. Establishes TLS infrastructure shared with HTTPS.

**Delivers:** `postgres://host/db?sslmode=require` connections to AWS RDS, Supabase, Neon, etc.

**Addresses:** PostgreSQL TLS (table stakes feature)

**Avoids:** Pitfall 11 (SSLRequest must precede TLS handshake), Pitfall 5 (GC + TLS stream interaction)

**Stack:** rustls 0.23 with ring provider, webpki-roots, rustls-pki-types (all already compiled)

**Architecture:** PgStream enum wrapping TcpStream/StreamOwned, SSLRequest negotiation before StartupMessage

**Research flag:** Standard TLS upgrade pattern, skip research-phase.

### Phase 2: TLS for HTTPS Server
**Rationale:** Second production blocker. Requires rewriting HTTP server (tiny_http removal) to integrate rustls 0.23. Builds on Phase 1's TLS infrastructure. Must run handshakes on dedicated threads.

**Delivers:** `HTTP.serve_tls(router, 443, cert_pem, key_pem)` for production HTTPS

**Addresses:** HTTPS server (table stakes feature)

**Avoids:** Pitfall 2 (TLS handshake starvation), tiny_http rustls 0.20 version conflict

**Stack:** Replace tiny_http, use rustls directly for server-side TLS, rustls-pki-types for PEM parsing

**Architecture:** Dedicated OS thread pool for TLS accepts, dispatch to actor-per-request after handshake

**Research flag:** HTTP/1.1 parsing is well-documented, but thread architecture needs careful design. Consider targeted research-phase for thread pooling pattern.

### Phase 3: Connection Pooling + Transactions
**Rationale:** These features are deeply coupled (pool must validate transaction state). Transaction API depends on pool for realistic usage. Pooling is foundational for production workloads.

**Delivers:** `Pool.open(url, config)`, `Pool.checkout/checkin`, `Pool.transaction(pool, fn)`, `Pg.begin/commit/rollback`

**Addresses:** Connection pooling, database transactions (both table stakes)

**Avoids:** Pitfall 1 (checkout blocking), Pitfall 3 (transaction poisoning), Pitfall 4 (dangling handles), Pitfall 6 (transaction leaks)

**Stack:** crossbeam-channel (existing), parking_lot (existing), generation-counted handles

**Architecture:** Actor-compatible pool with cooperative yields, ReadyForQuery status tracking, block-based transaction wrapper

**Research flag:** Connection pool + actor scheduler interaction is novel. NEEDS research-phase for handle lifecycle and checkout/yield semantics.

### Phase 4: Struct-to-Row Mapping
**Rationale:** Quality-of-life improvement. Can be built independently. Most valuable after pool + transactions are solid. Follows proven deriving(Json) pattern.

**Delivers:** `deriving(Row)` generates from_row(Map<String,String>) -> Result<T, String>, `Pg.query_as<User>(conn, sql, params)`

**Addresses:** Struct-to-row mapping (table stakes), deriving(Db, Json) combo (differentiator)

**Avoids:** Pitfall 7 (column name mismatch), Pitfall 10 (NULL handling), Pitfall 13 (type parsing)

**Stack:** Existing deriving infrastructure, Map operations, String parsing functions

**Architecture:** MIR-level code generation in lower.rs, runtime row.rs for query_as helpers

**Research flag:** Well-established pattern (sqlx FromRow, Diesel Queryable). Skip research-phase.

### Phase Ordering Rationale

- **TLS first (Phases 1-2):** Both are production blockers. PostgreSQL TLS is simpler (client-side) and establishes shared infrastructure. HTTPS TLS is more complex (requires thread pool redesign).
- **Pool + Transactions together (Phase 3):** Transaction safety depends on pool validation. These cannot be built independently without creating dangerous half-states.
- **Struct-to-row last (Phase 4):** Independent feature, most valuable after data pipeline (pool + transactions) is stable. Can be deferred if timeline pressured.
- **Dependencies:** Phase 2 uses Phase 1's TLS setup. Phase 3 uses Phase 1's TLS for secure pooled connections. Phase 4 uses Phase 3's pool for realistic query_as usage.

### Research Flags

Phases needing deeper research during planning:
- **Phase 3 (Connection Pooling + Transactions):** Complex interaction with actor scheduler. Handle lifecycle with generation counters is novel for Snow. Needs research-phase to validate cooperative checkout pattern and transaction cleanup semantics.
- **Phase 2 (HTTPS Thread Architecture):** Dedicated thread pool for TLS accepts is standard (BEAM dirty schedulers) but new for Snow. Consider research-phase for thread coordination pattern.

Phases with standard patterns (skip research-phase):
- **Phase 1 (PostgreSQL TLS):** SSLRequest flow is well-documented in PostgreSQL protocol docs. rustls StreamOwned pattern is standard for blocking I/O.
- **Phase 4 (Struct-to-Row):** Follows existing deriving(Json) infrastructure exactly. Type conversion (String -> Int/Float/Bool) is straightforward.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All TLS deps verified present via cargo tree. Versions confirmed compatible. Zero new build complexity. |
| Features | HIGH | Feature set derived from Elixir, Go, Rust ecosystems. All are table stakes for production backends. |
| Architecture | HIGH | Extends existing Snow patterns (opaque handles, four-layer integration, deriving codegen). Direct codebase analysis confirms fit. |
| Pitfalls | HIGH | Critical pitfalls identified via scheduler.rs, heap.rs, pg.rs analysis. Pool + actor scheduler interaction validated against corosensei constraints. |

**Overall confidence:** HIGH

### Gaps to Address

- **Pool handle lifecycle with generation counters:** The generation-counted slot pattern is standard in game engines but new to Snow. Needs validation during Phase 3 planning that it integrates with GC-safe handle pattern.
- **HTTPS accept thread pool coordination:** Pattern exists (timer threads in mod.rs), but scaling to concurrent accepts needs validation. Research during Phase 2 planning.
- **Column name convention (snake_case vs camelCase):** Auto-conversion strategy needs user validation. May need to expose override mechanism in future milestone.
- **NULL handling for Optional vs non-Optional fields:** Return Result::Err on NULL for non-Optional is correct but may surprise users. Document prominently.

## Sources

### Primary (HIGH confidence)
- Snow codebase: `snow-rt/src/db/pg.rs` (PostgreSQL wire protocol, PgConn struct, opaque handles)
- Snow codebase: `snow-rt/src/actor/scheduler.rs` (M:N scheduler, !Send coroutines, worker loop)
- Snow codebase: `snow-rt/src/actor/heap.rs` (per-actor GC, conservative stack scanning)
- Snow codebase: `snow-codegen/src/mir/lower.rs` (deriving infrastructure, generate_from_json_struct)
- `cargo tree -p snow-rt` output (2026-02-12) — verified rustls 0.23.36, ring 0.17.14, webpki-roots 1.0.6 as existing transitive deps
- PostgreSQL Wire Protocol: Message Flow (https://www.postgresql.org/docs/current/protocol-flow.html) — SSLRequest, ReadyForQuery transaction status
- rustls documentation (https://docs.rs/rustls/latest/rustls/) — StreamOwned, ClientConnection, crypto providers

### Secondary (MEDIUM confidence)
- Elixir db_connection source (https://github.com/elixir-ecto/db_connection) — actor-based pool pattern
- HikariCP best practices — pool sizing heuristics (2 * num_cpus default)
- SQLAlchemy connection pooling docs — health check strategies
- Rust sqlx FromRow derive macro — struct-to-row mapping pattern
- Django atomic() transactions — block-based transaction API
- tiny_http Cargo.toml — confirms rustls 0.20 dependency in ssl-rustls feature
- rustls crypto providers docs — ring vs aws-lc-rs build requirements

### Tertiary (LOW confidence)
- Building a Connection Pool from Scratch (Medium, 2025) — pool design patterns
- Cooperative Scheduling Pitfalls (Microsoft docs) — starvation patterns

---
*Research completed: 2026-02-12*
*Ready for roadmap: yes*
