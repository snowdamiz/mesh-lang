# Phase 50: JSON Serde -- Sum Types & Generics - Research

**Researched:** 2026-02-11
**Domain:** Compiler-generated JSON serialization/deserialization for Snow sum types via `deriving(Json)`, plus generic struct monomorphization for JSON
**Confidence:** HIGH

## Summary

Phase 50 extends the `deriving(Json)` system (completed for structs in Phase 49) to sum types and generic structs. The core work has two distinct parts: (1) generating `to_json`/`from_json` MIR functions for sum types that encode as tagged JSON objects `{"tag":"Variant","fields":[...]}`, and (2) ensuring the existing generic struct monomorphization infrastructure triggers JSON trait function generation.

The good news is that both patterns have strong precedents. Sum type deriving already works for Eq, Ord, Hash, Debug, and Display -- all use the same Match-on-self-with-Constructor-patterns approach. The `generate_hash_sum_type` function (lines 2749-2839) is the closest analogue for `to_json`, as it iterates variants, binds fields, and calls type-directed helpers per field. For generic structs, the `ensure_monomorphized_struct_trait_fns` function (lines 1645-1720) already checks `has_json` and calls `generate_to_json_struct`/`generate_from_json_struct` with the mangled name -- this was added in Phase 49 Plan 02. The primary gap is that typeck does not yet register ToJson/FromJson trait impls for sum types (the `register_sum_type_def` function stops at Display, line 2282), and the MIR lowerer does not yet call JSON generation in `lower_sum_type_def` (it stops at Hash, line 1780).

The from_json direction for sum types is the most complex piece. Decoding `{"tag":"Variant","fields":[...]}` requires: (1) extracting the "tag" string from the JSON object, (2) matching it against known variant names, (3) extracting the "fields" array, (4) decoding each field element according to the variant's type signature, and (5) constructing the correct variant using `MirExpr::ConstructVariant`. Error propagation must handle: missing "tag" key, unknown variant name, missing "fields" key, wrong number of fields, and type errors in individual fields.

**Primary recommendation:** Follow the `generate_hash_sum_type` pattern for to_json (Match on self, per-variant arms with type-directed field encoding). For from_json, extract "tag" as string and use If-chain matching (not Match, since runtime SnowResult pointers need If-based propagation per Phase 49's lesson). For generics, the infrastructure is already in place -- just verify it works end-to-end.

## Standard Stack

### Core

| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| `snow-codegen/src/mir/lower.rs` | Compiler | MIR generation for sum type ToJson/FromJson | Proven: 5 existing sum type deriving traits use identical infrastructure |
| `snow-typeck/src/infer.rs` | Compiler | ToJson/FromJson trait impl registration for sum types | Extends existing sum type trait registration (Eq, Ord, Hash, Debug, Display) |
| `snow-rt/src/json.rs` | Runtime | New helper: `snow_json_as_tag` to extract "tag" string from JSON object | Minimal runtime addition; all complex logic in compile-time codegen |
| `crates/snow-codegen/src/codegen/intrinsics.rs` | Compiler | LLVM declaration for new runtime functions | Standard registration point |

### Supporting

| Component | Location | Purpose | When to Use |
|-----------|----------|---------|-------------|
| Existing `snow_json_object_new/put/get` | Runtime | Build tagged JSON objects for encoding | Already registered from Phase 49 |
| Existing `snow_json_array_new/push` | Runtime | Build fields array for encoding | Already registered from Phase 49 |
| Existing `snow_json_as_string/int/float/bool` | Runtime | Decode individual field values | Already registered from Phase 49 |
| Existing `snow_result_is_ok/unwrap/alloc_result` | Runtime | If-based Result propagation | Already registered from Phase 49 |

### No New External Dependencies

This phase requires zero new crate dependencies. All work uses existing infrastructure.

## Architecture Patterns

### Recommended Project Structure (Touched Files)

```
crates/
  snow-typeck/src/
    infer.rs              # Add ToJson/FromJson impl registration for sum types
  snow-codegen/src/
    mir/lower.rs          # Add generate_to_json_sum_type, generate_from_json_sum_type
    codegen/intrinsics.rs # Register snow_json_as_tag (if new runtime function needed)
  snow-rt/src/
    json.rs               # Add snow_json_as_tag (extract "tag" string from JSON object)
tests/
  e2e/
    deriving_json_sum_type.snow     # Basic sum type JSON encode/decode
    deriving_json_generic.snow      # Generic struct JSON encode/decode
    deriving_json_nested_sum.snow   # Sum type containing struct containing list
```

### Pattern 1: Sum Type to_json Generation (generate_to_json_sum_type)

**What:** Generate a synthetic `ToJson__to_json__SumTypeName` function that matches on self and builds `{"tag":"Variant","fields":[...]}`.
**When to use:** For each sum type with `deriving(Json)`.

**Approach:** Match on self, one arm per variant. Each arm:
1. Creates an empty JSON array for fields
2. Pushes each field value (type-directed, using `emit_to_json_for_type`)
3. Creates a JSON object with "tag" (string) and "fields" (array)

**MIR structure:**
```
fn ToJson__to_json__Shape(self: SumType("Shape")) -> Ptr:
  Match self:
    Shape.Circle(field_0: Float) ->
      let arr = snow_json_array_new()
      let arr = snow_json_array_push(arr, snow_json_from_float(field_0))
      let obj = snow_json_object_new()
      let obj = snow_json_object_put(obj, "tag", snow_json_from_string("Circle"))
      let obj = snow_json_object_put(obj, "fields", arr)
      obj
    Shape.Rectangle(field_0: Float, field_1: Float) ->
      let arr = snow_json_array_new()
      let arr = snow_json_array_push(arr, snow_json_from_float(field_0))
      let arr = snow_json_array_push(arr, snow_json_from_float(field_1))
      let obj = snow_json_object_new()
      let obj = snow_json_object_put(obj, "tag", snow_json_from_string("Rectangle"))
      let obj = snow_json_object_put(obj, "fields", arr)
      obj
    Shape.Point ->
      let obj = snow_json_object_new()
      let obj = snow_json_object_put(obj, "tag", snow_json_from_string("Point"))
      let obj = snow_json_object_put(obj, "fields", snow_json_array_new())
      obj
```

**Reference:** `generate_hash_sum_type` (lines 2749-2839) uses the exact same Match-on-self + per-variant-arm + field iteration pattern. `generate_display_sum_type` (lines 3524-3630) is another close analogue.

### Pattern 2: Sum Type from_json Generation (generate_from_json_sum_type)

**What:** Generate a synthetic `FromJson__from_json__SumTypeName` function that extracts "tag" from a JSON object and dispatches to the correct variant decoder.
**When to use:** For each sum type with `deriving(Json)`.

**Approach:** Uses If-chain (not Match on SnowResult) for tag comparison, matching Phase 49's established pattern:
1. Extract "tag" string from JSON object via `snow_json_object_get` + `snow_json_as_string`
2. Compare tag string against each variant name using string equality
3. For the matching variant, extract "fields" array and decode each element
4. Construct the variant using `MirExpr::ConstructVariant`

**MIR structure (pseudocode):**
```
fn FromJson__from_json__Shape(json: Ptr) -> Ptr:
  let tag_res = snow_json_object_get(json, "tag")
  if snow_result_is_ok(tag_res):
    let tag_json = snow_result_unwrap(tag_res)
    let tag_str_res = snow_json_as_string(tag_json)
    if snow_result_is_ok(tag_str_res):
      let tag_str = snow_result_unwrap(tag_str_res)
      if snow_string_eq(tag_str, "Circle"):
        // decode Circle variant fields...
        let fields_res = snow_json_object_get(json, "fields")
        if snow_result_is_ok(fields_res):
          let fields_arr = snow_result_unwrap(fields_res)
          let f0_json = snow_json_array_get(fields_arr, 0)
          // ... decode field_0 as Float, construct Circle(field_0) ...
        else: fields_res  // propagate error
      else if snow_string_eq(tag_str, "Rectangle"):
        // decode Rectangle variant fields...
      else if snow_string_eq(tag_str, "Point"):
        // nullary: just construct Point
        snow_alloc_result(0, ConstructVariant("Shape", "Point", []))
      else:
        err_result("unknown variant: <tag_str>")
    else: tag_str_res
  else: tag_res
```

**Key decisions:**
- Use If-chain for tag matching (not Match), consistent with Phase 49's If-based Result propagation
- Need `snow_string_eq` for tag comparison (already registered as known function)
- Need array element access: either a new `snow_json_array_get(arr, index)` runtime function, or use the existing `snow_list_get` on the inner list of a JSON_ARRAY
- ConstructVariant in from_json requires the variant value to be a proper SumType, not a Ptr -- this is the same as how struct from_json uses alloc_result with a heap-allocated struct

### Pattern 3: Generic Struct JSON (Monomorphization)

**What:** Generic structs like `Wrapper<T>` with `deriving(Json)` get JSON functions generated at instantiation time.
**When to use:** Automatically, when a generic struct literal like `Wrapper { value: 42 }` is lowered.

**How it already works (Phase 49 infrastructure):**
1. `lower_struct_def` detects generic params (line 1595) and skips trait generation
2. When `lower_struct_literal` encounters `Wrapper { value: 42 }`, it calls `ensure_monomorphized_struct_trait_fns("Wrapper", Ty::App(Con("Wrapper"), [Con("Int")]))`
3. This function (line 1645) checks `trait_registry.has_impl("ToJson", typeck_ty)` (line 1689)
4. If true, calls `generate_to_json_struct(&mangled, &fields)` with mangled name like "Wrapper_Int" and concrete field types
5. The existing `generate_to_json_struct` and `generate_from_json_struct` handle this correctly since they work with concrete MirTypes

**Verification needed:** The monomorphization path (step 3-4) was added in Phase 49 but may not have been tested with actual generic structs. Need E2E tests to confirm `Wrapper<Int>` and `Wrapper<String>` both get correct JSON functions.

**Potential issue:** The `is_json_serializable` check in typeck uses the generic field type `T` (a type variable), not the concrete instantiation. For generic structs, all field types are type params, so `is_json_serializable` would return false. Need to verify: does the validation run on the generic def or on instantiations?

Looking at the code (line 1919-1931), the validation runs during struct def processing with the generic field types. Since `Ty::Con(TyCon("T"))` would not match any known serializable type and would not have a ToJson impl, `is_json_serializable` returns false. This means `deriving(Json)` on a generic struct would currently produce a `NonSerializableField` error for the generic type parameter.

**Fix needed:** Either skip the serializable check for generic params (fields whose type is a generic param), or check at monomorphization time. The simplest fix: in `is_json_serializable`, treat type variables (names matching generic params of the current struct) as serializable. This parallels how `deriving(Eq)` doesn't check that `T` is `Eq` at definition time.

### Pattern 4: Typeck Registration for Sum Type Json

**What:** Register ToJson/FromJson trait impls for sum types with `deriving(Json)`.
**Where:** In `register_sum_type_def` (infer.rs), after the Display registration (line 2282-2299).

**Structure (mirrors struct Json registration at lines 1918-1968):**
```rust
if derive_list.iter().any(|t| t == "Json") {
    // Validate all variant field types are JSON-serializable
    let mut json_valid = true;
    for variant in &variants {
        for field in &variant.fields {
            let field_ty = match field {
                VariantFieldInfo::Positional(ty) => ty,
                VariantFieldInfo::Named(_, ty) => ty,
            };
            if !is_json_serializable(field_ty, type_registry, trait_registry) {
                ctx.errors.push(TypeError::NonSerializableField {
                    struct_name: name.clone(),  // reuse struct_name field for sum type
                    field_name: format!("{}::{}", variant.name, field_name_or_index),
                    field_type: format!("{}", field_ty),
                });
                json_valid = false;
            }
        }
    }

    if json_valid {
        // Register ToJson impl
        // Register FromJson impl
    }
}
```

**Note:** The `NonSerializableField` error type uses `struct_name` -- should work for sum types too since the error message just says "field ... is not JSON-serializable". May want to add a separate error variant or adjust the message for clarity.

### Pattern 5: Json.encode Sum Type Dispatch

**What:** When `Json.encode(sum_val)` is called and the argument has type `SumType("Shape")`, dispatch to `ToJson__to_json__Shape`.
**Where:** In `lower_call_expr` (lower.rs line 4685-4706).

Currently only handles `MirType::Struct`. Need to add:
```rust
if let MirType::SumType(ref sum_name) = args[0].ty().clone() {
    let to_json_fn = format!("ToJson__to_json__{}", sum_name);
    if self.known_functions.contains_key(&to_json_fn) {
        // Same dispatch pattern as struct
    }
}
```

### Pattern 6: SumTypeName.from_json Dispatch

**What:** When `Shape.from_json(json_str)` is called, resolve to `__json_decode__Shape`.
**Where:** Two places need updating:
1. **Typeck** (infer.rs line 4689): Currently only checks `lookup_struct`. Add `lookup_sum_type` check.
2. **MIR lower** (lower.rs line 4845): Currently only checks `struct_defs`. Add `sum_type_defs` check.

### Anti-Patterns to Avoid

- **Match on SnowResult for tag extraction:** Use If + snow_result_is_ok/unwrap, not MirExpr::Match on Constructor patterns for runtime Result values. This is the lesson from Phase 49 (Ptr vs SumType mismatch in LLVM codegen).
- **Assuming variant field order from HashMap:** Variant fields are stored as `Vec<VariantFieldInfo>` in order. Never convert to HashMap.
- **Forgetting nullary variants:** Variants with no fields (like `Red`, `Green`, `Blue`) must still encode as `{"tag":"Red","fields":[]}` and decode correctly. The "fields" array is empty but must be present.
- **Using ConstructVariant with wrong type annotation:** ConstructVariant must use `MirType::SumType(name)` as its `ty` field, not `MirType::Ptr`. If the from_json function returns Ptr (for SnowResult), the ConstructVariant is embedded inside the Ok result.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Tag-based JSON object construction | Custom object builder | `snow_json_object_new/put` + `snow_json_from_string("tag")` | Already available from Phase 49 |
| Fields array construction | Custom array builder | `snow_json_array_new/push` | Already available from Phase 49 |
| Per-field type dispatch (encoding) | New dispatch table | Existing `emit_to_json_for_type` helper | Already handles Int/Float/Bool/String/Struct/Option |
| Per-field type dispatch (decoding) | New dispatch table | Existing `emit_from_json_for_type` helper | Already handles Int/Float/Bool/String/Struct/Option |
| Generic struct JSON generation | Manual per-instantiation generation | Existing `ensure_monomorphized_struct_trait_fns` | Already calls generate_to_json_struct for generic structs with Json impl |
| String comparison for tag matching | Custom string compare | Existing `snow_string_eq` | Already in known_functions |

**Key insight:** The entire sum type JSON system builds on Phase 49's struct foundation. The to_json side reuses `emit_to_json_for_type` per field. The from_json side reuses `emit_from_json_for_type` per field. The only genuinely new work is the Match-based to_json dispatch (straightforward), the If-chain from_json tag matching (complex but follows established If-based pattern), and wiring up typeck/MIR for sum types.

## Common Pitfalls

### Pitfall 1: ConstructVariant Inside alloc_result Requires Heap Allocation

**What goes wrong:** `from_json` returns `*mut SnowResult` (Ptr). The Ok path wraps a `ConstructVariant` value. But `ConstructVariant` produces a stack-allocated sum type value, and `snow_alloc_result(0, variant_val)` expects a pointer. The StructValue-to-Ptr coercion in codegen (expr.rs lines 906-927) heap-allocates structs automatically, but needs to also handle sum type values.
**Why it happens:** Sum types are inline LLVM struct values (not pointers), same as Snow structs. The existing coercion only checks for `StructValue`.
**How to avoid:** Verify that the StructValue-to-Ptr coercion in codegen also fires for sum type values passed to runtime functions. Sum types ARE LLVM StructValues, so this should work -- but test it. If not, need to add explicit GC heap allocation for sum type values before passing to `snow_alloc_result`.
**Warning signs:** LLVM type mismatch error like "expected ptr, got { i8, [7 x i8], ... }".

### Pitfall 2: Array Element Access for from_json Fields

**What goes wrong:** To decode variant fields from `{"fields":[...]}`, need to access individual elements of the JSON array. There is no `snow_json_array_get(arr, index)` function -- only `snow_list_get` which works on raw SnowList pointers.
**Why it happens:** Phase 49 did not need array element access (struct fields use `snow_json_object_get` by key name).
**How to avoid:** Either add a `snow_json_array_get(json_arr, index)` runtime function that validates the JSON is an array and returns the element, or unwrap the JSON_ARRAY to get its inner SnowList and use `snow_list_get`. The former is cleaner and returns a SnowResult for error handling.
**Warning signs:** Attempting to use `snow_list_get` directly on a SnowJson pointer (expects SnowList, not SnowJson).

### Pitfall 3: Typeck Validation for Sum Type Variant Fields

**What goes wrong:** The `NonSerializableField` error currently uses `struct_name` and `field_name` -- but sum type variant fields are either positional (index) or named, and the error should indicate which variant.
**Why it happens:** The error was designed for structs with named fields.
**How to avoid:** Format the field identifier as `"VariantName::field_0"` or `"VariantName::field_name"` for clarity in the error message. Reuse the same `NonSerializableField` error variant (adding a new error variant is unnecessary complexity).
**Warning signs:** Confusing error messages like "field 'Int' in Color is not JSON-serializable" when it should say "field 'Circle::0' in Shape is not JSON-serializable".

### Pitfall 4: Generic Struct deriving(Json) Blocked by is_json_serializable

**What goes wrong:** `struct Wrapper<T> do value :: T end deriving(Json)` produces `NonSerializableField` error because `T` is not a known serializable type.
**Why it happens:** `is_json_serializable` checks the raw typeck `Ty` for the field. Generic type parameters like `T` are represented as `Ty::Con(TyCon("T"))`, which does not match any primitive name and has no ToJson impl.
**How to avoid:** When checking field serializability for a struct with generic params, skip the check for fields whose type is a generic parameter. The actual check happens implicitly at monomorphization time: if `Wrapper<Pid>` is instantiated and `Pid` doesn't have ToJson, the generated code will fail to find `ToJson__to_json__Pid`.
**Warning signs:** Compile error "field 'value' of type 'T' is not JSON-serializable" when defining `Wrapper<T> deriving(Json)`.

### Pitfall 5: Sum Type with Struct Payload -- Nested Encode/Decode

**What goes wrong:** A sum type `Result(ok :: User, err :: String)` where User has `deriving(Json)` must recursively encode the User struct. The `emit_to_json_for_type` handles `MirType::Struct(inner)` by calling `ToJson__to_json__inner`, which works. But the `emit_from_json_for_type` for `MirType::Struct(inner)` calls `FromJson__from_json__inner` which returns a `*mut SnowResult`, not the struct value directly.
**Why it happens:** The from_json chain needs to unwrap the inner Result before constructing the variant.
**How to avoid:** In the from_json variant arm, when decoding a field with type `MirType::Struct(inner)`, call `FromJson__from_json__inner` and then unwrap the result with the standard If-based propagation pattern. This is the same pattern used in `generate_from_json_struct` for nested struct fields.
**Warning signs:** Type error when constructing variant: expecting Struct but got Ptr (the SnowResult pointer).

### Pitfall 6: emit_to_json_for_type Missing SumType Branch for Non-Option Sum Types

**What goes wrong:** Currently `emit_to_json_for_type` (line 2917-2967) handles `MirType::SumType(sum_name) if sum_name.starts_with("Option_")` specially, but falls through to the `_` catch-all for other sum types. If a struct has a sum type field (e.g., `status :: Color` where Color derives Json), the to_json generator won't encode it correctly.
**Why it happens:** Phase 49 only implemented struct-to-json, not sum-type-to-json. Sum type fields other than Option were not anticipated.
**How to avoid:** Add a general `MirType::SumType(sum_name)` branch to `emit_to_json_for_type` that calls `ToJson__to_json__<sum_name>(field_val)`, mirroring the `MirType::Struct(inner_name)` branch. Similarly for `emit_from_json_for_type`.
**Warning signs:** Sum type field values passed through as opaque pointers instead of being encoded as tagged JSON objects.

## Code Examples

### Sum Type to_json Generation Pattern

```rust
// Modeled after generate_hash_sum_type (lines 2749-2839)
fn generate_to_json_sum_type(&mut self, name: &str, variants: &[MirVariantDef]) {
    let mangled = format!("ToJson__to_json__{}", name);
    let sum_ty = MirType::SumType(name.to_string());
    let self_var = MirExpr::Var("self".to_string(), sum_ty.clone());

    let obj_new_ty = MirType::FnPtr(vec![], Box::new(MirType::Ptr));
    let obj_put_ty = MirType::FnPtr(
        vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
        Box::new(MirType::Ptr),
    );
    let arr_new_ty = MirType::FnPtr(vec![], Box::new(MirType::Ptr));
    let arr_push_ty = MirType::FnPtr(
        vec![MirType::Ptr, MirType::Ptr],
        Box::new(MirType::Ptr),
    );
    let from_string_ty = MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr));

    let arms: Vec<MirMatchArm> = variants.iter().map(|v| {
        // Bind fields as field_0, field_1, ...
        let field_pats: Vec<MirPattern> = v.fields.iter().enumerate()
            .map(|(i, ft)| MirPattern::Var(format!("field_{}", i), ft.clone()))
            .collect();
        let bindings: Vec<(String, MirType)> = v.fields.iter().enumerate()
            .map(|(i, ft)| (format!("field_{}", i), ft.clone()))
            .collect();

        // Build fields array
        let mut arr = MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_json_array_new".to_string(), arr_new_ty.clone())),
            args: vec![],
            ty: MirType::Ptr,
        };
        for (i, ft) in v.fields.iter().enumerate() {
            let field_var = MirExpr::Var(format!("field_{}", i), ft.clone());
            let json_val = self.emit_to_json_for_type(field_var, ft, name);
            arr = MirExpr::Call {
                func: Box::new(MirExpr::Var("snow_json_array_push".to_string(), arr_push_ty.clone())),
                args: vec![arr, json_val],
                ty: MirType::Ptr,
            };
        }

        // Build {"tag": "VariantName", "fields": [...]}
        let mut obj = MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_json_object_new".to_string(), obj_new_ty.clone())),
            args: vec![],
            ty: MirType::Ptr,
        };
        // Put "tag"
        let tag_key = MirExpr::StringLit("tag".to_string(), MirType::String);
        let tag_val = MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_json_from_string".to_string(), from_string_ty.clone())),
            args: vec![MirExpr::StringLit(v.name.clone(), MirType::String)],
            ty: MirType::Ptr,
        };
        obj = MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_json_object_put".to_string(), obj_put_ty.clone())),
            args: vec![obj, tag_key, tag_val],
            ty: MirType::Ptr,
        };
        // Put "fields"
        let fields_key = MirExpr::StringLit("fields".to_string(), MirType::String);
        obj = MirExpr::Call {
            func: Box::new(MirExpr::Var("snow_json_object_put".to_string(), obj_put_ty.clone())),
            args: vec![obj, fields_key, arr],
            ty: MirType::Ptr,
        };

        MirMatchArm {
            pattern: MirPattern::Constructor {
                type_name: name.to_string(),
                variant: v.name.clone(),
                fields: field_pats,
                bindings,
            },
            body: obj,
            guard: None,
        }
    }).collect();

    let body = MirExpr::Match {
        scrutinee: Box::new(self_var),
        arms,
        ty: MirType::Ptr,
    };

    let func = MirFunction {
        name: mangled.clone(),
        params: vec![("self".to_string(), sum_ty.clone())],
        return_type: MirType::Ptr,
        body,
        is_closure_fn: false,
        captures: vec![],
        has_tail_calls: false,
    };
    self.functions.push(func);
    self.known_functions.insert(
        mangled,
        MirType::FnPtr(vec![sum_ty], Box::new(MirType::Ptr)),
    );
}
```

### New Runtime Function: snow_json_array_get

```rust
// Extract element at index from a JSON array. Returns SnowResult.
#[no_mangle]
pub extern "C" fn snow_json_array_get(json_arr: *mut u8, index: i64) -> *mut u8 {
    unsafe {
        let j = json_arr as *mut SnowJson;
        if (*j).tag != JSON_ARRAY {
            return err_result("expected Array") as *mut u8;
        }
        let inner_list = (*j).value as *mut u8;
        let len = list::snow_list_length(inner_list);
        if index < 0 || index >= len as i64 {
            return err_result(&format!("array index {} out of bounds (length {})", index, len)) as *mut u8;
        }
        let elem = list::snow_list_get(inner_list, index as u64);
        alloc_result(0, elem as *mut u8) as *mut u8
    }
}
```

### Target Snow Code (Sum Type E2E Test)

```snow
type Shape do
  Circle(Float)
  Rectangle(Float, Float)
  Point
end deriving(Json)

fn main() do
  let c = Circle(3.14)
  let json = Json.encode(c)
  println(json)
  -- Expected: {"tag":"Circle","fields":[3.14]}

  let r = Rectangle(2.0, 5.0)
  let json2 = Json.encode(r)
  println(json2)
  -- Expected: {"tag":"Rectangle","fields":[2.0,5.0]}

  let p = Point
  let json3 = Json.encode(p)
  println(json3)
  -- Expected: {"tag":"Point","fields":[]}

  let result = Shape.from_json(json)
  case result do
    Ok(s) -> println("decoded ok")
    Err(e) -> println("Error: ${e}")
  end
end
```

### Target Snow Code (Generic Struct E2E Test)

```snow
struct Wrapper<T> do
  value :: T
end deriving(Json)

fn main() do
  let w1 = Wrapper { value: 42 }
  let json1 = Json.encode(w1)
  println(json1)
  -- Expected: {"value":42}

  let w2 = Wrapper { value: "hello" }
  let json2 = Json.encode(w2)
  println(json2)
  -- Expected: {"value":"hello"}
end
```

### Target Snow Code (Nested Combination Test)

```snow
type Shape do
  Circle(Float)
  Point
end deriving(Json)

struct Drawing do
  shapes :: List<Shape>
  name :: String
end deriving(Json)
-- Note: List<Shape> requires Shape to have ToJson impl

fn main() do
  -- This tests the success criterion: sum type containing generic struct containing list
  let d = Drawing {
    shapes: [Circle(1.0), Point, Circle(2.5)],
    name: "test"
  }
  let json = Json.encode(d)
  println(json)
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| deriving(Json) structs only | deriving(Json) structs + sum types | Phase 50 (this phase) | Sum types can round-trip through JSON |
| Generic structs can't derive Json | Generic struct JSON via monomorphization | Phase 50 (this phase) | `Wrapper<Int>` and `Wrapper<String>` both work |
| Sum type values not JSON-encodable | Tagged union encoding `{"tag":"V","fields":[...]}` | Phase 50 (this phase) | Standard tagged encoding for ADTs |

## Open Questions

1. **Array element access for from_json**
   - What we know: Need to get individual elements from the JSON "fields" array. No `snow_json_array_get` exists yet.
   - What's unclear: Whether to add a proper runtime function or just unwrap the JSON_ARRAY and call `snow_list_get`.
   - Recommendation: Add `snow_json_array_get(json_arr, index) -> *mut SnowResult` as a proper runtime function. It validates the JSON is an array, bounds-checks the index, and returns the element. This keeps error handling consistent with other extraction functions.

2. **Generic struct is_json_serializable check**
   - What we know: `is_json_serializable` rejects generic type params like `T`, blocking `deriving(Json)` on generic structs.
   - What's unclear: Whether to skip the check for generic params or defer validation to monomorphization time.
   - Recommendation: Skip the check for type variables that match the struct's generic_params. This is consistent with how other derives (Eq, Ord) don't validate that `T` satisfies the trait at definition time. Add a comment noting that invalid instantiations will fail at link time with missing function errors.

3. **Sum type fields stored in JSON: positional vs named**
   - What we know: Success criterion specifies `{"tag":"Variant","fields":[...]}` with an array for fields (positional).
   - What's unclear: Should named variant fields use `{"tag":"V","fields":{"name":"value"}}` instead?
   - Recommendation: Use array for all variant fields (both positional and named). Named fields are a parser-level detail; at runtime they're positional. The array format is simpler, unambiguous, and matches the success criterion. Named variant fields can be addressed in a future enhancement if needed.

4. **emit_to_json_for_type for non-Option SumType fields**
   - What we know: A struct field of type `Color` (a sum type with deriving(Json)) needs to call `ToJson__to_json__Color`.
   - What's unclear: The current `emit_to_json_for_type` only handles `MirType::SumType` for Option.
   - Recommendation: Add a general `MirType::SumType(sum_name)` branch that calls `ToJson__to_json__<sum_name>`, similar to the `MirType::Struct` branch. Guard Option separately (it uses special Some/None encoding, not tagged union).

5. **Pre-existing bugs affecting sum type JSON**
   - What we know: Phase 49 documented Option-in-struct segfault (marked #[ignore]) and struct == after from_json PHI node bug. These may also affect sum type from_json.
   - What's unclear: Whether pattern matching on decoded sum types triggers the same issues.
   - Recommendation: Use field-by-field verification in tests (same workaround as Phase 49). Test simple sum type from_json first (nullary variants), then payloads, then nested combinations.

## Sources

### Primary (HIGH confidence)
- **Snow codebase direct analysis (all locations verified by reading source):**
  - `crates/snow-codegen/src/mir/lower.rs`:
    - Lines 1594-1634: Struct def lowering, generic param check, derive dispatch
    - Lines 1636-1720: `ensure_monomorphized_struct_trait_fns` -- already has JSON check (line 1689, 1710-1712)
    - Lines 1724-1783: Sum type def lowering, derive dispatch (no Json yet)
    - Lines 2749-2839: `generate_hash_sum_type` -- Match + per-variant pattern reference
    - Lines 2846-2912: `generate_to_json_struct` -- existing struct to_json
    - Lines 2914-2967: `emit_to_json_for_type` -- type dispatch, Option special case
    - Lines 3101-3256: `generate_from_json_struct` -- existing struct from_json with If-based propagation
    - Lines 3376-3440: `generate_from_json_string_wrapper` -- chains parse + from_json
    - Lines 4685-4706: Json.encode struct dispatch
    - Lines 4843-4856: StructName.from_json MIR dispatch
  - `crates/snow-typeck/src/infer.rs`:
    - Lines 84-109: SumTypeDefInfo, VariantInfo, VariantFieldInfo
    - Lines 1918-1968: Struct deriving(Json) registration (ToJson/FromJson impls)
    - Lines 1978-2013: `is_json_serializable` -- validates field types
    - Lines 2168-2299: Sum type deriving registration (Eq/Ord/Hash/Display, NO Json yet)
    - Lines 4687-4699: from_json typeck dispatch (struct only)
  - `crates/snow-rt/src/json.rs`: Full runtime, 993 lines. All structured functions present.
  - `crates/snow-rt/src/io.rs`: Lines 17-54. SnowResult, alloc_result, result_is_ok, result_unwrap.
  - `crates/snow-codegen/src/codegen/expr.rs`: Lines 1447-1513. ConstructVariant codegen.
  - `crates/snow-codegen/src/mir/mod.rs`: Lines 217-222. ConstructVariant MIR node. Lines 584-600. MirSumTypeDef/MirVariantDef.

- **Phase 49 completion documentation:**
  - `.planning/phases/49-json-serde-structs/49-01-SUMMARY.md` -- Runtime foundation
  - `.planning/phases/49-json-serde-structs/49-02-SUMMARY.md` -- Struct codegen (key decisions on If-based Result propagation, polymorphic encode)
  - `.planning/phases/49-json-serde-structs/49-03-SUMMARY.md` -- E2E tests (key decisions on field-by-field comparison, unique binding names)

### Secondary (MEDIUM confidence)
- Existing E2E tests:
  - `tests/e2e/deriving_sum_type.snow` -- Sum type with Eq/Ord/Display/Debug/Hash derives
  - `tests/e2e/generic_deriving.snow` -- Generic struct with Display/Eq derives
  - `tests/e2e/deriving_json_*.snow` -- 7 struct JSON tests from Phase 49

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components identified from direct codebase analysis; patterns proven through Phase 49 and existing sum type derives
- Architecture: HIGH -- to_json is a direct analogue of generate_hash_sum_type; from_json uses established If-based Result propagation
- Pitfalls: HIGH -- all pitfalls identified from source analysis (ConstructVariant heap alloc, missing array_get, is_json_serializable generic check, emit_to_json SumType branch)
- Generic struct JSON: MEDIUM -- infrastructure exists (ensure_monomorphized_struct_trait_fns) but is_json_serializable may block generic defs; needs validation

**Research date:** 2026-02-11
**Valid until:** 2026-03-11 (stable -- no external dependencies, all based on existing codebase patterns)
