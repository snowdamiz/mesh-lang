# Phase 98 Research: Query Builder + Repo

**Phase:** 98 of 102
**Goal:** Developers can compose queries using pipe chains and execute them through a stateless Repo module, covering all standard CRUD operations, aggregation, transactions, and raw SQL escape hatches
**Plans:** 98-01 (Query struct + builder), 98-02 (Repo reads), 98-03 (Repo writes + transactions)
**Depends on:** Phase 97 (Schema metadata + SQL generation -- COMPLETED)

---

## 1. What Exists Today

### Phase 97 Deliverables (Foundation for Phase 98)

**Schema Metadata (97-01):** `deriving(Schema)` generates five metadata functions per struct:
- `User.__table__()` -> `"users"` (pluralized, lowercased; customizable via `table "custom"`)
- `User.__fields__()` -> `["id", "name", "email"]` (list of field name strings)
- `User.__field_types__()` -> `["id:TEXT", "name:TEXT", "age:BIGINT"]` (field:SQL_TYPE pairs)
- `User.__primary_key__()` -> `"id"` (customizable via `primary_key :uuid`)
- `User.__relationships__()` -> `["has_many:posts:Post"]` (encoded as kind:name:target)
- Per-field column accessors: `User.__name_col__()` -> `"name"`
- Schema options: `table`, `primary_key`, `timestamps` as contextual identifiers in struct body

**SQL Generation (97-02):** Four `Orm` module functions build parameterized SQL:
- `Orm.build_select(table, columns, where_clauses, order_by, limit, offset)` -> SQL string
- `Orm.build_insert(table, columns, returning)` -> SQL string
- `Orm.build_update(table, set_columns, where_clauses, returning)` -> SQL string
- `Orm.build_delete(table, where_clauses, returning)` -> SQL string

WHERE clause format: `"column op"` space-separated (e.g., `"name ="`, `"age >"`, `"status IS NULL"`). IS NULL/IS NOT NULL do not consume parameter slots. UPDATE parameter numbering: SET columns `$1..$N`, WHERE continues from `$N+1`.

### Existing Database Layer

**Pool module (mesh-rt/src/db/pool.rs):**
- `Pool.open(url, min, max, timeout)` -> `Result<PoolHandle, String>`
- `Pool.close(pool)` -> `Unit`
- `Pool.checkout(pool)` -> `Result<PgConn, String>`
- `Pool.checkin(pool, conn)` -> `Unit` (auto-ROLLBACK if dirty txn)
- `Pool.query(pool, sql, params)` -> `Result<List<Map<String, String>>, String>` (auto checkout/checkin)
- `Pool.execute(pool, sql, params)` -> `Result<Int, String>` (auto checkout/checkin)
- `Pool.query_as(pool, sql, params, from_row_fn)` -> `Result<List<Result<T, String>>, String>`

**Pg module (mesh-rt/src/db/pg.rs):**
- `Pg.query(conn, sql, params)` -> raw query on single connection
- `Pg.execute(conn, sql, params)` -> raw execute on single connection
- `Pg.begin(conn)` / `Pg.commit(conn)` / `Pg.rollback(conn)` -> transaction control
- `Pg.transaction(conn, callback_fn)` -> BEGIN/callback/COMMIT|ROLLBACK with panic safety

**Key insight:** `Pool.query` does automatic checkout-use-checkin per call. For transactions, we need `Pool.checkout` to hold a single connection across multiple operations, then `Pool.checkin` after commit/rollback.

### Existing Struct Infrastructure

- Structs are lowered to `MirStructDef { name, fields: Vec<(String, MirType)> }`
- Struct creation: `MirExpr::StructLit { name, fields, ty }`
- Struct field access: `MirExpr::FieldAccess { object, field, ty }`
- Struct update: `MirExpr::StructUpdate { base, overrides, ty }` (Phase 96-03)
- All structs are GC-traced Ptr at runtime

### Module Registration Pattern

Adding a new stdlib module requires changes in three places:

1. **Type checker** (`mesh-typeck/src/infer.rs`):
   - Add to `STDLIB_MODULE_NAMES` array (line ~1046)
   - Register module functions with type signatures in `build_stdlib_modules()`
   - Each function: `mod.insert("fn_name", Scheme::mono(Ty::fun(param_types, return_type)))`

2. **MIR lowerer** (`mesh-codegen/src/mir/lower.rs`):
   - Add to `STDLIB_MODULES` array (line ~10055)
   - Register runtime function signatures in `register_known_functions()` with `known_functions.insert("mesh_mod_fn", MirType::FnPtr(...))`
   - Add name mappings in `map_builtin_name()`: `"mod_fn" => "mesh_mod_fn"`

3. **Runtime** (`mesh-rt/src/db/`):
   - Implement `extern "C"` functions matching the declared signatures
   - Functions receive/return GC-safe types (Ptr=`*mut u8`, Int=`i64`, etc.)

---

## 2. Key Design Decisions Required

### Decision 1: `User |> Query.where(...)` -- How Does the Schema Name Start a Pipe?

**The problem:** Success criteria #1 says `User |> Query.where(:name, "Alice")`. But `User` is a struct constructor type in the type env, not a Query struct or a table name string. Pipe desugars this to `Query.where(User, :name, "Alice")`.

**Option A -- Compiler-level schema-to-table transformation (RECOMMENDED):**
When the MIR lowerer encounters a pipe chain where the LHS is a struct name that has `deriving(Schema)`, it automatically transforms `User` to `User.__table__()` (i.e., a call to the generated table name function). This produces a String, which `Query.where` accepts as a "starting point" that creates a new Query.

- Pro: Matches success criteria syntax exactly
- Pro: Compile-time resolution, no runtime overhead
- Con: Requires special-casing in MIR pipe lowering

**Option B -- Query.from() required in chain:**
Require explicit `Query.from("users") |> Query.where(:name, "Alice")`. The success criteria is interpreted loosely.

- Pro: No compiler changes, purely runtime
- Con: Doesn't match stated success criteria

**Option C -- Runtime type checking in Query.where:**
`Query.where` checks at runtime if first arg is a Query struct or a String, creates a Query if needed. Requires a runtime tag/discriminant.

- Pro: Flexible
- Con: Mesh has no runtime type tags; all values are untyped Ptr. Cannot reliably distinguish a String pointer from a Query struct pointer at runtime.

**Recommendation:** Option A. The compiler already has infrastructure for pipe-chain analysis (96-05: `pipe_chain_has_string_keys()` backward walk). Adding "if LHS of pipe is a Schema struct, emit `__table__()` call" fits the established pattern. When the first arg to any Query function is a String instead of a Query, the runtime creates a new Query from that table name.

### Decision 2: Query -- Opaque Ptr Handle (Not Mesh Struct)

**Critical finding from codegen analysis:** Mesh structs defined via `deriving(Schema)` or user code are `MirType::Struct(name)` which maps to LLVM struct types (VALUE types, not pointers). This means struct values are passed by value at the ABI level. A 13-field struct would be a 13-field LLVM struct -- passing this to `extern "C"` runtime functions is problematic because the runtime Rust function would need an exact matching C struct layout.

**The established pattern for complex runtime-managed types:** `List`, `Map`, `Set`, `Range`, `Queue`, `Router`, `Request`, `Response` are all `MirType::Ptr` (opaque pointers). The runtime manages them as heap-allocated objects. This is how Query should work.

**Design: Query is an opaque Ptr managed by the Rust runtime.**

The type checker registers `Query` as a type that resolves to `MirType::Ptr` (like List, Map, etc.). The runtime allocates Query objects on the heap and manages their internal structure. Query builder functions take a `*mut u8` pointer and return a new `*mut u8` pointer.

Internal representation in Rust:
```rust
struct QueryData {
    source: *mut u8,            // MeshString
    select_fields: *mut u8,     // List<String>
    where_clauses: *mut u8,     // List<String>
    where_params: *mut u8,      // List<String>
    order_fields: *mut u8,      // List<String>
    limit_val: i64,             // -1 = no limit
    offset_val: i64,            // -1 = no offset
    join_clauses: *mut u8,      // List<String>
    group_fields: *mut u8,      // List<String>
    having_clauses: *mut u8,    // List<String>
    having_params: *mut u8,     // List<String>
    fragment_parts: *mut u8,    // List<String>
    fragment_params: *mut u8,   // List<String>
}
```

- Pro: Matches established pattern for all other complex runtime types
- Pro: Simple `extern "C"` function signatures (`*mut u8` everywhere)
- Pro: No ABI mismatch between Rust runtime and LLVM-compiled code
- Pro: GC does not need to trace Query internals (the fields are traced via the inner List/String pointers)
- Con: Query fields are not directly accessible from Mesh code (no `query.source`)
- Con: Not composable via struct update syntax

**GC safety:** The Query object itself must be allocated via `mesh_gc_alloc_actor` (GC-traced allocation). The GC needs to trace through it to find the inner List/String pointers. This requires either: (a) the GC knows the Query layout and traces its pointer fields, OR (b) the Query fields are all stored in a way the GC can find them.

**Simpler GC approach:** Since GC traces all allocations as arrays of potential pointers, allocating the QueryData as a sequence of pointer-sized slots works. The GC will scan all slots looking for valid heap pointers. Integer fields (limit_val, offset_val) use tagged integers (`val << 1 | 1`) which the GC recognizes as non-pointers.

**Recommendation:** Opaque Ptr. Register `"Query"` in `resolve_con` to return `MirType::Ptr` (alongside List, Map, etc.). All Query module functions take and return `*mut u8`.

### Decision 3: Query Module Functions -- Where Do They Live?

Since Mesh has no `.mpl` standard library, Query and Repo functions must be **runtime Rust functions** registered as a stdlib module, following the same pattern as Pool, Orm, HTTP, etc.

Each function:
1. Type signature registered in `build_stdlib_modules()` in typeck
2. Runtime function declared in `register_known_functions()` in MIR lowerer
3. Name mapped in `map_builtin_name()`
4. Implemented as `extern "C"` in `mesh-rt/src/db/query.rs` and `mesh-rt/src/db/repo.rs`

### Decision 4: Repo Return Types -- `Map<String, String>` vs Typed Structs

**Architecture decision (already established):** Repo functions return `Map<String, String>` (raw rows) or `List<Map<String, String>>`. Callers apply `User.from_row(row)` for type conversion. This is because Mesh has no dynamic dispatch/trait objects -- Repo cannot call a generic `from_row` for arbitrary T.

**Pattern:**
```
let row = Repo.get(pool, "users", user_id)?
let user = User.from_row(row)?
```

Future: `Repo.all_as(pool, query, User.from_row)` could take a from_row callback (like existing `Pool.query_as`).

### Decision 5: Repo.transaction -- Pool-Level Transaction Design

**The problem:** `Pool.query/Pool.execute` auto-checkout per call. Transactions need a SINGLE connection across multiple operations.

**Design:**
```
Repo.transaction(pool, fn(conn) do
  let _ = Pg.execute(conn, sql1, params1)?
  let _ = Pg.execute(conn, sql2, params2)?
  Ok(result)
end)
```

**Implementation:** New `mesh_repo_transaction` runtime function:
1. `Pool.checkout(pool)` -- acquire connection
2. `Pg.begin(conn)` -- start transaction
3. Call user closure with conn handle
4. On `Ok`: `Pg.commit(conn)`, `Pool.checkin(pool, conn)`, return result
5. On `Err`: `Pg.rollback(conn)`, `Pool.checkin(pool, conn)`, return error
6. On panic: `Pg.rollback(conn)`, `Pool.checkin(pool, conn)`, return error

The callback receives a `PgConn` handle (same as `Pool.checkout` returns). Inside the callback, the developer uses `Pg.query(conn, ...)` and `Pg.execute(conn, ...)` directly -- NOT `Pool.query(pool, ...)`.

**Type signature:** `Repo.transaction(pool, fn(conn) -> Result<T, String>) -> Result<T, String>`

This follows the existing `Pg.transaction(conn, callback)` pattern but adds pool checkout/checkin.

### Decision 6: Having Clause and Fragment -- SQL Generation Gaps

The existing `Orm.build_select` does NOT support:
- GROUP BY (not in current signature)
- HAVING clauses
- JOIN clauses
- Raw SQL fragment injection

**Approach:** Extend `mesh_orm_build_select` OR add new runtime functions. The Query struct stores all clause data; the SQL generation function consumes it all at once when Repo executes the query. This means the SQL builder must be enhanced in Phase 98 to handle the full Query struct.

**Recommended: New comprehensive builder.** Create `mesh_query_to_sql(query_struct_ptr)` that reads all Query struct fields and produces `(sql_string, params_list)`. This replaces calling `Orm.build_select` with separate arguments.

---

## 3. What Must Be Built (by Plan)

### Plan 98-01: Query Struct + Builder Functions

**Compiler changes:**
- Register `Query` struct in typeck as a built-in struct type (or generate it synthetically)
- Register `Query` module in `STDLIB_MODULE_NAMES` and `STDLIB_MODULES`
- Register all Query function types in `build_stdlib_modules()`
- Register runtime functions in `known_functions`
- Add name mappings in `map_builtin_name()`
- Add pipe-chain schema-to-table transformation for `User |> Query.where(...)` pattern

**Type signatures for Query module:**
```
Query.from(table :: String) -> Query
Query.where(query :: Query, field :: Atom, value :: String) -> Query
Query.where(query :: Query, field :: Atom, op :: Atom, value :: String) -> Query
Query.select(query :: Query, fields :: List<String>) -> Query
Query.order_by(query :: Query, field :: Atom, direction :: Atom) -> Query
Query.limit(query :: Query, n :: Int) -> Query
Query.offset(query :: Query, n :: Int) -> Query
Query.join(query :: Query, type :: Atom, table :: String, on_clause :: String) -> Query
Query.group_by(query :: Query, field :: Atom) -> Query
Query.having(query :: Query, clause :: String, value :: String) -> Query
Query.fragment(query :: Query, sql :: String, params :: List<String>) -> Query
```

**Note on Atom parameters:** Phase 96-01 established atoms lower to `MirExpr::StringLit`. So `:name` becomes `"name"` at MIR level. The Query runtime functions receive strings. The Atom type in typeck provides compile-time distinction (`:name` not `"name"`).

**Runtime implementation (mesh-rt/src/db/query.rs):**
- Each function allocates a new opaque Query object via `mesh_gc_alloc_actor`
- Layout: 13 pointer-sized slots (8 bytes each = 104 bytes total)
- Slots contain either `*mut u8` pointers (to MeshString/List) or tagged integers
- Each builder function reads the old Query's slots, creates a new allocation, copies unchanged slots, and modifies the relevant slots

**Immutability pattern:** Each builder function creates a NEW Query object. The original Query is never modified. This matches the immutable pipe-chain semantics. Example for `Query.where`:
1. Allocate new QueryData via `mesh_gc_alloc_actor(104, 8)`
2. Copy all 13 slots from input Query
3. Read the `where_clauses` list pointer (slot 2)
4. Create a new list by appending the new clause: `mesh_list_append(old_where_clauses, new_clause)`
5. Store the new list pointer in slot 2 of the new Query
6. Similarly append to `where_params` list (slot 3) if not IS NULL
7. Return the new Query pointer

**Field access pattern:** Query slots are accessed by byte offset:
```rust
unsafe fn query_get_field(query: *mut u8, field_index: usize) -> *mut u8 {
    *(query.add(field_index * 8) as *mut *mut u8)
}
unsafe fn query_set_field(query: *mut u8, field_index: usize, value: *mut u8) {
    *(query.add(field_index * 8) as *mut *mut u8) = value;
}
```

### Plan 98-02: Repo Read Operations

**New module:** `Repo` in `STDLIB_MODULE_NAMES` and `STDLIB_MODULES`.

**Type signatures:**
```
Repo.all(pool :: PoolHandle, query :: Query) -> Result<List<Map<String, String>>, String>
Repo.one(pool :: PoolHandle, query :: Query) -> Result<Map<String, String>, String>
Repo.get(pool :: PoolHandle, table :: String, id :: String) -> Result<Map<String, String>, String>
Repo.get_by(pool :: PoolHandle, table :: String, field :: String, value :: String) -> Result<Map<String, String>, String>
Repo.count(pool :: PoolHandle, query :: Query) -> Result<Int, String>
Repo.exists(pool :: PoolHandle, query :: Query) -> Result<Bool, String>
```

**Implementation pattern for Repo.all:**
1. Extract Query struct fields (source, where_clauses, where_params, etc.)
2. Build SQL via enhanced SQL builder (or call existing `build_select_sql`)
3. Call `mesh_pool_query(pool, sql_ptr, params_ptr)` -- reuse existing pool infrastructure
4. Return the result directly (already `List<Map<String, String>>`)

**Repo.one:** Same as Repo.all but adds `LIMIT 1` and returns first row or `Err("not found")`.

**Repo.get:** Shorthand for `SELECT * FROM table WHERE primary_key = $1 LIMIT 1`.

**Repo.get_by:** Shorthand for `SELECT * FROM table WHERE field = $1 LIMIT 1`.

**Repo.count:** Builds `SELECT COUNT(*) FROM table WHERE ...`, parses integer from result.

**Repo.exists:** Builds `SELECT EXISTS(SELECT 1 FROM table WHERE ...)`, parses boolean.

### Plan 98-03: Repo Write Operations + Transactions

**Type signatures:**
```
Repo.insert(pool :: PoolHandle, table :: String, fields :: Map<String, String>) -> Result<Map<String, String>, String>
Repo.update(pool :: PoolHandle, table :: String, id :: String, fields :: Map<String, String>) -> Result<Map<String, String>, String>
Repo.delete(pool :: PoolHandle, table :: String, id :: String) -> Result<Map<String, String>, String>
Repo.transaction(pool :: PoolHandle, callback :: fn(PgConn) -> Result<T, String>) -> Result<T, String>
```

**Note on Repo.insert/update signatures:** The requirements say `Repo.insert(pool, changeset)` but Changesets are Phase 99. For Phase 98, we provide a simpler map-based API. Phase 99 adds changeset overloads.

**Repo.insert implementation:**
1. Extract keys/values from the fields Map
2. Build INSERT SQL with `Orm.build_insert`
3. Call `Pool.query(pool, sql, params)` (query not execute, for RETURNING)
4. Return first row from RETURNING

**Repo.update implementation:**
1. Extract changed fields from Map
2. Build UPDATE SQL with `Orm.build_update`, WHERE on primary key
3. Execute with RETURNING *
4. Return updated row

**Repo.delete implementation:**
1. Build DELETE SQL with WHERE on primary key
2. Execute with RETURNING * for the deleted row
3. Return deleted row or error

**Repo.transaction implementation:** See Decision 5 above.

---

## 4. Technical Constraints and Pitfalls

### Constraint 1: Query Object Slot Layout Must Be Stable

The runtime Rust code reads/writes Query object fields by byte offset. If the slot order changes, all runtime functions break. The slot layout must be fixed at phase start and never reordered.

**Canonical slot layout (each slot is 8 bytes):**
```
slot 0  (offset  0): source          (*mut u8 -> MeshString)
slot 1  (offset  8): select_fields   (*mut u8 -> List<String>)
slot 2  (offset 16): where_clauses   (*mut u8 -> List<String>)
slot 3  (offset 24): where_params    (*mut u8 -> List<String>)
slot 4  (offset 32): order_fields    (*mut u8 -> List<String>)
slot 5  (offset 40): limit_val       (i64 tagged: val << 1 | 1, -1 = none)
slot 6  (offset 48): offset_val      (i64 tagged: val << 1 | 1, -1 = none)
slot 7  (offset 56): join_clauses    (*mut u8 -> List<String>)
slot 8  (offset 64): group_fields    (*mut u8 -> List<String>)
slot 9  (offset 72): having_clauses  (*mut u8 -> List<String>)
slot 10 (offset 80): having_params   (*mut u8 -> List<String>)
slot 11 (offset 88): fragment_parts  (*mut u8 -> List<String>)
slot 12 (offset 96): fragment_params (*mut u8 -> List<String>)
```
Total size: 104 bytes (13 slots x 8 bytes).

Int fields (limit_val, offset_val) use tagged integer encoding. The GC sees tagged integers as non-pointers (LSB = 1) and does not attempt to trace them.

### Constraint 2: GC Safety for Query Object Construction

Runtime Rust functions that construct Query objects must allocate via `mesh_gc_alloc_actor(104, 8)` and fill ALL 13 slots immediately before returning. The GC scans allocations as arrays of potential pointers. A partially-filled Query with uninitialized slots could cause the GC to dereference garbage.

**Pattern:** Allocate, zero-fill or set all slots to valid values (empty lists, tagged -1 for ints), then set actual values, then return. Use `std::ptr::write_bytes(ptr, 0, 104)` to zero-fill after allocation as a safety measure.

### Constraint 3: Atom-to-String Lowering

Atoms (`:name`, `:asc`, `:desc`, `:inner`, `:left`, `:right`) lower to string literals at MIR level (Phase 96-01 decision). The Query runtime functions receive plain strings. The type checker enforces Atom type at call sites, but runtime just sees String pointers.

### Constraint 4: WHERE Clause Encoding

Phase 97 established the `"column op"` encoding (e.g., `"name ="`, `"age >"`, `"status IS NULL"`). The Query struct stores these in `where_clauses` list and corresponding values in `where_params` list (parallel arrays). IS NULL/IS NOT NULL clauses have entries in `where_clauses` but NOT in `where_params`.

**Query.where(:name, "Alice")** -> appends `"name ="` to where_clauses, `"Alice"` to where_params
**Query.where(:age, :gt, "21")** -> appends `"age >"` to where_clauses, `"21"` to where_params
**Query.where(:status, :is_null)** -> appends `"status IS NULL"` to where_clauses, nothing to where_params

### Constraint 5: No Pool-Level Transaction Function Exists

There is no `mesh_pool_transaction`. The runtime has `mesh_pg_transaction(conn, fn_ptr, env_ptr)` which operates on a raw PgConn. For `Repo.transaction(pool, callback)`, the implementation must:
1. Call `mesh_pool_checkout` to get a conn
2. Call `mesh_pg_transaction(conn, callback_fn_ptr, callback_env_ptr)` -- reuse existing transaction logic
3. Call `mesh_pool_checkin` to return the conn

This is a composition of existing primitives, not a new transaction protocol.

### Constraint 6: IN Operator Requires Special Handling

`WHERE name IN ($1, $2, $3)` needs a variable number of parameter slots. The current `"column op"` encoding doesn't support this. Options:
- Encode as `"name IN:3"` (column IN:count) and consume N parameter slots
- Accept a list value and expand at SQL generation time
- Defer IN to fragment() escape hatch

**Recommendation:** Support IN via a special encoding in where_clauses (e.g., `"name IN:N"` where N is the count of values). The runtime SQL builder expands `IN:N` to `IN ($M, $M+1, ..., $M+N-1)`. The values list gets N entries appended.

### Constraint 7: LIKE Operator

LIKE with `%` wildcards is passed as a regular value. The clause is `"name LIKE"` and the value is `"%alice%"`. The `%` is part of the parameter value, not SQL structure, so it's safe from injection.

### Constraint 8: Fragment Safety

`Query.fragment(query, sql, params)` injects raw SQL. The SQL string is NOT parameterized -- it is literal SQL injected into the query. Only the params list uses `$N` binding. This is the escape hatch for complex expressions (e.g., `fragment("date_trunc('day', created_at)")` or `fragment("COALESCE(?, 0)", [default_val])`).

**Convention:** Fragment SQL uses `?` as parameter placeholder. The runtime replaces `?` with `$N` using the next available parameter index. This matches Ecto's fragment convention.

### Constraint 9: JOIN Clause Encoding

JOIN clauses need: join type (INNER/LEFT/RIGHT), target table, ON condition. Encoding options:
- `"INNER:comments:comments.post_id = posts.id"` (type:table:on_clause)
- Separate lists for join types, tables, and on conditions

**Recommendation:** Single encoded string `"type:table:on_clause"` in `join_clauses` list. The SQL builder parses and generates `INNER JOIN "comments" ON comments.post_id = posts.id`. The ON clause is raw SQL (like fragment) since join conditions are structural, not parameterized.

---

## 5. Existing SQL Generation Gap Analysis

### Current `Orm.build_select` Signature
```
build_select(table, columns, where_clauses, order_by, limit, offset) -> String
```

### Missing for Full Query Support
- **GROUP BY**: Not in current signature. Need to add group_fields parameter.
- **HAVING**: Not in current signature. Need having_clauses + having_params.
- **JOIN**: Not in current signature. Need join_clauses list.
- **Fragment**: Not in current signature. Fragments must be injected into appropriate positions (SELECT fragments, WHERE fragments, etc.).

### Options for SQL Generation Enhancement

**Option A -- Extend existing build_select (many parameters):**
Add more parameters to `mesh_orm_build_select`. This quickly becomes unwieldy (10+ parameters).

**Option B -- New comprehensive builder from Query struct (RECOMMENDED):**
Create `mesh_query_to_sql(query_struct_ptr) -> (sql_string, params_list)` that reads the Query struct directly and builds complete SQL. This replaces the current decomposed approach. The existing `Orm.build_select/insert/update/delete` remain for direct use.

**Option C -- Multiple builder calls composed:**
Build SQL incrementally with separate calls. Too many FFI crossings.

**Recommendation:** Option B. A single `mesh_query_to_sql` function reads the Query struct, handling all clause types, and returns a tuple of (SQL string, parameter list). Repo functions call this once before executing.

### Return Type for SQL Generation

The `mesh_query_to_sql` function needs to return TWO values: the SQL string and the params list. Options:
- Return a Mesh Tuple `(String, List<String>)` -- MIR supports tuples
- Return a 2-element List
- Write two functions: `mesh_query_to_sql_string(query)` and `mesh_query_to_sql_params(query)`

**Recommendation:** Two separate functions is simplest. The Repo functions call `mesh_query_to_sql_string(query)` then `mesh_query_to_sql_params(query)`. Alternatively, pack both into Repo directly -- the Repo.all runtime function reads the Query struct, builds SQL internally, calls Pool.query, and returns results. SQL generation is an internal detail of Repo, not exposed to user code.

---

## 6. Composable Scopes Design (QBLD-09)

Composable scopes are pure Mesh functions that take and return a Query:

```
pub fn active(q) do
  q |> Query.where(:status, "active")
end

pub fn recent(q) do
  q |> Query.order_by(:created_at, :desc) |> Query.limit(10)
end

# Usage:
Query.from("users") |> active() |> recent() |> Repo.all(pool)
```

**No compiler or runtime changes needed.** Scopes are just regular Mesh functions. The Query struct is a regular value that can be passed to and returned from any function. The pipe operator already supports this pattern -- `query |> active()` desugars to `active(query)`.

**Type inference:** The scope function `fn active(q) do ... end` infers `q :: Query` from the call to `Query.where(q, ...)`. Standard HM inference handles this.

---

## 7. Query.where Overloads

The success criteria uses `Query.where(:name, "Alice")` (2-arg) and the requirements list operators (3-arg). Since Mesh supports multi-clause functions (multiple definitions with different arities):

```
# 2-arg: field = value (default equality)
Query.where(query, :name, "Alice")

# 3-arg: field op value
Query.where(query, :age, :gt, "21")

# IS NULL variant (2-arg with :is_null atom)
Query.where(query, :status, :is_null)
```

**Challenge:** Mesh does NOT have function overloading by type. Multi-clause functions match by ARITY, not by type. So `where/3` (query, field, value) and `where/3` (query, field, op) cannot coexist.

**Solution:** Use different function names or different arities:
- `Query.where(query, field, value)` -> equality (default `=`)
- `Query.where_op(query, field, op, value)` -> with operator
- `Query.where_null(query, field)` -> IS NULL
- `Query.where_not_null(query, field)` -> IS NOT NULL
- `Query.where_in(query, field, values)` -> IN clause

OR: Use a single `Query.where` with 4 args where op defaults to `"="`:
- `Query.where(query, :name, :eq, "Alice")` -> name = $1
- `Query.where(query, :age, :gt, "21")` -> age > $1

**Recommendation:** Two overloads by arity:
- `Query.where(query, field, value)` -> equality (`=`) -- 3 args
- `Query.where(query, field, op, value)` -> with operator -- 4 args

Plus special variants:
- `Query.where_in(query, field, values_list)` -> IN
- `Query.where_null(query, field)` -> IS NULL
- `Query.where_not_null(query, field)` -> IS NOT NULL

This avoids arity collision while keeping the common case (equality) concise.

---

## 8. Atom-to-Operator Mapping

The success criteria uses atoms for operators (`:asc`, `:desc`). These need mapping to SQL:

```
:eq  -> "="
:neq -> "!="
:lt  -> "<"
:gt  -> ">"
:lte -> "<="
:gte -> ">="
:like -> "LIKE"
:in  -> "IN"
:asc -> "ASC"
:desc -> "DESC"
:inner -> "INNER"
:left  -> "LEFT"
:right -> "RIGHT"
```

The runtime functions receive these as strings (atoms lower to strings) and do a simple match/lookup to produce SQL operators.

---

## 9. Testing Strategy

### E2E Tests (compilable without database)

All Query builder functions can be tested without a database:
1. Create Query via `Query.from("users")`
2. Chain builder functions
3. Verify query struct fields (access struct fields directly)
4. Verify generated SQL by calling SQL builder function

### E2E Tests (with mock/print)

For Repo functions, the e2e test harness compiles and runs programs. Without a real PostgreSQL instance, Repo tests would fail. Options:
- Test SQL generation only (verify the SQL string produced)
- Add mock runtime functions for testing
- Integration tests in a separate test harness with Docker PostgreSQL

**Recommendation:** Phase 98 e2e tests focus on Query struct construction and SQL generation. Repo execution tests are integration tests (manual verification or Docker-based CI).

---

## 10. File Changes Summary

### New Files
- `crates/mesh-rt/src/db/query.rs` -- Query struct runtime functions
- `crates/mesh-rt/src/db/repo.rs` -- Repo runtime functions

### Modified Files
- `crates/mesh-typeck/src/infer.rs` -- Add Query/Repo to STDLIB_MODULE_NAMES, register function types
- `crates/mesh-codegen/src/mir/lower.rs` -- Add Query/Repo to STDLIB_MODULES, register known_functions, add map_builtin_name mappings, add pipe schema-to-table transformation
- `crates/mesh-rt/src/db/mod.rs` -- Add `pub mod query; pub mod repo;`
- `crates/mesh-rt/src/db/orm.rs` -- Extend SQL builders for GROUP BY, HAVING, JOIN (or add new comprehensive builder)
- `crates/meshc/tests/e2e.rs` -- Add e2e tests for Query builder and SQL generation

### Unchanged Files
- `crates/mesh-lexer/` -- No new tokens
- `crates/mesh-parser/` -- No new grammar
- `crates/mesh-rt/src/db/pg.rs` -- No changes
- `crates/mesh-rt/src/db/pool.rs` -- No changes (Repo.transaction composes existing checkout/checkin)

---

## 11. Dependencies Between Plans

```
98-01: Query struct + builder functions
  |
  +-> 98-02: Repo read operations (needs Query struct to read fields for SQL generation)
  |
  +-> 98-03: Repo write operations (needs Query struct for delete with conditions;
                                     transaction uses Repo pattern from 98-02)
```

98-01 is strictly prerequisite for 98-02 and 98-03. 98-02 and 98-03 could theoretically be parallel but share the Repo module registration, so sequential is safer.

---

## 12. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Query struct field access from Rust runtime is fragile | MEDIUM | HIGH | Document field layout; add assertions; follow existing struct access patterns in codegen |
| GC collection during Query construction corrupts partially-built struct | LOW | HIGH | Allocate struct, fill all fields immediately; follow existing StructLit codegen pattern |
| IN operator encoding complexity | LOW | MEDIUM | Defer to fragment() if encoding is too complex; IN is not in success criteria |
| Pipe schema-to-table transformation has edge cases | MEDIUM | MEDIUM | Fall back to explicit Query.from() if transformation has issues |
| Repo.transaction closure captures complex environment | LOW | MEDIUM | Reuse existing Pg.transaction infrastructure which handles closure splitting |
| WHERE parameter numbering with mixed IS NULL and regular conditions | LOW | HIGH | Already solved in Phase 97 (IS NULL doesn't consume slots) |

---

## 13. Success Criteria Verification Plan

1. **SC-1:** `User |> Query.where(:name, "Alice") |> Query.order_by(:name, :asc) |> Query.limit(10) |> Repo.all(pool)` returns `List<Map<String, String>>`
   - Verify: E2E test with Query construction + SQL generation verification. Integration test with real DB for full pipeline.

2. **SC-2:** All query builder functions are pipe-composable and return new immutable Query structs
   - Verify: E2E tests chaining multiple functions, verify original query unchanged after pipe.

3. **SC-3:** Repo CRUD operations work correctly
   - Verify: Integration tests with real PostgreSQL (or SQL string verification for e2e).

4. **SC-4:** Repo.count, Repo.exists, Repo.transaction work
   - Verify: E2E for count/exists SQL generation. Integration test for transaction.

5. **SC-5:** Composable scopes as pure functions
   - Verify: E2E test defining scope function, composing with pipe chain, verifying SQL output.
