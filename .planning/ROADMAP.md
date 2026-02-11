# Roadmap: Snow

## Milestones

- [x] **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- [x] **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- [x] **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- [x] **v1.3 Traits & Protocols** - Phases 18-22 (shipped 2026-02-08)
- [x] **v1.4 Compiler Polish** - Phases 23-25 (shipped 2026-02-08)
- [x] **v1.5 Compiler Correctness** - Phases 26-29 (shipped 2026-02-09)
- [x] **v1.6 Method Dot-Syntax** - Phases 30-32 (shipped 2026-02-09)
- [x] **v1.7 Loops & Iteration** - Phases 33-36 (shipped 2026-02-09)
- [x] **v1.8 Module System** - Phases 37-42 (shipped 2026-02-09)
- [x] **v1.9 Stdlib & Ergonomics** - Phases 43-48 (shipped 2026-02-10)
- [ ] **v2.0 Database & Serialization** - Phases 49-54 (in progress)

## Phases

<details>
<summary>v1.0 MVP (Phases 1-10) - SHIPPED 2026-02-07</summary>

See milestones/v1.0-ROADMAP.md for full phase details.
55 plans across 10 phases. 52,611 lines of Rust. 213 commits.

</details>

<details>
<summary>v1.1 Language Polish (Phases 11-15) - SHIPPED 2026-02-08</summary>

See milestones/v1.1-ROADMAP.md for full phase details.
10 plans across 5 phases. 56,539 lines of Rust (+3,928). 45 commits.

</details>

<details>
<summary>v1.2 Runtime & Type Fixes (Phases 16-17) - SHIPPED 2026-02-08</summary>

See milestones/v1.2-ROADMAP.md for full phase details.
6 plans across 2 phases. 57,657 lines of Rust (+1,118). 22 commits.

</details>

<details>
<summary>v1.3 Traits & Protocols (Phases 18-22) - SHIPPED 2026-02-08</summary>

See milestones/v1.3-ROADMAP.md for full phase details.
18 plans across 5 phases. 63,189 lines of Rust (+5,532). 65 commits.

</details>

<details>
<summary>v1.4 Compiler Polish (Phases 23-25) - SHIPPED 2026-02-08</summary>

See milestones/v1.4-ROADMAP.md for full phase details.
5 plans across 3 phases. 64,548 lines of Rust (+1,359). 13 commits.

</details>

<details>
<summary>v1.5 Compiler Correctness (Phases 26-29) - SHIPPED 2026-02-09</summary>

See milestones/v1.5-ROADMAP.md for full phase details.
6 plans across 4 phases. 66,521 lines of Rust (+1,973). 29 commits.

</details>

<details>
<summary>v1.6 Method Dot-Syntax (Phases 30-32) - SHIPPED 2026-02-09</summary>

See milestones/v1.6-ROADMAP.md for full phase details.
6 plans across 3 phases. 67,546 lines of Rust (+1,025). 24 commits.

</details>

<details>
<summary>v1.7 Loops & Iteration (Phases 33-36) - SHIPPED 2026-02-09</summary>

See milestones/v1.7-ROADMAP.md for full phase details.
8 plans across 4 phases. 70,501 lines of Rust (+2,955). 34 commits.

</details>

<details>
<summary>v1.8 Module System (Phases 37-42) - SHIPPED 2026-02-09</summary>

See milestones/v1.8-ROADMAP.md for full phase details.
12 plans across 6 phases. 73,384 lines of Rust (+2,883). 52 commits.

</details>

<details>
<summary>v1.9 Stdlib & Ergonomics (Phases 43-48) - SHIPPED 2026-02-10</summary>

See milestones/v1.9-ROADMAP.md for full phase details.
13 plans across 6 phases. 76,100 lines of Rust (+2,716). 56 commits.

</details>

### v2.0 Database & Serialization (In Progress)

**Milestone Goal:** Make Snow viable for real backend applications with JSON serde, database drivers (SQLite + PostgreSQL), and HTTP routing improvements.

- [x] **Phase 49: JSON Serde -- Structs** - Struct-aware JSON encode/decode via `deriving(Json)` (shipped 2026-02-11)
- [x] **Phase 50: JSON Serde -- Sum Types & Generics** - Complete JSON coverage for all Snow types (shipped 2026-02-11)
- [x] **Phase 51: HTTP Path Parameters** - Dynamic route segments and method-specific routing (shipped 2026-02-11)
- [x] **Phase 52: HTTP Middleware** - Function pipeline for request/response processing (shipped 2026-02-11)
- [ ] **Phase 53: SQLite Driver** - Embedded database access with parameterized queries
- [ ] **Phase 54: PostgreSQL Driver** - Production database access with wire protocol and auth

## Phase Details

### Phase 49: JSON Serde -- Structs
**Goal**: Users can serialize and deserialize Snow structs to/from JSON strings with full type safety
**Depends on**: Nothing (first phase of v2.0)
**Requirements**: JSON-01, JSON-02, JSON-03, JSON-04, JSON-05, JSON-06, JSON-07, JSON-10, JSON-11
**Success Criteria** (what must be TRUE):
  1. User writes `deriving(Json)` on a struct and can call `Json.encode(value)` to get a JSON string
  2. User calls `Json.decode(json_string)` and gets back `Result<T, String>` with the original struct values
  3. Structs with nested `deriving(Json)` structs, `Option<T>` fields, `List<T>` fields, and `Map<String, V>` fields all round-trip correctly through JSON
  4. Compiler emits a clear error when `deriving(Json)` is used on a struct with a non-serializable field type
  5. Int and Float values survive JSON round-trip without type confusion (42 stays Int, 3.14 stays Float)
**Plans:** 3 plans

Plans:
- [x] 49-01-PLAN.md -- Runtime foundation: JSON INT/FLOAT tag split + 14 runtime functions with three-point registration
- [x] 49-02-PLAN.md -- Compiler pipeline: typeck deriving(Json) validation + MIR to_json/from_json generation + call dispatch wiring
- [x] 49-03-PLAN.md -- E2E test suite: 7 tests + 1 compile-fail covering all 9 requirements

### Phase 50: JSON Serde -- Sum Types & Generics
**Goal**: Users can serialize any Snow data type to JSON, including sum types and generic structs
**Depends on**: Phase 49
**Requirements**: JSON-08, JSON-09
**Success Criteria** (what must be TRUE):
  1. Sum type values encode as tagged JSON objects (`{"tag":"Variant","fields":[...]}`) and decode back to the correct variant
  2. Generic structs like `Wrapper<Int>` and `Wrapper<String>` both derive Json correctly via monomorphization
  3. Nested combinations (sum type containing a generic struct containing a list) round-trip through JSON
**Plans:** 2 plans

Plans:
- [x] 50-01-PLAN.md -- Sum type JSON codegen: runtime array_get + typeck registration + MIR to_json/from_json generation + dispatch wiring + generic struct fix
- [x] 50-02-PLAN.md -- E2E test suite: sum type encode/decode, generic struct, nested combinations, compile-fail

### Phase 51: HTTP Path Parameters
**Goal**: Users can define REST-style routes with dynamic segments and extract parameters from requests
**Depends on**: Nothing (independent of JSON phases)
**Requirements**: HTTP-01, HTTP-02, HTTP-03
**Success Criteria** (what must be TRUE):
  1. User defines a route like `/users/:id` and the router matches requests to `/users/42`
  2. User calls `Request.param(req, "id")` inside a handler and gets `Some("42")`
  3. User registers routes with `HTTP.on_get`, `HTTP.on_post`, `HTTP.on_put`, `HTTP.on_delete` and only matching HTTP methods dispatch to the handler
  4. Exact routes take priority over parameterized routes (`/users/me` matches before `/users/:id`)
**Plans:** 2 plans

Plans:
- [x] 51-01-PLAN.md -- Runtime + compiler pipeline: path param matching, method routing, request accessor, intrinsics, typeck, MIR lowering
- [x] 51-02-PLAN.md -- E2E test: Snow fixture with path params + method routing + priority, verified via real HTTP server

### Phase 52: HTTP Middleware
**Goal**: Users can wrap request handling with composable middleware functions for logging, auth, and cross-cutting concerns
**Depends on**: Phase 51
**Requirements**: HTTP-04, HTTP-05, HTTP-06
**Success Criteria** (what must be TRUE):
  1. User adds middleware via `HTTP.use(router, middleware_fn)` and it runs on every request
  2. Middleware function receives the request and a `next` function, can inspect/modify the request before calling next, and can inspect/modify the response after
  3. Multiple middleware functions execute in registration order (first added = outermost), forming a composable pipeline
**Plans:** 2 plans

Plans:
- [x] 52-01-PLAN.md -- Runtime + compiler pipeline: middleware storage, chain execution with trampoline, intrinsics, typeck, MIR lowering
- [x] 52-02-PLAN.md -- E2E test: Snow fixture with middleware passthrough, short-circuit, and 404 handling

### Phase 53: SQLite Driver
**Goal**: Users can store and retrieve data from SQLite databases with safe parameterized queries
**Depends on**: Nothing (independent of JSON/HTTP phases)
**Requirements**: SQLT-01, SQLT-02, SQLT-03, SQLT-04, SQLT-05, SQLT-06, SQLT-07
**Success Criteria** (what must be TRUE):
  1. User opens a SQLite database with `Sqlite.open("path.db")` and gets `Result<SqliteConn, String>`
  2. User executes `Sqlite.query(conn, "SELECT * FROM users WHERE age > ?", [18])` and gets `Result<List<Map<String, String>>, String>` with rows
  3. User executes `Sqlite.execute(conn, "INSERT INTO users (name) VALUES (?)", ["Alice"])` and gets `Result<Int, String>` with rows affected
  4. SQLite is bundled into the compiled binary with zero system dependencies (no `apt install libsqlite3-dev` needed)
  5. Database handles survive garbage collection (opaque u64, not GC-managed pointers)
**Plans**: TBD

Plans:
- [ ] 53-01: TBD
- [ ] 53-02: TBD

### Phase 54: PostgreSQL Driver
**Goal**: Users can connect to PostgreSQL for production database workloads with secure authentication
**Depends on**: Nothing (independent, but benefits from SQLite API patterns)
**Requirements**: PG-01, PG-02, PG-03, PG-04, PG-05, PG-06, PG-07, PG-08
**Success Criteria** (what must be TRUE):
  1. User connects to PostgreSQL with `Pg.connect("postgres://user:pass@host/db")` and gets `Result<PgConn, String>`
  2. User executes `Pg.query(conn, "SELECT * FROM users WHERE id = $1", [42])` and gets `Result<List<Map<String, String>>, String>` with rows
  3. User executes `Pg.execute(conn, "INSERT INTO users (name) VALUES ($1)", ["Alice"])` and gets `Result<Int, String>` with rows affected
  4. Connection works with SCRAM-SHA-256 authentication (production PostgreSQL, cloud providers)
  5. Connection works with MD5 authentication (local development PostgreSQL)
**Plans**: TBD

Plans:
- [ ] 54-01: TBD
- [ ] 54-02: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 49 -> 50 -> 51 -> 52 -> 53 -> 54

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1-10 | v1.0 | 55/55 | Complete | 2026-02-07 |
| 11-15 | v1.1 | 10/10 | Complete | 2026-02-08 |
| 16-17 | v1.2 | 6/6 | Complete | 2026-02-08 |
| 18-22 | v1.3 | 18/18 | Complete | 2026-02-08 |
| 23-25 | v1.4 | 5/5 | Complete | 2026-02-08 |
| 26-29 | v1.5 | 6/6 | Complete | 2026-02-09 |
| 30-32 | v1.6 | 6/6 | Complete | 2026-02-09 |
| 33-36 | v1.7 | 8/8 | Complete | 2026-02-09 |
| 37-42 | v1.8 | 12/12 | Complete | 2026-02-09 |
| 43-48 | v1.9 | 13/13 | Complete | 2026-02-10 |
| 49. JSON Serde -- Structs | v2.0 | 3/3 | Complete | 2026-02-11 |
| 50. JSON Serde -- Sum Types & Generics | v2.0 | 2/2 | Complete | 2026-02-11 |
| 51. HTTP Path Parameters | v2.0 | 2/2 | Complete | 2026-02-11 |
| 52. HTTP Middleware | v2.0 | 2/2 | Complete | 2026-02-11 |
| 53. SQLite Driver | v2.0 | 0/TBD | Not started | - |
| 54. PostgreSQL Driver | v2.0 | 0/TBD | Not started | - |

**Total: 52 phases shipped across 10 milestones. 150 plans completed. 2 phases remaining in v2.0.**
