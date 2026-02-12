# Phase 58: Struct-to-Row Mapping - Research

**Researched:** 2026-02-12
**Domain:** Compiler-generated database row mapping for Snow structs via `deriving(Row)`
**Confidence:** HIGH

## Summary

This phase adds `deriving(Row)` support for Snow structs, enabling automatic compile-time generation of a `from_row` function that converts a `Map<String, String>` database query result row into a typed struct instance. The implementation follows the established `deriving(Json)` infrastructure from Phase 49, which is the closest direct precedent: both generate synthetic MIR functions that iterate struct fields and call runtime helpers to extract and convert values.

The key technical distinction from JSON serde is the input format: JSON operates on a structured `SnowJson` tagged union, while Row operates on `Map<String, String>` where all values are text strings (the PostgreSQL wire protocol text format). This means `from_row` needs runtime string-to-type parsing functions (`snow_row_parse_int`, `snow_row_parse_float`, `snow_row_parse_bool`) that return `*mut SnowResult` (Ok with the parsed value, or Err with a descriptive error). For String fields, the value passes through directly. For `Option<T>` fields, empty strings (the NULL sentinel from `snow_pg_query`) map to `None`; for non-Option fields, empty strings produce an error.

The second deliverable, `Pg.query_as(conn, sql, params, from_row_fn)`, is a runtime function that combines query execution with row mapping. It calls `snow_pg_query` internally, then applies the `from_row_fn` callback to each row in the result list, returning `List<Result<T, String>>`. This avoids the user needing to manually iterate and map. The function accepts the `from_row_fn` as a function pointer argument (the generated `FromRow__from_row__StructName` function), following the same callback pattern used by `snow_list_map` and `snow_json_from_list`.

The compile-time validation (ROW-06) mirrors `is_json_serializable` but with a narrower set of "row-mappable" types: Int, Float, Bool, String, and `Option<T>` where T is one of these primitives. Nested structs, List, Map, and custom types are NOT row-mappable because `Map<String, String>` is a flat key-value structure with no nested objects.

**Primary recommendation:** Follow the `deriving(Json)` pattern exactly for `deriving(Row)`. The `generate_from_row_struct` function mirrors `generate_from_json_struct` but uses `snow_map_get` + `snow_row_parse_*` instead of `snow_json_object_get` + `snow_json_as_*`. No `to_row` (encoding) is needed -- Row is decode-only.

## Standard Stack

### Core

| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| `snow-rt/src/db/row.rs` (new) | Runtime | Row parsing functions (`snow_row_parse_int`, `snow_row_parse_float`, `snow_row_parse_bool`, `snow_row_from_row_get`) | New file for Row-specific runtime helpers; parallels `json.rs` |
| `snow-codegen/src/mir/lower.rs` | Compiler | MIR function generation for `FromRow__from_row__StructName` | Proven pattern: 7 existing deriving traits use identical infrastructure |
| `snow-typeck/src/infer.rs` | Compiler | Trait validation and registration for `Row` derive | Extends existing `valid_derives` list and trait impl registration |
| `snow-codegen/src/codegen/intrinsics.rs` | Compiler | LLVM function declarations for new runtime functions | Standard registration point for all extern "C" functions |

### Supporting

| Component | Location | Purpose | When to Use |
|-----------|----------|---------|-------------|
| `snow-rt/src/db/pg.rs` | Runtime | `snow_pg_query_as` runtime function | For ROW-05: `Pg.query_as(conn, sql, params, from_row_fn)` |
| `snow-rt/src/collections/map.rs` | Runtime | `snow_map_get`, `snow_map_has_key` | Reading column values from row maps |
| `snow-rt/src/io.rs` | Runtime | `SnowResult`, `alloc_result`, `snow_result_is_ok`, `snow_result_unwrap` | Error propagation in `from_row` |
| `snow-rt/src/option.rs` | Runtime | `SnowOption`, `alloc_option` | Option<T> field handling for NULL columns |

### No External Dependencies Needed

This phase requires zero new crate dependencies. All work uses existing infrastructure: the `Map<String, String>` from `snow_pg_query`, the `SnowResult` error handling, and the existing MIR/codegen pipeline.

## Architecture Patterns

### Recommended Project Structure

```
crates/snow-rt/src/db/
├── pg.rs                  # Existing PG wire protocol (add snow_pg_query_as)
├── pool.rs                # Existing connection pool
├── row.rs                 # NEW: Row parsing helpers
└── mod.rs                 # Add pub mod row;

crates/snow-codegen/src/mir/lower.rs   # Add generate_from_row_struct, emit_from_row_for_type
crates/snow-codegen/src/codegen/intrinsics.rs  # Add row runtime function declarations
crates/snow-typeck/src/infer.rs        # Add Row to valid_derives, is_row_mappable check
crates/snow-typeck/src/error.rs        # Add NonMappableField error variant
crates/snow-typeck/src/diagnostics.rs  # Add error message formatting
```

### Pattern 1: Three-Point Runtime Function Registration

**What:** Every new runtime function must be registered in exactly three places.
**When to use:** For all new Row runtime functions.

**Points:**
1. **`snow-rt/src/db/row.rs`**: `#[no_mangle] pub extern "C" fn snow_row_parse_int(...)` implementation
2. **`snow-codegen/src/codegen/intrinsics.rs`**: `module.add_function("snow_row_parse_int", ...)` LLVM declaration
3. **`snow-codegen/src/mir/lower.rs`**: `self.known_functions.insert("snow_row_parse_int", ...)` plus module name mapping in `map_builtin_name`

### Pattern 2: generate_from_row_struct (the core pattern)

**What:** Generate a synthetic `FromRow__from_row__StructName` MIR function that extracts fields from a Map<String, String> with nested Result propagation.
**When to use:** For each struct that has `deriving(Row)`.

**Reference:** `generate_from_json_struct` at `mir/lower.rs` lines 3671-3826.

**Structure:**
```rust
fn generate_from_row_struct(&mut self, name: &str, fields: &[(String, MirType)]) {
    let mangled = format!("FromRow__from_row__{}", name);
    // param: row :: Ptr (a Map<String, String>)
    // return: Ptr (a SnowResult -- Ok with heap struct, or Err with error string)

    // For each field (last to first, building nested If/Let):
    //   1. snow_row_from_row_get(row, "field_name") -> SnowResult (Ok=string, Err=missing)
    //   2. If Ok:
    //      a. For String fields: use the string value directly
    //      b. For Int fields: snow_row_parse_int(string_val) -> SnowResult
    //      c. For Float fields: snow_row_parse_float(string_val) -> SnowResult
    //      d. For Bool fields: snow_row_parse_bool(string_val) -> SnowResult
    //      e. For Option<T> fields: check empty string -> None, else parse inner
    //   3. If Err: propagate the error

    // Innermost: snow_alloc_result(0, StructLit with all field vars)
}
```

### Pattern 3: Row Field Extraction (snow_row_from_row_get)

**What:** Extract a column value from a `Map<String, String>` row by column name.
**Why separate function:** Unlike `snow_map_get` (which returns 0 for missing keys), this returns a `SnowResult` with descriptive error "missing column: <name>".

**Runtime function:**
```rust
#[no_mangle]
pub extern "C" fn snow_row_from_row_get(
    row: *mut u8,      // Map<String, String>
    col_name: *mut u8, // SnowString
) -> *mut u8 {
    // 1. Create a Snow string for lookup
    // 2. snow_map_has_key(row, col_name_u64)
    // 3. If found: snow_map_get(row, col_name_u64) -> Ok(value_string_ptr)
    // 4. If not found: Err("missing column: <name>")
    // Returns *mut SnowResult
}
```

### Pattern 4: NULL Handling for Option<T> Fields

**What:** Empty strings (the NULL sentinel) map to `None` for Option fields.
**When to use:** During `generate_from_row_struct` when field type is `SumType("Option_*")`.

**Logic:**
```
For Option<Int> field "age":
  let col_result = snow_row_from_row_get(row, "age")
  if is_ok(col_result):
    let col_str = unwrap(col_result)
    let str_len = snow_string_length(col_str)
    if str_len == 0:
      // NULL -> alloc_result(0, alloc_option(1, null))  // Ok(None)
    else:
      let parse_result = snow_row_parse_int(col_str)
      if is_ok(parse_result):
        let val = unwrap(parse_result)
        // alloc_result(0, alloc_option(0, val))  // Ok(Some(val))
      else:
        // propagate parse error
  else:
    // Missing column for Option field -> Ok(None) [missing = NULL]
```

For non-Option fields, both empty string AND missing column produce `Err("column 'age' is NULL but field is not Option")`.

### Pattern 5: Pg.query_as Runtime Function

**What:** A runtime function that combines query + row mapping.
**Signature:** `snow_pg_query_as(conn: u64, sql: ptr, params: ptr, from_row_fn: ptr) -> ptr`

**Runtime implementation:**
```rust
#[no_mangle]
pub extern "C" fn snow_pg_query_as(
    conn_handle: u64,
    sql: *const SnowString,
    params: *mut u8,
    from_row_fn: *mut u8,  // fn(row_map: *mut u8) -> *mut u8 (SnowResult)
) -> *mut u8 {
    // 1. Call snow_pg_query(conn_handle, sql, params) -> SnowResult<List<Map>, String>
    // 2. If Err: propagate
    // 3. If Ok: iterate list, apply from_row_fn to each map
    //    Result: List<SnowResult<T, String>>
    // 4. Return alloc_result(0, result_list)
}
```

**Return type:** `Result<List<Result<T, String>>, String>` -- the outer Result captures query errors, the inner Results capture per-row mapping errors.

### Pattern 6: StructName.from_row Resolution

**What:** `User.from_row(row_map)` resolves to `FromRow__from_row__User(row_map)`.
**How:** Follows the same pattern as `User.from_json(str)` -> `__json_decode__User(str)`.

In `lower_field_access` (lines 5429-5445), add a check:
```rust
if field == "from_row" {
    let fn_name = format!("FromRow__from_row__{}", base_name);
    if let Some(fn_ty) = self.known_functions.get(&fn_name).cloned() {
        return MirExpr::Var(fn_name, fn_ty);
    }
}
```

Unlike `from_json` which has a wrapper (`__json_decode__`) that chains `snow_json_parse + FromJson__from_json__`, `from_row` does NOT need a wrapper -- it takes the `Map<String, String>` directly. The user calls `User.from_row(row_map)` where `row_map` is already a `Map<String, String>`.

### Anti-Patterns to Avoid

- **Don't generate `to_row`:** Row mapping is decode-only. There's no database INSERT from struct fields in this phase (that would be ORM-level functionality, out of scope).
- **Don't support nested structs:** `Map<String, String>` is flat. A field of type `Address` in `User` cannot be populated from a flat row. This must be a compile error (ROW-06).
- **Don't confuse empty string with NULL at compile time:** The NULL distinction happens at runtime via empty string check. The compile-time check only validates types.
- **Don't modify `snow_pg_query`'s NULL representation:** Changing NULL from empty string to something else would break existing code. Accept the empty string convention.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| String-to-Int parsing | Custom parser | Existing `str::parse::<i64>()` in Rust runtime | Handles all edge cases (negatives, overflow, whitespace) |
| String-to-Float parsing | Custom parser | Existing `str::parse::<f64>()` in Rust runtime | Handles scientific notation, infinity, NaN |
| String-to-Bool parsing | Custom parser | Match "true"/"t"/"1"/"yes" / "false"/"f"/"0"/"no" | Matches PostgreSQL's boolean text representations |
| Map column lookup | Manual GEP into map internals | `snow_map_has_key` + `snow_map_get` from collections/map.rs | Map internals are opaque; must use public API |
| Error propagation | Custom error handling | `SnowResult` with `snow_result_is_ok` / `snow_result_unwrap` pattern from `generate_from_json_struct` | Proven MIR pattern for nested error propagation |
| List iteration for query_as | Manual pointer arithmetic | `snow_list_length` + `snow_list_get` + `snow_list_append` | List internals are opaque; must use public API |

**Key insight:** The `from_row` generation is almost identical to `from_json` generation. The differences are: (1) the extraction function (`snow_row_from_row_get` vs `snow_json_object_get`), (2) the type conversion functions (`snow_row_parse_int` vs `snow_json_as_int`), and (3) the input type (`Map<String, String>` vs `SnowJson*`). The MIR structure (nested If/Let chains with `snow_result_is_ok` guards) is identical.

## Common Pitfalls

### Pitfall 1: NULL Detection Must Use Empty String Check, Not Missing Key

**What goes wrong:** If `from_row` only checks for missing columns (via `snow_map_has_key`), it will miss NULL columns because `snow_pg_query` always inserts the column with an empty string value for NULL.
**Why it happens:** `snow_pg_query` (pg.rs lines 1043-1045) maps NULL to `String::new()` and still inserts the key-value pair into the map.
**How to avoid:** After extracting the string value, check if it's empty (`snow_string_length(val) == 0`). If empty AND field is `Option<T>`, return `Ok(None)`. If empty AND field is non-Option, return `Err("column 'name' is NULL but field is not Option")`.
**Warning signs:** Option fields receive `Some("")` instead of `None` for NULL database columns.

### Pitfall 2: Boolean Parsing Must Handle PostgreSQL Text Representations

**What goes wrong:** PostgreSQL returns booleans as text strings like "t", "f", "true", "false", "1", "0", "yes", "no". If the parser only handles "true"/"false", it will reject common PostgreSQL output.
**Why it happens:** The Extended Query protocol with text format returns whatever `::text` casting produces. PostgreSQL's boolean output is typically "t" or "f".
**How to avoid:** The `snow_row_parse_bool` runtime function should accept: "true", "t", "1", "yes" -> Ok(true); "false", "f", "0", "no" -> Ok(false); anything else -> Err("cannot parse '<val>' as Bool").
**Warning signs:** `from_row` fails on `active` column with "cannot parse 't' as Bool".

### Pitfall 3: Float Parsing Must Handle "NaN" and "Infinity"

**What goes wrong:** PostgreSQL can return "NaN", "Infinity", "-Infinity" as float text representations. Rust's `str::parse::<f64>()` handles "NaN", "inf", "-inf" but NOT "Infinity" / "-Infinity".
**Why it happens:** PostgreSQL uses its own text format.
**How to avoid:** In `snow_row_parse_float`, pre-process: if value is "Infinity", replace with "inf"; if "-Infinity", replace with "-inf". Then parse.
**Warning signs:** Parsing error on "Infinity" string from PostgreSQL float column.

### Pitfall 4: Row-Mappable Type Validation Must Be Strict (ROW-06)

**What goes wrong:** If the compile-time check allows nested structs, List<T>, Map<K,V>, or custom types, the generated `from_row` function will try to parse a flat string into a complex type, producing garbage or crashes.
**Why it happens:** `Map<String, String>` is inherently flat. A column "address" containing the string "123 Main St" cannot be deserialized into an `Address` struct.
**How to avoid:** The `is_row_mappable` function must be strict: only Int, Float, Bool, String, and `Option<T>` where T is one of these four. Reject Struct, SumType (except Option), List, Map, Ptr, and any other type.
**Warning signs:** Linker errors or crashes when trying to use `from_row` on a struct with nested types.

### Pitfall 5: Pg.query_as Must Accept from_row_fn as Function Pointer

**What goes wrong:** If `Pg.query_as` is implemented as a purely compile-time transformation (expanding `Pg.query_as(conn, sql, params, User.from_row)` into `Pg.query(conn, sql, params).map(User.from_row)`), it requires Map/List higher-order function support in the MIR.
**Why it happens:** Snow's function pointer / callback support has specific patterns (bare fn vs closure fn).
**How to avoid:** Implement as a runtime function that receives the `from_row_fn` as a bare function pointer (`extern "C" fn(*mut u8) -> *mut u8`). The MIR lowerer resolves `User.from_row` to `FromRow__from_row__User` and passes it as a function pointer argument. The runtime iterates the list and calls the function pointer for each row.
**Warning signs:** Type errors or segfaults when passing `User.from_row` as an argument.

### Pitfall 6: Pool.query_as Should Also Be Supported

**What goes wrong:** Users expect `Pool.query_as` alongside `Pg.query_as`, since Phase 57 introduced the Pool module.
**Why it happens:** The requirements say `Pg.query_as` but the connection pool is the recommended way to manage connections.
**How to avoid:** Add `snow_pool_query_as` as well, mirroring `snow_pool_query`. The implementation is identical to `snow_pg_query_as` but uses pool checkout/checkin internally.
**Warning signs:** Users forced to checkout from pool, call `Pg.query_as`, and checkin manually.

## Code Examples

### New Runtime Functions (snow-rt/src/db/row.rs)

```rust
// Source: Codebase analysis of pg.rs and json.rs patterns

use crate::collections::map::{snow_map_get, snow_map_has_key};
use crate::io::{alloc_result, SnowResult};
use crate::string::{snow_string_new, SnowString};

/// Get a column value from a row Map<String, String>.
/// Returns SnowResult: Ok = string value ptr, Err = "missing column: <name>".
#[no_mangle]
pub extern "C" fn snow_row_from_row_get(row: *mut u8, col_name: *mut u8) -> *mut u8 {
    unsafe {
        let col_key = col_name as u64;
        if snow_map_has_key(row, col_key) != 0 {
            let val = snow_map_get(row, col_key);
            alloc_result(0, val as *mut u8) as *mut u8
        } else {
            let name = &*(col_name as *const SnowString);
            let msg = format!("missing column: {}", name.as_str());
            let err_str = snow_string_new(msg.as_ptr(), msg.len() as u64) as *mut u8;
            alloc_result(1, err_str) as *mut u8
        }
    }
}

/// Parse a string to Int. Returns SnowResult<Int, String>.
#[no_mangle]
pub extern "C" fn snow_row_parse_int(s: *mut u8) -> *mut u8 {
    unsafe {
        let text = (*(s as *const SnowString)).as_str().trim();
        match text.parse::<i64>() {
            Ok(val) => alloc_result(0, val as *mut u8) as *mut u8,
            Err(_) => {
                let msg = format!("cannot parse '{}' as Int", text);
                let err_str = snow_string_new(msg.as_ptr(), msg.len() as u64) as *mut u8;
                alloc_result(1, err_str) as *mut u8
            }
        }
    }
}

/// Parse a string to Float. Returns SnowResult<Float, String>.
#[no_mangle]
pub extern "C" fn snow_row_parse_float(s: *mut u8) -> *mut u8 {
    unsafe {
        let text = (*(s as *const SnowString)).as_str().trim();
        // Handle PostgreSQL-specific representations
        let normalized = match text {
            "Infinity" => "inf",
            "-Infinity" => "-inf",
            other => other,
        };
        match normalized.parse::<f64>() {
            Ok(val) => alloc_result(0, f64::to_bits(val) as *mut u8) as *mut u8,
            Err(_) => {
                let msg = format!("cannot parse '{}' as Float", text);
                let err_str = snow_string_new(msg.as_ptr(), msg.len() as u64) as *mut u8;
                alloc_result(1, err_str) as *mut u8
            }
        }
    }
}

/// Parse a string to Bool. Returns SnowResult<Bool, String>.
/// Accepts PostgreSQL boolean text representations.
#[no_mangle]
pub extern "C" fn snow_row_parse_bool(s: *mut u8) -> *mut u8 {
    unsafe {
        let text = (*(s as *const SnowString)).as_str().trim().to_lowercase();
        match text.as_str() {
            "true" | "t" | "1" | "yes" => alloc_result(0, 1i64 as *mut u8) as *mut u8,
            "false" | "f" | "0" | "no" => alloc_result(0, 0i64 as *mut u8) as *mut u8,
            _ => {
                let msg = format!("cannot parse '{}' as Bool", text);
                let err_str = snow_string_new(msg.as_ptr(), msg.len() as u64) as *mut u8;
                alloc_result(1, err_str) as *mut u8
            }
        }
    }
}
```

### Typeck Deriving Registration

```rust
// In infer.rs, struct def processing (~line 1945):
let valid_derives = ["Eq", "Ord", "Display", "Debug", "Hash", "Json", "Row"];

// After Json registration (~line 2127), add:
if derive_list.iter().any(|t| t == "Row") {
    // Validate all fields are row-mappable BEFORE registering impls.
    let mut row_valid = true;
    for (field_name, field_ty) in &fields {
        if !is_row_mappable(field_ty) {
            ctx.errors.push(TypeError::NonMappableField {
                struct_name: name.clone(),
                field_name: field_name.clone(),
                field_type: format!("{}", field_ty),
            });
            row_valid = false;
        }
    }

    if row_valid {
        let mut from_row_methods = FxHashMap::default();
        from_row_methods.insert(
            "from_row".to_string(),
            ImplMethodSig {
                has_self: false,
                param_count: 1, // takes a Map<String, String>
                return_type: Some(Ty::result(
                    Ty::Con(TyCon::new(&name)),
                    Ty::string(),
                )),
            },
        );
        let _ = trait_registry.register_impl(TraitImplDef {
            trait_name: "FromRow".to_string(),
            impl_type: impl_ty.clone(),
            impl_type_name: name.clone(),
            methods: from_row_methods,
        });
    }
}
```

### is_row_mappable Check

```rust
/// Check if a type is row-mappable for deriving(Row) validation.
/// Row-mappable types: Int, Float, Bool, String, Option<T> where T is row-mappable.
/// NOT mappable: nested structs, sum types (except Option), List, Map, Ptr, custom types.
fn is_row_mappable(ty: &Ty) -> bool {
    match ty {
        Ty::Con(con) => matches!(con.name.as_str(), "Int" | "Float" | "Bool" | "String"),
        Ty::App(base, args) => {
            if let Ty::Con(con) = base.as_ref() {
                if con.name == "Option" {
                    args.first().map_or(false, |t| is_row_mappable(t))
                } else {
                    false
                }
            } else {
                false
            }
        }
        _ => false,
    }
}
```

### MIR Lower Deriving Dispatch

```rust
// In lower_struct_def (~line 1668), after Json block:
if derive_list.iter().any(|t| t == "Row") {
    self.generate_from_row_struct(&name, &fields);
}
```

### generate_from_row_struct Structure

```rust
fn generate_from_row_struct(&mut self, name: &str, fields: &[(String, MirType)]) {
    let mangled = format!("FromRow__from_row__{}", name);

    // Mirrors generate_from_json_struct exactly:
    // 1. Build innermost: alloc_result(0, StructLit { fields... })
    // 2. For each field (rev), wrap with:
    //    a. snow_row_from_row_get(row, "field_name") -> get_result
    //    b. if is_ok(get_result):
    //         let col_str = unwrap(get_result)
    //         [For String]: use col_str directly
    //         [For Int]: snow_row_parse_int(col_str) -> parse_result
    //         [For Float]: snow_row_parse_float(col_str) -> parse_result
    //         [For Bool]: snow_row_parse_bool(col_str) -> parse_result
    //         [For Option<T>]: check empty string -> None, else parse inner
    //       else: propagate get error

    let func = MirFunction {
        name: mangled.clone(),
        params: vec![("row".to_string(), MirType::Ptr)], // Map<String, String>
        return_type: MirType::Ptr, // *mut SnowResult
        body,
        is_closure_fn: false,
        captures: vec![],
        has_tail_calls: false,
    };
    self.functions.push(func);
    self.known_functions.insert(
        mangled,
        MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
    );
}
```

### Pg.query_as Runtime Function

```rust
// In snow-rt/src/db/pg.rs (or row.rs):
#[no_mangle]
pub extern "C" fn snow_pg_query_as(
    conn_handle: u64,
    sql: *const SnowString,
    params: *mut u8,
    from_row_fn: *mut u8,
) -> *mut u8 {
    type FromRowFn = unsafe extern "C" fn(*mut u8) -> *mut u8;

    unsafe {
        // 1. Call snow_pg_query
        let query_result = snow_pg_query(conn_handle, sql, params);
        let result = &*(query_result as *const SnowResult);
        if result.tag != 0 {
            return query_result; // Propagate query error
        }

        // 2. Got List<Map<String, String>>
        let rows_list = result.value;
        let len = snow_list_length(rows_list);
        let f: FromRowFn = std::mem::transmute(from_row_fn);

        // 3. Map each row through from_row_fn
        let mut result_list = snow_list_new();
        for i in 0..len {
            let row = snow_list_get(rows_list, i) as *mut u8;
            let mapped = f(row);
            result_list = snow_list_append(result_list, mapped as u64);
        }

        // 4. Return Ok(result_list)
        alloc_result(0, result_list) as *mut u8
    }
}
```

### E2E Test Example (Target Snow Code)

```snow
struct User do
  name :: String
  age :: Int
  score :: Float
  active :: Bool
end deriving(Row)

fn main() do
  let row = Map.new()
    |> Map.put("name", "Alice")
    |> Map.put("age", "30")
    |> Map.put("score", "95.5")
    |> Map.put("active", "t")

  let result = User.from_row(row)
  case result do
    Ok(u) -> println("${u.name} ${u.age} ${u.score} ${u.active}")
    Err(e) -> println("Error: ${e}")
  end
end
```

### E2E Test: Option Fields and NULL

```snow
struct Profile do
  name :: String
  bio :: Option<String>
  age :: Option<Int>
end deriving(Row)

fn main() do
  -- bio is NULL (empty string), age is present
  let row = Map.new()
    |> Map.put("name", "Bob")
    |> Map.put("bio", "")
    |> Map.put("age", "25")

  let result = Profile.from_row(row)
  case result do
    Ok(p) ->
      println(p.name)
      case p.bio do
        Some(b) -> println("bio: ${b}")
        None -> println("bio: none")
      end
    Err(e) -> println("Error: ${e}")
  end
end
```

### Compile-Fail Test: Non-Mappable Field

```snow
struct BadRow do
  name :: String
  tags :: List<String>
end deriving(Row)

fn main() do
  println("should not compile")
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual `Map.get(row, "col")` + parse | `deriving(Row)` auto-generated `from_row` | Phase 58 (this phase) | Users get type-safe row mapping without manual field extraction |
| `Pg.query(conn, sql, params)` returns `List<Map<String, String>>` | `Pg.query_as(conn, sql, params, from_row)` returns `List<Result<T, String>>` | Phase 58 (this phase) | One-step query + hydration |
| No compile-time validation | `deriving(Row)` compile error on non-mappable fields | Phase 58 (this phase) | Users get clear feedback on schema mismatches at compile time |

**Coexistence:** The existing `Pg.query` returning `List<Map<String, String>>` continues to work. `deriving(Row)` and `Pg.query_as` add a typed layer on top. Both APIs coexist.

## Open Questions

1. **Missing column vs NULL for Option fields**
   - What we know: ROW-04 says "NULL columns (empty string) map to None for Option fields." But what about a column that's literally missing from the row map (e.g., a SELECT that doesn't include all struct fields)?
   - What's unclear: Should missing columns also map to None for Option fields, or should they error?
   - Recommendation: Missing columns should also map to None for Option fields (be lenient), but error for non-Option fields with "missing column: <name>". This is more user-friendly when queries use `SELECT col1, col2` instead of `SELECT *`.

2. **Pool.query_as support**
   - What we know: Phase 57 added `Pool.query` and `Pool.execute`. Users would naturally expect `Pool.query_as`.
   - What's unclear: Should this be in Phase 58 scope or a later enhancement?
   - Recommendation: Include `Pool.query_as` in Phase 58 since the implementation is trivial (identical to `Pg.query_as` but using pool's internal checkout/query/checkin). It follows the same runtime function pattern.

3. **Pg.query_as return type: `Result<List<Result<T, String>>, String>` vs `Result<List<T>, String>`**
   - What we know: The requirement says `Pg.query_as` returns `List<Result<T, String>>`. This means each row can independently fail.
   - What's unclear: Should the outer Result be kept (for query-level errors), making it `Result<List<Result<T, String>>, String>`?
   - Recommendation: Yes, keep the outer Result. Query errors (connection issues, SQL syntax errors) produce the outer Err. Per-row mapping errors produce inner Err for individual rows. The outer Ok wraps the list of per-row results.

4. **String field NULL handling ambiguity**
   - What we know: Empty string is the NULL sentinel. A String field receiving empty string would get `""` as its value.
   - What's unclear: Should `""` for a non-Option String field be treated as NULL (error) or as a valid empty string?
   - Recommendation: For non-Option String fields, empty string is a valid value (not an error). Only non-Option Int/Float/Bool fields should error on empty string (since empty string cannot be parsed to these types anyway). This avoids breaking the case where a database column legitimately contains an empty string.

## Sources

### Primary (HIGH confidence)
- **Snow codebase direct analysis:**
  - `crates/snow-codegen/src/mir/lower.rs` lines 1620-1668 (struct deriving dispatch), lines 3356-3488 (generate_to_json_struct, emit_to_json_for_type), lines 3671-3826 (generate_from_json_struct, the core pattern for from_row), lines 3828-3868 (emit_from_json_for_type), lines 3956-4015 (generate_from_json_string_wrapper), lines 5429-5445 (struct-qualified from_json resolution), lines 8842-8845 (STDLIB_MODULES), lines 9039-9058 (map_builtin_name for Pg/Pool)
  - `crates/snow-codegen/src/mir/lower.rs` lines 684-706 (known_functions for Pg/Pool)
  - `crates/snow-codegen/src/codegen/intrinsics.rs` lines 512-583 (Pg/Pool LLVM declarations)
  - `crates/snow-rt/src/db/pg.rs` lines 954-1086 (snow_pg_query implementation -- Map<String, String> construction), lines 1043-1045 (NULL -> empty string)
  - `crates/snow-rt/src/io.rs` lines 17-53 (SnowResult, alloc_result, snow_result_is_ok, snow_result_unwrap)
  - `crates/snow-rt/src/option.rs` lines 17-33 (SnowOption, alloc_option)
  - `crates/snow-rt/src/string.rs` lines 339-363 (snow_string_to_int, snow_string_to_float -- precedent for string parsing)
  - `crates/snow-rt/src/collections/map.rs` lines 103-172 (snow_map_new_typed, snow_map_put, snow_map_get, snow_map_has_key)
  - `crates/snow-rt/src/collections/list.rs` lines 82-214 (snow_list_length, snow_list_get, snow_list_map, snow_list_new, snow_list_append)
  - `crates/snow-typeck/src/infer.rs` lines 1935-2177 (struct deriving validation, Json registration, is_json_serializable -- the template for Row)
  - `crates/snow-typeck/src/infer.rs` lines 654-742 (Pg module and Pool module type signatures in typeck)
  - `crates/snow-typeck/src/error.rs` lines 243-301 (UnsupportedDerive, NonSerializableField error variants)
  - `crates/snow-parser/src/ast/item.rs` lines 259-320 (StructDef, has_deriving_clause, deriving_traits)

- **Phase 49 research:**
  - `.planning/phases/49-json-serde-structs/49-RESEARCH.md` -- complete architecture for `deriving(Json)` which is the direct template for `deriving(Row)`

### Secondary (MEDIUM confidence)
- PostgreSQL wire protocol text format representations (booleans as "t"/"f", floats as "Infinity"/"-Infinity") -- based on PostgreSQL documentation and common PG text mode behavior

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components identified from direct codebase analysis; follows proven `deriving(Json)` pattern exactly
- Architecture: HIGH -- the `generate_from_json_struct` pattern is directly reusable; only the extraction functions differ
- Pitfalls: HIGH -- all pitfalls identified from direct source analysis (NULL as empty string from pg.rs, PostgreSQL text format edge cases from protocol knowledge, flat Map limitation from the data structure)

**Research date:** 2026-02-12
**Valid until:** 2026-03-12 (stable -- no external dependencies, all based on existing codebase patterns)
