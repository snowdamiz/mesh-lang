# Phase 31: Extended Method Support - Research

**Researched:** 2026-02-08
**Domain:** Compiler feature -- extending method dot-syntax to primitives, generics, chaining, and mixed field/method access
**Confidence:** HIGH

## Summary

Phase 31 extends the Phase 30 method dot-syntax infrastructure to handle four new scenarios: (1) methods on primitive types (`42.to_string()`), (2) methods on generic types (`my_list.to_string()`), (3) chained method calls (`point.to_string().length()`), and (4) mixed field access and method calls (`person.name.length()`). The implementation requires changes in three areas: the type checker (fix the retry-based method detection for non-struct types), the trait registry (register Display for `List<T>` and other collection types), and the MIR lowering (add stdlib module method fallback in `resolve_trait_callee`).

The critical architectural insight is that Phase 30's method resolution has a gap for non-struct types. When `42.to_string()` is attempted, `infer_field_access(is_method_call=false)` is called first. Since `Int` is not a struct, there is no struct field lookup to fail -- the function falls through to `Ok(ctx.fresh_var())` at line 4126, returning success with an unresolved type. Since it doesn't fail, the retry mechanism that would call `infer_field_access(is_method_call=true)` never triggers. The fix is to change the non-struct fallback path (lines 4101-4126): when `is_method_call=false` AND the base type is not a struct, the function should try method resolution first (returning the function type if found) before falling back to `fresh_var`. Alternatively, the non-struct path should error with NoSuchField when the base type is a known concrete non-struct type (Int, Float, String, Bool), which would trigger the retry path.

For chaining and mixed field/method access (CHAIN-01, CHAIN-02), the parser already produces the correct nested CST structure, and the MIR lowering's `is_module_or_special` guard correctly passes through non-NameRef bases. However, `length()` on a String is not a trait method -- it's a stdlib module function (`String.length(str)` / `snow_string_length`). The method resolution needs a fallback: after trait method lookup fails, check if the method name matches a stdlib module function for the receiver's type (e.g., `length` on String -> `snow_string_length`). This can be implemented as a type-to-module mapping in `resolve_trait_callee` at the MIR level, and a corresponding mapping in the type checker for `infer_field_access`.

**Primary recommendation:** Fix the type checker's non-struct-type method resolution gap, register Display for `List<T>` in builtins, and add stdlib module method fallback in both the type checker and MIR lowering. The parser and codegen layers need no changes.

## Standard Stack

### Core

This phase modifies existing compiler crates only. No new dependencies.

| Crate | File | Purpose | Why Modified |
|-------|------|---------|--------------|
| `snow-typeck` | `infer.rs` | Type inference engine | Fix non-struct-type method resolution gap; add stdlib module method fallback for chaining |
| `snow-typeck` | `builtins.rs` | Built-in type/trait registration | Register Display impl for `List<T>`, `Map<K,V>`, `Set` |
| `snow-codegen` | `mir/lower.rs` | MIR lowering | Add stdlib module method fallback in `resolve_trait_callee` |

### Supporting

| Crate | File | Purpose | Status |
|-------|------|---------|--------|
| `snow-typeck` | `traits.rs` | Trait registry | No changes -- `find_method_traits`, `resolve_trait_method`, `find_method_sig` already work for primitives and generics |
| `snow-typeck` | `error.rs` | Error definitions | No changes -- `NoSuchMethod`, `AmbiguousMethod` already defined |
| `snow-typeck` | `diagnostics.rs` | Error rendering | No changes -- diagnostics already rendered |
| `snow-parser` | parser + AST | Parser and AST nodes | No changes -- parser already produces correct CST for chained calls |
| `snow-codegen` | `mir/mod.rs` | MIR node definitions | No changes -- `MirExpr::Call` reused |
| `snow-codegen` | `mir/types.rs` | MIR type utilities | No changes -- `mir_type_to_ty` and `mir_type_to_impl_name` handle primitives |
| `snow-codegen` | codegen layer | LLVM codegen | No changes -- receives desugared `MirExpr::Call` |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Stdlib module method fallback in resolve_trait_callee | Register all String functions as trait methods in a new "Stringable" trait | Would require defining a new trait, clutters the TraitRegistry, and doesn't generalize to List/Map/Set; fallback approach is simpler and more general |
| Fix retry for non-struct types | Always call `infer_field_access(is_method_call=true)` for FieldAccess callees | Would change the error semantics for field access on non-struct types; the retry approach is more targeted |
| Register Display for List<T> in builtins.rs | Auto-derive Display when "Display" is checked in collection dispatch | Would scatter Display registration logic; builtins.rs is the single source of truth for builtin trait impls |

## Architecture Patterns

### Pattern 1: Fix Non-Struct Method Resolution Gap

**What:** In `infer_field_access`, when the base type resolves to a concrete non-struct type (Int, Float, String, Bool) and `is_method_call=false`, the current code returns `Ok(ctx.fresh_var())` -- a fresh unresolved type variable. This means the outer `infer_call` path succeeds (incorrectly) and the retry-based method resolution never triggers.

**The Gap (lines 4101-4126 in infer.rs):**
```rust
// Base type is not a struct (or no struct def found).
if is_method_call {
    // ... method resolution (already works) ...
}

// THIS IS THE PROBLEM: returns fresh_var instead of error for concrete non-struct types
Ok(ctx.fresh_var())
```

**When this matters:** Every call to a method on a primitive type. `42.to_string()`, `true.to_string()`, `"hello".length()`.

**Fix approach:** Before returning `fresh_var`, check if the resolved base type is a known concrete type (Int, Float, String, Bool, or any `Ty::Con`). If so, emit `NoSuchField` -- which triggers the retry path in `infer_call` to try `is_method_call=true`. The `fresh_var` fallback should only apply to truly unresolved type variables (`Ty::Var`).

```rust
// Base type is not a struct (or no struct def found).
if is_method_call {
    // ... existing method resolution (already works for primitives via TraitRegistry)
}

// For known concrete non-struct types, emit NoSuchField to trigger method retry
match &resolved_base {
    Ty::Con(_) | Ty::App(_, _) => {
        let err = TypeError::NoSuchField {
            ty: resolved_base,
            field_name,
            span: fa.syntax().text_range(),
        };
        ctx.errors.push(err.clone());
        return Err(err);
    }
    _ => {} // Ty::Var, etc. -- leave as fresh_var for unresolved types
}

Ok(ctx.fresh_var())
```

**Why this works:** The `infer_call` retry mechanism (line 2693-2742) catches `Err(NoSuchField)` on FieldAccess callees and retries with `is_method_call=true`. The second call to `infer_field_access` then takes the `is_method_call=true` path at line 4102, which calls `find_method_traits` and `resolve_trait_method`. These already work for primitives because Display, Debug, Hash, etc. are registered for Int/Float/String/Bool in `builtins.rs`.

### Pattern 2: Register Display for Collection Types

**What:** Add `impl Display for List<T>`, `impl Display for Map<K,V>`, and `impl Display for Set` to the trait registry in `builtins.rs`.

**Why needed:** METH-05 requires `my_list.to_string()` to work. Currently, `resolve_trait_method("to_string", &List<Int>)` returns `None` because no Display impl is registered for List<T>. Collection display currently works only through the special `wrap_collection_to_string` codepath in MIR lowering.

**Implementation:**
```rust
// In register_compiler_known_traits, after the Display impls for primitives:

// Display for List<T>
{
    let list_of_t = Ty::App(
        Box::new(Ty::Con(TyCon::new("List"))),
        vec![Ty::Con(TyCon::new("T"))],
    );
    let mut methods = FxHashMap::default();
    methods.insert("to_string".to_string(), ImplMethodSig {
        has_self: true,
        param_count: 0,
        return_type: Some(Ty::string()),
    });
    let _ = registry.register_impl(ImplDef {
        trait_name: "Display".to_string(),
        impl_type: list_of_t,
        impl_type_name: "List<T>".to_string(),
        methods,
    });
}
```

**Caution:** The MIR lowering already has `wrap_collection_to_string` for string interpolation. The method call path in `resolve_trait_callee` will produce `Display__to_string__List` (or similar), which needs to be handled. The existing collection Display dispatch in the method interception path (lines 3516-3530) already handles `to_string` on `MirType::Ptr` by checking typeck types -- this should work for `my_list.to_string()` method calls.

### Pattern 3: Stdlib Module Method Fallback

**What:** After trait method lookup fails in `resolve_trait_callee`, check if the method name matches a stdlib module function for the receiver's type.

**Why needed:** `String.length(str)` is a module function, not a trait method. For `"hello".length()` or `point.to_string().length()` to work, the method resolution needs to fall back to module functions.

**Type-to-module mapping:**
| Receiver MirType | Module | Functions |
|------------------|--------|-----------|
| `MirType::String` | String | length, slice, contains, starts_with, ends_with, trim, to_upper, to_lower, replace |
| `MirType::Ptr` (List) | List | length, append, get, head, tail, map, filter, reduce, reverse, concat |

**Implementation in resolve_trait_callee:**
```rust
// After trait lookup fails and before the defense-in-depth warning:

// Stdlib module method fallback: check if this is a module function
// callable as a method on the receiver's type.
let module_method = match first_arg_ty {
    MirType::String => {
        let prefixed = format!("string_{}", name);
        let runtime = map_builtin_name(&prefixed);
        if self.known_functions.contains_key(&runtime) {
            Some(runtime)
        } else {
            None
        }
    }
    // Future: MirType::Ptr for List/Map/Set methods
    _ => None,
};
if let Some(runtime_name) = module_method {
    return MirExpr::Var(runtime_name, var_ty.clone());
}
```

**Type checker side:** Similarly, in `infer_field_access` when `is_method_call=true` and trait resolution fails for a known type, check `stdlib_modules()` for matching functions. For String methods like `length`, look up `String` module's `length` function and return its function type (with `self` type prepended). This avoids the NoSuchMethod error.

### Pattern 4: Chaining Works Naturally

**What:** Chained method calls like `a.b().c()` already work through the existing infrastructure -- no new code needed for the chaining mechanism itself.

**CST structure for `a.b().c()`:**
```
CALL_EXPR                    <-- outer call: .c()
  FIELD_ACCESS
    CALL_EXPR                <-- inner call: .b()
      FIELD_ACCESS
        NAME_REF "a"
        DOT
        IDENT "b"
      ARG_LIST
    DOT
    IDENT "c"
  ARG_LIST
```

**Why it works:**
1. The inner `CALL_EXPR(FIELD_ACCESS(a, b), ARG_LIST)` is processed first by `lower_call_expr`
2. Its result has a concrete MIR type (the return type of method `b`)
3. The outer `CALL_EXPR(FIELD_ACCESS(<inner_result>, c), ARG_LIST)` is then processed
4. Since the FieldAccess base is a CALL_EXPR (not a NameRef), `is_module_or_special` is `false`
5. The outer call falls through to method interception

**What enables this:** The `is_module_or_special` guard (lines 3448-3463) only checks NameRef bases. When the base is any other expression type (CallExpr, FieldAccess, etc.), it always falls through to the method call path. This is correct behavior -- only `TypeName.method()` (where TypeName is a module/service/variant/struct) should be intercepted as special.

**Prerequisite:** The methods being chained must be resolvable (via trait registry or stdlib module fallback). So `point.to_string().length()` requires both `to_string` (trait method, already works) AND `length` (needs stdlib module fallback).

### Pattern 5: Mixed Field/Method Access Works Naturally

**What:** Mixed access like `person.name.length()` already works through existing infrastructure.

**CST structure for `person.name.length()`:**
```
CALL_EXPR
  FIELD_ACCESS
    FIELD_ACCESS              <-- person.name (struct field access)
      NAME_REF "person"
      DOT
      IDENT "name"
    DOT
    IDENT "length"
  ARG_LIST
```

**How it flows:**
1. The outer `CallExpr` has a FieldAccess callee
2. The FieldAccess base is another FieldAccess (`person.name`)
3. Base is not a NameRef, so `is_module_or_special` is `false`
4. Method interception fires: lowers `person.name` as receiver, calls `resolve_trait_callee("length", ...)`
5. `person.name` is lowered by `lower_expr` -> `lower_field_access` -> `MirExpr::FieldAccess { object, field: "name", ty: String }`
6. The receiver type is String, so `length` resolves via stdlib module fallback

**Prerequisite:** Same as chaining -- `length` needs the stdlib module method fallback. The struct field access part (`person.name`) already works perfectly.

### Anti-Patterns to Avoid

- **Do not modify the retry mechanism in `infer_call`.** The retry mechanism works correctly -- the issue is that `infer_field_access` returns `Ok(fresh_var)` for non-struct types instead of an error. Fix the root cause, not the detection mechanism.

- **Do not add `length`, `trim`, etc. as trait methods on String.** These are stdlib module functions. Creating a "Stringable" trait would duplicate function signatures and require keeping them in sync. The fallback approach is more maintainable.

- **Do not modify the parser or CST.** The parser already produces the correct nested structure for all chaining patterns. No new nodes are needed.

- **Do not modify the `is_module_or_special` guard for chaining.** It correctly handles chaining by only checking NameRef bases. Changing this guard would break existing module-qualified calls.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Primitive method resolution | Custom primitive type check | Existing `TraitRegistry.resolve_trait_method` | Display/Debug/Hash/Eq already registered for all primitives in `builtins.rs` |
| Generic type method resolution | Custom generic handling | Existing `freshen_type_params` + unification in `resolve_trait_method` | Structural unification already handles `List<T>` matching `List<Int>` |
| Collection to_string | Duplicate of wrap_collection_to_string | Existing collection Display dispatch path (lines 3516-3530) | Already handles `MirType::Ptr` with typeck type lookup |
| Method chaining | New chaining mechanism | Existing nested CST + recursive lowering | Parser naturally nests CALL_EXPR(FIELD_ACCESS(CALL_EXPR(...))) |
| Field + method mixed access | Special-case detection | Existing lower_expr recursion | FieldAccess base lowered by `lower_expr`, method interception handles the call |

**Key insight:** Most of Phase 31's requirements are already handled by existing infrastructure. The primary work is fixing the non-struct method resolution gap (a ~10-line change in infer.rs), registering missing trait impls (Display for collections in builtins.rs), and adding stdlib module method fallback (~30 lines in lower.rs, ~30 lines in infer.rs).

## Common Pitfalls

### Pitfall 1: Non-Struct fresh_var Masking Method Resolution

**What goes wrong:** `42.to_string()` silently produces wrong types because `infer_field_access(is_method_call=false)` returns `Ok(fresh_var)` for `Int.to_string`, which unifies as a generic function call instead of triggering method resolution.

**Why it happens:** Phase 30's `infer_field_access` was designed for struct types. The non-struct fallback at line 4126 returns `Ok(ctx.fresh_var())` to handle cases where the base type is still unresolved (`Ty::Var`). But for concrete primitive types (Int, Float, etc.), this masks the fact that field access on a primitive should fail.

**How to avoid:** Return `Err(NoSuchField)` for concrete non-struct types (any `Ty::Con` or `Ty::App` that is not in the struct registry). Only return `fresh_var` for truly unresolved types (`Ty::Var`). This triggers the existing retry path in `infer_call`.

**Warning signs:** `42.to_string()` compiles but produces `?N` type variables instead of `String`. Or it crashes in MIR lowering because the type is unresolved.

### Pitfall 2: Missing Display Impl for Collection Types

**What goes wrong:** `my_list.to_string()` fails at the type checker with NoSuchMethod because the TraitRegistry has no Display impl for List<T>.

**Why it happens:** Collection Display is handled by special-case codepaths (string interpolation -> `wrap_collection_to_string`). The TraitRegistry was never updated to include Display for List/Map/Set because it wasn't needed before method dot-syntax.

**How to avoid:** Register `impl Display for List<T>` (and Map<K,V>, Set) in `register_compiler_known_traits` in `builtins.rs`. The structural unification will handle generic type args automatically.

**Warning signs:** `my_list.to_string()` produces "no method `to_string` on type `List<Int>`" error while `"${my_list}"` works fine.

### Pitfall 3: Stdlib Module Method Name Collision

**What goes wrong:** A method name like `length` on String conflicts with a user-defined function named `length`, or with List's `length` on a different type.

**Why it happens:** The stdlib module method fallback maps bare names to prefixed runtime names (`length` -> `snow_string_length`). But `length` could also be a user function or a List method.

**How to avoid:** The fallback MUST be gated on the receiver's MirType. Only String methods are mapped when `first_arg_ty` is `MirType::String`. Only List methods are mapped when the receiver is a list. The type-based dispatch prevents collisions. In the type checker, the stdlib_modules lookup must be gated on the resolved base type matching the module's domain type.

**Warning signs:** A user function named `length` gets incorrectly routed to `snow_string_length`.

### Pitfall 4: Double Receiver Prepend for Chained Calls

**What goes wrong:** In a chained call like `a.b().c()`, the inner call already prepends the receiver. If the outer call somehow also prepends the inner call's receiver, the arg list is wrong.

**Why it happens:** Each CALL_EXPR is independently processed by `lower_call_expr`. The inner call produces a MirExpr with a concrete type. The outer call treats this result as the receiver for the method `.c()`. This is correct -- but bugs in how the FieldAccess base is lowered could cause the base to be lowered twice.

**How to avoid:** In the method interception path, the receiver is lowered exactly once: `fa.base().map(|e| self.lower_expr(&e))`. Since `lower_expr` on a `CallExpr` base will recursively call `lower_call_expr`, the inner call is fully processed before being used as the receiver. No special chaining logic needed.

**Warning signs:** Method calls with doubled arguments or incorrect arg counts in MIR output.

### Pitfall 5: Type Checker Stdlib Fallback Missing While MIR Has It

**What goes wrong:** MIR lowering resolves `"hello".length()` correctly via stdlib fallback, but the type checker rejects it with NoSuchMethod because it doesn't know about the stdlib fallback.

**Why it happens:** The stdlib module method fallback is added to MIR lowering's `resolve_trait_callee` but not mirrored in the type checker's `infer_field_access`. The type checker sees `length` on String, finds no trait method, and emits NoSuchMethod before MIR lowering ever runs.

**How to avoid:** Add the stdlib module method fallback in BOTH the type checker (`infer_field_access` when `is_method_call=true`) AND MIR lowering (`resolve_trait_callee`). In the type checker, look up `stdlib_modules()["String"]["length"]` when the resolved base type is String and trait resolution fails. Return the function type (with self parameter prepended).

**Warning signs:** Tests that bypass the type checker (MIR-level tests) pass, but compile-and-run e2e tests fail with type errors.

## Code Examples

### Existing: Primitive Display Impls Already Registered (builtins.rs)

```rust
// Source: crates/snow-typeck/src/builtins.rs, lines 768-789
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

This confirms that `resolve_trait_method("to_string", &Ty::int())` already returns `Some(Ty::string())`. The method resolution logic works for primitives -- the only problem is that the type checker's retry mechanism never triggers for non-struct types.

### Existing: MIR Primitive Builtin Redirects (lower.rs)

```rust
// Source: crates/snow-codegen/src/mir/lower.rs, lines 3392-3407
let resolved = match mangled.as_str() {
    "Display__to_string__Int" | "Debug__inspect__Int" => {
        "snow_int_to_string".to_string()
    }
    "Display__to_string__Float" | "Debug__inspect__Float" => {
        "snow_float_to_string".to_string()
    }
    "Display__to_string__Bool" | "Debug__inspect__Bool" => {
        "snow_bool_to_string".to_string()
    }
    // ... Hash redirects ...
    _ => mangled,
};
```

This confirms that `42.to_string()` will correctly route through `resolve_trait_callee` -> `Display__to_string__Int` -> `snow_int_to_string` at the MIR level. No MIR changes needed for primitive method calls (METH-04).

### Existing: Collection Display Dispatch in Method Path (lower.rs)

```rust
// Source: crates/snow-codegen/src/mir/lower.rs, lines 3516-3530
if let MirExpr::Var(ref name, _) = callee {
    if (name == "to_string" || name == "debug" || name == "inspect")
        && args.len() == 1
        && matches!(args[0].ty(), MirType::Ptr)
    {
        if let Some(base_expr) = fa.base() {
            if let Some(typeck_ty) = self.get_ty(base_expr.syntax().text_range()).cloned() {
                if let Some(collection_call) = self.wrap_collection_to_string(&args[0], &typeck_ty) {
                    return collection_call;
                }
            }
        }
    }
}
```

This shows that `my_list.to_string()` ALREADY has MIR-level support through the collection Display dispatch. The remaining issue is purely at the type checker level -- registering Display for List<T> so the type checker accepts the call.

### Existing: Stdlib Module Functions in Type Checker (infer.rs)

```rust
// Source: crates/snow-typeck/src/infer.rs, lines 214-217
// String module
let mut string_mod = HashMap::new();
string_mod.insert(
    "length".to_string(),
    Scheme::mono(Ty::fun(vec![Ty::string()], Ty::int())),
);
```

This is the data source for the stdlib module method fallback in the type checker. When `"hello".length()` triggers method resolution and trait lookup fails, the type checker can look up `stdlib_modules()["String"]["length"]` and return `Ty::fun(vec![Ty::string()], Ty::int())`.

### Existing: Non-Struct Fallback Path (infer.rs, line 4126)

```rust
// Source: crates/snow-typeck/src/infer.rs, lines 4101-4126
// Base type is not a struct (or no struct def found).
if is_method_call {
    let matching_traits = trait_registry.find_method_traits(&field_name, &resolved_base);
    // ... method resolution (already works for primitives) ...
}

// PROBLEM: returns Ok(fresh_var) for concrete non-struct types
Ok(ctx.fresh_var())
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Method dot-syntax only works on struct types | Extending to primitives, generics, collections | Phase 31 (this phase) | `42.to_string()`, `my_list.to_string()`, chaining all work |
| Collection Display via string interpolation only | Display trait registered for List<T>/Map/Set | Phase 31 (this phase) | `my_list.to_string()` works identically to `"${my_list}"` |
| Module functions only via `Module.func(val)` | Module functions also callable as `val.func()` | Phase 31 (this phase) | `str.length()` works like `String.length(str)` |

**Key design principle (preserved from Phase 30):** Method dot-syntax remains pure desugaring. `42.to_string()` and `to_string(42)` produce identical MIR and runtime behavior. The dot is syntactic sugar for "prepend receiver as first arg, resolve via trait registry or stdlib module lookup."

## Open Questions

1. **Should List module functions (length, append, get, etc.) also be callable via dot syntax?**
   - What we know: The success criteria only mentions `to_string()` and `length()` in examples. `length()` on String is explicitly required.
   - What's unclear: Whether `my_list.length()` (List.length) should also work, or if only `my_list.to_string()` (Display trait) is needed.
   - Recommendation: Implement the stdlib module method fallback generically so it works for ALL modules (String, List, Map, etc.). The mapping from receiver type to module is: `MirType::String` -> "String", `MirType::Ptr` when typeck type is `List<T>` -> "List", etc. However, for Phase 31, only String module methods are required by success criteria. List/Map/Set module methods can be deferred if the generic approach is too complex.

2. **How should the type checker handle multi-parameter stdlib methods via dot syntax?**
   - What we know: `String.length(str)` takes 1 arg (the string). As a method call, `str.length()` takes 0 explicit args (receiver is implicit). But `String.slice(str, start, end)` takes 3 args. As a method, `str.slice(0, 5)` would take 2 explicit args.
   - What's unclear: Whether `build_method_fn_type` correctly handles multi-parameter stdlib methods. The function type from `stdlib_modules()` includes the self parameter, so we need to check that it is compatible with the method call form.
   - Recommendation: When falling back to stdlib module functions, the function type from `stdlib_modules()` already includes the first parameter (the self/receiver type). Return this type directly from `infer_field_access` -- `infer_call` will prepend the receiver and unify, matching the first param against the receiver type. This is the same pattern used for trait methods.

3. **Should the type checker `fresh_var` fallback be completely removed for all non-Var types?**
   - What we know: The `Ok(ctx.fresh_var())` fallback at line 4126 currently fires for any non-struct type. This includes unresolved `Ty::Var` (where it's correct) and concrete types like `Ty::Con("Int")` (where it's wrong -- masks method resolution).
   - What's unclear: Whether there are other non-struct, non-primitive types where `fresh_var` is the correct behavior.
   - Recommendation: Return `Err(NoSuchField)` for `Ty::Con(_)` and `Ty::App(_, _)` when the base type is not in the struct registry. Keep `Ok(ctx.fresh_var())` only for `Ty::Var` (truly unresolved types). This is the most correct behavior -- if we know the type is concrete and it's not a struct, field access should fail.

## Sources

### Primary (HIGH confidence)

- **Direct codebase analysis** -- all findings verified by reading Snow compiler source code
  - `crates/snow-typeck/src/infer.rs` -- `infer_field_access` (line 3962), non-struct fallback (line 4126), retry mechanism (lines 2693-2742), `infer_call` (line 2671), `stdlib_modules()` (line 210), `build_method_fn_type` (line 3938)
  - `crates/snow-typeck/src/traits.rs` -- `TraitRegistry`, `find_method_traits` (line 269), `resolve_trait_method` (line 212), `find_method_sig` (line 245), `freshen_type_params` (line 322)
  - `crates/snow-typeck/src/builtins.rs` -- `register_builtins` (line 30), `register_compiler_known_traits` (line 556), Display impls for primitives (lines 756-789), NO Display for List<T> in builtins
  - `crates/snow-codegen/src/mir/lower.rs` -- `lower_call_expr` (line 3436), `resolve_trait_callee` (line 3377), `is_module_or_special` guard (lines 3448-3463), collection Display dispatch (lines 3516-3530), `wrap_collection_to_string` (line 4832), `lower_field_access` (line 3824), `STDLIB_MODULES` (line 6780), `map_builtin_name` (line 6789)
  - `crates/snow-codegen/src/mir/types.rs` -- `mir_type_to_ty` (line 189), `mir_type_to_impl_name` (line 205)
  - `crates/snow-parser/tests/snapshots/parser_tests__mixed_postfix.snap` -- CST structure for chained postfix

- **Phase 30 documentation** -- `.planning/phases/30-core-method-resolution/` (research, plans, summaries, verification)

### Secondary (MEDIUM-HIGH confidence)

- Snow compiler ROADMAP.md -- phase dependencies, requirements, success criteria
- Snow compiler REQUIREMENTS.md -- METH-04, METH-05, CHAIN-01, CHAIN-02 definitions

## Metadata

**Confidence breakdown:**
- Primitive method resolution (METH-04): HIGH -- root cause identified (non-struct fresh_var fallback), fix approach verified against existing retry mechanism
- Generic type methods (METH-05): HIGH -- missing Display impl for List<T> identified, TraitRegistry structural unification already handles generics
- Chaining (CHAIN-01): HIGH -- parser/MIR infrastructure works naturally, stdlib module fallback is the only missing piece
- Mixed field/method (CHAIN-02): HIGH -- same as chaining, struct field access + method resolution compose naturally
- Stdlib module method fallback: MEDIUM-HIGH -- approach is sound but implementation details for multi-parameter functions and type-to-module mapping need validation during planning

**Research date:** 2026-02-08
**Valid until:** 90 days (compiler internals, stable codebase)
