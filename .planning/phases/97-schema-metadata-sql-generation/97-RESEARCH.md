# Phase 97: Schema Metadata + SQL Generation - Research

**Researched:** 2026-02-16
**Domain:** Mesh compiler codegen (deriving(Schema) enhancements) + Rust runtime (SQL query builder)
**Confidence:** HIGH

## Summary

Phase 97 extends the existing `deriving(Schema)` infrastructure from Phase 96 and adds a new runtime SQL generation module. Phase 96 already implemented the core Schema derive machinery: `__table__()` returns a naive lowercased+s pluralized name, `__fields__()` returns field name strings, `__primary_key__()` returns a hardcoded "id", and `__relationships__()` returns encoded relationship strings. Phase 97 must enhance this foundation in two distinct areas:

**Area 1 -- Compiler codegen enhancements (Plan 97-01):** Extend `generate_schema_metadata()` in `crates/mesh-codegen/src/mir/lower.rs` to produce richer metadata functions. This includes: (a) a `__field_types__()` function that returns field-to-SQL-type mappings (the field name and MIR type are already available in the `fields: &[(String, MirType)]` parameter), (b) schema options support for configurable table names and primary keys (requires parser changes to accept `@table "custom_name"` or similar option syntax before `end`), (c) timestamps support that auto-injects `inserted_at` and `updated_at` fields when `@timestamps true` is specified, and (d) per-field column accessor functions (`User.name_col()`) that return the column name string for type-safe query references. The type checker in `crates/mesh-typeck/src/infer.rs` must register these new functions.

**Area 2 -- Runtime SQL generation (Plan 97-02):** A new Rust module `crates/mesh-rt/src/db/orm.rs` providing `extern "C"` functions: `mesh_orm_build_select`, `mesh_orm_build_insert`, `mesh_orm_build_update`, `mesh_orm_build_delete`. These accept structured parameters (table name, field lists, conditions as List<String>) and produce parameterized SQL strings with `$1, $2, ...` placeholders and proper identifier quoting. These runtime functions will be called by Phase 98's Query Builder but must be designed now. They follow the same pattern as existing runtime functions (`mesh_pg_query`, `mesh_pool_open`, etc.) -- `extern "C"` functions declared in `intrinsics.rs` and called via MIR `Call` expressions.

**Primary recommendation:** Implement in two plans: compiler codegen first (field type metadata, schema options, timestamps, column accessors), then runtime SQL generation. Both changes are independent and testable -- compiler changes are validated via e2e tests, runtime changes via Rust unit tests.

## Standard Stack

### Core

| Crate | Location | Purpose | Relevance |
|-------|----------|---------|-----------|
| mesh-parser | crates/mesh-parser | Parse schema options | New `@option value` syntax inside struct bodies |
| mesh-typeck | crates/mesh-typeck | Register new Schema functions | `__field_types__()`, `__sql_types__()`, column accessors |
| mesh-codegen | crates/mesh-codegen | Generate MIR for enhanced metadata | Extend `generate_schema_metadata()` with new functions |
| mesh-rt | crates/mesh-rt | Runtime SQL builders | New `db/orm.rs` module with parameterized query generation |

### Supporting

| Library | Version | Purpose | When Used |
|---------|---------|---------|-----------|
| inkwell | 0.8.0 (LLVM 21.1) | LLVM IR generation | Declaring new runtime function intrinsics |
| insta | 1.46 | Snapshot testing | Verifying parser output for schema options |

## Architecture Patterns

### Existing Compiler Pipeline

```
Source text
    --> Lexer (mesh-lexer) --> Vec<Token>
    --> Parser (mesh-parser) --> rowan GreenNode (CST)
    --> Typed AST wrappers (mesh-parser/ast/) --> zero-cost views
    --> Type checker (mesh-typeck/infer.rs) --> TypeckResult
    --> MIR lowering (mesh-codegen/mir/lower.rs) --> MirModule
    --> LLVM codegen (mesh-codegen/codegen/) --> LLVM IR --> native binary
```

### Pattern 1: Schema Metadata Function Generation (Established in Phase 96)

**What:** `generate_schema_metadata()` in `lower.rs` (line 4354) creates synthetic `MirFunction` entries that return compile-time constants. Functions are registered in `known_functions` and callable via `StructName.__method__()` syntax.

**Current implementation:**
```rust
fn generate_schema_metadata(
    &mut self,
    name: &str,
    fields: &[(String, MirType)],       // <-- field types already available!
    relationships: &[RelationshipDecl],
) {
    // __table__() -> StringLit("users")
    // __fields__() -> ListLit of StringLit field names
    // __primary_key__() -> StringLit("id")
    // __relationships__() -> ListLit of "kind:name:target" strings
}
```

**Key insight:** The `fields` parameter already contains `(field_name, MirType)` tuples. Phase 97 extends this function to also emit:
- `__field_types__()` -> ListLit of "field_name:sql_type" strings (using MirType -> SQL type mapping)
- `__sql_types__()` -> ListLit of "field_name:mesh_type:sql_type" triples for the Query Builder
- Column accessor functions per field: `{StructName}____{field}_col__()` -> StringLit("field_name")

### Pattern 2: Type Checker Function Registration (Established)

**What:** Schema metadata functions are registered in `infer.rs` (line 2315) as module-level functions.

**Current:**
```rust
if derive_list.iter().any(|t| t == "Schema") {
    env.insert(format!("{}.__table__", name), Scheme::mono(Ty::fun(vec![], Ty::string())));
    env.insert(format!("{}.__fields__", name), Scheme::mono(Ty::fun(vec![], Ty::list(Ty::string()))));
    env.insert(format!("{}.__primary_key__", name), Scheme::mono(Ty::fun(vec![], Ty::string())));
    env.insert(format!("{}.__relationships__", name), Scheme::mono(Ty::fun(vec![], Ty::list(Ty::string()))));
}
```

**Extension:** Add registrations for `__field_types__`, `__sql_types__`, and per-field `{field}_col` functions.

### Pattern 3: Runtime Function Declaration (Established)

**What:** Runtime functions are declared in `intrinsics.rs` and implemented in `mesh-rt/src/`. The pattern is consistent across 96 phases.

**Steps for new runtime functions:**
1. Implement `extern "C"` function in `crates/mesh-rt/src/db/orm.rs`
2. Add `pub mod orm;` to `crates/mesh-rt/src/db/mod.rs`
3. Re-export from `crates/mesh-rt/src/lib.rs`
4. Declare in `crates/mesh-codegen/src/codegen/intrinsics.rs`
5. Register in `known_functions` in `crates/mesh-codegen/src/mir/lower.rs`
6. Unit test in `mesh-rt` and e2e test in `meshc`

### Pattern 4: Schema Option Syntax

**What:** Phase 97 needs schema-level options (configurable table name, primary key, timestamps). The struct body parser currently recognizes fields (`name :: Type`) and relationship declarations (`belongs_to :name, Type`).

**Recommended approach:** Add `@option_name value` syntax inside struct bodies, parsed as SCHEMA_OPTION nodes. This avoids adding keywords and follows the annotation pattern used in many languages.

**Alternative approach (simpler):** Use keyword-like declarations inside the struct body, similar to relationship declarations. E.g., `table "custom_table_name"` / `primary_key :uuid` / `timestamps true`. These are contextual identifiers (not keywords) -- the same mechanism as `belongs_to`, `has_many`, `has_one`.

**Recommended (simplest):** Use contextual identifiers matching the relationship pattern. The parser already handles contextual identifiers inside struct bodies (line 284-292 in items.rs). Add `"table"`, `"primary_key"`, and `"timestamps"` to the check:

```
struct User do
  table "users"
  primary_key :id
  timestamps true

  id :: String
  name :: String
  email :: String
  belongs_to :org, Organization
end deriving(Schema)
```

This is the least invasive approach: same parser mechanism, same AST pattern, no new syntax characters.

### Pattern 5: SQL Type Mapping

**What:** Mesh types need to map to PostgreSQL column types for the SQL generation layer.

**Mapping table (MirType -> SQL type):**
| MirType | SQL Type | Notes |
|---------|----------|-------|
| Int | BIGINT | i64 maps to 8-byte integer |
| Float | DOUBLE PRECISION | f64 maps to 8-byte float |
| Bool | BOOLEAN | |
| String | TEXT | Default string type in PG |
| Ptr (when Option) | type + NULL | Nullable column |
| Struct | (not directly mapped) | Foreign key reference |

This mapping is used by `__sql_types__()` and the SQL generation functions to produce correct DDL and parameterized queries.

### Anti-Patterns to Avoid

- **Complex map construction in MIR for metadata:** Phase 96 established that metadata uses encoded strings (`"kind:name:target"`) rather than map literals to avoid complex MIR. Phase 97 should follow this same pattern for field type metadata: `"field_name:mesh_type:sql_type"` as encoded strings in a ListLit.
- **Parsing schema options as keyword arguments:** While keyword args exist in the language (Phase 96), they are a function-call desugaring. Schema options are struct-level declarations, not function arguments. Use the contextual identifier pattern from relationships.
- **Runtime SQL generation without parameterization:** Always use `$1, $2, ...` placeholders. Never interpolate values into SQL strings. This is PostgreSQL's extended query protocol requirement and prevents SQL injection.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Identifier quoting | Manual string concatenation | Quote function wrapping names in `"..."` | Handles reserved words, special characters |
| Parameter numbering | Manual counter tracking | Sequential parameter index from field position | Consistent, no off-by-one errors |
| Type mapping | Ad-hoc match statements everywhere | Single `mir_type_to_sql_type()` function | One source of truth, easy to extend |
| MIR function registration | Manual boilerplate per function | Extend `generate_schema_metadata()` | Follows established pattern |

**Key insight:** The runtime SQL builders are string manipulation functions. They are NOT query builders (that is Phase 98). They produce raw SQL strings from structured inputs. Keep them simple -- they are utility functions.

## Common Pitfalls

### Pitfall 1: Schema Options Conflicting with Field Names

**What goes wrong:** If someone names a field `table` or `timestamps`, the parser might interpret it as a schema option instead of a field.
**Why it happens:** Both fields and schema options start with an IDENT token in the struct body.
**How to avoid:** Schema options should either: (a) require a specific sigil (`@table`), (b) appear BEFORE any field declarations (enforced by parser ordering), or (c) be disambiguated by what follows (field has `::`, option has a value). The recommendation is approach (b): schema options must come first, then fields, then relationships. The parser already enforces fields before relationships; extend to options before fields.
**Warning signs:** Parse error when a field name matches an option keyword.

### Pitfall 2: Timestamps Fields Not Included in __fields__() and from_row

**What goes wrong:** `timestamps true` auto-injects `inserted_at` and `updated_at` but these are not in the user-written field list, so `__fields__()` and `from_row` don't know about them.
**Why it happens:** The timestamp fields are synthetic -- they exist only in metadata, not in the AST.
**How to avoid:** When `timestamps true` is detected during MIR lowering, append `("inserted_at", MirType::String)` and `("updated_at", MirType::String)` to the fields vector BEFORE generating any metadata functions or the MirStructDef. This ensures `__fields__()`, `from_row`, and the struct layout all include the timestamp fields.
**Warning signs:** Runtime error "missing column: inserted_at" when querying a schema struct with timestamps.

### Pitfall 3: Column Accessor Name Collision with Existing Methods

**What goes wrong:** `User.id_col()` clashes if the user defines their own `id_col` function.
**Why it happens:** Column accessors are generated as module-level functions with the naming pattern `{StructName}.{field}_col`.
**How to avoid:** Use the same double-underscore mangling as other schema functions: `__id_col__()`. This follows the existing `__table__()`, `__fields__()` pattern and is unlikely to collide with user code.
**Warning signs:** Type checker error "function already defined" for column accessor names.

### Pitfall 4: SQL Injection in Runtime Query Builders

**What goes wrong:** Table names or column names containing SQL metacharacters cause syntax errors or injection.
**Why it happens:** Identifiers are not quoted in the generated SQL.
**How to avoid:** Always double-quote identifiers in generated SQL: `"users"."name"`. PostgreSQL double-quoted identifiers handle any valid identifier, including reserved words and special characters. Use a helper function `quote_ident(name: &str) -> String` that wraps in `"..."` and escapes any embedded `"` as `""`.
**Warning signs:** SQL error "syntax error at or near..." when table/column names are PostgreSQL reserved words.

### Pitfall 5: MirStructDef Field Count Mismatch with Timestamps

**What goes wrong:** The MirStructDef has N fields but the runtime struct expects N+2 fields (including timestamps), causing memory corruption.
**Why it happens:** Timestamp fields were added to metadata but not to the struct definition.
**How to avoid:** Timestamps must be injected at the field level (into the MirStructDef fields list), not just at the metadata level. The struct layout, codegen, from_row, and all metadata functions must agree on the field list. Inject timestamp fields early in `lower_struct_def()` before any derive processing.
**Warning signs:** LLVM segfault or incorrect GEP indices when accessing fields after timestamp fields.

### Pitfall 6: Schema Options Only Processed When deriving(Schema) Present

**What goes wrong:** Schema options (`table`, `primary_key`, `timestamps`) are parsed but have no effect without `deriving(Schema)`.
**Why it happens:** The parser accepts them in any struct body, but the codegen only processes them inside the Schema derive branch.
**How to avoid:** Two options: (a) Only parse schema options when deriving(Schema) is present (but deriving clause comes AFTER the body, so the parser doesn't know yet), or (b) parse them always but only process in codegen when Schema derive is detected. Option (b) is correct -- the parser is syntax-level, semantics are checked later. Add a warning/error in typeck if schema options appear without deriving(Schema).
**Warning signs:** Silent no-op when schema options are used without deriving(Schema).

## Code Examples

### Enhanced Schema Metadata Generation (MIR Lowering)

```rust
// In generate_schema_metadata(), after existing __table__() generation:

// ── __field_types__() ─────────────────────────────────────────
// Returns List<String> where each entry is "field_name:sql_type".
let field_type_elements: Vec<MirExpr> = fields
    .iter()
    .map(|(fname, fty)| {
        let sql_type = mir_type_to_sql_type(fty);
        MirExpr::StringLit(
            format!("{}:{}", fname, sql_type),
            MirType::String,
        )
    })
    .collect();
let field_types_fn_name = format!("{}____field_types__", name);
self.functions.push(MirFunction {
    name: field_types_fn_name.clone(),
    params: vec![],
    return_type: MirType::Ptr,
    body: MirExpr::ListLit {
        elements: field_type_elements,
        ty: MirType::Ptr,
    },
    is_closure_fn: false,
    captures: vec![],
    has_tail_calls: false,
});
```

### MIR Type to SQL Type Mapping

```rust
fn mir_type_to_sql_type(ty: &MirType) -> &'static str {
    match ty {
        MirType::Int => "BIGINT",
        MirType::Float => "DOUBLE PRECISION",
        MirType::Bool => "BOOLEAN",
        MirType::String => "TEXT",
        MirType::SumType(s) if s.starts_with("Option_") => {
            // For Option<T>, return the inner type (nullable)
            let inner = s.strip_prefix("Option_").unwrap_or("String");
            match inner {
                "Int" => "BIGINT",
                "Float" => "DOUBLE PRECISION",
                "Bool" => "BOOLEAN",
                _ => "TEXT",
            }
        }
        _ => "TEXT", // Default fallback
    }
}
```

### Column Accessor Functions

```rust
// Per-field column accessor: User.__name_col__() -> "name"
for (fname, _fty) in fields {
    let col_fn_name = format!("{}____{}_col__", name, fname);
    self.functions.push(MirFunction {
        name: col_fn_name.clone(),
        params: vec![],
        return_type: MirType::String,
        body: MirExpr::StringLit(fname.clone(), MirType::String),
        is_closure_fn: false,
        captures: vec![],
        has_tail_calls: false,
    });
    self.known_functions.insert(
        col_fn_name,
        MirType::FnPtr(vec![], Box::new(MirType::String)),
    );
}
```

### Runtime SQL Builder (mesh-rt/src/db/orm.rs)

```rust
use crate::collections::list::{mesh_list_get, mesh_list_length};
use crate::string::{mesh_string_new, MeshString};

/// Quote a SQL identifier with double quotes (PostgreSQL convention).
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Build a SELECT query.
///
/// mesh_orm_build_select(table: *const MeshString, columns: *mut u8,
///     where_clauses: *mut u8, order_by: *mut u8,
///     limit: i64, offset: i64) -> *mut u8 (String)
///
/// columns: List<String> of column names
/// where_clauses: List<String> of "column_name op" entries (values are parameterized)
/// order_by: List<String> of "column_name direction" entries
#[no_mangle]
pub extern "C" fn mesh_orm_build_select(
    table: *const MeshString,
    columns: *mut u8,
    where_clauses: *mut u8,
    order_by: *mut u8,
    limit: i64,
    offset: i64,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        // Build SELECT column_list FROM "table"
        // ... with WHERE, ORDER BY, LIMIT, OFFSET as parameterized SQL
    }
}
```

### Schema Options -- Parser Extension

```rust
// In parse_struct_def(), inside the field/relationship loop:
// Check for schema option declarations: table, primary_key, timestamps
if p.at(SyntaxKind::IDENT) {
    let text = p.current_text().to_string();
    match text.as_str() {
        "belongs_to" | "has_many" | "has_one" => {
            parse_relationship_decl(p);
            continue;
        }
        "table" | "primary_key" | "timestamps" => {
            parse_schema_option(p);
            continue;
        }
        _ => {}
    }
}
// Fall through to parse_struct_field(p);
```

### Timestamp Field Injection

```rust
// In lower_struct_def(), when processing Schema derive:
if derive_list.iter().any(|t| t == "Schema") {
    let relationships = struct_def.relationships();
    let has_timestamps = /* check for timestamps option */;

    let mut schema_fields = fields.clone();
    if has_timestamps {
        schema_fields.push(("inserted_at".to_string(), MirType::String));
        schema_fields.push(("updated_at".to_string(), MirType::String));
    }

    self.generate_schema_metadata(&name, &schema_fields, &relationships, &options);
}
```

### E2E Test Examples

```rust
#[test]
fn e2e_schema_field_types() {
    let output = compile_and_run(r#"
struct User do
  id :: String
  name :: String
  age :: Int
  active :: Bool
end deriving(Schema)

fn main() do
  let types = User.__field_types__()
  let t0 = List.get(types, 0)
  println(t0)
  let t2 = List.get(types, 2)
  println(t2)
end
"#);
    assert_eq!(output, "id:TEXT\nage:BIGINT\n");
}

#[test]
fn e2e_schema_column_accessor() {
    let output = compile_and_run(r#"
struct User do
  id :: String
  name :: String
end deriving(Schema)

fn main() do
  println(User.__name_col__())
end
"#);
    assert_eq!(output, "name\n");
}
```

## State of the Art

| Old Approach (Phase 96) | New Approach (Phase 97) | Impact |
|--------------------------|-------------------------|--------|
| `__table__()` returns naive lowercase+s | Configurable via `table "custom_name"` option | Handles irregular plurals and custom table names |
| `__primary_key__()` always returns "id" | Configurable via `primary_key :custom_pk` option | Supports non-standard primary key columns |
| No field type metadata | `__field_types__()` returns field-to-SQL-type mapping | Enables SQL generation with correct column types |
| No timestamp support | `timestamps true` auto-injects inserted_at/updated_at | Convention-over-configuration for audit timestamps |
| No column accessors | `User.__name_col__()` returns column name string | Type-safe column references (no raw string "name") |
| No SQL generation | Runtime `mesh_orm_build_*` functions | Parameterized query generation for Phase 98 |

## Open Questions

1. **Schema option syntax -- annotation vs contextual identifier**
   - What we know: The parser already handles contextual identifiers (`belongs_to`, `has_many`, `has_one`) inside struct bodies.
   - What's unclear: Should schema options use `@table "name"` (annotation syntax requiring lexer changes) or `table "name"` (contextual identifier, no lexer changes)?
   - Recommendation: Use contextual identifiers (`table "name"`, `primary_key :id`, `timestamps true`). This requires no lexer changes and follows the existing relationship declaration pattern. The parser check is a simple text comparison on IDENT tokens (already done for relationships at line 286 of items.rs).

2. **Timestamps as struct fields vs metadata-only**
   - What we know: Success criteria states "automatically include `inserted_at` and `updated_at` fields in their metadata and SQL generation."
   - What's unclear: Should timestamp fields be added to the actual struct runtime layout (accessible via `user.inserted_at`), or only in SQL generation metadata?
   - Recommendation: Add them to the struct layout. The fields must exist for `from_row` to populate them when querying. If they are metadata-only, queried rows cannot include timestamp values. However, they should only be included in INSERT/UPDATE SQL when the ORM manages them (not when the user explicitly sets them).

3. **SQL builder return type -- String vs structured type**
   - What we know: The runtime SQL builders need to return both a SQL string and a parameter list.
   - What's unclear: Should they return a single String (SQL only, caller manages params separately), or a Tuple/Struct with both?
   - Recommendation: Return a two-element result: the SQL string and a List<String> of ordered parameter values. Use a MeshResult-like pair or a Tuple. The simplest approach is to return a MeshList where element 0 is the SQL string ptr and element 1 is the params list ptr. Phase 98 will call these functions and use both values.

4. **Column accessor naming convention**
   - What we know: Success criteria says `User.name_col()`. Phase 96 established `__table__()` with double underscores.
   - What's unclear: Should column accessors use `User.name_col()` (no underscores, matching success criteria) or `User.__name_col__()` (matching existing pattern)?
   - Recommendation: Use `User.__name_col__()` to match the established `__table__()`, `__fields__()` pattern. This avoids collisions with user-defined methods. The MIR lowerer resolution at line 6246 already checks for `__table__` etc.; extend this check to match `*_col__` patterns.

5. **SQL builder scope for Phase 97 vs Phase 98**
   - What we know: Phase 97 builds the SQL generation runtime functions. Phase 98 builds the Mesh-level Query builder.
   - What's unclear: How much intelligence goes in the runtime SQL builders vs the Mesh-level Query builder?
   - Recommendation: Keep the runtime SQL builders as simple string-formatting functions. They take explicit inputs (table name, column list, conditions with operators, ordering specs) and produce a SQL string. All query composition logic (where clause building, join handling) lives in Phase 98's Mesh code. The runtime functions are a "SQL string template engine" -- they handle quoting, parameterization, and SQL dialect, but not query semantics.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 4347-4450 -- existing `generate_schema_metadata()` implementation, field type info available as `&[(String, MirType)]`
- Codebase analysis: `crates/mesh-typeck/src/infer.rs` lines 2315-2332 -- Schema function registration in type checker
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 6238-6254 -- MIR resolution for Schema metadata function calls
- Codebase analysis: `crates/mesh-parser/src/parser/items.rs` lines 246-320 -- struct body parser with relationship detection
- Codebase analysis: `crates/mesh-parser/src/ast/item.rs` lines 261-313 -- StructDef AST accessors
- Codebase analysis: `crates/mesh-rt/src/db/pg.rs` -- PG wire protocol, parameterized queries with $N placeholders
- Codebase analysis: `crates/mesh-rt/src/db/pool.rs` -- Pool.query/Pool.execute signatures
- Codebase analysis: `crates/mesh-rt/src/db/row.rs` -- from_row runtime functions (type parsing: int, float, bool, string)
- Codebase analysis: `crates/mesh-codegen/src/mir/mod.rs` -- MirType enum, MirExpr variants, MirFunction structure
- Codebase analysis: `crates/mesh-codegen/src/mir/types.rs` -- resolve_type() Ty -> MirType conversion
- Codebase analysis: `crates/mesh-codegen/src/codegen/intrinsics.rs` -- runtime function declaration pattern
- Codebase analysis: `mesher/storage/queries.mpl` -- real-world raw SQL patterns that ORM will replace
- Codebase analysis: `mesher/storage/schema.mpl` -- DDL patterns showing PostgreSQL type conventions
- Codebase analysis: `mesher/types/user.mpl` -- current struct definitions with deriving(Json, Row)
- Codebase analysis: `.planning/ROADMAP.md` -- Phase 97 requirements and success criteria
- Codebase analysis: `.planning/REQUIREMENTS.md` -- SCHM-01 through SCHM-05 requirement definitions
- Codebase analysis: `.planning/STATE.md` -- Phase 96 decisions relevant to Phase 97

### Secondary (MEDIUM confidence)
- Phase 96 research: `.planning/phases/096-compiler-additions/96-RESEARCH.md` -- compiler pipeline patterns
- Phase 96-04 plan: `.planning/phases/096-compiler-additions/96-04-PLAN.md` -- Schema derive implementation details

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- This is the existing Mesh compiler and runtime, fully understood through 96 shipped phases
- Architecture (compiler codegen): HIGH -- Extending an existing function (`generate_schema_metadata`) with the same pattern; field type info already available
- Architecture (runtime SQL): HIGH -- Following established pattern for runtime functions (extern "C", declared in intrinsics.rs, unit-tested in mesh-rt)
- Schema options (parser extension): HIGH -- Same contextual identifier mechanism as relationship declarations; no new syntax tokens needed
- SQL type mapping: HIGH -- Direct mapping from MirType variants to PostgreSQL type names; confirmed against mesher/storage/schema.mpl DDL patterns
- Timestamps injection: MEDIUM -- Field injection must propagate to MirStructDef, from_row, and all metadata; ordering of operations in lower_struct_def() is critical

**Research date:** 2026-02-16
**Valid until:** 2026-03-16 (compiler internals are stable, controlled by this project)
