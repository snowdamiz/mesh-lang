# Phase 77: From/Into Conversion - Research

**Researched:** 2026-02-13
**Domain:** Compiler trait system extension -- From/Into conversion traits, synthetic impl generation, built-in primitive conversions, and ? operator error type auto-conversion
**Confidence:** HIGH

## Summary

Phase 77 adds conversion traits (`From<T>` and `Into<T>`) to Mesh's trait system, builds automatic `Into` synthesis from `From` registrations, registers built-in primitive conversions, and extends the `?` operator to auto-convert error types via `From`. This phase builds on the associated types and trait infrastructure completed in Phases 74-76 but is architecturally simpler than those phases because `From<T>` uses a generic type parameter (not an associated type) and the core challenge is wiring up synthetic impl generation plus extending an existing operator.

The most important insight from codebase analysis is that `From<T>` is a **parameterized trait** -- `impl From<Int> for Float` means "From parameterized with Int, implemented for Float". The parser already supports `impl Trait<T> for Type` syntax (generic args on traits in impl blocks), but the type checker (`infer_impl_def`) currently **ignores** the generic type arguments on the trait name. The trait is registered just as `"From"` with the impl type, losing the `<Int>` parameter. This is the primary gap that must be filled.

The second key finding is that `From<T>` enables multiple impls per implementing type (e.g., `impl From<Int> for String` AND `impl From<Float> for String`), which means the trait registry must distinguish impls not just by trait name + impl type, but by trait name + trait type args + impl type. The current `find_impl` looks up by `(trait_name, impl_type)` only. This needs to be extended.

The third finding is that the `?` operator lowering (`lower_try_result` in lower.rs:8003) currently directly rewraps the error value in the function's return type without any conversion. Extending it for `From`-based error conversion requires: (1) detecting when the expression's error type differs from the function's return error type, (2) looking up `From<ExprErr> for ReturnErr` in the trait registry, and (3) wrapping the error in a `From.from()` call before the early return. The type checker (`infer_try_expr` in infer.rs:6904) currently unifies error types directly -- it needs to accept mismatched error types when a `From` impl exists.

**Primary recommendation:** Extend the trait registry to support parameterized trait lookup (`find_impl_with_type_args`), add synthetic `Into` generation as a post-registration hook, register built-in `From` impls for primitive conversions as compiler-known traits, and extend both `infer_try_expr` and `lower_try_result` for `From`-based error conversion.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| mesh-parser | in-tree | Already parses `impl Trait<T> for Type` and `interface Name<T>` | No parser changes needed for basic From/Into syntax |
| mesh-typeck | in-tree | TraitRegistry, infer_impl_def, infer_try_expr | Primary modification target for parameterized trait support |
| mesh-codegen | in-tree | MIR lower.rs (lower_try_result, resolve_trait_callee), codegen intrinsics | Extend ? operator lowering, add From dispatch |
| mesh-rt | in-tree | Runtime conversion functions (mesh_int_to_string, etc.) | Built-in From impls reuse existing runtime functions |

### Supporting
No new external dependencies needed. All changes are internal to existing crates.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Synthetic Into generation (auto-register concrete impl when From registered) | Blanket impl mechanism (`impl<T: From<U>> Into<U> for T`) | Blanket impls would require recursive trait solving, coherence checks, and fundamentally new infrastructure. Synthetic generation is simpler and sufficient. |
| Extending find_impl with type args | Encoding type args into trait name string (e.g., "From_Int") | String encoding is fragile and prevents proper type-level matching. A proper type_args field is cleaner. |
| Module-qualified static calls (Float.from) | Free function (from_float) | Module-qualified calls match Mesh's existing Type.method() idiom used throughout stdlib |

## Architecture Patterns

### Current State (Before Phase 77)

**Trait registry** stores impls keyed by `(trait_name: String, impl_type: Ty)`. No concept of type arguments on the trait name itself. `impl From<Int> for Float` would be stored as trait_name="From", impl_type=Float, losing the `<Int>` parameter.

**infer_impl_def** (infer.rs:2874) extracts trait name as the first IDENT from the first PATH node. It does NOT extract GENERIC_ARG_LIST children from the trait path. Generic type args on the trait name are silently ignored.

**lower_try_result** (lower.rs:8003) desugars `result?` into a match that directly rewraps the error value with the function's return type. No conversion step.

**infer_try_expr** (infer.rs:6904) unifies the expression's error type with the function return type's error type directly (line 6994: `ctx.unify(err_ty, args[1].clone(), ...)`). Mismatched error types produce a unification error.

**Existing conversion functions:**
- `mesh_int_to_string(i64) -> *MeshString` -- runtime function
- `mesh_float_to_string(f64) -> *MeshString` -- runtime function
- `mesh_bool_to_string(i8) -> *MeshString` -- runtime function
- `mesh_int_to_float` -- codegen intrinsic (sitofp instruction, no runtime call)
- `mesh_float_to_int` -- codegen intrinsic (fptosi instruction, no runtime call)

**Module-qualified static calls:**
- `Float.from(42)` would route through `lower_field_access` -> STDLIB_MODULES check -> `map_builtin_name("float_from")`. Currently Float module only has `to_int`.
- `String.from(42)` would route similarly through String module.
- Pattern: `Type.method(arg)` -> `type_method` -> `map_builtin_name` -> `mesh_type_method` runtime function

### Required Changes

```
crates/mesh-typeck/src/
├── traits.rs               # Add trait_type_args to ImplDef
│                           # Add find_impl_with_type_args method
│                           # Add post-registration hook for From->Into synthesis
├── builtins.rs             # Register From<T> and Into<T> trait definitions
│                           # Register built-in From impls for primitives
├── infer.rs                # Extract GENERIC_ARG_LIST from trait path in infer_impl_def
│                           # Extend infer_try_expr to accept From-convertible error types
│                           # Add From<T>.from() to Float/String/Int module functions
│                           # Add into() as polymorphic function

crates/mesh-codegen/src/
├── mir/lower.rs            # Extend lower_try_result to insert From.from() call
│                           # Map From__from__Type to runtime/intrinsic functions
│                           # Add Float/String module "from" entries to map_builtin_name
├── codegen/expr.rs         # Add From__from__Float (sitofp), From__from__String_Int
│                           #   as codegen intrinsics or route to existing runtime functions
├── codegen/intrinsics.rs   # Declare any new runtime function signatures
```

### Pattern 1: Parameterized Trait Storage

**What:** Extend `ImplDef` with a `trait_type_args: Vec<Ty>` field to store the type arguments on the trait name. `impl From<Int> for Float` stores `trait_type_args: [Ty::int()]`.

**When to use:** Every impl registration for parameterized traits.

**Example:**
```rust
// Extended ImplDef
pub struct ImplDef {
    pub trait_name: String,
    pub trait_type_args: Vec<Ty>,  // NEW: e.g., [Ty::int()] for From<Int>
    pub impl_type: Ty,
    pub impl_type_name: String,
    pub methods: FxHashMap<String, ImplMethodSig>,
    pub associated_types: FxHashMap<String, Ty>,
}

// New lookup method on TraitRegistry
pub fn find_impl_with_type_args(
    &self,
    trait_name: &str,
    trait_type_args: &[Ty],
    impl_ty: &Ty,
) -> Option<&ImplDef> {
    let impls = self.impls.get(trait_name)?;
    for impl_def in impls {
        if impl_def.trait_type_args.len() != trait_type_args.len() {
            continue;
        }
        let mut ctx = InferCtx::new();
        let freshened_impl = freshen_type_params(&impl_def.impl_type, &mut ctx);
        if ctx.unify(freshened_impl, impl_ty.clone(), ConstraintOrigin::Builtin).is_err() {
            continue;
        }
        // Also check trait type args match
        let mut all_match = true;
        for (stored, query) in impl_def.trait_type_args.iter().zip(trait_type_args) {
            let freshened = freshen_type_params(stored, &mut ctx);
            if ctx.unify(freshened, query.clone(), ConstraintOrigin::Builtin).is_err() {
                all_match = false;
                break;
            }
        }
        if all_match {
            return Some(impl_def);
        }
    }
    None
}
```

### Pattern 2: Synthetic Into Generation

**What:** When `impl From<A> for B` is registered, automatically also register `impl Into<B> for A`. The Into method body calls `From.from()`.

**When to use:** Every From impl registration.

**Example:**
```rust
// In register_impl, after storing the From impl:
if impl_def.trait_name == "From" && !impl_def.trait_type_args.is_empty() {
    let source_ty = impl_def.trait_type_args[0].clone(); // A in From<A>
    let target_ty = impl_def.impl_type.clone();           // B in "for B"

    // Synthesize: impl Into<B> for A
    let mut into_methods = FxHashMap::default();
    into_methods.insert("into".to_string(), ImplMethodSig {
        has_self: true,
        param_count: 0,
        return_type: Some(target_ty.clone()),
    });

    let into_impl = ImplDef {
        trait_name: "Into".to_string(),
        trait_type_args: vec![target_ty],
        impl_type: source_ty,
        impl_type_name: /* derive from source_ty */,
        methods: into_methods,
        associated_types: FxHashMap::default(),
    };
    // Register without re-triggering synthesis (guard against recursion)
    self.impls.entry("Into".to_string()).or_default().push(into_impl);
}
```

### Pattern 3: From-Based Error Conversion in ? Operator

**What:** When `expr?` is used where the expression's error type E1 differs from the function's return error type E2, check if `From<E1> for E2` exists. If so, wrap the error value in a `From.from()` call before the early return.

**When to use:** In `infer_try_expr` (type checker) and `lower_try_result` (MIR lowering).

**Type checker change (infer_try_expr):**
```rust
// Current (line ~6994): directly unify error types
// let _ = ctx.unify(err_ty.clone(), args[1].clone(), ConstraintOrigin::Builtin);

// New: try unification first; if it fails, check for From impl
let fn_err_ty = args[1].clone();
if ctx.unify(err_ty.clone(), fn_err_ty.clone(), ConstraintOrigin::Builtin).is_err() {
    // Check if From<err_ty> for fn_err_ty exists
    if !trait_registry.has_impl_with_type_args("From", &[err_ty.clone()], &fn_err_ty) {
        ctx.errors.push(TypeError::TryIncompatibleReturn { ... });
    }
    // If From exists, the type check passes -- conversion will be inserted at MIR level
}
```

**MIR lowering change (lower_try_result):**
```rust
// In the Err arm body, before constructing the return Err:
// Check if error types differ and From conversion is needed
let fn_err_ty = /* extract from fn_ret_ty */;
if error_ty != fn_err_ty {
    // Wrap: From__from__FnErrType(err_value)
    let converted_err = MirExpr::Call {
        func: Box::new(MirExpr::Var(
            format!("From__from__{}", mir_type_to_impl_name(&fn_err_mir_ty)),
            from_fn_ty,
        )),
        args: vec![MirExpr::Var(err_name, error_ty)],
        ty: fn_err_mir_ty,
    };
    // Use converted_err in the Err variant construction
}
```

### Pattern 4: Built-in From Impls as Compiler-Known Traits

**What:** Register From/Into trait defs and built-in impls in `register_compiler_known_traits`. Primitive conversions route to existing runtime functions or codegen intrinsics.

**Conversions to register:**
| From | To | Runtime/Codegen |
|------|----|----------------|
| Int | Float | codegen intrinsic (sitofp) -- existing `mesh_int_to_float` |
| Int | String | runtime `mesh_int_to_string` |
| Float | String | runtime `mesh_float_to_string` |
| Bool | String | runtime `mesh_bool_to_string` |

**Dispatch mapping:**
- `From__from__Float` (From<Int> for Float) -> `mesh_int_to_float` codegen intrinsic
- `From__from__String` with Int arg -> `mesh_int_to_string` runtime
- `From__from__String` with Float arg -> `mesh_float_to_string` runtime
- `From__from__String` with Bool arg -> `mesh_bool_to_string` runtime

**Note:** Because `From<Int> for String` and `From<Float> for String` both target String, the mangled name `From__from__String` would collide. The mangling must incorporate the trait type arg: `From_Int__from__String` vs `From_Float__from__String`.

### Anti-Patterns to Avoid

- **Implementing blanket impls:** Do NOT add a general blanket impl mechanism. Use synthetic concrete impl generation instead. Blanket impls create recursive trait resolution problems (Pitfall 3 from domain PITFALLS.md).
- **Ignoring trait type args in mangling:** Mangled names must include trait type args to avoid collisions. `From<Int> for String` and `From<Float> for String` must produce different mangled function names.
- **Modifying the unifier for From resolution:** From-based conversion is NOT a unification concern. It's a trait lookup that happens after unification fails. Never add From logic to the unify() function.
- **Breaking existing ? operator behavior:** The existing `?` behavior (same error types, direct unification) must continue to work unchanged. From-based conversion is a fallback when direct unification fails.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Int->Float conversion | Custom runtime function | Existing codegen intrinsic `mesh_int_to_float` (sitofp) | Already proven, zero overhead |
| Int/Float/Bool->String | Custom conversion logic | Existing runtime functions `mesh_int_to_string`, `mesh_float_to_string`, `mesh_bool_to_string` | Already tested and GC-aware |
| Trait method dispatch | Custom From dispatch | Existing `resolve_trait_callee` pattern | Already handles Display, Debug, Hash dispatch; extend for From |
| Module-qualified calls | New dispatch mechanism | Existing STDLIB_MODULES + `map_builtin_name` pattern | Float.from(42) fits the established Type.method(arg) pattern |

**Key insight:** Almost all runtime machinery already exists. This phase is primarily about wiring up the trait registry, type checker, and MIR lowerer -- not about writing new runtime code.

## Common Pitfalls

### Pitfall 1: Trait Type Args Lost During infer_impl_def
**What goes wrong:** `infer_impl_def` currently extracts only the trait name from the first PATH node, ignoring GENERIC_ARG_LIST children. `impl From<Int> for Float` registers as trait_name="From" with no record of `<Int>`.
**Why it happens:** When `infer_impl_def` was written, no parameterized trait impls existed. The generic args on impl blocks' trait path were never consumed.
**How to avoid:** In `infer_impl_def` (infer.rs:2872), after extracting trait_name from the first PATH, also look for a GENERIC_ARG_LIST child node and parse its type arguments. Store them in the ImplDef.
**Warning signs:** All `From` impls for the same target type appear identical to the trait registry (cannot distinguish `From<Int> for String` from `From<Float> for String`).

### Pitfall 2: Mangled Name Collision for Multiple From Impls
**What goes wrong:** `From<Int> for String` and `From<Float> for String` both produce mangled name `From__from__String` since `extract_impl_names` only uses trait_name and impl_type_name.
**Why it happens:** The mangling scheme `Trait__method__Type` does not include trait type arguments.
**How to avoid:** Extend the mangling to include trait type args: `From_Int__from__String` vs `From_Float__from__String`. Update `extract_impl_names` and the corresponding call sites in `lower_item`.
**Warning signs:** Linker duplicate symbol errors or wrong conversion function called at runtime.

### Pitfall 3: find_impl Cannot Distinguish Parameterized Impls
**What goes wrong:** `find_impl("From", &Ty::string())` returns the FIRST From impl for String, regardless of whether the caller wants `From<Int>` or `From<Float>`. All From-for-String impls match.
**Why it happens:** `find_impl` matches on `(trait_name, impl_type)` only. It has no concept of trait type arguments.
**How to avoid:** Add `find_impl_with_type_args` that also matches on `trait_type_args`. Use this for From/Into lookups where the source type matters.
**Warning signs:** Type checker finds wrong From impl; wrong conversion function dispatched.

### Pitfall 4: Synthetic Into Registration Triggers Duplicate Detection
**What goes wrong:** When `impl From<Int> for Float` is registered, the synthetic `impl Into<Float> for Int` is auto-generated. If user also writes `impl Into<Float> for Int` explicitly, the duplicate detection fires.
**Why it happens:** The post-registration hook generates a concrete impl that the trait registry cannot distinguish from a user-written one.
**How to avoid:** Either (a) skip synthesis if an explicit Into impl already exists for the same type pair, or (b) mark synthetic impls and exclude them from duplicate detection against user impls.
**Warning signs:** DuplicateImpl error when user writes both From and Into for the same type pair.

### Pitfall 5: ? Operator Type Check Must Not Break Existing Same-Error-Type Case
**What goes wrong:** The current `infer_try_expr` directly unifies error types (line 6994). If the From-based fallback is inserted incorrectly (e.g., by checking From before attempting unification), it could interfere with the common case where error types already match.
**Why it happens:** From lookup is more expensive than simple unification and should only be attempted when unification fails.
**How to avoid:** Always try direct unification first. Only fall back to From lookup when unification fails AND both error types are concrete (resolved). If either error type is still an inference variable, prefer unification.
**Warning signs:** Existing try_result tests fail; programs that used to compile now require explicit error type annotations.

### Pitfall 6: Static Method Syntax Float.from() vs Instance Method value.into()
**What goes wrong:** `Float.from(42)` is a static call (no self parameter), while `42.into()` is an instance method call (has self). The MIR lowerer handles these through completely different code paths: static calls go through `lower_field_access` -> STDLIB_MODULES, instance calls go through method call interception -> `resolve_trait_callee`.
**Why it happens:** Mesh uses `.` for both module-qualified static calls and instance method calls, but routes them differently based on whether the base is a known module/type name.
**How to avoid:** For `Float.from(42)`: add "from" to the Float module functions in `stdlib_modules()` (infer.rs) and "float_from" -> "mesh_int_to_float" to `map_builtin_name` (lower.rs). For `value.into()`: register Into trait method so resolve_trait_callee dispatches it.
**Warning signs:** `Float.from(42)` resolves as a field access instead of a function call; `42.into()` cannot find the method.

### Pitfall 7: MIR Lowering Must Distinguish From Source Types
**What goes wrong:** When lowering `From__from__String` with an Int argument, the codegen should call `mesh_int_to_string`. With a Float argument, it should call `mesh_float_to_string`. But the mangled name alone doesn't tell you the argument type.
**Why it happens:** Standard trait dispatch uses the implementing type (String) for mangling, not the trait type parameter (Int vs Float).
**How to avoid:** Include the trait type arg in the mangled name: `From_Int__from__String` maps to `mesh_int_to_string`, `From_Float__from__String` maps to `mesh_float_to_string`. Add these as explicit entries in the codegen dispatch table (similar to how Display__to_string__Int maps to mesh_int_to_string).
**Warning signs:** Codegen emits wrong runtime call, or "unresolved symbol" linker error.

## Code Examples

### Example 1: User-Facing Syntax

```mesh
# Defining From<T> impls for custom types
struct Celsius do
  value :: Float
end

struct Fahrenheit do
  value :: Float
end

impl From<Celsius> for Fahrenheit do
  fn from(c :: Celsius) -> Fahrenheit do
    Fahrenheit { value: c.value * 1.8 + 32.0 }
  end
end

fn main() do
  let c = Celsius { value: 100.0 }
  let f = Fahrenheit.from(c)
  println("${f.value}")   # 212.0

  # Built-in conversions
  let x = Float.from(42)       # 42.0
  let s = String.from(42)      # "42"
  let s2 = String.from(3.14)   # "3.14"
  let s3 = String.from(true)   # "true"
end
```

### Example 2: ? Operator with Error Conversion

```mesh
struct AppError do
  message :: String
  code :: Int
end

impl From<String> for AppError do
  fn from(msg :: String) -> AppError do
    AppError { message: msg, code: 0 }
  end
end

fn parse_input(s :: String) -> Int!String do
  if s == "" do
    return Err("empty input")
  end
  Ok(42)
end

fn process() -> Int!AppError do
  # The ? auto-converts String error to AppError via From<String>
  let n = parse_input("hello")?
  Ok(n + 1)
end
```

### Example 3: Built-in From Trait Registration (Rust)

```rust
// In register_compiler_known_traits (builtins.rs)

// ── From<T> trait ──────────────────────────────────────────────
registry.register_trait(TraitDef {
    name: "From".to_string(),
    methods: vec![TraitMethodSig {
        name: "from".to_string(),
        has_self: false,  // static method: fn from(value :: T) -> Self
        param_count: 1,
        return_type: None, // Self -- resolved per impl
        has_default_body: false,
    }],
    associated_types: vec![],
});

// ── Into<T> trait ──────────────────────────────────────────────
registry.register_trait(TraitDef {
    name: "Into".to_string(),
    methods: vec![TraitMethodSig {
        name: "into".to_string(),
        has_self: true,
        param_count: 0,
        return_type: None, // T -- the target type
        has_default_body: false,
    }],
    associated_types: vec![],
});

// ── Built-in From impls ───────────────────────────────────────

// impl From<Int> for Float
{
    let mut methods = FxHashMap::default();
    methods.insert("from".to_string(), ImplMethodSig {
        has_self: false,
        param_count: 1,
        return_type: Some(Ty::float()),
    });
    let _ = registry.register_impl(ImplDef {
        trait_name: "From".to_string(),
        trait_type_args: vec![Ty::int()],
        impl_type: Ty::float(),
        impl_type_name: "Float".to_string(),
        methods,
        associated_types: FxHashMap::default(),
    });
    // Synthetic Into<Float> for Int is auto-generated by register_impl hook
}

// impl From<Int> for String
// impl From<Float> for String
// impl From<Bool> for String
// ... (similar pattern, mapping to existing runtime functions)
```

### Example 4: Trait Type Args in infer_impl_def (Rust)

```rust
// In infer_impl_def (infer.rs), after extracting trait_name:

// Extract trait type arguments from GENERIC_ARG_LIST if present.
let trait_type_args: Vec<Ty> = paths
    .first()
    .into_iter()
    .flat_map(|path| path.children())
    .filter(|n| n.kind() == SyntaxKind::GENERIC_ARG_LIST)
    .flat_map(|gal| {
        // Parse each type argument in the generic arg list
        gal.children()
            .filter(|n| n.kind() == SyntaxKind::TYPE_ANNOTATION || n.kind() == SyntaxKind::IDENT)
            .filter_map(|n| {
                // Simple case: bare IDENT token like "Int"
                n.children_with_tokens()
                    .filter_map(|t| t.into_token())
                    .find(|t| t.kind() == SyntaxKind::IDENT)
                    .map(|t| name_to_type(&t.text().to_string()))
            })
    })
    .collect();
```

### Example 5: Mangled Name Extension

```rust
// Current mangling: Trait__method__Type
// New mangling for parameterized traits: Trait_TypeArg__method__ImplType

fn mangle_trait_method(
    trait_name: &str,
    trait_type_args: &[String],
    method_name: &str,
    impl_type_name: &str,
) -> String {
    if trait_type_args.is_empty() {
        format!("{}__{}__{}", trait_name, method_name, impl_type_name)
    } else {
        let args_str = trait_type_args.join("_");
        format!("{}_{}__{}__{}", trait_name, args_str, method_name, impl_type_name)
    }
}

// Examples:
// From<Int> for Float  -> "From_Int__from__Float"
// From<Int> for String -> "From_Int__from__String"
// From<Float> for String -> "From_Float__from__String"
// Display for Int -> "Display__to_string__Int" (unchanged)
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No parameterized trait impl support | Must add trait_type_args to ImplDef | Phase 77 (now) | Enables From<T>, future parameterized traits |
| ? operator: direct error type unification | Must add From-based fallback conversion | Phase 77 (now) | Enables cross-error-type ? usage |
| Int.to_float() module function | Float.from(42) as trait-based conversion | Phase 77 (now) | Unified conversion interface |
| Manual error wrapping | Automatic From-based ? conversion | Phase 77 (now) | Ergonomic error handling |

## Requirement Mapping

| Requirement | What It Needs | Implementation Approach |
|-------------|---------------|------------------------|
| CONV-01: User defines From<T> impls | Parser: already supports `impl Trait<T> for Type`. TypeChecker: extract trait type args in infer_impl_def. TraitRegistry: store trait_type_args in ImplDef. | Extend infer_impl_def to parse GENERIC_ARG_LIST from trait path. Add trait_type_args field to ImplDef. |
| CONV-02: Auto Into from From | TraitRegistry: post-registration hook. When From<A> for B registered, synthesize Into<B> for A. | Add synthesis logic in register_impl when trait_name == "From". |
| CONV-03: Built-in From impls | Builtins: register From<Int> for Float, From<Int/Float/Bool> for String. Codegen: map From mangled names to existing intrinsics/runtime. | Registration in register_compiler_known_traits. Dispatch mapping in resolve_trait_callee/codegen. |
| CONV-04: ? auto-converts errors via From | TypeChecker: extend infer_try_expr to accept From-convertible error types. MIR: extend lower_try_result to insert From.from() call. | Fallback after failed error type unification; wrap error in From call during MIR lowering. |

## File Touch Points

### mesh-typeck (Type System)
1. **`traits.rs`** -- Add `trait_type_args: Vec<Ty>` to ImplDef. Add `find_impl_with_type_args` method. Add `has_impl_with_type_args` method. Add synthetic Into generation in register_impl. Update all existing ImplDef constructions to include `trait_type_args: vec![]`.
2. **`builtins.rs`** -- Register From<T> and Into<T> trait definitions. Register built-in From impls (Int->Float, Int->String, Float->String, Bool->String). Add Float/String "from" entries to stdlib_modules().
3. **`infer.rs`** -- Extend infer_impl_def to extract trait GENERIC_ARG_LIST and store as trait_type_args. Extend infer_try_expr to accept From-convertible mismatched error types. Add "from" to Float/String/Int module functions in stdlib_modules(). Optionally add polymorphic `into()` function.

### mesh-codegen (Code Generation)
4. **`mir/lower.rs`** -- Extend `extract_impl_names` to also extract trait type args. Extend mangling to include trait type args. Extend `lower_try_result` to insert From conversion call when error types differ. Add From dispatch mappings to `resolve_trait_callee` (From_Int__from__Float -> mesh_int_to_float, etc.). Add `map_builtin_name` entries for float_from, string_from, etc.
5. **`codegen/expr.rs`** -- Add From__from dispatch entries to codegen intrinsics (route to existing mesh_int_to_float, mesh_int_to_string, etc.).
6. **`codegen/intrinsics.rs`** -- Possibly declare new function signatures if needed (though most exist already).

### mesh-rt (Runtime)
7. **No new runtime functions needed** for built-in conversions. Existing functions cover all required conversions. User-defined From impls generate normal trait method functions that the user implements.

### Test Files
8. **New e2e tests** -- From impl for custom types, built-in conversions (Float.from, String.from), automatic Into generation, ? operator error conversion.
9. **New compile-fail tests** -- Missing From impl for ? conversion, type mismatch in From impls.

## Open Questions

1. **Should `into()` be a free function or method?**
   - What we know: Rust uses `value.into()` as a method call. Mesh could support `42.into()` (method on Int) or `into(42)` (free function). The success criteria mention `Into<B>` being "available on values of type A", suggesting method call syntax.
   - What's unclear: Whether `42.into()` should require a type annotation on the binding (`let f: Float = 42.into()`) or whether context-driven inference is sufficient.
   - Recommendation: Support `value.into()` as a method call routed through `resolve_trait_callee`. Require type annotations for disambiguation since HM inference flows bottom-up. Defer `into()` as a free function to simplify the initial implementation; it can be added later. Focus on `Type.from(value)` as the primary user-facing API since it is unambiguous.

2. **Mangling scheme for trait type args**
   - What we know: Current scheme is `Trait__method__Type`. Need to extend for parameterized traits.
   - What's unclear: Whether to use underscores (`From_Int__from__String`) or a different separator to avoid ambiguity with type names containing underscores.
   - Recommendation: Use `Trait_TypeArg__method__ImplType` with single underscore between trait name and type arg. Mesh type names are PascalCase and never contain underscores, so this is unambiguous.

3. **Priority of explicit Into impl vs synthetic Into**
   - What we know: Synthetic Into is auto-generated from From. If user also writes explicit Into, there's a conflict.
   - What's unclear: Should user-written Into override the synthetic one, or should it be an error?
   - Recommendation: Make it an error (DuplicateImpl). Users who write From should never need to also write Into. If they want custom Into behavior, they should not write From for that pair.

4. **Extracting GENERIC_ARG_LIST from trait path in infer_impl_def**
   - What we know: The parser produces GENERIC_ARG_LIST as a child of the IMPL_DEF node (after the first PATH, before FOR_KW). The existing code extracts PATH children but doesn't look at GENERIC_ARG_LIST.
   - What's unclear: The exact CST structure -- is GENERIC_ARG_LIST a child of PATH or a sibling?
   - Recommendation: Inspect the CST output for `impl From<Int> for Float` to determine the exact tree structure before implementing extraction. The parser code (items.rs:665-678) shows GENERIC_ARG_LIST is created as a child of IMPL_DEF, separate from PATH nodes.

## Sources

### Primary (HIGH confidence)
- `crates/mesh-typeck/src/traits.rs` -- TraitRegistry, ImplDef (no trait_type_args), find_impl (no type arg matching), register_impl (no From synthesis)
- `crates/mesh-typeck/src/infer.rs:2872-3047` -- infer_impl_def (no GENERIC_ARG_LIST extraction)
- `crates/mesh-typeck/src/infer.rs:6900-7045` -- infer_try_expr (direct error type unification, no From fallback)
- `crates/mesh-typeck/src/builtins.rs:821-1485` -- register_compiler_known_traits (no From/Into)
- `crates/mesh-typeck/src/infer.rs:590-598` -- Int/Float module stdlib definitions (no "from" function)
- `crates/mesh-codegen/src/mir/lower.rs:7931-8060` -- lower_try_expr/lower_try_result (no From conversion)
- `crates/mesh-codegen/src/mir/lower.rs:5298-5388` -- resolve_trait_callee (dispatch pattern)
- `crates/mesh-codegen/src/mir/lower.rs:92-122` -- extract_impl_names (no trait type arg extraction)
- `crates/mesh-codegen/src/mir/lower.rs:9408-9413` -- STDLIB_MODULES list
- `crates/mesh-codegen/src/mir/lower.rs:9414-9682` -- map_builtin_name dispatch table
- `crates/mesh-codegen/src/codegen/expr.rs:790-804` -- mesh_int_to_float/mesh_float_to_int codegen intrinsics
- `crates/mesh-rt/src/string.rs:115-131` -- mesh_int_to_string, mesh_float_to_string, mesh_bool_to_string
- `crates/mesh-parser/src/parser/items.rs:648-719` -- parse_impl_def (already parses GENERIC_ARG_LIST on trait name)
- `.planning/research/FEATURES.md` -- From/Into feature analysis
- `.planning/research/PITFALLS.md` -- Pitfall 3 (blanket impl recursion)

### Secondary (MEDIUM confidence)
- `.planning/research/SUMMARY.md` -- v7.0 research noting synthetic impl generation approach
- `.planning/phases/74-associated-types/74-RESEARCH.md` -- Associated type patterns informing trait extension approach

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- no new dependencies; all changes to existing crates verified against source code
- Architecture: HIGH -- all integration points identified with exact line numbers; existing dispatch patterns well-understood
- Pitfalls: HIGH -- 7 pitfalls identified from direct codebase analysis; each verified against specific code paths
- Code examples: HIGH -- patterns derived from existing codebase conventions (builtins registration, module function pattern, trait dispatch)

**Research date:** 2026-02-13
**Valid until:** 2026-03-13 (stable -- compiler internals don't change externally)
