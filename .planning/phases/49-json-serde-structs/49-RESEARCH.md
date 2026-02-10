# Phase 49: JSON Serde -- Structs - Research

**Researched:** 2026-02-10
**Domain:** Compiler-generated JSON serialization/deserialization for Snow structs via `deriving(Json)`
**Confidence:** HIGH

## Summary

This phase adds `deriving(Json)` support for Snow structs, enabling automatic compile-time generation of `to_json` and `from_json` functions. The implementation follows the established deriving infrastructure (Eq, Hash, Debug, Display, Ord) that already exists in the codebase and has been proven through 5 working trait derivations across 48 shipped phases. The core pattern is well-understood: typeck validates the derive name and registers trait implementations, MIR lowering generates synthetic functions that iterate struct fields and call runtime helpers, and codegen emits the standard LLVM IR Call nodes.

The primary new work is: (1) adding ~9 new runtime functions in `snow-rt/src/json.rs` for structured JSON construction/extraction (`snow_json_object_new`, `snow_json_object_put`, `snow_json_object_get`, `snow_json_array_new`, `snow_json_array_push`, `snow_json_as_int/float/string/bool`), (2) splitting the current single `JSON_NUMBER` tag into separate `JSON_INT` and `JSON_FLOAT` tags for round-trip fidelity, (3) generating `ToJson__to_json__StructName` and `FromJson__from_json__StructName` MIR functions in `mir/lower.rs`, and (4) adding a compile-time check that all fields of a `deriving(Json)` struct have JSON-serializable types.

The existing runtime already has primitive-to-JSON helpers (`snow_json_from_int`, `snow_json_from_float`, `snow_json_from_bool`, `snow_json_from_string`) and a JSON parse/encode pipeline backed by `serde_json`. The new work extends this from primitive encoding to structured object construction (building a JSON object field-by-field) and structured extraction (pulling typed values from a JSON object by key name). The user-facing API is `Json.encode(value)` which calls the trait-generated `to_json` function internally, and `Json.decode(json_string)` which parses the string and calls `from_json`.

**Primary recommendation:** Follow the `generate_hash_struct` pattern exactly. The new `generate_to_json_struct` and `generate_from_json_struct` functions should mirror the structure of existing derive generators, using `MirExpr::FieldAccess` for reading fields and `MirExpr::Call` for invoking runtime helpers. Split the JSON NUMBER tag first to establish Int/Float distinction before any struct-aware code.

## Standard Stack

### Core

| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| `snow-rt/src/json.rs` | Runtime | JSON value construction/extraction functions | Extends existing JSON runtime; `serde_json` already a dependency |
| `snow-codegen/src/mir/lower.rs` | Compiler | MIR function generation for `ToJson`/`FromJson` | Proven pattern: 5 existing deriving traits use identical infrastructure |
| `snow-typeck/src/infer.rs` | Compiler | Trait validation and registration for `Json` derive | Extends existing `valid_derives` list and trait impl registration |
| `snow-codegen/src/codegen/intrinsics.rs` | Compiler | LLVM function declarations for new runtime functions | Standard registration point for all extern "C" functions |

### Supporting

| Component | Location | Purpose | When to Use |
|-----------|----------|---------|-------------|
| `serde_json` | Runtime dependency | JSON parsing and string encoding | Already used by `snow_json_parse` and `snow_json_encode` |
| `snow-rt/src/collections/list.rs` | Runtime | SnowList operations for JSON arrays | Building/reading JSON array values |
| `snow-rt/src/collections/map.rs` | Runtime | SnowMap operations for JSON objects | Building/reading JSON object key-value pairs |
| `snow-rt/src/option.rs` | Runtime | SnowOption for Option<T> field handling | Encoding `Some`/`None` to JSON value/null |

### No External Dependencies Needed

This phase requires zero new crate dependencies. All work uses existing infrastructure: `serde_json` (already in `snow-rt/Cargo.toml`), the existing `SnowJson` tagged union, and the existing MIR/codegen pipeline.

## Architecture Patterns

### Pattern 1: Three-Point Runtime Function Registration

**What:** Every new runtime function must be registered in exactly three places.
**When to use:** For all 9 new JSON runtime functions.

**Points:**
1. **`snow-rt/src/json.rs`**: `#[no_mangle] pub extern "C" fn snow_json_object_new(...) -> ...` implementation
2. **`snow-codegen/src/codegen/intrinsics.rs`**: `module.add_function("snow_json_object_new", ...)` LLVM declaration
3. **`snow-codegen/src/mir/lower.rs`**: `self.known_functions.insert("snow_json_object_new", MirType::FnPtr(...))` plus module name mapping in `map_builtin_name`

**Example (from existing `snow_json_from_int`):**
```rust
// 1. Runtime (json.rs line 281-283)
#[no_mangle]
pub extern "C" fn snow_json_from_int(val: i64) -> *mut u8 {
    alloc_json(JSON_NUMBER, val as u64) as *mut u8
}

// 2. Intrinsics (intrinsics.rs line 363-364)
module.add_function("snow_json_from_int",
    ptr_type.fn_type(&[i64_type.into()], false),
    Some(inkwell::module::Linkage::External));

// 3. MIR lower.rs (line 630)
self.known_functions.insert("snow_json_from_int".to_string(),
    MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)));
```

### Pattern 2: Deriving Trait Generation (the core pattern)

**What:** Generate synthetic MIR functions that iterate struct fields and build expressions.
**When to use:** For `generate_to_json_struct` and `generate_from_json_struct`.

**Reference:** `generate_hash_struct` at `mir/lower.rs` lines 2658-2713.

**Structure for to_json (encoding):**
```rust
fn generate_to_json_struct(&mut self, name: &str, fields: &[(String, MirType)]) {
    let mangled = format!("ToJson__to_json__{}", name);
    let struct_ty = MirType::Struct(name.to_string());
    let self_var = MirExpr::Var("self".to_string(), struct_ty.clone());

    // Build body: create JSON object, put each field
    // let obj = snow_json_object_new()
    // let obj = snow_json_object_put(obj, "field_name", <encode field value>)
    // ... for each field ...
    // return obj

    let func = MirFunction {
        name: mangled.clone(),
        params: vec![("self".to_string(), struct_ty.clone())],
        return_type: MirType::Ptr,  // Returns SnowJson*
        body,
        is_closure_fn: false,
        captures: vec![],
        has_tail_calls: false,
    };
    self.functions.push(func);
    self.known_functions.insert(mangled, MirType::FnPtr(vec![struct_ty], Box::new(MirType::Ptr)));
}
```

**Structure for from_json (decoding):**
```rust
fn generate_from_json_struct(&mut self, name: &str, fields: &[(String, MirType)]) {
    let mangled = format!("FromJson__from_json__{}", name);
    let json_param_ty = MirType::Ptr;  // Receives SnowJson*

    // Build body: nested Match chains extracting each field
    // let field1_json = snow_json_object_get(json, "field1")
    // let field1_result = snow_json_as_<type>(field1_json)
    // Match { scrutinee: field1_result, arms: [
    //   Ok(field1_val) -> ... extract next field ...
    //   Err(e) -> ConstructVariant("Result", "Err", [e])
    // ]}

    let func = MirFunction {
        name: mangled.clone(),
        params: vec![("json".to_string(), json_param_ty.clone())],
        return_type: MirType::SumType(format!("Result_{}_String", name)),
        body,
        is_closure_fn: false,
        captures: vec![],
        has_tail_calls: false,
    };
    self.functions.push(func);
    self.known_functions.insert(mangled, ...);
}
```

### Pattern 3: Type-Directed Field Encoding (emit_to_json_for_type)

**What:** A helper that dispatches to the correct `snow_json_from_*` function based on field type.
**When to use:** Inside `generate_to_json_struct` for each field.

**Dispatch table (mirrors `emit_hash_for_type` at line 3027-3081):**

| Field MirType | Runtime Call | Returns |
|---------------|-------------|---------|
| `MirType::Int` | `snow_json_from_int(field_val)` | `*mut SnowJson` (tag=INT) |
| `MirType::Float` | `snow_json_from_float(field_val)` | `*mut SnowJson` (tag=FLOAT) |
| `MirType::Bool` | `snow_json_from_bool(field_val)` | `*mut SnowJson` (tag=BOOL) |
| `MirType::String` | `snow_json_from_string(field_val)` | `*mut SnowJson` (tag=STR) |
| `MirType::Struct(inner)` | `ToJson__to_json__<inner>(field_val)` | `*mut SnowJson` (recursive) |
| `MirType::SumType("Option_T")` | Match Some/None, encode inner or null | `*mut SnowJson` |
| `MirType::Ptr` (List) | Iterate list, encode each element | `*mut SnowJson` (tag=ARRAY) |
| `MirType::Ptr` (Map) | Iterate map, encode each value | `*mut SnowJson` (tag=OBJECT) |

### Pattern 4: Nested Result Propagation for from_json

**What:** Each field extraction returns a `Result`, and failures must propagate.
**When to use:** Inside `generate_from_json_struct`.

**The nested Match pattern from ARCHITECTURE.md:**
```
For struct User { name: String, age: Int }:

let name_json = snow_json_object_get(json, "name")
let name_result = snow_json_as_string(name_json)
Match name_result:
  Ok(name_val) ->
    let age_json = snow_json_object_get(json, "age")
    let age_result = snow_json_as_int(age_json)
    Match age_result:
      Ok(age_val) ->
        ConstructVariant("Result", "Ok",
          [StructLit("User", [("name", name_val), ("age", age_val)])])
      Err(e) -> ConstructVariant("Result", "Err", [e])
  Err(e) -> ConstructVariant("Result", "Err", [e])
```

This nesting depth equals the number of struct fields. Each level extracts one field and chains on success.

### Pattern 5: Typeck Registration for deriving(Json)

**What:** Register `Json` in `valid_derives` and register `ToJson`/`FromJson` trait impls.
**When to use:** In `snow-typeck/src/infer.rs` during struct def processing.

**Locations to modify:**
1. `valid_derives` array (~line 1775): Add `"Json"` to the list `["Eq", "Ord", "Display", "Debug", "Hash", "Json"]`
2. Same for sum type validation (~line 2073)
3. Register `ToJson` impl with method `to_json` (self -> Ptr)
4. Register `FromJson` impl with method `from_json` (Ptr -> Result<T, String>)
5. Update diagnostics help text (~line 1379): "only Eq, Ord, Display, Debug, Hash, and Json are derivable"

### Pattern 6: Module-Qualified API (Json.encode / Json.decode)

**What:** The user calls `Json.encode(value)` and `Json.decode(json_string)`.
**How it works:** `JSON` is already in `STDLIB_MODULES` (lower.rs line 7583). `Json.encode(value)` resolves via module-qualified access to `snow_json_encode`. But for deriving(Json), `Json.encode(my_struct)` needs to call `ToJson__to_json__MyStruct(my_struct)` first to convert to SnowJson, then `snow_json_encode(json_ptr)` to get the string.

**Recommended approach:** Generate a wrapper function or handle at the call site. When `Json.encode(expr)` is called and `expr` has type `Struct("User")`:
- Check if `ToJson__to_json__User` exists in `known_functions`
- If yes: emit `snow_json_encode(ToJson__to_json__User(expr))` -- chain the two calls
- If no: fall back to existing `snow_json_encode(expr)` (for opaque Json values)

Similarly, `Json.decode(str)` for a typed target needs special handling via type annotation or explicit generic syntax (e.g., `Json.decode<User>(str)`). This is a design decision.

### Anti-Patterns to Avoid

- **HashMap iteration for field order:** Always use `Vec<(String, MirType)>` for struct fields. The field order is preserved as `Vec` through AST -> typeck StructDefInfo -> MIR lower fields. Never convert to HashMap.
- **Missing nested type check:** If `User { addr: Address }` derives Json but `Address` does not, this must be a compile error, not a runtime crash. Add the check in typeck or MIR lowering.
- **Treating JSON NUMBER as single tag:** Must split into INT/FLOAT before any struct-aware code. Otherwise `from_json` for Float fields produces garbage.
- **Bypassing the Result pattern in from_json:** Every field extraction can fail (missing key, wrong type). All failures must produce `Result::Err`, never panic.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| JSON parsing from string | Custom parser | `serde_json::from_str` via existing `snow_json_parse` | Already works, handles all JSON edge cases (unicode escapes, nested structures, etc.) |
| JSON string encoding | Custom serializer | `serde_json::to_string` via existing `snow_json_encode` | Already handles string escaping, number formatting, etc. |
| SnowJson GC allocation | Manual memory management | `alloc_json()` helper in json.rs | Handles GC allocation, padding, alignment correctly |
| Struct field metadata | Runtime reflection | Compile-time generation from `StructDefInfo.fields` | Snow has no runtime reflection; all metadata available at compile time via type registry |
| Error propagation in from_json | Custom error handling | Nested `MirExpr::Match` on `MirPattern::Constructor` for Result variants | Follows established MIR pattern for sum type matching |

**Key insight:** The entire JSON serde system is compile-time code generation. There is zero runtime reflection. The compiler has complete struct metadata (`StructDefInfo` in the type registry) and generates field-by-field encode/decode functions at MIR lowering time. This is fundamentally the same approach as Rust's `serde_derive`.

## Common Pitfalls

### Pitfall 1: JSON Number Tag Must Be Split Before Struct-Aware Code

**What goes wrong:** The current `SnowJson` uses a single `JSON_NUMBER` tag (2) for both Int and Float. `snow_json_from_float` stores `f64::to_bits()` as the value, but `snow_json_to_serde_value` (line 129-135) always interprets the value as i64. Round-tripping a Float through JSON produces garbage.
**Why it happens:** The original JSON implementation was opaque -- users explicitly called `encode_int` or `encode_float`. With `deriving(Json)`, the runtime needs to distinguish them automatically.
**How to avoid:** Add `JSON_INT` (tag 2) and `JSON_FLOAT` (tag 6) tags. Update `serde_value_to_snow_json` to use `as_i64()` for INT and `as_f64()` for FLOAT. Update `snow_json_to_serde_value` to check both tags. This MUST be done before any struct-aware encoding/decoding.
**Warning signs:** Round-trip test `from_json(to_json({ x: 3.14 }))` produces `{ x: 4614253070214989087 }` (the bit pattern of 3.14 interpreted as i64).

### Pitfall 2: Nested Struct Missing deriving(Json) Causes Runtime Crash

**What goes wrong:** If `User { addr: Address }` derives Json but `Address` does not, the generated MIR calls `ToJson__to_json__Address(self.addr)` which does not exist. This causes an undefined function error at link time or a crash at runtime.
**Why it happens:** The MIR generator emits calls to trait functions based on field types but does not verify those functions exist.
**How to avoid:** Add a compile-time check: when generating `to_json` for a struct, verify that every field with a `Struct` type has a `ToJson` impl registered in the trait registry. Emit a clear error: "type Address does not derive Json, required by field 'addr' in User deriving(Json)".
**Warning signs:** Linker errors like "undefined symbol: ToJson__to_json__Address" or "Unknown function" panics in codegen.

### Pitfall 3: Option<T> Field Handling Requires Special Case

**What goes wrong:** `Option<T>` is a sum type (`Some(T)` / `None`), resolved as `MirType::SumType("Option_T")`. The `to_json` generator must handle this specially: `Some(v)` encodes as the JSON value, `None` encodes as JSON null. The generic `emit_to_json_for_type` can't just call `ToJson__to_json__Option_Int` because that function doesn't exist (Option doesn't derive Json -- it's a builtin sum type).
**Why it happens:** Option is a special case throughout the Snow compiler. It's a sum type but with built-in semantics.
**How to avoid:** In `emit_to_json_for_type`, when the type is `SumType("Option_*")`, generate inline Match logic: `Match self.field { Some(v) -> emit_to_json_for_type(v, inner_type), None -> alloc_json(JSON_NULL, 0) }`. Similarly for `emit_from_json_for_type`: JSON null -> None, JSON value -> decode inner type and wrap in Some.
**Warning signs:** Option fields serialize as `{"tag":"Some","fields":[...]}` instead of the plain value or null.

### Pitfall 4: List<T> and Map<String, V> Require Type-Aware Element Encoding

**What goes wrong:** `List<T>` and `Map<String, V>` resolve to `MirType::Ptr` (opaque pointer). The `to_json` generator cannot determine the element type from `MirType::Ptr` alone -- it needs the original typeck `Ty` to know `T` or `V`.
**Why it happens:** Collections are erased to `MirType::Ptr` in the MIR type system (mir/types.rs line 77). The element type information is lost.
**How to avoid:** During MIR generation, when processing a `deriving(Json)` struct, look up the original `Ty` for each field (available from `StructDefInfo.fields` in the type registry, which stores typeck-level types with generic args). Use the typeck type to determine the element type and generate appropriate encoding/decoding logic (e.g., iterate the list and call `emit_to_json_for_type` for each element).
**Warning signs:** List fields produce empty arrays or generic pointer-to-string encoding. Map fields produce string-only values.

### Pitfall 5: from_json Must Handle Missing Fields With Clear Error Messages

**What goes wrong:** If a JSON object is missing a field (e.g., `{"name":"Alice"}` for `User { name: String, age: Int }`), the generated `from_json` must return `Err("missing field 'age'")`, not crash or return a zero-initialized struct.
**Why it happens:** `snow_json_object_get` returns null for missing keys. If the generated code passes null to `snow_json_as_int`, it dereferences a null pointer.
**How to avoid:** `snow_json_object_get` should return a SnowResult or a sentinel. Alternatively, add a `snow_json_object_has` check before each get. The simplest approach: make `snow_json_object_get` return a `*mut SnowResult` (Ok with the SnowJson value, or Err with "missing field: <name>"). This keeps the error propagation in the existing nested Match pattern.
**Warning signs:** Segfault when decoding JSON with missing fields.

## Code Examples

### New Runtime Functions Needed

```rust
// Source: ARCHITECTURE.md verified against json.rs patterns

// Create empty JSON object (tag=OBJECT, value=empty SnowMap)
#[no_mangle]
pub extern "C" fn snow_json_object_new() -> *mut u8 {
    let map = map::snow_map_new();
    alloc_json(JSON_OBJECT, map as u64) as *mut u8
}

// Add key-value pair to JSON object (returns new object)
// key: *mut SnowString, val: *mut SnowJson
#[no_mangle]
pub extern "C" fn snow_json_object_put(obj: *mut u8, key: *mut u8, val: *mut u8) -> *mut u8 {
    unsafe {
        let json = obj as *mut SnowJson;
        let map = (*json).value as *mut u8;
        let new_map = map::snow_map_put(map, key as u64, val as u64);
        alloc_json(JSON_OBJECT, new_map as u64) as *mut u8
    }
}

// Get value from JSON object by key (returns *mut SnowResult)
// Ok = SnowJson value, Err = "missing field: <name>"
#[no_mangle]
pub extern "C" fn snow_json_object_get(obj: *mut u8, key: *mut u8) -> *mut SnowResult {
    unsafe {
        let json = obj as *mut SnowJson;
        if (*json).tag != JSON_OBJECT {
            return err_result("expected JSON object");
        }
        let map = (*json).value as *mut u8;
        if map::snow_map_has_key(map, key as u64) != 0 {
            let val = map::snow_map_get(map, key as u64);
            alloc_result(0, val as *mut u8)
        } else {
            let key_str = &*(key as *const SnowString);
            err_result(&format!("missing field: {}", key_str.as_str()))
        }
    }
}

// Extract Int from SnowJson (returns *mut SnowResult)
#[no_mangle]
pub extern "C" fn snow_json_as_int(json: *mut u8) -> *mut SnowResult {
    unsafe {
        let j = json as *const SnowJson;
        match (*j).tag {
            JSON_INT => alloc_result(0, (*j).value as i64 as *mut u8),
            JSON_FLOAT => {
                // Coerce float to int (truncate)
                let f = f64::from_bits((*j).value);
                alloc_result(0, f as i64 as *mut u8)
            }
            _ => err_result("expected Int, got non-number JSON value"),
        }
    }
}

// Extract Float from SnowJson
#[no_mangle]
pub extern "C" fn snow_json_as_float(json: *mut u8) -> *mut SnowResult {
    unsafe {
        let j = json as *const SnowJson;
        match (*j).tag {
            JSON_FLOAT => {
                let bits = (*j).value;
                // Return f64 bits as u64 (caller interprets as f64)
                alloc_result(0, bits as *mut u8)
            }
            JSON_INT => {
                // Promote int to float
                let i = (*j).value as i64;
                let f = i as f64;
                alloc_result(0, f.to_bits() as *mut u8)
            }
            _ => err_result("expected Float, got non-number JSON value"),
        }
    }
}
```

### SnowJson Tag Split

```rust
// Before (current json.rs):
const JSON_NULL: u8 = 0;
const JSON_BOOL: u8 = 1;
const JSON_NUMBER: u8 = 2;  // Single tag for Int AND Float
const JSON_STR: u8 = 3;
const JSON_ARRAY: u8 = 4;
const JSON_OBJECT: u8 = 5;

// After (required for Phase 49):
const JSON_NULL: u8 = 0;
const JSON_BOOL: u8 = 1;
const JSON_INT: u8 = 2;     // Int stored as i64
const JSON_STR: u8 = 3;
const JSON_ARRAY: u8 = 4;
const JSON_OBJECT: u8 = 5;
const JSON_FLOAT: u8 = 6;   // Float stored as f64::to_bits()
```

### Typeck Deriving Check Pattern

```rust
// In infer.rs, struct def processing (~line 1775):
let valid_derives = ["Eq", "Ord", "Display", "Debug", "Hash", "Json"];

// After existing trait registrations (~line 1828), add:
if derive_list.iter().any(|t| t == "Json") {
    // Register ToJson impl
    let mut to_json_methods = FxHashMap::default();
    to_json_methods.insert(
        "to_json".to_string(),
        ImplMethodSig {
            has_self: true,
            param_count: 0,
            return_type: Some(Ty::Con(TyCon::new("Json"))),
        },
    );
    let _ = trait_registry.register_impl(TraitImplDef {
        trait_name: "ToJson".to_string(),
        impl_type: impl_ty.clone(),
        impl_type_name: name.clone(),
        methods: to_json_methods,
    });

    // Register FromJson impl
    let mut from_json_methods = FxHashMap::default();
    from_json_methods.insert(
        "from_json".to_string(),
        ImplMethodSig {
            has_self: false,
            param_count: 1,  // takes a Json value
            return_type: Some(Ty::result(
                Ty::Con(TyCon::new(&name)),
                Ty::string()
            )),
        },
    );
    let _ = trait_registry.register_impl(TraitImplDef {
        trait_name: "FromJson".to_string(),
        impl_type: impl_ty.clone(),
        impl_type_name: name.clone(),
        methods: from_json_methods,
    });
}
```

### MIR Lower Deriving Check

```rust
// In lower_struct_def (~line 1597), after existing derive checks:
if derive_list.iter().any(|t| t == "Json") {
    self.generate_to_json_struct(&name, &fields);
    self.generate_from_json_struct(&name, &fields);
}
```

### E2E Test Example (Target Snow Code)

```snow
struct Point do
  x :: Int
  y :: Int
end deriving(Json)

struct User do
  name :: String
  age :: Int
  score :: Float
  active :: Bool
end deriving(Json)

fn main() do
  let p = Point { x: 1, y: 2 }
  let json = Json.encode(p)
  println(json)  -- {"x":1,"y":2}

  let decoded = Json.decode(json)
  case decoded do
    Ok(p2) -> println("${p2.x}, ${p2.y}")  -- 1, 2
    Err(e) -> println("Error: ${e}")
  end
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Opaque JSON only (`Json.parse`/`Json.encode`) | Struct-aware `deriving(Json)` | Phase 49 (this phase) | Users get type-safe JSON round-trips without manual field extraction |
| Single `JSON_NUMBER` tag | Separate `JSON_INT` / `JSON_FLOAT` tags | Phase 49 (this phase) | Float values survive JSON round-trip correctly |
| `Json.encode(opaque_json)` | `Json.encode(any_deriving_json_value)` | Phase 49 (this phase) | encode() works on structs directly, not just opaque Json values |

**Coexistence:** The existing opaque `Json` type (parse/get/encode) continues to work. `deriving(Json)` adds a typed layer on top. Both systems coexist via the existing STDLIB_MODULES "JSON" mapping.

## Open Questions

1. **Json.decode type dispatch**
   - What we know: `Json.encode(value)` can dispatch to `ToJson__to_json__Type` based on the argument's type at the call site. The MIR lowerer knows the type of `value` and can insert the trait call.
   - What's unclear: How does `Json.decode(str)` know the target type? Options: (a) explicit type annotation `Json.decode<User>(str)`, (b) inference from usage context `let user: User = Json.decode(str)`, (c) make `from_json` a method on the type itself: `User.from_json(str)`.
   - Recommendation: Use option (c) -- `User.from_json(str)`. This is simplest to implement (just call `FromJson__from_json__User` via normal trait dispatch) and avoids needing generic function syntax. `Json.encode(value)` works because the value's type is known; `Json.decode` would need explicit target type, so putting it on the target type is cleaner. Can also support `Json.decode(str)` with type inference from assignment context as a later enhancement.

2. **List<T> and Map<String, V> element type recovery**
   - What we know: Collections erase to `MirType::Ptr`. The MIR generator needs element type info to generate correct encoding/decoding.
   - What's unclear: Exactly how to recover the typeck `Ty` for a struct field during MIR lowering when the MIR type is `Ptr`.
   - Recommendation: In the deriving generator, look up the original `StructDefInfo` from the type registry (using the struct name). The `StructDefInfo.fields` vector has `Ty`-level types with generic args (e.g., `Ty::App(Con("List"), [Con("Int")])`). Use these to determine element types. This information is available because the type registry is accessible during MIR lowering (`self.registry`).

3. **Non-serializable field type detection (JSON-10)**
   - What we know: The requirement says "compiler emits error when deriving(Json) on struct with non-serializable field type." Serializable types are: Int, Float, Bool, String, structs with deriving(Json), Option<T> where T is serializable, List<T> where T is serializable, Map<String, V> where V is serializable.
   - What's unclear: Should this check happen in typeck or MIR lowering? Typeck has the trait registry; MIR lowering has both the registry and the concrete MIR types.
   - Recommendation: Do the check in MIR lowering, inside `generate_to_json_struct`. For each field, check if the type is: a primitive (Int/Float/Bool/String), a Ptr that resolves to a known collection with serializable elements (via typeck Ty lookup), or a Struct/SumType that has a `ToJson` impl in the trait registry. If none of these, emit a compile error. This parallels how `emit_hash_for_type` has a fallback case for unsupported types.

## Sources

### Primary (HIGH confidence)
- **Snow codebase direct analysis:**
  - `crates/snow-codegen/src/mir/lower.rs` lines 1574-1608 (struct deriving dispatch), lines 2658-2713 (generate_hash_struct pattern), lines 3027-3081 (emit_hash_for_type dispatch), lines 622-633 (JSON known_functions), lines 7731-7749 (JSON module mapping), lines 7581-7585 (STDLIB_MODULES)
  - `crates/snow-codegen/src/mir/mod.rs` lines 42-58 (MirFunction), lines 64-92 (MirType enum), lines 205-215 (StructLit/FieldAccess)
  - `crates/snow-codegen/src/mir/types.rs` lines 23-64 (Ty to MirType resolution), lines 67-91 (resolve_con), lines 94-136 (resolve_app), lines 141-148 (mangle_type_name)
  - `crates/snow-codegen/src/codegen/intrinsics.rs` lines 342-373 (JSON intrinsic declarations)
  - `crates/snow-codegen/src/codegen/expr.rs` lines 1313-1392 (struct lit and field access codegen)
  - `crates/snow-rt/src/json.rs` lines 23-40 (SnowJson structure), lines 44-55 (alloc_json), lines 77-118 (serde_value_to_snow_json), lines 123-170 (snow_json_to_serde_value), lines 280-301 (snow_json_from_* primitives)
  - `crates/snow-rt/src/io.rs` lines 16-20 (SnowResult structure)
  - `crates/snow-rt/src/option.rs` lines 17-20 (SnowOption structure)
  - `crates/snow-typeck/src/infer.rs` lines 1770-1828 (struct deriving validation and trait registration), lines 2068-2114 (sum type deriving)
  - `crates/snow-typeck/src/error.rs` lines 243-252 (UnsupportedDerive, MissingDerivePrerequisite)
  - `crates/snow-typeck/src/diagnostics.rs` lines 1360-1381 (error message formatting)
  - `crates/snow-parser/src/ast/item.rs` lines 282-305 (has_deriving_clause, deriving_traits)
- **Project research:**
  - `.planning/research/ARCHITECTURE.md` -- full encode/decode data flow diagrams, component boundaries, runtime function specifications
  - `.planning/research/PITFALLS.md` -- Pitfall 4 (field order), Pitfall 9 (number precision), Pitfall 12 (opaque JSON coexistence)
  - `.planning/research/SUMMARY.md` -- executive summary, stack recommendations, risk assessment

### Secondary (MEDIUM confidence)
- Existing deriving tests:
  - `tests/e2e/deriving_struct.snow` -- demonstrates `deriving(Eq, Ord, Display, Debug, Hash)` syntax
  - `tests/e2e/deriving_selective.snow` -- demonstrates selective `deriving(Eq)` only
  - `tests/e2e/stdlib_json_parse_roundtrip.snow` -- demonstrates existing `JSON.encode_int()` usage

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components identified from direct codebase analysis; no external dependencies needed
- Architecture: HIGH -- the deriving pattern is proven through 5 existing traits with identical infrastructure
- Pitfalls: HIGH -- all pitfalls identified from direct source analysis (JSON_NUMBER tag issue from json.rs, field order from StructDefInfo, nested type check from trait registry patterns)

**Research date:** 2026-02-10
**Valid until:** 2026-03-10 (stable -- no external dependencies, all based on existing codebase patterns)
