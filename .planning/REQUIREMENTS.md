# Requirements: Snow

**Defined:** 2026-02-10
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.

## v2.0 Requirements

Requirements for Database & Serialization milestone. Each maps to roadmap phases.

### JSON Serde

- [ ] **JSON-01**: User can add `deriving(Json)` to a struct to enable automatic JSON encode/decode
- [ ] **JSON-02**: User can encode a struct to JSON string via `Json.encode(value)`
- [ ] **JSON-03**: User can decode a JSON string to a typed struct via `Json.decode(str)` returning `Result<T, String>`
- [ ] **JSON-04**: JSON serde handles nested structs (struct field with another `deriving(Json)` struct)
- [ ] **JSON-05**: JSON serde handles `Option<T>` fields (`Some` → value, `None` → JSON null)
- [ ] **JSON-06**: JSON serde handles `List<T>` fields (serialized as JSON arrays)
- [ ] **JSON-07**: JSON serde handles `Map<String, V>` fields (serialized as JSON objects)
- [ ] **JSON-08**: JSON serde handles sum types as tagged unions (`{"tag":"Variant","fields":[...]}`)
- [ ] **JSON-09**: JSON serde handles generic structs via monomorphization
- [ ] **JSON-10**: Compiler emits error when `deriving(Json)` on struct with non-Json-serializable field type
- [ ] **JSON-11**: JSON number representation distinguishes Int and Float for round-trip fidelity

### SQLite

- [ ] **SQLT-01**: User can open a SQLite database with `Sqlite.open(path)` → `Result<SqliteConn, String>`
- [ ] **SQLT-02**: User can close a connection with `Sqlite.close(conn)`
- [ ] **SQLT-03**: User can query with `Sqlite.query(conn, sql, params)` → `Result<List<Map<String, String>>, String>`
- [ ] **SQLT-04**: User can execute mutations with `Sqlite.execute(conn, sql, params)` → `Result<Int, String>`
- [ ] **SQLT-05**: Query parameters use `?` placeholders with List of typed values for SQL injection prevention
- [ ] **SQLT-06**: SQLite is bundled (compiled from source via `libsqlite3-sys`, zero system dependencies)
- [ ] **SQLT-07**: Database handles are opaque u64 values safe from GC collection

### PostgreSQL

- [ ] **PG-01**: User can connect with `Pg.connect(url)` → `Result<PgConn, String>`
- [ ] **PG-02**: User can close a connection with `Pg.close(conn)`
- [ ] **PG-03**: User can query with `Pg.query(conn, sql, params)` → `Result<List<Map<String, String>>, String>`
- [ ] **PG-04**: User can execute mutations with `Pg.execute(conn, sql, params)` → `Result<Int, String>`
- [ ] **PG-05**: Query parameters use `$1, $2` placeholders with List of typed values
- [ ] **PG-06**: Pure wire protocol implementation (zero C dependencies beyond crypto crates)
- [ ] **PG-07**: SCRAM-SHA-256 authentication supported for production/cloud PostgreSQL
- [ ] **PG-08**: MD5 authentication supported for local development

### HTTP Enhancements

- [ ] **HTTP-01**: Router supports path parameters (`/users/:id`) with segment-based matching
- [ ] **HTTP-02**: User can extract path parameters via `Request.param(req, "id")` → `Option<String>`
- [ ] **HTTP-03**: Router supports method-specific routes (`HTTP.get`, `HTTP.post`, `HTTP.put`, `HTTP.delete`)
- [ ] **HTTP-04**: User can add global middleware via `HTTP.use(router, middleware_fn)`
- [ ] **HTTP-05**: Middleware receives request and next function, can modify request/response
- [ ] **HTTP-06**: Multiple middleware functions compose in registration order (first added = outermost)

## Future Requirements

Deferred to future release. Tracked but not in current roadmap.

### Database Enhancements

- **DBEX-01**: Connection pooling for SQLite and PostgreSQL
- **DBEX-02**: Struct-to-row mapping (query results as typed structs)
- **DBEX-03**: Transaction support (begin/commit/rollback)
- **DBEX-04**: TLS/SSL for PostgreSQL connections

### Serialization Enhancements

- **SERX-01**: JSON pretty-print (`Json.encode_pretty(value)`)
- **SERX-02**: Custom field name mapping (e.g., `@json_name("user_name")`)
- **SERX-03**: Ignore fields during serialization

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| ORM / query builder | Massive scope, controversial in Go/Rust communities, not needed for v2.0 |
| Connection pooling | Single connections sufficient for v2.0; pooling is future work |
| Database migrations | Build tool concern, not language runtime |
| WebSocket support | Different protocol, separate milestone |
| GraphQL | Specialized, not table-stakes for backend apps |
| Redis/MongoDB drivers | PostgreSQL + SQLite cover primary use cases |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| JSON-01 | — | Pending |
| JSON-02 | — | Pending |
| JSON-03 | — | Pending |
| JSON-04 | — | Pending |
| JSON-05 | — | Pending |
| JSON-06 | — | Pending |
| JSON-07 | — | Pending |
| JSON-08 | — | Pending |
| JSON-09 | — | Pending |
| JSON-10 | — | Pending |
| JSON-11 | — | Pending |
| SQLT-01 | — | Pending |
| SQLT-02 | — | Pending |
| SQLT-03 | — | Pending |
| SQLT-04 | — | Pending |
| SQLT-05 | — | Pending |
| SQLT-06 | — | Pending |
| SQLT-07 | — | Pending |
| PG-01 | — | Pending |
| PG-02 | — | Pending |
| PG-03 | — | Pending |
| PG-04 | — | Pending |
| PG-05 | — | Pending |
| PG-06 | — | Pending |
| PG-07 | — | Pending |
| PG-08 | — | Pending |
| HTTP-01 | — | Pending |
| HTTP-02 | — | Pending |
| HTTP-03 | — | Pending |
| HTTP-04 | — | Pending |
| HTTP-05 | — | Pending |
| HTTP-06 | — | Pending |

**Coverage:**
- v2.0 requirements: 32 total
- Mapped to phases: 0
- Unmapped: 32 ⚠️

---
*Requirements defined: 2026-02-10*
*Last updated: 2026-02-10 after initial definition*
