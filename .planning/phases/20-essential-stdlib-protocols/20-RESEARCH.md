# Phase 20: Essential Stdlib Protocols - Research

**Researched:** 2026-02-07
**Domain:** Snow compiler -- Display, Debug, Eq, Ord protocols for user-defined types
**Confidence:** HIGH (codebase investigation, verified against actual source)

## Summary

This phase adds Display, Debug, Eq, and Ord protocols for user-defined types (structs and sum types) to the Snow language. The infrastructure for trait definitions, impl registration, method lowering, and static dispatch via monomorphized mangled names is already complete from Phases 18-19. The core work is: (1) defining Display and Debug as compiler-known traits with builtin primitive impls, (2) wiring string interpolation to call Display impls for non-primitive types, (3) extending Eq/Ord operator dispatch to struct and sum types with field-by-field comparison.

A critical prerequisite is fixing the typeck type identity issue: `Ty::Con("Point")` (from `name_to_type()` in impls) does not unify with `Ty::App(Con("Point"), [])` (from `infer_struct_literal()` for non-generic structs). Without this fix, no user-defined trait impl can type-check correctly at call sites. This fix is small (one case in `unify.rs`) but must come first.

The runtime already has `snow_int_to_string`, `snow_float_to_string`, `snow_bool_to_string`, and `snow_string_concat` functions. For Display on user types, the impl method body generates the string. For Debug/inspect, the compiler can either generate field-access-based string building in the impl body or provide a runtime helper. The approach for Eq/Ord on structs is to emit field-by-field comparison in the generated MIR for the impl method body.

**Primary recommendation:** Fix the typeck `Ty::Con` vs `Ty::App(Con, [])` unification gap first, then layer Display/Debug/Eq/Ord as compiler-known traits with builtin struct/sum-type impls registered via TraitRegistry.

## Standard Stack

No new external dependencies. All implementation is within existing crates.

### Core Crates Affected

| Crate | Purpose | What Changes |
|-------|---------|-------------|
| snow-typeck/builtins.rs | Trait definitions + builtin impls | Register Display, Debug traits; extend Eq/Ord impls for String/Bool |
| snow-typeck/unify.rs | Type unification | Fix `Con(c)` vs `App(Con(c), [])` identity case |
| snow-typeck/traits.rs | TraitRegistry | No structural changes; just more impls registered |
| snow-codegen/mir/lower.rs | MIR lowering | Update `wrap_to_string()` for trait dispatch; operator dispatch for Gt/GtEq/LtEq/NotEq |
| snow-codegen/codegen/expr.rs | LLVM codegen | Struct/sum field-by-field comparison codegen |
| snow-rt/string.rs | Runtime string helpers | No changes needed (existing functions sufficient) |
| snow-codegen/codegen/intrinsics.rs | Runtime function declarations | No changes needed |

## Architecture Patterns

### Pattern 1: Compiler-Known Trait Registration (Established)

Built-in traits are registered in `builtins.rs::register_compiler_known_traits()`. Each trait follows the pattern:

1. Register `TraitDef` with `TraitMethodSig` entries
2. Register `ImplDef` for each primitive type that implements it
3. The impl stores `Ty::int()`, `Ty::float()`, etc. as `impl_type`

**Code location:** `crates/snow-typeck/src/builtins.rs:560-689`

```rust
// Existing pattern for Eq:
registry.register_trait(TraitDef {
    name: "Eq".to_string(),
    methods: vec![TraitMethodSig {
        name: "eq".to_string(),
        has_self: true,
        param_count: 1,
        return_type: Some(Ty::bool()),
    }],
});

for (ty, ty_name) in &[(Ty::int(), "Int"), (Ty::float(), "Float"), ...] {
    let mut methods = FxHashMap::default();
    methods.insert("eq".to_string(), ImplMethodSig { ... });
    let _ = registry.register_impl(ImplDef {
        trait_name: "Eq".to_string(),
        impl_type: ty.clone(),
        impl_type_name: ty_name.to_string(),
        methods,
    });
}
```

Display and Debug follow the exact same pattern.

### Pattern 2: String Interpolation Lowering (Established)

String interpolation `"value is ${x}"` is lowered in `lower_string_expr()`:
1. Walk `STRING_CONTENT` and `INTERPOLATION` children
2. Each interpolated expression is lowered, then wrapped via `wrap_to_string()`
3. Segments are chained with `snow_string_concat` calls

**Current `wrap_to_string()` behavior** (line 2494-2533 of lower.rs):
- `MirType::String` -> pass through
- `MirType::Int` -> call `snow_int_to_string`
- `MirType::Float` -> call `snow_float_to_string`
- `MirType::Bool` -> call `snow_bool_to_string`
- Everything else -> call bare `to_string` (which doesn't resolve to anything useful)

**Required change:** For `MirType::Struct(name)` and `MirType::SumType(name)`, use the existing trait call rewriting to emit `Display__to_string__TypeName` instead of a bare `to_string` call. This is the same pattern used in `lower_call_expr()` at line 1564.

### Pattern 3: Operator Dispatch for User Types (Established)

Binary operator dispatch for user types is in `lower_binary_expr()` (line 1435-1467 of lower.rs):
1. Check if LHS is `MirType::Struct(_)` or `MirType::SumType(_)`
2. Map `BinOp` to `(trait_name, method_name)` pair
3. Check `trait_registry.has_impl(trait_name, &ty)`
4. If impl exists, emit `MirExpr::Call` with mangled name `Trait__Method__Type`

**Current operator map:**
- `BinOp::Add` -> `("Add", "add")`
- `BinOp::Sub` -> `("Sub", "sub")`
- `BinOp::Mul` -> `("Mul", "mul")`
- `BinOp::Eq` -> `("Eq", "eq")`
- `BinOp::Lt` -> `("Ord", "lt")`
- Everything else -> `(None, "")` -- falls through to hardware BinOp

**Required extension:** Add entries for `BinOp::NotEq`, `BinOp::Gt`, `BinOp::LtEq`, `BinOp::GtEq`. These can be expressed in terms of Eq and Ord:
- `NotEq` -> negate result of `Eq__eq__Type`
- `Gt` -> `Ord__cmp__Type` and check for Greater
- `LtEq` -> `Ord__cmp__Type` and check for Less or Equal
- `GtEq` -> `Ord__cmp__Type` and check for Greater or Equal

Or more simply, since the Ord method currently returns Bool (a direct comparison), just add more dispatch entries.

### Pattern 4: Struct Field-by-Field Operations

Struct fields are accessed in codegen via GEP (GetElementPtr) at `codegen_field_access()` (line 1036-1078 of expr.rs):
1. Alloca the struct value
2. GEP to the field index
3. Load the field value

MIR struct definitions (`MirStructDef`) carry ordered field lists:
```rust
pub struct MirStructDef {
    pub name: String,
    pub fields: Vec<(String, MirType)>,
}
```

For structural equality/comparison on structs, the generated impl method body would:
1. Access each field of `self` and `other`
2. Compare field-by-field using the field type's Eq/Ord
3. Return Bool (for Eq) or Ordering (for Ord)

### Pattern 5: Sum Type Variant Tag Comparison

Sum types are represented as `{tag: i8, payload_ptr: ptr}` in LLVM IR:
- Tag values are assigned sequentially (0, 1, 2, ...)
- Payloads are heap-allocated structs with variant-specific fields

For sum type equality:
1. Compare tags -- if different, return false
2. If same tag, compare payload fields (variant-specific)

### Recommended Project Structure for Changes

```
crates/snow-typeck/src/
  unify.rs            # Fix Con vs App(Con, []) case
  builtins.rs         # Add Display, Debug traits + impls; extend Eq String impl, Ord String impl
crates/snow-codegen/src/mir/
  lower.rs            # Update wrap_to_string(), extend operator dispatch
crates/snow-codegen/src/codegen/
  expr.rs             # (possibly) struct field-by-field comparison codegen if not done purely in MIR
```

### Anti-Patterns to Avoid

- **Generating runtime functions for struct Eq/Ord:** Since structs are statically known, comparison should be generated at compile time (MIR-level field access), not via runtime reflection. The compiler knows all fields and their types.
- **Trying to use Ordering sum type before it exists:** The Ordering sum type (Less | Equal | Greater) must be defined before Ord impls can return it. Either define it as a builtin or generate it. For the initial implementation, Ord can return Bool (like the existing primitive Ord impls do) and the Ordering type can be added later if needed.
- **Adding trait dispatch for all operator variants simultaneously:** Start with Eq (simplest), verify it works, then add Ord. The operator dispatch extension for NotEq/Gt/LtEq/GtEq can be deferred or expressed as wrappers.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Trait registration for Display/Debug | Custom registration system | `register_compiler_known_traits()` pattern in builtins.rs | Established pattern, integrates with has_impl/find_method_traits |
| Trait method dispatch at call sites | Custom dispatch logic | Existing `find_method_traits()` + mangled name rewriting in lower.rs | Already handles all the edge cases |
| String conversion for primitives | New runtime functions | Existing `snow_int_to_string`, `snow_float_to_string`, `snow_bool_to_string` | Already declared in intrinsics.rs and implemented in snow-rt |
| Struct field iteration | Manual AST walking | `MirStructDef.fields` ordered list | Already available after lowering |
| Type identity checking | String comparison on type names | `TraitRegistry.has_impl()` with structural unification | Handles generics and type variables correctly |

## Common Pitfalls

### Pitfall 1: Typeck Type Identity Gap (CRITICAL -- Must Fix First)

**What goes wrong:** Any user-written `impl Trait for MyStruct` will fail at the call site with "expected MyStruct, found MyStruct". The impl method body type-checks, but calling the method fails because the argument type and parameter type are structurally different `Ty` variants.

**Why it happens:** `infer_struct_literal()` returns `Ty::App(Con("Point"), [])` (line 3628-3631 of infer.rs) for non-generic structs. But `name_to_type("Point")` returns `Ty::Con(TyCon::new("Point"))` (line 5111 of infer.rs), which is what's used for impl type in `infer_impl_def()`. The unifier in `unify.rs` has no case for `(Con(c), App(Con(c), []))` -- it falls through to the mismatch catch-all.

**How to avoid:** Add a unification case in `unify.rs`:
```rust
// Con(c) should unify with App(Con(c), []) -- same type, different representation
(Ty::Con(c), Ty::App(box Ty::Con(ref ac), ref args))
| (Ty::App(box Ty::Con(ref ac), ref args), Ty::Con(c))
    if c.name == ac.name && args.is_empty() =>
{
    Ok(())
}
```

This is a ~6-line fix but MUST come before any trait method calls can work end-to-end.

**Warning signs:** "expected X, found X" errors where both types have the same name.

### Pitfall 2: wrap_to_string Falls Through to Bare "to_string"

**What goes wrong:** String interpolation with a struct value emits a call to bare `to_string` function, which is not a valid function name. The trait call rewriting in `lower_call_expr()` might catch it, but `wrap_to_string()` is called outside the call expr path.

**Why it happens:** `wrap_to_string()` at line 2521-2531 has a catch-all `_ =>` case that emits `MirExpr::Call { func: Var("to_string", ...) }` -- this won't resolve to the mangled `Display__to_string__Point` name.

**How to avoid:** In `wrap_to_string()`, for `MirType::Struct(name)` and `MirType::SumType(name)`:
1. Use `mir_type_to_ty()` to get the Ty
2. Check `self.trait_registry.find_method_traits("to_string", &ty)`
3. If Display impl found, emit `MirExpr::Call` with mangled name `Display__to_string__TypeName`

### Pitfall 3: Operator Dispatch Only Handles Eq and Lt

**What goes wrong:** `!=`, `>`, `<=`, `>=` on user-defined types fall through to the hardware BinOp path, which errors with "Unsupported binop type: Struct" at codegen.

**Why it happens:** The operator dispatch map in `lower_binary_expr()` only handles `BinOp::Eq -> Eq` and `BinOp::Lt -> Ord`. NotEq, Gt, LtEq, GtEq return `(None, "")` and fall through.

**How to avoid:** Extend the dispatch map. Two approaches:
- **Simple:** Add `NotEq -> Eq` (negate), `Gt/LtEq/GtEq -> Ord` with appropriate comparisons
- **Correct:** Since the current Ord trait method is `cmp` returning Bool (not Ordering), add separate `lt`, `gt`, `lte`, `gte` methods, or express them in terms of the existing comparison

**Recommended approach:** For now, extend the dispatch for `NotEq` (negate Eq result) and `Gt`/`GtEq`/`LtEq` (call `Ord__cmp__Type` with swapped args or negation). Since the success criteria say `<`, `>`, `<=`, `>=` should work, all four comparison operators need dispatch.

### Pitfall 4: Ord Return Type Mismatch

**What goes wrong:** The success criteria mention an `Ordering` sum type (Less | Equal | Greater), but the current Ord trait definition in builtins.rs has `return_type: Some(Ty::bool())`.

**Why it happens:** The original Ord registration was designed for primitive comparison operators which return Bool directly. Structural comparison that returns `Ordering` is a different semantic.

**How to avoid:** Decision needed:
- **Option A:** Keep Ord returning Bool for the comparison operators (consistent with current behavior), and the comparison methods (`lt`, `gt`, etc.) each return Bool directly. A separate `cmp` method can return Ordering later.
- **Option B:** Define the `Ordering` sum type, change Ord's `cmp` to return it, and derive `<`/`>`/etc. from the Ordering result.

**Recommendation:** Option A for Phase 20 -- keep it simple. The operators already return Bool. Define the Ordering sum type for completeness but don't require Ord impls to use it yet.

### Pitfall 5: Struct Eq/Ord Without Recursive Field Comparison

**What goes wrong:** A struct with a nested struct field won't compare correctly if the nested struct's Eq/Ord isn't also invoked.

**Why it happens:** Field-by-field comparison uses the field's type to determine the comparison method. For primitive fields, hardware comparison works. For nested struct fields, the field's own Eq/Ord impl must be called.

**How to avoid:** When generating structural comparison:
- For primitive fields: use direct comparison
- For string fields: use `snow_string_eq`
- For struct/sum fields: recursively call the field type's `Eq__eq__Type` or `Ord__cmp__Type`

### Pitfall 6: Display/Debug for Sum Types with Payloads

**What goes wrong:** Generating Debug output for sum type variants that have fields requires knowing the variant tag at runtime to determine which fields to access.

**Why it happens:** Sum types are tagged unions. At runtime, you must check the tag to know which variant's fields to display.

**How to avoid:** For user-written Display impls, this is the user's problem (they pattern-match). For auto-generated Debug impls, the generated code must include a match on the tag and field access for each variant.

## Code Examples

### Fix for Typeck Identity Gap (unify.rs)

```rust
// In InferCtx::unify(), add this case before the App-App case:
// Non-generic struct identity: Con("Point") == App(Con("Point"), [])
(Ty::Con(ref c), Ty::App(ref con, ref args))
| (Ty::App(ref con, ref args), Ty::Con(ref c))
    if args.is_empty() && matches!(con.as_ref(), Ty::Con(ref ac) if ac.name == c.name) =>
{
    Ok(())
}
```

### Display Trait Registration (builtins.rs)

```rust
registry.register_trait(TraitDef {
    name: "Display".to_string(),
    methods: vec![TraitMethodSig {
        name: "to_string".to_string(),
        has_self: true,
        param_count: 0,
        return_type: Some(Ty::string()),
    }],
});

// Register impls for Int, Float, String, Bool
for (ty, ty_name) in &[
    (Ty::int(), "Int"),
    (Ty::float(), "Float"),
    (Ty::string(), "String"),
    (Ty::bool(), "Bool"),
] {
    let mut methods = FxHashMap::default();
    methods.insert("to_string".to_string(), ImplMethodSig {
        has_self: true,
        param_count: 0,
        return_type: Some(Ty::string()),
    });
    let _ = registry.register_impl(ImplDef {
        trait_name: "Display".to_string(),
        impl_type: ty.clone(),
        impl_type_name: ty_name.to_string(),
        methods,
    });
}
```

### Updated wrap_to_string for Trait Dispatch (lower.rs)

```rust
fn wrap_to_string(&self, expr: MirExpr) -> MirExpr {
    match expr.ty() {
        MirType::String => expr,
        MirType::Int => MirExpr::Call {
            func: Box::new(MirExpr::Var(
                "snow_int_to_string".to_string(),
                MirType::FnPtr(vec![MirType::Int], Box::new(MirType::String)),
            )),
            args: vec![expr],
            ty: MirType::String,
        },
        MirType::Float => MirExpr::Call { /* snow_float_to_string */ },
        MirType::Bool => MirExpr::Call { /* snow_bool_to_string */ },
        MirType::Struct(name) | MirType::SumType(name) => {
            // Use Display trait dispatch: Display__to_string__TypeName
            let ty_for_lookup = mir_type_to_ty(expr.ty());
            let matching = self.trait_registry.find_method_traits("to_string", &ty_for_lookup);
            if !matching.is_empty() {
                let trait_name = &matching[0];
                let type_name = mir_type_to_impl_name(expr.ty());
                let mangled = format!("{}__{}__{}", trait_name, "to_string", type_name);
                MirExpr::Call {
                    func: Box::new(MirExpr::Var(
                        mangled,
                        MirType::FnPtr(vec![expr.ty().clone()], Box::new(MirType::String)),
                    )),
                    args: vec![expr],
                    ty: MirType::String,
                }
            } else {
                // Fallback: bare to_string (will likely fail at codegen)
                MirExpr::Call {
                    func: Box::new(MirExpr::Var("to_string".to_string(), /* ... */)),
                    args: vec![expr],
                    ty: MirType::String,
                }
            }
        }
        _ => { /* existing fallback */ }
    }
}
```

### Extended Operator Dispatch for NotEq, Gt, LtEq, GtEq (lower.rs)

```rust
// In lower_binary_expr(), extend the trait dispatch map:
let (trait_name, method_name) = match op {
    BinOp::Add => (Some("Add"), "add"),
    BinOp::Sub => (Some("Sub"), "sub"),
    BinOp::Mul => (Some("Mul"), "mul"),
    BinOp::Eq | BinOp::NotEq => (Some("Eq"), "eq"),   // NotEq negates result
    BinOp::Lt => (Some("Ord"), "cmp"),
    BinOp::Gt => (Some("Ord"), "cmp"),                  // swap args or negate
    BinOp::LtEq => (Some("Ord"), "cmp"),
    BinOp::GtEq => (Some("Ord"), "cmp"),
    _ => (None, ""),
};

// After the trait method call, for NotEq: wrap result in logical NOT
// For Gt: generate cmp(rhs, lhs) instead of cmp(lhs, rhs)
// For LtEq: negate cmp(rhs, lhs), i.e., NOT (rhs < lhs)
// For GtEq: negate cmp(lhs, rhs), i.e., NOT (lhs < rhs)
```

**Simpler approach for Gt/GtEq/LtEq when Ord returns Bool (meaning "less than"):**
- `a > b` is `b < a` (swap args to `Ord__cmp__Type`)
- `a <= b` is `NOT (b < a)` (swap args + negate)
- `a >= b` is `NOT (a < b)` (negate)
- `a != b` is `NOT (a == b)` (negate Eq result)

### Structural Equality for Structs (generated MIR pattern)

For a struct `Point { x: Int, y: Int }`, the generated `Eq__eq__Point` should produce MIR equivalent to:
```
fn Eq__eq__Point(self: Point, other: Point) -> Bool {
    self.x == other.x && self.y == other.y
}
```

In MIR:
```rust
MirExpr::BinOp {
    op: BinOp::And,
    lhs: Box::new(MirExpr::BinOp {
        op: BinOp::Eq,
        lhs: Box::new(MirExpr::FieldAccess { object: self_var, field: "x", ty: Int }),
        rhs: Box::new(MirExpr::FieldAccess { object: other_var, field: "x", ty: Int }),
        ty: Bool,
    }),
    rhs: Box::new(MirExpr::BinOp {
        op: BinOp::Eq,
        lhs: Box::new(MirExpr::FieldAccess { object: self_var, field: "y", ty: Int }),
        rhs: Box::new(MirExpr::FieldAccess { object: other_var, field: "y", ty: Int }),
        ty: Bool,
    }),
    ty: Bool,
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Primitive-only Eq/Ord via hardware ops | Trait-based Eq/Ord with TraitRegistry | Phase 18-19 | Eq/Ord now extensible to user types |
| Direct `snow_*_to_string` calls in interpolation | Still direct, but catch-all falls to bare `to_string` | Phase 5 | Need to upgrade catch-all to use Display dispatch |
| No Display/Debug traits | N/A (this phase adds them) | Phase 20 | First user-extensible string conversion |

**Existing infrastructure reused (HIGH confidence):**
- TraitRegistry: trait defs, impl registration, has_impl, find_method_traits
- Mangled name convention: `Trait__Method__Type`
- Operator dispatch in lower_binary_expr
- String interpolation desugaring in lower_string_expr
- Runtime functions: snow_int_to_string, snow_float_to_string, snow_bool_to_string, snow_string_concat
- Struct/sum type field access in codegen (GEP-based)

## Critical Prerequisite: Typeck Identity Fix

**This MUST be the first task in any plan.** Without it, no user-defined trait impl can be called.

**Root cause analysis (verified from source):**

1. `infer_struct_literal()` (infer.rs:3628) returns `Ty::App(Box::new(Ty::Con(TyCon::new(&struct_name))), generic_vars)` where `generic_vars` is `[]` for non-generic structs.

2. `infer_impl_def()` (infer.rs:1709) uses `name_to_type(&impl_type_name)` which returns `Ty::Con(TyCon::new(name))` for non-primitive types.

3. When `infer_call()` unifies the argument type (from struct literal) with the parameter type (from impl method's Ty::Fun), it's unifying `Ty::App(Con("Point"), [])` with `Ty::Con("Point")`.

4. In `unify.rs`, the match arms are: `(Con, Con)`, `(Fun, Fun)`, `(Con("Pid"), App(Con("Pid"), _))` (Pid escape hatch), `(App, App)`. There is no `(Con, App)` case for the general case. Falls through to mismatch.

**Fix:** Add a unification case: `Con(c)` unifies with `App(Con(c), [])` when args are empty. This is semantically correct -- they represent the same non-generic type.

**Alternative fix:** Change `infer_struct_literal()` to return `Ty::Con(TyCon::new(&struct_name))` for non-generic structs (when `generic_vars` is empty). This is simpler and more correct -- but affects more call sites.

**Recommendation:** Use the unifier fix (safer, more general). It handles all cases where `Con` and `App(Con, [])` representations arise.

## Design Decisions Needed by Planner

### Decision 1: Display vs to_string as a Standalone Function

**Options:**
- **A (Trait method only):** `to_string(x)` resolves via trait dispatch to `Display__to_string__Type`
- **B (Trait + builtin function):** Register `to_string` as a polymorphic builtin in the env that dispatches through Display

**Recommendation:** Option A. The call `to_string(42)` is already lowered as a regular call. The trait call rewriting in `lower_call_expr()` will pick it up: it checks if callee is not a known function, gets the first arg's type, calls `find_method_traits("to_string", &ty)`, and rewrites to the mangled name. This works automatically.

### Decision 2: Debug/inspect Implementation

**Options:**
- **A (Compiler-generated impls):** Automatically register Debug impls for all struct/sum types at typeck time based on their definitions
- **B (Runtime helper):** Add `snow_struct_inspect(ptr, metadata_ptr) -> SnowString*` runtime function
- **C (User-written only):** Require users to write their own Debug impls

**Recommendation:** Option A for structs and sum types. The compiler has full knowledge of field names and types at typeck time. Generate synthetic `Debug__inspect__TypeName` functions in MIR lowering that access each field, convert it to string, and format as `TypeName { field: value, ... }`. This keeps the "zero new runtime dependencies" constraint.

### Decision 3: Ordering Sum Type

**Options:**
- **A (Bool-returning Ord):** Keep current Ord trait returning Bool (less-than). Express `>`, `<=`, `>=` as transformations of `<`.
- **B (Ordering-returning Ord):** Define `Ordering = Less | Equal | Greater` sum type, change Ord to return it, derive comparisons from Ordering.

**Recommendation:** Option A for this phase. It's simpler, consistent with the existing Ord registration, and the success criteria say operators should "return results consistent with the Ordering sum type" -- which can be satisfied by defining the Ordering type for documentation purposes while keeping the actual operator dispatch simple. The Ordering type itself is a nice-to-have for user code but not required for operators to work.

### Decision 4: Auto-Deriving Eq/Ord for Structs

**Options:**
- **A (Compiler auto-derives):** Automatically register Eq/Ord impls for all struct/sum types
- **B (User must write impls):** Users write `impl Eq for Point do ... end`
- **C (Hybrid):** Auto-derive unless user provides an explicit impl

**Recommendation:** Option A for Eq (structural equality is the universal default) and Option A for Ord (lexicographic comparison). The success criteria state these should "work on user-defined structs" without mentioning user impls. Auto-derive at either typeck time (register in TraitRegistry) or MIR lowering time (generate the function). The tricky part: auto-derived impls need to generate actual MIR function bodies for the mangled names.

### Decision 5: Where to Generate Auto-Derived Impl Bodies

**Options:**
- **A (MIR lowering):** In the `lower_item(Item::StructDef)` path, after lowering the struct definition, also generate `Eq__eq__StructName` and `Ord__cmp__StructName` MIR functions
- **B (Typeck + MIR):** Register impls in typeck (so has_impl works), generate bodies in MIR lowering
- **C (Codegen):** Generate comparison LLVM IR directly during codegen when encountering mangled names that don't have MIR functions

**Recommendation:** Option B. The impl must be registered in typeck so that `has_impl("Eq", &Ty::Con("Point"))` returns true (this is what the operator dispatch checks). The MIR function body must be generated during lowering so that codegen can find it. Registration in typeck happens during struct/sum type inference. MIR body generation happens in `lower_struct_def` / `lower_sum_type_def`.

## Open Questions

1. **Auto-derive timing for forward references:**
   - What we know: Struct definitions are processed sequentially during typeck inference. Auto-registering Eq/Ord impls for a struct requires the struct definition to be complete.
   - What's unclear: If struct A has a field of type B, and B is defined after A, can A's auto-derived Eq access B's Eq impl?
   - Recommendation: Process auto-derivation in a second pass after all struct definitions are collected, or register impls eagerly and generate bodies lazily.

2. **Display for primitives -- how are builtin impls called?**
   - What we know: `Display__to_string__Int` is registered as a builtin impl. But the actual code for `snow_int_to_string` is in the runtime. There's no MIR function body for the builtin impl.
   - What's unclear: When `to_string(42)` is rewritten to `Display__to_string__Int(42)`, how does codegen find the function body?
   - Recommendation: For primitive Display impls, the MIR lowerer should directly emit calls to the runtime functions (`snow_int_to_string` etc.) instead of going through mangled names. Alternatively, register the mangled names in `known_functions` and have codegen map them to runtime calls.

3. **Impact of typeck identity fix on existing tests:**
   - What we know: 1,018 tests currently pass. The fix adds a new unification case.
   - What's unclear: Could the fix cause previously-failing tests to now succeed in unexpected ways?
   - Recommendation: The fix is strictly more permissive (unifies things that previously failed). Run full test suite after the fix. Any newly passing tests are a bonus.

## Sources

### Primary (HIGH confidence)
- `crates/snow-typeck/src/builtins.rs:560-689` - Existing compiler-known trait registration pattern
- `crates/snow-typeck/src/traits.rs:1-290` - TraitRegistry implementation, has_impl, find_method_traits
- `crates/snow-typeck/src/unify.rs:139-238` - Unification engine, missing Con/App case
- `crates/snow-typeck/src/infer.rs:1672-1822` - infer_impl_def, name_to_type
- `crates/snow-typeck/src/infer.rs:3535-3632` - infer_struct_literal, returns App(Con, [])
- `crates/snow-typeck/src/infer.rs:5105-5113` - name_to_type, returns Con for structs
- `crates/snow-codegen/src/mir/lower.rs:1435-1467` - Operator dispatch for user types
- `crates/snow-codegen/src/mir/lower.rs:1561-1596` - Trait call rewriting
- `crates/snow-codegen/src/mir/lower.rs:2426-2533` - String interpolation lowering + wrap_to_string
- `crates/snow-codegen/src/mir/lower.rs:598-677` - lower_impl_method
- `crates/snow-codegen/src/mir/types.rs:189-215` - mir_type_to_ty, mir_type_to_impl_name
- `crates/snow-codegen/src/codegen/intrinsics.rs:1-411` - Runtime function declarations
- `crates/snow-codegen/src/codegen/expr.rs:209-310` - BinOp codegen for primitives
- `crates/snow-codegen/src/codegen/expr.rs:1036-1091` - Struct field access codegen
- `crates/snow-rt/src/string.rs:1-478` - Runtime string functions

### Secondary (HIGH confidence -- phase 19 summaries)
- `.planning/phases/19-trait-method-codegen/19-04-SUMMARY.md` - Typeck identity issue documentation
- `.planning/phases/19-trait-method-codegen/19-VERIFICATION.md` - Phase 19 verification results

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Directly verified from codebase source
- Architecture patterns: HIGH - All patterns verified against actual implementation
- Typeck identity fix: HIGH - Root cause traced through exact source lines
- Pitfalls: HIGH - Derived from actual code analysis, not speculation
- Code examples: HIGH - Based on existing patterns in codebase

**Research date:** 2026-02-07
**Valid until:** Indefinite (internal codebase analysis, not version-dependent)
