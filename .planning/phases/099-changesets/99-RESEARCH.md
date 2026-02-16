# Phase 99: Changesets - Research

**Researched:** 2026-02-16
**Domain:** Mesh runtime (Changeset opaque struct, validation pipeline, type coercion, PG constraint mapping) + compiler (Changeset module registration, Repo overloads)
**Confidence:** HIGH

## Summary

Phase 99 adds a Changeset validation pipeline to the Mesh ORM, enabling developers to validate and coerce external data before persistence. The Changeset is an opaque runtime object (like Query in Phase 98) that accumulates validated changes and errors. The implementation splits into two areas: (1) the Changeset struct itself with cast, validation functions, and pipe-chain composition, and (2) integration with Repo.insert/Repo.update plus PostgreSQL constraint error mapping.

The Changeset follows the same architectural pattern as Query: an opaque `*mut u8` pointer to a GC-allocated slot array, managed entirely by Rust runtime functions. The type checker registers a `Changeset` module with function signatures, the MIR lowerer maps to `mesh_changeset_*` extern C functions, and the runtime implements the logic. Validation functions (validate_required, validate_length, validate_format, validate_inclusion, validate_number) take a Changeset and return a new Changeset with errors appended -- never short-circuiting. The pipe-chain pattern works because each validator takes Changeset as its first argument and returns Changeset, matching the existing pipe semantics.

The constraint error mapping requires enhancing the PG wire protocol parser (`pg.rs`) to extract additional ErrorResponse fields beyond the 'M' (message) field -- specifically 'C' (SQLSTATE code), 'D' (detail), and 'n' (constraint name). SQLSTATE 23505 = unique violation, 23503 = foreign key violation. The runtime then parses constraint names to determine the affected field and maps the error to a human-readable message on the Changeset.

**Primary recommendation:** Implement as two plans. Plan 99-01: Changeset struct, cast function, all five validators, and pipe-chain validation. Plan 99-02: Enhanced PG error parsing, constraint-to-changeset error mapping, and Repo.insert/Repo.update changeset overloads.

## Standard Stack

### Core

| Crate | Location | Purpose | Relevance |
|-------|----------|---------|-----------|
| mesh-typeck | crates/mesh-typeck | Register Changeset module functions | Type signatures for cast, validators, field accessors |
| mesh-codegen | crates/mesh-codegen | MIR lowering + name mapping | Register known_functions, map_builtin_name for changeset_* |
| mesh-rt | crates/mesh-rt | Runtime implementation | New `db/changeset.rs` with extern C functions |

### Supporting

| Library | Version | Purpose | When Used |
|---------|---------|---------|-----------|
| inkwell | 0.8.0 (LLVM 21.1) | LLVM IR generation | Declaring new intrinsics in intrinsics.rs |

## Architecture Patterns

### Pattern 1: Changeset as Opaque Ptr (Following Query Pattern)

**What:** The Changeset is a GC-allocated slot array, identical in architecture to the Query struct from Phase 98. Each slot is 8 bytes (pointer-sized). The Changeset is immutable -- each operation creates a new Changeset with the modified state.

**Why not a Mesh struct?** Same reason as Query: Mesh structs are VALUE types at LLVM level. Passing a multi-field struct across the `extern "C"` boundary would require exact C ABI layout matching. The opaque Ptr pattern is established across all complex runtime types (List, Map, Set, Range, Queue, Router, Request, Response, Query).

**Changeset slot layout (8 slots, 64 bytes):**

```
slot 0 (offset  0): data          (*mut u8 -> Map<String,String>)  -- original struct data
slot 1 (offset  8): changes       (*mut u8 -> Map<String,String>)  -- validated/coerced field changes
slot 2 (offset 16): errors        (*mut u8 -> Map<String,String>)  -- field -> error message mapping
slot 3 (offset 24): valid         (i64: 1 = valid, 0 = invalid)   -- computed from errors being empty
slot 4 (offset 32): field_types   (*mut u8 -> List<String>)        -- "field:SQL_TYPE" from __field_types__()
slot 5 (offset 40): table         (*mut u8 -> MeshString)          -- table name for SQL generation
slot 6 (offset 48): primary_key   (*mut u8 -> MeshString)          -- primary key field name
slot 7 (offset 56): action        (i64: 0 = insert, 1 = update)   -- determines SQL operation
```

Total size: 64 bytes (8 slots x 8 bytes).

**Design rationale for each slot:**
- `data`: The original struct's data as a Map. For inserts, this may be empty. For updates, it contains the existing record's values.
- `changes`: The validated, coerced changes. Only fields that passed through `cast()` with allowed fields and successful type coercion appear here.
- `errors`: A Map<String, String> mapping field names to error messages. Multiple validators can add errors to different fields. If a field already has an error, the first error wins (Ecto convention).
- `valid`: Computed. True when errors map is empty. Each validator updates this after modifying errors.
- `field_types`: Schema field type metadata, carried along for use by validators that need type information.
- `table`: Table name for Repo integration (avoids passing table name separately).
- `primary_key`: Primary key field name for Repo.update WHERE clause.
- `action`: Whether this changeset is for insert (0) or update (1). Determines which SQL operation Repo performs.

### Pattern 2: Changeset.cast -- Type Coercion Pipeline

**What:** `Changeset.cast(struct_or_data, params, allowed_fields)` creates a Changeset by:
1. Taking the original data (either a Map<String,String> or empty for new records)
2. Filtering `params` to only include `allowed_fields`
3. Coercing string values to match schema field types (using `field_types` metadata)
4. Storing coerced values in `changes`

**Type coercion rules (string params -> schema types):**
- String -> String: no coercion needed (identity)
- String -> Int: parse via `str::parse::<i64>()`, error on failure
- String -> Float: parse via `str::parse::<f64>()`, error on failure
- String -> Bool: accept "true"/"false"/"1"/"0"/"t"/"f", error on others

**Cast signature options:**

The success criteria says `Changeset.cast(user, params, [:name, :email])`. This implies `user` is either:
- (a) A struct instance -- but at runtime, struct field access from Rust requires knowing the layout. The Changeset runtime cannot dynamically access struct fields by name.
- (b) A Map<String, String> representing the struct's data. This is what Repo functions already return (Phase 98 established Map<String,String> as the row representation).

**Recommendation:** `Changeset.cast` takes three arguments:
1. `data` :: Ptr (a Map<String,String> of the existing record, or an empty map for inserts)
2. `params` :: Ptr (a Map<String,String> of incoming parameters)
3. `allowed` :: Ptr (a List<String> of allowed field names as atoms)

The field_types, table name, and primary key are passed separately or derived. For ergonomic use:
- `Changeset.cast(data, params, allowed)` -- basic cast
- `Changeset.change(data)` -- create changeset without params (for manual field setting)

To get schema metadata into the Changeset, there are two approaches:

**Approach A (explicit metadata passing):**
```
Changeset.cast(User.__field_types__(), User.__table__(), data, params, [:name, :email])
```
Too many arguments, poor ergonomics.

**Approach B (Changeset.new + cast chain, RECOMMENDED):**
```
Changeset.new(User.__table__(), User.__primary_key__(), User.__field_types__())
  |> Changeset.cast(data, params, [:name, :email])
  |> validate_required([:name])
```
But this is also verbose.

**Approach C (cast takes field_types + table, simplified):**
```
Changeset.cast(data, params, [:name, :email], User.__field_types__(), User.__table__(), User.__primary_key__())
```
6 args is too many for a clean API.

**Approach D (two-phase: Changeset.cast returns changeset with metadata built-in, RECOMMENDED):**
The simplest approach that matches the success criteria syntax is:
```
Changeset.cast(data, params, allowed_fields, field_types)
```
Where:
- `data` = existing record as Map (or empty map for insert)
- `params` = incoming params as Map
- `allowed_fields` = List<String> of permitted field names
- `field_types` = List<String> from `__field_types__()` for coercion rules

The table name and primary key are provided when calling Repo.insert/Repo.update, since those already take a table name. The changeset carries the changes + errors; Repo provides the routing.

**Wait -- the success criteria says `Changeset.cast(user, params, [:name, :email])`** with only 3 args. This means the Changeset must get field types from the data itself. But at runtime, `user` is a Map<String,String> with no type metadata.

**Resolution:** Accept 4 args at the runtime level, but make field_types optional. If field_types is an empty list (or null), no type coercion is performed -- all string values pass through as-is. This allows:
- `Changeset.cast(data, params, [:name, :email])` -- no coercion (3 args)
- `Changeset.cast(data, params, [:name, :email], User.__field_types__())` -- with coercion (4 args)

Both work. The type checker registers both overloads via different arities.

### Pattern 3: Error Accumulation (No Short-Circuit)

**What:** Each validator checks its condition. If the condition fails, it adds an error to the errors map. It never stops processing -- all validators run regardless of previous errors.

**Implementation:** Each validator:
1. Clones the changeset (allocate new 64-byte slot array, copy all slots)
2. Checks the condition on the relevant field in `changes`
3. If condition fails AND the field does not already have an error, adds a new entry to `errors` map
4. Updates `valid` flag based on whether `errors` is empty
5. Returns the new changeset

**Error message format:** Each error is a simple string associated with a field name.
- `validate_required(:name)` -> `"can't be blank"` on field "name"
- `validate_length(:name, min: 2)` -> `"should be at least 2 character(s)"` on field "name"
- `validate_format(:email, "@")` -> `"has invalid format"` on field "email"
- `validate_inclusion(:role, ["admin", "user"])` -> `"is invalid"` on field "role"
- `validate_number(:age, greater_than: 0)` -> `"must be greater than 0"` on field "age"

### Pattern 4: Validator Function Signatures

**What:** Validators are `Changeset` module functions that follow pipe-chain conventions.

**Signatures (at Mesh level):**
```
Changeset.validate_required(changeset, fields :: List<Atom>) -> Changeset
Changeset.validate_length(changeset, field :: Atom, opts :: Map<String,String>) -> Changeset
Changeset.validate_format(changeset, field :: Atom, pattern :: String) -> Changeset
Changeset.validate_inclusion(changeset, field :: Atom, values :: List<String>) -> Changeset
Changeset.validate_number(changeset, field :: Atom, opts :: Map<String,String>) -> Changeset
```

**Keyword args for opts:** Phase 96-02 established that keyword arguments desugar to Map literals. So:
- `validate_length(:name, min: 2, max: 100)` -> `validate_length(:name, %{"min" => "2", "max" => "100"})`
- `validate_number(:age, greater_than: 0)` -> `validate_number(:age, %{"greater_than" => "0"})`

**CRITICAL:** Keyword args desugar keys to strings and values follow standard expression parsing. `min: 2` becomes `%{"min" => 2}`. But the Map is Map<String, T> -- since 2 is an Int, we need the map to work with mixed types.

**Resolution:** At the runtime level, opts arrive as a Map<String, String> (keyword values are converted to strings by the pipe-chain string key detection from Phase 96-05) OR as Map<String, Ptr>. The safest approach: the runtime reads the map entries and parses values as strings. Since atoms lower to strings and integers lower to i64, the runtime should handle both representations. However, the simplest approach matching existing patterns: validators accept keyword arg maps where values are strings. `min: 2` means the value is the integer 2, but read as a string "2" at runtime is fragile.

**Better approach: separate opts params per validator:**
```
Changeset.validate_length(changeset, field, min, max, is)
```
With -1 meaning "not set" for unused opts. This avoids the map type complexity.

**RECOMMENDED: Use keyword args for ergonomics but parse them correctly at runtime.**
The keyword arg map has string keys and values that are either strings (from quoted values) or integers (from numeric literals). Since the map stores `u64` values and strings are `*mut u8` pointers while integers are raw i64 values, the runtime cannot reliably distinguish them.

**FINAL DECISION: Use explicit parameters, not opts maps.**

For `validate_length`:
- `Changeset.validate_length(changeset, field, min, max)` -- 4 args, min/max as Int, -1 = not set

For `validate_number`:
- `Changeset.validate_number(changeset, field, gt, lt, gte, lte)` -- 6 args, each Int, -1 = not set

This avoids the map type ambiguity entirely and is simpler at runtime.

**Alternative for validate_length with `is` (exact match):**
- `Changeset.validate_length_is(changeset, field, exact_length)` -- separate function

### Pattern 5: PG Constraint Error Mapping

**What:** When Repo.insert/update encounters a PostgreSQL error, parse the ErrorResponse fields to extract constraint information and map it to a human-readable changeset error.

**PostgreSQL ErrorResponse fields (RFC 7.4.2):**
- `S` = Severity (ERROR, FATAL, PANIC)
- `V` = Severity (non-localized)
- `C` = SQLSTATE code (5 characters)
- `M` = Message (human-readable)
- `D` = Detail (optional extra info)
- `H` = Hint
- `n` = Constraint name
- `t` = Table name
- `c` = Column name
- `s` = Schema name

**Relevant SQLSTATE codes:**
- `23505` = unique_violation
- `23503` = foreign_key_violation
- `23502` = not_null_violation
- `23514` = check_violation
- `23P01` = exclusion_violation

**Constraint name convention for field mapping:**
PostgreSQL constraint names typically follow patterns:
- Unique index: `users_email_key` or `users_pkey` (table_column_key)
- Foreign key: `posts_user_id_fkey` (table_column_fkey)

The runtime can parse the constraint name to extract the field: strip the table prefix and the `_key`/`_fkey`/`_pkey` suffix. Example:
- `users_email_key` -> field: `email`, message: `"has already been taken"`
- `posts_user_id_fkey` -> field: `user_id`, message: `"does not exist"`

**Implementation approach:**
1. Enhance `parse_error_response()` in `pg.rs` to return a structured error (or multiple fields) instead of just the message string.
2. Add a new function or modify the existing error path to extract 'C' (SQLSTATE), 'n' (constraint name), 't' (table name), 'c' (column name).
3. In `mesh_repo_insert`/`mesh_repo_update`, when the query returns an Err, check if the error string starts with a known SQLSTATE prefix or parse structured error info.
4. Map the constraint violation to a changeset error on the appropriate field.

**Simpler approach (RECOMMENDED):** Rather than changing the entire PG error protocol, enhance `parse_error_response` to return a structured string that encodes all fields. For example: `"23505:users_email_key:duplicate key value violates unique constraint \"users_email_key\""`. The Repo changeset integration layer parses this structured error string. This avoids changing the PgConn/Pool API signatures (which are used by many callers).

### Pattern 6: Repo Changeset Integration

**What:** Repo.insert and Repo.update gain changeset overloads. When called with a Changeset, they:
1. Check `changeset.valid` -- if false, return Err(changeset) immediately without executing SQL
2. Extract `changes` map for the SQL operation
3. Execute SQL
4. On success: return Ok(row)
5. On PG constraint error: parse constraint, add error to changeset, return Err(changeset_with_error)

**New function signatures:**
```
Repo.insert_changeset(pool, table, changeset) -> Result<Map<String,String>, Ptr>
Repo.update_changeset(pool, table, id, changeset) -> Result<Map<String,String>, Ptr>
```

The Err variant carries the changeset pointer (with errors attached) rather than a plain error string. This is a departure from the existing `Result<Map, String>` pattern but matches the success criteria requirement of `Result<T, Changeset>`.

**Type implications:** The MeshResult struct uses tag 0 = Ok, tag 1 = Err. The `value` field is `*mut u8` in both cases. For changeset operations:
- Ok: value = Map<String,String> (the inserted/updated row)
- Err: value = Changeset opaque pointer (with errors attached)

This works because both are `*mut u8` (Ptr). The caller pattern-matches on Ok/Err and knows the payload type from context. The type checker registers the return as `Ptr` (opaque), matching both cases.

### Pattern 7: Changeset Field Accessors

**What:** Mesh code needs to inspect changeset state (errors, valid flag, changes). Since the Changeset is opaque, field access requires runtime functions.

**Accessor functions:**
```
Changeset.valid(changeset) -> Bool       -- check if changeset has no errors
Changeset.errors(changeset) -> Map       -- get errors map
Changeset.changes(changeset) -> Map      -- get changes map
Changeset.get_change(changeset, field) -> String  -- get a specific change value
Changeset.get_error(changeset, field) -> String   -- get error for a field
```

### Anti-Patterns to Avoid

- **Changeset as a Mesh struct:** Do NOT define Changeset as a user-level Mesh struct. It must be an opaque Ptr managed by runtime, following the Query pattern. ABI mismatch between LLVM-compiled struct layouts and Rust `extern "C"` functions would cause crashes.
- **Short-circuiting validators:** Validators MUST NOT skip execution if the changeset is already invalid. All validators run, accumulating all errors. This is explicit in the success criteria.
- **Mutating Changeset in place:** Each validator returns a NEW Changeset. The original is never modified. This ensures pipe-chain semantics work correctly (each step in the pipe receives the result of the previous step).
- **Storing errors as List<String>:** Errors must be keyed by field name (Map<String,String>), not a flat list. This enables field-specific error display in UIs and constraint error mapping to specific fields.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| GC-safe opaque allocation | Manual malloc | `mesh_gc_alloc_actor(64, 8)` | GC traces allocations; manual malloc invisible to GC |
| String comparison in validators | Custom comparison | `mesh_string_eq()` or Rust `str::eq()` | Already handles encoding, null safety |
| Map operations for errors | Custom key-value store | `mesh_map_new_typed(1)` + `mesh_map_put()` | String-keyed maps already work, proven in PG query results |
| Integer tagged encoding | Manual bit manipulation | Direct i64 storage in slots | Changeset integer slots (valid, action) stored as raw i64, not tagged |
| String-to-number parsing | Custom parser | `str::parse::<i64>()` / `str::parse::<f64>()` | Rust stdlib handles all edge cases |

**Key insight:** The Changeset is a composition of existing runtime primitives (Map, List, String, GC allocation). No new data structures are needed -- just orchestration.

## Common Pitfalls

### Pitfall 1: Changeset Slot Layout Must Match Across changeset.rs and repo.rs

**What goes wrong:** If changeset.rs defines slots in one order and repo.rs reads them in a different order, data corruption occurs.
**Why it happens:** Slot indices are magic numbers scattered across files.
**How to avoid:** Define slot constants in changeset.rs and either make them `pub(crate)` for repo.rs to import, or define in a shared location. Phase 98 handled this by duplicating constants (query.rs and repo.rs both define SLOT_SOURCE etc.) -- this worked but is fragile. For Phase 99, share constants via `pub(crate)` exports.
**Warning signs:** Runtime reads wrong data from changeset slots; validators corrupt each other's state.

### Pitfall 2: Error Map Key Collision -- Only First Error Per Field

**What goes wrong:** Two validators both add an error for the same field, and the second overwrites the first.
**Why it happens:** `mesh_map_put` overwrites existing keys.
**How to avoid:** Before adding an error, check if the field already has an error using `mesh_map_has_key`. Only add the error if no error exists for that field yet. This matches Ecto's convention where the first validation failure for a field is the one that sticks.
**Warning signs:** validate_required error is overwritten by validate_length error for the same field.

### Pitfall 3: Type Coercion in Cast Fails Silently

**What goes wrong:** A string param value that cannot be coerced to the target type (e.g., "abc" to Int) is silently dropped or stored as-is, causing later SQL errors.
**Why it happens:** Cast coercion errors are not added to the changeset errors.
**How to avoid:** When coercion fails, add an error to the changeset: field -> "is invalid". The field is NOT added to `changes`. Only successfully coerced values enter the changes map.
**Warning signs:** SQL error "invalid input syntax for type integer" instead of changeset validation error.

### Pitfall 4: PG Error Parsing Returns Only Message Field

**What goes wrong:** `parse_error_response()` currently only extracts the 'M' field. Constraint mapping needs 'C' (SQLSTATE), 'n' (constraint name), and optionally 'D' (detail).
**Why it happens:** Phase 54 implemented the minimal error parser for basic error reporting.
**How to avoid:** Enhance `parse_error_response()` to return a struct or a formatted string containing all relevant fields. The existing callers use it for plain error messages, so the change must be backward-compatible.
**Warning signs:** Constraint violations show raw "duplicate key value..." messages instead of field-specific changeset errors.

### Pitfall 5: validate_format Uses String.contains, Not Regex

**What goes wrong:** Developers expect regex pattern matching but the runtime only has `String.contains()`.
**Why it happens:** Mesh runtime has no regex engine. Adding one would be a significant dependency.
**How to avoid:** Document that `validate_format(:email, "@")` uses substring matching (String.contains), NOT regex. For the MVP, substring matching is sufficient. The success criteria example `validate_format(:email, "@")` only checks for "@" presence, which substring matching handles perfectly.
**Warning signs:** Developers expecting `validate_format(:email, "^[a-z]+@.*")` to work as regex.

### Pitfall 6: Changeset from Update vs Insert -- Different Semantics

**What goes wrong:** An update changeset includes only changed fields, but the original data is needed for validators that check existing values.
**Why it happens:** Cast for update should merge params into existing data, while cast for insert starts from scratch.
**How to avoid:** The `data` slot stores the original record. Validators can check both `changes` (new values) and `data` (original values). For `validate_required`, check if the field exists in `changes` OR in `data`. For a field to be "present", it must exist in `changes` (if cast allowed it) or in `data` (for unchanged fields).
**Warning signs:** Update changeset fails validate_required for fields that weren't changed but exist in the original record.

## Code Examples

### Changeset Slot Layout and Allocation

```rust
// In crates/mesh-rt/src/db/changeset.rs

const CS_SLOTS: usize = 8;
const CS_SIZE: usize = CS_SLOTS * 8; // 64 bytes

pub(crate) const SLOT_DATA: usize = 0;
pub(crate) const SLOT_CHANGES: usize = 1;
pub(crate) const SLOT_ERRORS: usize = 2;
pub(crate) const SLOT_VALID: usize = 3;
pub(crate) const SLOT_FIELD_TYPES: usize = 4;
pub(crate) const SLOT_TABLE: usize = 5;
pub(crate) const SLOT_PK: usize = 6;
pub(crate) const SLOT_ACTION: usize = 7;

unsafe fn alloc_changeset() -> *mut u8 {
    let cs = mesh_gc_alloc_actor(CS_SIZE as u64, 8);
    std::ptr::write_bytes(cs, 0, CS_SIZE);
    // Initialize maps to empty
    cs_set(cs, SLOT_DATA, mesh_map_new_typed(1));       // string-keyed
    cs_set(cs, SLOT_CHANGES, mesh_map_new_typed(1));     // string-keyed
    cs_set(cs, SLOT_ERRORS, mesh_map_new_typed(1));      // string-keyed
    cs_set_int(cs, SLOT_VALID, 1);                       // valid until proven otherwise
    cs_set(cs, SLOT_FIELD_TYPES, mesh_list_new());       // empty list
    cs_set_int(cs, SLOT_ACTION, 0);                      // insert by default
    cs
}

unsafe fn clone_changeset(src: *mut u8) -> *mut u8 {
    let dst = mesh_gc_alloc_actor(CS_SIZE as u64, 8);
    std::ptr::copy_nonoverlapping(src, dst, CS_SIZE);
    dst
}
```

### Changeset.cast Implementation

```rust
#[no_mangle]
pub extern "C" fn mesh_changeset_cast(
    data: *mut u8,       // Map<String,String> -- existing record data
    params: *mut u8,     // Map<String,String> -- incoming params
    allowed: *mut u8,    // List<String> -- allowed field names
    field_types: *mut u8,// List<String> -- "field:SQL_TYPE" (can be empty list)
) -> *mut u8 {
    unsafe {
        let cs = alloc_changeset();
        cs_set(cs, SLOT_DATA, data);
        cs_set(cs, SLOT_FIELD_TYPES, field_types);

        // Build field_type_map for coercion lookup
        let ft_entries = list_to_strings(field_types);
        let type_map: HashMap<String, String> = ft_entries.iter()
            .filter_map(|entry| {
                let parts: Vec<&str> = entry.splitn(2, ':').collect();
                if parts.len() == 2 { Some((parts[0].to_string(), parts[1].to_string())) }
                else { None }
            })
            .collect();

        // Filter params to allowed fields and coerce types
        let allowed_names = list_to_strings(allowed);
        let mut changes_map = mesh_map_new_typed(1);
        let mut errors_map = mesh_map_new_typed(1);

        for field_name in &allowed_names {
            let key_mesh = rust_str_to_mesh(field_name);
            if mesh_map_has_key(params, key_mesh as u64) != 0 {
                let val = mesh_map_get(params, key_mesh as u64);
                let val_str = mesh_str_ref(val as *mut u8);

                // Type coercion based on field_types
                if let Some(sql_type) = type_map.get(field_name) {
                    match coerce_value(val_str, sql_type) {
                        Ok(coerced) => {
                            let coerced_mesh = rust_str_to_mesh(&coerced);
                            changes_map = mesh_map_put(changes_map, key_mesh as u64, coerced_mesh as u64);
                        }
                        Err(_) => {
                            let err_msg = rust_str_to_mesh("is invalid");
                            errors_map = mesh_map_put(errors_map, key_mesh as u64, err_msg as u64);
                        }
                    }
                } else {
                    // No type info -- pass through as-is
                    changes_map = mesh_map_put(changes_map, key_mesh as u64, val);
                }
            }
        }

        cs_set(cs, SLOT_CHANGES, changes_map);
        cs_set(cs, SLOT_ERRORS, errors_map);
        // valid = errors is empty
        let has_errors = mesh_map_length(errors_map) > 0;
        cs_set_int(cs, SLOT_VALID, if has_errors { 0 } else { 1 });
        cs
    }
}

fn coerce_value(val: &str, sql_type: &str) -> Result<String, ()> {
    match sql_type {
        "TEXT" => Ok(val.to_string()),
        "BIGINT" => val.trim().parse::<i64>().map(|v| v.to_string()).map_err(|_| ()),
        "DOUBLE PRECISION" => val.trim().parse::<f64>().map(|v| v.to_string()).map_err(|_| ()),
        "BOOLEAN" => match val.to_lowercase().as_str() {
            "true" | "t" | "1" | "yes" => Ok("true".to_string()),
            "false" | "f" | "0" | "no" => Ok("false".to_string()),
            _ => Err(()),
        },
        _ => Ok(val.to_string()), // unknown type -- pass through
    }
}
```

### Validator Example: validate_required

```rust
#[no_mangle]
pub extern "C" fn mesh_changeset_validate_required(
    cs: *mut u8,         // Changeset
    fields: *mut u8,     // List<String> -- required field names
) -> *mut u8 {
    unsafe {
        let new_cs = clone_changeset(cs);
        let field_names = list_to_strings(fields);
        let changes = cs_get(new_cs, SLOT_CHANGES);
        let data = cs_get(new_cs, SLOT_DATA);
        let mut errors = cs_get(new_cs, SLOT_ERRORS);

        for field in &field_names {
            let key_mesh = rust_str_to_mesh(field);
            let key_u64 = key_mesh as u64;

            // Check if field has a value in changes or data
            let has_in_changes = mesh_map_has_key(changes, key_u64) != 0;
            let has_in_data = mesh_map_has_key(data, key_u64) != 0;

            let is_present = if has_in_changes {
                let val = mesh_map_get(changes, key_u64);
                let s = mesh_str_ref(val as *mut u8);
                !s.is_empty()
            } else if has_in_data {
                let val = mesh_map_get(data, key_u64);
                let s = mesh_str_ref(val as *mut u8);
                !s.is_empty()
            } else {
                false
            };

            if !is_present {
                // Only add error if no error exists for this field yet
                if mesh_map_has_key(errors, key_u64) == 0 {
                    let msg = rust_str_to_mesh("can't be blank");
                    errors = mesh_map_put(errors, key_u64, msg as u64);
                }
            }
        }

        cs_set(new_cs, SLOT_ERRORS, errors);
        cs_set_int(new_cs, SLOT_VALID, if mesh_map_length(errors) > 0 { 0 } else { 1 });
        new_cs
    }
}
```

### Enhanced PG Error Response Parsing

```rust
// In pg.rs, replace parse_error_response with a richer version:

struct PgError {
    sqlstate: String,       // 'C' field
    message: String,        // 'M' field
    detail: Option<String>, // 'D' field
    constraint: Option<String>, // 'n' field
    table: Option<String>,  // 't' field
    column: Option<String>, // 'c' field
}

fn parse_error_response_full(body: &[u8]) -> PgError {
    let mut error = PgError {
        sqlstate: String::new(),
        message: "unknown PostgreSQL error".to_string(),
        detail: None,
        constraint: None,
        table: None,
        column: None,
    };
    let mut i = 0;
    while i < body.len() {
        let field_type = body[i];
        i += 1;
        if field_type == 0 { break; }
        let start = i;
        while i < body.len() && body[i] != 0 { i += 1; }
        let value = String::from_utf8_lossy(&body[start..i]).into_owned();
        match field_type {
            b'C' => error.sqlstate = value,
            b'M' => error.message = value,
            b'D' => error.detail = Some(value),
            b'n' => error.constraint = Some(value),
            b't' => error.table = Some(value),
            b'c' => error.column = Some(value),
            _ => {}
        }
        i += 1; // skip null terminator
    }
    error
}
```

### Constraint Error Mapping

```rust
// In changeset.rs or repo.rs:

fn map_constraint_to_changeset_error(
    pg_error: &PgError,
    table_name: &str,
) -> Option<(String, String)> {
    match pg_error.sqlstate.as_str() {
        "23505" => {
            // Unique violation
            let field = extract_field_from_constraint(
                pg_error.constraint.as_deref()?,
                table_name,
            )?;
            Some((field, "has already been taken".to_string()))
        }
        "23503" => {
            // Foreign key violation
            let field = extract_field_from_constraint(
                pg_error.constraint.as_deref()?,
                table_name,
            )?;
            Some((field, "does not exist".to_string()))
        }
        "23502" => {
            // Not null violation
            let col = pg_error.column.as_deref()?;
            Some((col.to_string(), "can't be blank".to_string()))
        }
        _ => None,
    }
}

fn extract_field_from_constraint(constraint_name: &str, table_name: &str) -> Option<String> {
    // Common PostgreSQL constraint naming conventions:
    // "users_email_key" -> strip "users_" prefix and "_key" suffix -> "email"
    // "posts_user_id_fkey" -> strip "posts_" prefix and "_fkey" suffix -> "user_id"
    // "users_pkey" -> strip "users_" prefix and "_pkey" suffix -> "" (primary key)
    let prefixed = format!("{}_", table_name);
    let without_prefix = constraint_name.strip_prefix(&prefixed)?;
    let field = without_prefix
        .strip_suffix("_key")
        .or_else(|| without_prefix.strip_suffix("_fkey"))
        .or_else(|| without_prefix.strip_suffix("_pkey"))
        .or_else(|| without_prefix.strip_suffix("_check"))
        .unwrap_or(without_prefix);
    if field.is_empty() { None } else { Some(field.to_string()) }
}
```

### Type Checker Registration

```rust
// In infer.rs, inside build_stdlib_modules():

// ── Changeset module (Phase 99) ──────────────────────────
{
    let ptr_t = Ty::Con(TyCon::new("Ptr"));
    let atom_t = Ty::Con(TyCon::new("Atom"));
    let mut cs_mod = HashMap::new();

    // Changeset.cast(Ptr, Ptr, Ptr) -> Ptr  (data, params, allowed -> changeset)
    cs_mod.insert("cast".to_string(), Scheme::mono(Ty::fun(
        vec![ptr_t.clone(), ptr_t.clone(), ptr_t.clone()],
        ptr_t.clone(),
    )));
    // Changeset.cast(Ptr, Ptr, Ptr, Ptr) -> Ptr  (data, params, allowed, field_types -> changeset)
    // NOTE: overload by arity (4-arg version with field_types)
    // Since Mesh doesn't support same-name different-arity, use cast_with_types
    // OR register only the 4-arg version and let 3-arg callers pass empty list

    // Changeset.validate_required(Ptr, Ptr) -> Ptr  (changeset, fields_list -> changeset)
    cs_mod.insert("validate_required".to_string(), Scheme::mono(Ty::fun(
        vec![ptr_t.clone(), ptr_t.clone()],
        ptr_t.clone(),
    )));
    // Changeset.validate_length(Ptr, Atom, Int, Int) -> Ptr  (changeset, field, min, max)
    cs_mod.insert("validate_length".to_string(), Scheme::mono(Ty::fun(
        vec![ptr_t.clone(), atom_t.clone(), Ty::int(), Ty::int()],
        ptr_t.clone(),
    )));
    // Changeset.validate_format(Ptr, Atom, String) -> Ptr  (changeset, field, pattern)
    cs_mod.insert("validate_format".to_string(), Scheme::mono(Ty::fun(
        vec![ptr_t.clone(), atom_t.clone(), Ty::string()],
        ptr_t.clone(),
    )));
    // Changeset.validate_inclusion(Ptr, Atom, Ptr) -> Ptr  (changeset, field, values_list)
    cs_mod.insert("validate_inclusion".to_string(), Scheme::mono(Ty::fun(
        vec![ptr_t.clone(), atom_t.clone(), ptr_t.clone()],
        ptr_t.clone(),
    )));
    // Changeset.validate_number(Ptr, Atom, Int, Int, Int, Int) -> Ptr
    //   (changeset, field, gt, lt, gte, lte) -- -1 = not set
    cs_mod.insert("validate_number".to_string(), Scheme::mono(Ty::fun(
        vec![ptr_t.clone(), atom_t.clone(), Ty::int(), Ty::int(), Ty::int(), Ty::int()],
        ptr_t.clone(),
    )));
    // Changeset.valid(Ptr) -> Bool
    cs_mod.insert("valid".to_string(), Scheme::mono(Ty::fun(
        vec![ptr_t.clone()],
        Ty::bool(),
    )));
    // Changeset.errors(Ptr) -> Ptr  (changeset -> Map<String,String>)
    cs_mod.insert("errors".to_string(), Scheme::mono(Ty::fun(
        vec![ptr_t.clone()],
        ptr_t.clone(),
    )));
    // Changeset.changes(Ptr) -> Ptr  (changeset -> Map<String,String>)
    cs_mod.insert("changes".to_string(), Scheme::mono(Ty::fun(
        vec![ptr_t.clone()],
        ptr_t.clone(),
    )));

    modules.insert("Changeset".to_string(), cs_mod);
}
```

### E2E Test Example

```rust
#[test]
fn e2e_changeset_cast_and_validate() {
    let output = compile_and_run(r#"
import Changeset

fn main() do
  let data = %{}
  let params = %{"name" => "Al", "email" => "bad", "role" => "admin"}
  let cs = Changeset.cast(data, params, [:name, :email])
    |> Changeset.validate_required([:name, :email])
    |> Changeset.validate_length(:name, 3, -1)
    |> Changeset.validate_format(:email, "@")

  if Changeset.valid(cs) do
    println("valid")
  else
    println("invalid")
  end
end
"#);
    assert_eq!(output, "invalid\n");
}
```

## State of the Art

| Old Approach (Phase 98) | New Approach (Phase 99) | Impact |
|--------------------------|-------------------------|--------|
| Repo.insert takes raw Map<String,String> | Repo.insert_changeset takes validated Changeset | Data validation before SQL execution |
| PG errors return raw message string | PG errors parsed for SQLSTATE + constraint name | Human-readable field-specific error messages |
| No input validation layer | Pipe-chain validation pipeline | Consistent, composable data validation |
| No type coercion from external params | Changeset.cast coerces strings to schema types | Safe handling of form/API input data |

## Open Questions

1. **Changeset.cast arity -- 3 args or 4 args?**
   - What we know: Success criteria shows `Changeset.cast(user, params, [:name, :email])` (3 args). But type coercion requires field_types metadata.
   - What's unclear: Whether the 3-arg version should skip coercion or derive types from the data somehow.
   - Recommendation: Register a single 4-arg version `Changeset.cast(data, params, allowed, field_types)`. The user calls it as `Changeset.cast(%{}, params, [:name, :email], User.__field_types__())`. This is explicit and avoids magic. Alternatively, if a simpler 3-arg API is desired, the planner can add a `Changeset.cast/3` that takes (data, params, allowed) with no coercion.

2. **validate_length and validate_number opts -- explicit params vs keyword map?**
   - What we know: Success criteria shows `validate_length(:name, min: 2)` with keyword syntax.
   - What's unclear: Whether keyword args can be reliably read at runtime given the mixed-type Map issue.
   - Recommendation: Use explicit integer parameters. `validate_length(changeset, :name, 2, -1)` where 2 = min, -1 = no max. The planner should evaluate whether the keyword desugaring from Phase 96-02 can work here. If keyword args produce Map<String,Ptr> where values are properly typed at runtime, the opts map approach may work. Otherwise, explicit params are safer.

3. **Repo.insert changeset overload vs separate function?**
   - What we know: Success criteria says "Repo.insert and Repo.update accept Changeset structs".
   - What's unclear: Whether to overload existing Repo.insert (by arity) or create Repo.insert_changeset.
   - Recommendation: Create separate functions `Repo.insert_changeset` and `Repo.update_changeset`. Mesh does not support type-based overloading, only arity-based. Since Repo.insert already takes (pool, table, map) at 3 args, and a changeset version would be (pool, table, changeset) at 3 args, they would collide. Separate names avoid the collision.

4. **Enhanced PG error parsing -- backward compatibility?**
   - What we know: `parse_error_response()` currently returns a plain String. Many callers use this.
   - What's unclear: Whether to change the return type or add a new function.
   - Recommendation: Add `parse_error_response_full()` as a new function returning a struct. Keep `parse_error_response()` unchanged for backward compatibility by having it call the full parser and return just the message field. Only the Repo changeset functions call the full parser.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `crates/mesh-rt/src/db/query.rs` -- Query opaque Ptr pattern (slot layout, GC allocation, immutable clone-and-modify)
- Codebase analysis: `crates/mesh-rt/src/db/repo.rs` -- Repo write operations, map extraction, MeshResult usage
- Codebase analysis: `crates/mesh-rt/src/db/pg.rs` lines 544-566 -- `parse_error_response()` extracting only 'M' field
- Codebase analysis: `crates/mesh-rt/src/db/pg.rs` lines 934-937, 1068-1071 -- ErrorResponse handling in execute/query
- Codebase analysis: `crates/mesh-rt/src/db/orm.rs` -- SQL builder pure Rust functions, pub(crate) wrappers
- Codebase analysis: `crates/mesh-rt/src/db/row.rs` -- Type parsing (int, float, bool) patterns for coercion
- Codebase analysis: `crates/mesh-typeck/src/infer.rs` lines 1120-1177 -- Query/Repo module registration pattern
- Codebase analysis: `crates/mesh-typeck/src/infer.rs` lines 1183-1193 -- STDLIB_MODULE_NAMES array
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 879-899 -- known_functions for Repo
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 10106-10115 -- STDLIB_MODULES array
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 10354-10364 -- map_builtin_name for repo_*
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 4460-4604 -- generate_schema_metadata with __field_types__
- Codebase analysis: `crates/mesh-codegen/src/mir/types.rs` lines 67-106 -- resolve_con for Ptr, Atom type constructors
- Codebase analysis: `crates/mesh-rt/src/io.rs` -- MeshResult struct (tag/value), alloc_result pattern
- Codebase analysis: `crates/mesh-rt/src/gc.rs` lines 144-151 -- mesh_gc_alloc_actor
- Codebase analysis: `crates/mesh-rt/src/collections/map.rs` -- mesh_map_new_typed, mesh_map_put, mesh_map_get, mesh_map_has_key
- Codebase analysis: `crates/meshc/tests/e2e.rs` lines 3779-3827 -- existing ORM e2e test patterns
- Codebase analysis: `.planning/ROADMAP.md` lines 236-249 -- Phase 99 requirements and success criteria
- Codebase analysis: `.planning/STATE.md` -- accumulated decisions from Phases 96-98

### Secondary (MEDIUM confidence)
- PostgreSQL wire protocol: ErrorResponse field types (C/M/D/n/t/c) -- from PostgreSQL documentation and the existing parser structure in pg.rs

## Metadata

**Confidence breakdown:**
- Changeset struct design: HIGH -- Directly follows the Query opaque Ptr pattern established in Phase 98 with 13 slots. Changeset uses 8 slots with the same allocation/clone pattern.
- Validation pipeline: HIGH -- Each validator is a pure function: clone changeset, check condition, add error if needed, update valid flag, return. No complex control flow.
- Type coercion in cast: HIGH -- Reuses the same parsing patterns from row.rs (parse_int, parse_float, parse_bool). The field_types metadata from __field_types__() provides the SQL type for lookup.
- PG constraint mapping: MEDIUM -- The PG wire protocol ErrorResponse field format is well-documented, but the constraint name parsing (extracting field names from constraint names like "users_email_key") is heuristic-based and depends on PostgreSQL naming conventions.
- Repo changeset integration: HIGH -- Extends existing Repo.insert/update with a changeset variant. The MeshResult struct supports the Ok(Map)/Err(Changeset) pattern since both are Ptr.

**Research date:** 2026-02-16
**Valid until:** 2026-03-16 (compiler internals and runtime are stable, controlled by this project)
