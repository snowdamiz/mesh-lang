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
- [x] **v2.0 Database & Serialization** - Phases 49-54 (shipped 2026-02-12)
- [ ] **v3.0 Production Backend** - Phases 55-58 (shipping)

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

<details>
<summary>v2.0 Database & Serialization (Phases 49-54) - SHIPPED 2026-02-12</summary>

See milestones/v2.0-ROADMAP.md for full phase details.
13 plans across 6 phases. 81,006 lines of Rust (+4,906). 52 commits.

</details>

### v3.0 Production Backend (In Progress)

**Milestone Goal:** Make Snow viable for production backend deployments with TLS encryption, connection pooling, database transactions, and automatic struct-to-row mapping.

- [x] **Phase 55: PostgreSQL TLS** - Encrypted connections to cloud databases via SSLRequest protocol upgrade (completed 2026-02-12)
- [x] **Phase 56: HTTPS Server** - Production HTTP serving with TLS via hand-rolled HTTP/1.1 parser replacing tiny_http (completed 2026-02-12)
- [x] **Phase 57: Connection Pooling & Transactions** - Actor-compatible pool manager with transaction lifecycle and automatic cleanup (completed 2026-02-12)
- [x] **Phase 58: Struct-to-Row Mapping** - Automatic database row to struct hydration via deriving(Row) (completed 2026-02-12)

## Phase Details

### Phase 55: PostgreSQL TLS
**Goal**: Snow programs can connect to TLS-required PostgreSQL databases (AWS RDS, Supabase, Neon) using encrypted connections
**Depends on**: Phase 54 (v2.0 PostgreSQL driver)
**Requirements**: TLS-01, TLS-02, TLS-03, TLS-06
**Success Criteria** (what must be TRUE):
  1. User can connect to a PostgreSQL database with `sslmode=require` and all queries execute over an encrypted connection
  2. User can connect with `sslmode=prefer` and the driver automatically upgrades to TLS when the server supports it, falling back to plaintext otherwise
  3. User can connect with `sslmode=disable` and the connection works identically to v2.0 behavior (no TLS negotiation)
  4. Existing v2.0 PostgreSQL code (plaintext connections) continues to work without modification
**Plans**: 1 plan

Plans:
- [x] 55-01-PLAN.md -- PgStream enum, SSLRequest handshake, sslmode URL parsing, CryptoProvider install

### Phase 56: HTTPS Server
**Goal**: Snow programs can serve HTTP traffic over TLS for production deployments
**Depends on**: Phase 55 (shared rustls/ring infrastructure)
**Requirements**: TLS-04, TLS-05
**Success Criteria** (what must be TRUE):
  1. User can call `Http.serve_tls(router, port, cert_path, key_path)` and the server accepts HTTPS connections with a valid certificate
  2. Existing `Http.serve(router, port)` continues to work for plaintext HTTP without modification
  3. All existing HTTP features (path parameters, method routing, middleware) work identically over HTTPS
  4. TLS handshakes do not block the actor scheduler -- unrelated actors continue executing during handshake processing
**Plans**: 2 plans

Plans:
- [x] 56-01-PLAN.md -- Replace tiny_http with hand-rolled HTTP/1.1 parser for plaintext TCP
- [x] 56-02-PLAN.md -- Add Http.serve_tls with HttpStream enum and codegen integration

### Phase 57: Connection Pooling & Transactions
**Goal**: Snow programs can manage database connections efficiently with pooling and execute multi-statement operations atomically with transactions
**Depends on**: Phase 55 (TLS-enabled PG connections for pooled secure connections)
**Requirements**: POOL-01, POOL-02, POOL-03, POOL-04, POOL-05, POOL-06, POOL-07, TXN-01, TXN-02, TXN-03, TXN-04, TXN-05
**Success Criteria** (what must be TRUE):
  1. User can create a PostgreSQL connection pool with `Pool.open(url, config)` specifying min/max connections and checkout timeout, and multiple actors can concurrently execute queries through the pool without connection conflicts
  2. User can call `Pg.transaction(conn, fn(conn) do ... end)` and the block auto-commits on success or auto-rollbacks on error/panic, with the connection returned to a clean state
  3. User can call `Pool.query(pool, sql, params)` for single queries with automatic checkout-use-checkin, and the pool recycles connections transparently
  4. Pool detects and replaces dead connections via health check so stale connections from server restarts do not surface as user-visible errors
  5. User can call `Sqlite.begin/commit/rollback` for manual SQLite transaction control
**Plans**: 3 plans

Plans:
- [x] 57-01-PLAN.md -- PG txn_status tracking, PG/SQLite transaction intrinsics, Pg.transaction with catch_unwind
- [x] 57-02-PLAN.md -- Mutex+Condvar connection pool with health check, checkout/checkin, auto query/execute
- [x] 57-03-PLAN.md -- Compiler pipeline: LLVM declarations, MIR lowering, type checker for Pool/Pg.txn/Sqlite.txn

### Phase 58: Struct-to-Row Mapping
**Goal**: Snow programs can automatically map database query results to typed structs without manual field extraction
**Depends on**: Phase 57 (connection pooling for realistic query_as usage)
**Requirements**: ROW-01, ROW-02, ROW-03, ROW-04, ROW-05, ROW-06
**Success Criteria** (what must be TRUE):
  1. User can add `deriving(Row)` to a struct and call the generated `from_row` function to convert a `Map<String, String>` query result into a typed struct instance
  2. User can call `Pg.query_as(conn, sql, params, from_row_fn)` and receive a `List<Result<T, String>>` of hydrated structs directly from a query
  3. NULL database columns map to `None` for `Option<T>` fields and return a descriptive error for non-Option fields, so the user gets clear feedback on schema mismatches
  4. Compiler emits an error when `deriving(Row)` is used on a struct with a field type that cannot be parsed from a string (e.g., nested structs, custom types)
**Plans**: 2 plans

Plans:
- [x] 58-01-PLAN.md -- Runtime row parsing functions + query_as + three-point LLVM registration
- [x] 58-02-PLAN.md -- Typeck validation, MIR generation for from_row, Pg/Pool.query_as type signatures, E2E tests

## Progress

**Execution Order:**
Phases execute in numeric order: 55 -> 56 -> 57 -> 58

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
| 49-54 | v2.0 | 13/13 | Complete | 2026-02-12 |
| 55. PG TLS | v3.0 | 1/1 | Complete | 2026-02-12 |
| 56. HTTPS | v3.0 | 2/2 | Complete | 2026-02-12 |
| 57. Pool+Txn | v3.0 | 3/3 | Complete | 2026-02-12 |
| 58. Row Map | v3.0 | 2/2 | Complete | 2026-02-12 |

**Total: 58 phases shipped across 12 milestones. 162 plans completed.**
