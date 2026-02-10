# Project Research Summary

**Project:** Snow v2.0 Database & Serialization
**Domain:** Compiler infrastructure for database drivers, JSON serde, and HTTP enhancements
**Researched:** 2026-02-10
**Confidence:** HIGH

## Executive Summary

This milestone adds database access (SQLite, PostgreSQL), JSON serialization (via `deriving(Json)`), and HTTP enhancements (path parameters, middleware) to the Snow compiled language. The recommended approach follows established patterns: compile-time code generation for JSON serde (following the existing `deriving(Eq/Hash/Debug)` infrastructure), C FFI for SQLite with bundled amalgamation, and pure Rust implementation of PostgreSQL wire protocol v3. HTTP enhancements are runtime-only changes requiring no compiler modifications.

The critical architectural decision is blocking I/O handling: SQLite/PostgreSQL operations block OS threads in Snow's M:N actor scheduler. Following the existing HTTP server pattern (server.rs: "Blocking I/O accepted, similar to BEAM NIFs"), database operations will use dedicated worker threads or accept blocking with explicit documentation. SQLite should use `libsqlite3-sys` with `bundled` feature (compiles from source, zero system dependencies). PostgreSQL should be pure Rust TCP to avoid async runtime conflicts with corosensei coroutines.

The main risks are GC safety (conservative GC + C FFI requires careful pointer discipline), SQL injection (parameterized queries must be the primary API), and JSON number representation (Int/Float distinction requires splitting the existing JSON_NUMBER tag). All three are addressable through established patterns documented in PITFALLS.md.

## Key Findings

### Recommended Stack

The work requires 3-5 new Rust crate dependencies in `snow-rt` (SQLite C bindings, MD5/SHA-256/HMAC/PBKDF2 for PostgreSQL auth) and zero changes to compiler toolchain dependencies (Inkwell, LLVM). The division is clean: compiler-side work is 100% compile-time code generation following existing deriving patterns; runtime-side work is new extern "C" functions in `snow-rt`.

**Core technologies:**
- `libsqlite3-sys` 0.36 with `bundled` feature: Compiles SQLite 3.51.1 from source, zero system dependencies, cross-platform, used by rusqlite (15K+ stars)
- PostgreSQL wire protocol v3 (pure Rust): No libpq dependency, full control, avoids Tokio runtime conflicts with corosensei M:N scheduler
- `md5` 0.8 + `sha2` 0.10 + `hmac` 0.12 + `pbkdf2` 0.12: PostgreSQL authentication (SCRAM-SHA-256 is default since PG 10, mandatory on cloud providers)
- Existing deriving infrastructure: JSON serde follows identical pattern to existing `deriving(Debug)`, `deriving(Eq)`, `deriving(Hash)` in mir/lower.rs

**Critical version requirements:**
- None. All new dependencies are stable. SQLite bundled version is 3.51.1. PostgreSQL wire protocol is stable since 2003 (v3).

### Expected Features

**Must have (table stakes):**
- `deriving(Json)` for structs: Automatic JSON encode/decode from struct metadata, universal across Go/Rust/Elixir
- SQLite driver with parameterized queries: `Sqlite.open`, `Sqlite.query`, `Sqlite.execute`, `Sqlite.close` — matches Go database/sql, Rust rusqlite patterns
- PostgreSQL driver with parameterized queries: Same API surface as SQLite (connect/execute/query/close) using PG native `$1, $2` syntax
- HTTP path parameters: `/users/:id` captures `id` param, universal across Express, Phoenix, Go 1.22, Axum
- HTTP middleware: `fn(Request, Next) -> Response` pattern, enables logging, auth, CORS — universal across all web frameworks

**Should have (competitive):**
- Consistent database API: Both drivers return `Result<List<Map<String, String>>, String>` for queries — switching databases is straightforward
- JSON pretty-print: `JSON.encode_pretty(value)` for debugging
- Typed parameter binding: Runtime inspects Snow value type tag and calls `sqlite3_bind_int64` for Int, `sqlite3_bind_text` for String — more ergonomic than all-text

**Defer (v2+):**
- ORM/query builder: Massive scope, controversial in Go/Rust communities, Snow's type system not ready
- Connection pooling: Single connection is fine for v2.0, pool is a future enhancement
- Struct-to-row mapping: Return `List<Map<String, String>>`, users extract fields manually
- SCRAM-SHA-256 (initially): MD5 auth covers 90% of local/dev use; SCRAM adds crypto complexity but is needed for production

### Architecture Approach

The architecture follows Snow's established patterns: compiler generates MIR functions that call runtime intrinsics, runtime exposes `extern "C"` functions, no runtime reflection. JSON serde is 100% compile-time — the compiler emits LLVM IR that calls `snow_json_from_*` field-by-field when `deriving(Json)` is present. Database drivers are runtime-only modules with no compiler changes beyond typeck (adding module types) and intrinsics (declaring extern functions).

**Major components:**
1. **MIR lowering (mir/lower.rs)**: Generate `ToJson__to_json__Type` and `FromJson__from_json__Type` functions from struct field metadata — follows existing `generate_hash_struct` pattern at lines 2658-2713
2. **Runtime JSON (json.rs)**: Add structured JSON construction/extraction functions (`snow_json_object_new/put/get`, `snow_json_array_new/push`) — existing `snow_json_from_*` functions provide primitives
3. **Runtime SQLite (db/sqlite.rs)**: Wrap SQLite C API via libsqlite3-sys — opaque handle pattern matching router.rs line 59 (`Box::into_raw`)
4. **Runtime PostgreSQL (db/postgres.rs)**: Pure Rust TCP implementation of Parse/Bind/Execute/Sync extended query protocol — 500-800 lines total based on rust-postgres `postgres_protocol` subcrate
5. **Runtime HTTP (router.rs, server.rs)**: Extend path matching with `:param` segment extraction, add middleware chain execution before handler dispatch

### Critical Pitfalls

1. **Conservative GC + SQLite pointers**: Snow's GC (heap.rs) scans coroutine stacks and GC-managed objects. SQLite pointers (`sqlite3*`, `sqlite3_stmt*`) point to C heap. If a GC-managed SnowString is passed to SQLite and the only reference is held by SQLite (not on Snow stack), the GC will collect it and SQLite reads freed memory. **Mitigation**: Store sqlite3 pointers as opaque u64 (not GC-allocated), always use `SQLITE_TRANSIENT` for text binds so SQLite copies immediately, copy SQL strings to C heap before FFI calls.

2. **Blocking I/O starves M:N scheduler**: SQLite `sqlite3_step()` and PostgreSQL TCP I/O are blocking C/syscalls. When an actor blocks, the entire OS worker thread blocks. Since corosensei coroutines are `!Send` and thread-pinned (scheduler.rs line 2), all actors on that worker are starved. With N workers and N concurrent queries, the entire scheduler deadlocks. **Mitigation**: Follow existing HTTP server pattern (server.rs: "Blocking I/O accepted"), dedicate worker threads for database operations, or implement connection pool as a single actor (gen_server pattern).

3. **SQL injection through string interpolation**: Snow has string concatenation. Without parameterized queries as the primary API, users will construct SQL via `"SELECT * WHERE name = '" ++ input ++ "'"` leading to SQL injection. **Mitigation**: Make parameterized queries the primary API (`Db.query(conn, "SELECT * WHERE name = ?", [name])`), use `sqlite3_prepare_v2` + `sqlite3_bind_*` for all queries with params, PostgreSQL extended protocol (Parse/Bind/Execute) naturally separates query from params, document prominently that string concatenation is insecure.

4. **deriving(Json) field order and nested types**: Struct field iteration order must be stable (definition order, not HashMap order). Nested types require compile-time trait resolution: if `User { addr: Address }` derives Json, Address must also have Json impl or compile error. **Mitigation**: Use `Vec<FieldInfo>` (already in StructDefInfo), maintain definition order through AST -> typeck -> MIR -> codegen, add compile-time check that all field types have ToJson/FromJson implementations.

5. **PostgreSQL wire protocol state machine**: Server can send asynchronous messages (NoticeResponse, ParameterStatus) at ANY time, even mid-query. Naive sequential reading (expect RowDescription -> DataRow -> CommandComplete) hangs on unexpected NoticeResponse. **Mitigation**: Implement as state machine with message dispatch loop, every state handles async messages, buffer extended protocol messages until Sync, test with pgbouncer/connection poolers.

## Implications for Roadmap

Based on research, suggested phase structure follows dependency chains and risk management:

### Phase 1: JSON Serde Runtime Helpers + deriving(Json) for Structs
**Rationale:** Self-contained, touches all compiler layers but uses established patterns (existing deriving for Eq/Debug/Hash), no external dependencies, prerequisite for database result handling (typed parameters use JSON-like value representation)
**Delivers:** `deriving(Json)` on structs with primitives, nested structs, Option, List fields — full struct-aware JSON roundtrip
**Addresses:** Table stakes feature from FEATURES.md (automatic JSON serialization is universal)
**Avoids:** Pitfall 4 (field order) through definition-order iteration, Pitfall 9 (number precision) through separate INT/FLOAT tags
**Stack:** Existing deriving infrastructure, existing SnowJson runtime (json.rs), new `snow_json_object_*` and `snow_json_array_*` functions

### Phase 2: deriving(Json) for Sum Types + Generics
**Rationale:** Extends Phase 1 to full language coverage, sum types use Constructor patterns already established, completes JSON serde before database work begins
**Delivers:** Sum types encode as `{"tag":"Variant","fields":[...]}`, generic structs like `Wrapper<T>` derive Json via monomorphization
**Addresses:** Differentiator from FEATURES.md (few languages handle sum type JSON elegantly)
**Stack:** MIR `ensure_monomorphized_struct_trait_fns` (lines 1656-1681 in lower.rs) for generics, sum type pattern matching for variant encoding

### Phase 3: HTTP Path Parameters
**Rationale:** Small runtime-only change, no compiler modifications, enables REST APIs, needed before middleware (middleware should see path params)
**Delivers:** `/users/:id` pattern matching, `Request.param(req, "id")` accessor, method-specific routes (`HTTP.get`, `HTTP.post`)
**Addresses:** Table stakes from FEATURES.md (path parameters universal in web frameworks)
**Avoids:** Pitfall 7 (routing ambiguity) through explicit priority: exact > parameterized > wildcard
**Stack:** Existing router.rs (segment matching), existing server.rs (request struct), new `snow_http_request_param` function

### Phase 4: HTTP Middleware
**Rationale:** Builds on closures (working) and path params (Phase 3), runtime-only using existing closure calling convention
**Delivers:** `HTTP.use(router, middleware_fn)` global middleware, `fn(Request, Next) -> Response` onion model
**Addresses:** Table stakes from FEATURES.md (middleware for logging, auth, CORS is universal)
**Avoids:** Pitfall 8 (execution order) through Next-style chaining, supports both pre-handler and post-handler logic
**Stack:** Existing closure system (fn_ptr + env_ptr), router middleware storage, server chain execution

### Phase 5: SQLite Driver
**Rationale:** C FFI well-understood, single-file database with no network complexity, establishes parameterized query patterns reused by PostgreSQL
**Delivers:** `Sqlite.open`, `Sqlite.query`, `Sqlite.execute`, `Sqlite.close` with `?` placeholders
**Addresses:** Table stakes from FEATURES.md (SQLite is most common embedded database)
**Avoids:** Pitfall 1 (GC + C pointers) via opaque u64 handles and SQLITE_TRANSIENT, Pitfall 2 (blocking) explicitly documented, Pitfall 3 (SQL injection) via params-first API, Pitfall 6 (statement leaks) via `sqlite3_close_v2` and terminate callbacks
**Stack:** `libsqlite3-sys` 0.36 bundled feature, new db/sqlite.rs module (~300 lines)

### Phase 6: PostgreSQL Driver
**Rationale:** Most complex — pure Rust TCP + binary wire protocol, depends on patterns established by SQLite (parameterized query API), risk contained to runtime only
**Delivers:** `Pg.connect`, `Pg.query`, `Pg.execute`, `Pg.close` with `$1, $2` placeholders, MD5 and SCRAM-SHA-256 auth
**Addresses:** Table stakes from FEATURES.md (PostgreSQL is most common production database)
**Avoids:** Pitfall 5 (state machine) through flexible message dispatch, Pitfall 2 (blocking) same as SQLite, Pitfall 14 (auth) via scram-sha-256 from day one
**Stack:** `md5` 0.8, `sha2` 0.10, `hmac` 0.12, `pbkdf2` 0.12, std::net::TcpStream, new db/postgres module (~800 lines)

### Phase Ordering Rationale

- **JSON serde before databases**: Typed parameters for database queries use the JSON value representation (SnowJson tagged union) to determine bind types (Int -> `sqlite3_bind_int64`, String -> `sqlite3_bind_text`)
- **HTTP before databases**: Path parameters and middleware are simpler, give confidence in runtime changes before tackling C FFI and wire protocols
- **SQLite before PostgreSQL**: SQLite is simpler (no network, no auth), establishes the database API pattern (open/query/execute/close, parameterized queries, Result-based errors), PostgreSQL follows the same user-facing API
- **Phases 1-4 are compiler/HTTP foundation, Phases 5-6 are database drivers**: Clean separation allows parallel work (compiler team on JSON serde, runtime team on HTTP) before converging on databases

### Research Flags

Phases likely needing deeper research during planning:
- **Phase 6 (PostgreSQL)**: SCRAM-SHA-256 crypto implementation details, wire protocol edge cases (async messages, error recovery), testing with connection poolers — ARCHITECTURE.md covers basics but production hardening needs validation

Phases with standard patterns (skip research-phase):
- **Phase 1-2 (JSON serde)**: Deriving infrastructure proven (5 traits), MIR lowering pattern established (lines 1574-1688 lower.rs), high confidence
- **Phase 3-4 (HTTP)**: Router/middleware patterns universal (Express, Axum, Plug), closure calling convention established, straightforward implementation
- **Phase 5 (SQLite)**: C FFI well-documented, `libsqlite3-sys` + rusqlite are reference implementations (15K+ stars), bundled feature solves cross-platform linking

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All technologies verified: libsqlite3-sys bundled feature is standard (used by rusqlite), PostgreSQL wire protocol v3 is stable and well-documented, MD5/SHA-256 crates are RustCrypto (battle-tested), deriving infrastructure exists and is proven |
| Features | HIGH | All features are table stakes in respective domains: JSON serde is universal (Rust serde, Go struct tags, Elixir Jason), SQLite/PostgreSQL are standard databases, HTTP path params/middleware are in every web framework |
| Architecture | HIGH | All patterns verified through direct codebase analysis: deriving generation pattern at lines 2658-2713, opaque handle pattern at router.rs line 59, extern "C" pattern in 512 lines of intrinsics.rs, blocking I/O pattern explicitly documented in server.rs |
| Pitfalls | HIGH | All pitfalls based on direct source analysis (GC from heap.rs, scheduler from scheduler.rs, existing JSON from json.rs) + official protocol docs (SQLite C API, PostgreSQL wire protocol v3) + OWASP security guides |

**Overall confidence:** HIGH

### Gaps to Address

**PostgreSQL SCRAM-SHA-256 complexity**: Research covers the protocol flow but crypto implementation (HMAC-SHA-256, PBKDF2, base64 encoding, nonce generation) needs careful testing. The `sha2`, `hmac`, `pbkdf2` crates provide building blocks but the SCRAM message exchange is stateful and failure-prone. Mitigation: implement MD5 auth first (simple), validate with local PostgreSQL instances, add SCRAM in a sub-phase with thorough integration tests against real PG 14+ servers.

**Database connection lifecycle in actor model**: Research identifies blocking I/O as a risk but the solution (dedicated worker threads vs connection pool actor vs accept blocking) is a design choice requiring benchmarking. Mitigation: start with explicit blocking (document that database operations block the actor), measure performance under load, add connection pooling in Phase 7 if needed.

**JSON number type ambiguity**: Current SnowJson uses one NUMBER tag for both Int and Float. Splitting to separate INT/FLOAT tags is necessary for round-trip fidelity but affects existing opaque JSON code. Mitigation: the split is backward-compatible at the runtime level (existing JSON parser can produce both tags), update `snow_json_parse` in Phase 1 to distinguish types, verify existing HTTP/JSON tests still pass.

## Sources

### Primary (HIGH confidence)
- Snow codebase direct analysis:
  - `crates/snow-codegen/src/mir/lower.rs` lines 1574-1688 (deriving infrastructure), lines 2658-2713 (hash generation pattern), lines 7583-7787 (module registration)
  - `crates/snow-rt/src/json.rs` lines 82-91 (SnowJson representation), lines 288-292 (snow_json_from_float)
  - `crates/snow-rt/src/actor/heap.rs` lines 390-448 (conservative GC scanning), lines 460-490 (find_object_containing)
  - `crates/snow-rt/src/actor/scheduler.rs` line 2 (coroutine threading model), line 607 (process exit)
  - `crates/snow-rt/src/http/router.rs` lines 42-59 (exact/wildcard matching, opaque handle)
  - `crates/snow-rt/src/http/server.rs` lines 197-313 (actor-per-connection, blocking I/O comment)
- PostgreSQL Wire Protocol v3: https://www.postgresql.org/docs/current/protocol.html (message formats, startup flow, extended query)
- PostgreSQL Authentication: https://www.postgresql.org/docs/current/sasl-authentication.html (SCRAM-SHA-256), https://www.postgresql.org/docs/current/auth-password.html (MD5 method)
- SQLite C/C++ Interface: https://sqlite.org/cintro.html (API functions), https://sqlite.org/threadsafe.html (threading modes)
- libsqlite3-sys crate: https://crates.io/crates/libsqlite3-sys (bundled feature), https://docs.rs/libsqlite3-sys (FFI signatures)

### Secondary (MEDIUM confidence)
- rust-postgres: https://github.com/sfackler/rust-postgres (wire protocol reference implementation, 2K+ stars)
- rusqlite: https://docs.rs/rusqlite/ (SQLite wrapper patterns, 15K+ stars)
- RustCrypto hashes: https://github.com/RustCrypto/hashes (sha2, hmac, pbkdf2 ecosystem)
- OWASP SQL Injection Prevention: https://cheatsheetseries.owasp.org/cheatsheets/SQL_Injection_Prevention_Cheat_Sheet.html (parameterized query best practices)
- Go 1.22 routing: https://go.dev/blog/routing-enhancements (path parameter patterns)
- Axum Path extractor: https://docs.rs/axum/latest/axum/extract/struct.Path.html (type-safe path parameters)

### Tertiary (LOW confidence)
- PgDog wire protocol blog: https://pgdog.dev/blog/hacking-postgres-wire-protocol (practical implementation experience)
- Threading Models in Coroutines and SQLite: https://medium.com/androiddevelopers/threading-models-in-coroutines-and-android-sqlite-api (concurrency patterns)

---
*Research completed: 2026-02-10*
*Ready for roadmap: yes*
