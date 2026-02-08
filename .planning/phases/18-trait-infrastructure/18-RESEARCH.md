# Phase 18: Trait Infrastructure - Research

**Researched:** 2026-02-07
**Domain:** Trait resolution, type matching, dispatch unification in a custom compiler (Rust/LLVM)
**Confidence:** HIGH

## Summary

Phase 18 fixes the foundational trait resolution machinery in `snow-typeck` so that trait impls resolve correctly for all type shapes (including parameterized types like `List<Int>`), duplicate impls are detected and rejected, method name collisions between traits produce clear errors, and compiler-known operator dispatch (hardcoded `Int`/`Float` logic in codegen) merges with `TraitRegistry`-based user trait dispatch into a single path.

The current trait system has a working `TraitRegistry` with `type_to_key` string-based lookup (`traits.rs:199-213`), which correctly handles simple types (`Int`, `Float`, etc.) but fails for generic impls (`impl Display for List<T>` cannot match `List<Int>` because `type_to_key` produces the literal string `"List<T>"` which never matches `"List<Int>"`). The fix is structural type matching using the existing `InferCtx` unification engine (`unify.rs`). Additionally, `register_impl` (`traits.rs:92-132`) uses `HashMap::insert` which silently overwrites duplicate impls, and there is no mechanism to detect method name collisions between different traits.

A critical discovery: `TraitRegistry` is created locally in `infer()` (`infer.rs:494`) and is **not** included in `TypeckResult` (`lib.rs:50-63`). It is never exposed to the MIR/codegen layer. For plan 18-03 (dispatch unification) and the subsequent Phase 19 (codegen), `TraitRegistry` must be added to `TypeckResult` and threaded to the `Lowerer` struct in `lower.rs`. This is a prerequisite for any dispatch unification work.

**Primary recommendation:** Work bottom-up: (1) replace `type_to_key` with structural matching using temporary unification, (2) add check-before-insert duplicate detection + method name collision errors, (3) expose `TraitRegistry` in `TypeckResult` and unify codegen dispatch through it.

## Standard Stack

This phase uses zero new dependencies (locked decision). All work is within existing crates.

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| snow-typeck | internal | Trait registry, unification, type inference | Contains all code being modified |
| snow-codegen | internal | MIR lowering, LLVM codegen | Codegen dispatch to be unified |
| ena | existing dep | Union-find for HM unification | Already used by `InferCtx` |
| rustc-hash | existing dep | `FxHashMap` used throughout | Already used everywhere |
| ariadne | existing dep | Error diagnostic rendering | Already used for all errors |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| rowan | existing dep | CST/AST text ranges for error spans | Error reporting for duplicate impls |
| snow-parser | internal | AST node types (ImplDef, InterfaceDef) | MIR lowerer needs AST access |

### Alternatives Considered
None. Zero new Rust crate dependencies is a locked decision.

## Architecture Patterns

### Current Architecture (What Exists)

```
snow-typeck/src/
  traits.rs          # TraitRegistry, TraitDef, ImplDef, type_to_key()
  unify.rs           # InferCtx with unification engine (ena-based)
  ty.rs              # Ty enum (Con, App, Fun, Var, Tuple, Never)
  builtins.rs        # register_compiler_known_traits() -- Add/Sub/Mul/Div/Mod/Eq/Ord/Not for Int/Float
  infer.rs           # infer_interface_def(), infer_impl_def(), infer_binary() with trait dispatch
  error.rs           # TypeError variants including TraitNotSatisfied, MissingTraitMethod
  diagnostics.rs     # Ariadne-based rendering
  lib.rs             # TypeckResult (does NOT include TraitRegistry)

snow-codegen/src/
  mir/lower.rs       # Lowerer -- currently lower_fn_def() for impl methods, hardcoded BinOp dispatch
  mir/types.rs       # resolve_type(), mangle_type_name()
  codegen/expr.rs    # codegen_binop() -- hardcoded Int/Float/Bool/String dispatch
  mir/mod.rs         # MirExpr::BinOp with BinOp enum, MirExpr::Call for function dispatch
```

### Pattern 1: Structural Type Matching via Temporary Unification

**What:** Replace `type_to_key` string comparison with structural matching. For each registered impl, attempt to unify the query type with the impl's stored `Ty` using a temporary `InferCtx`. If unification succeeds, the impl matches.

**When to use:** All impl lookups: `has_impl()`, `find_impl()`, `resolve_trait_method()`.

**Approach:**
```rust
// In TraitRegistry -- conceptual pattern:
fn find_impl_structural(&self, trait_name: &str, query_ty: &Ty) -> Option<&ImplDef> {
    for ((tn, _), impl_def) in &self.impls {
        if tn != trait_name {
            continue;
        }
        // Create a temporary unification context
        let mut temp_ctx = InferCtx::new();
        // Clone the impl type, replacing type params with fresh vars
        let impl_ty_fresh = freshen_type_params(&impl_def.impl_type, &mut temp_ctx);
        // Try unification
        if temp_ctx.unify(
            impl_ty_fresh,
            query_ty.clone(),
            ConstraintOrigin::Builtin,
        ).is_ok() {
            return Some(impl_def);
        }
    }
    None
}
```

**Key detail:** The impl's stored `Ty` may contain type variables (for generic impls like `impl Display for List<T>`). These must be freshened before unification. The query type (e.g., `List<Int>`) is always concrete at lookup time.

### Pattern 2: Check-Before-Insert for Duplicate Detection

**What:** Before inserting an impl, check if a structurally equivalent impl already exists.

**Approach:**
```rust
pub fn register_impl(&mut self, impl_def: ImplDef) -> Vec<TypeError> {
    let mut errors = Vec::new();

    // Check for duplicate/overlapping impl
    if let Some(existing) = self.find_impl_structural(&impl_def.trait_name, &impl_def.impl_type) {
        errors.push(TypeError::DuplicateImpl {
            trait_name: impl_def.trait_name.clone(),
            impl_type: impl_def.impl_type_name.clone(),
            // Include both locations for the error message
            existing_span: existing.span,  // Need to add span to ImplDef
            duplicate_span: impl_def.span,
        });
        return errors;  // Don't insert the duplicate
    }

    // ... existing validation ...
    // Store using structural key
    self.impls.push(impl_def);  // Switch from HashMap to Vec
    errors
}
```

### Pattern 3: Dispatch Unification via TraitRegistry

**What:** The codegen layer currently has two completely separate dispatch paths:
1. **Hardcoded path** (`codegen_binop` in `expr.rs:209-244`): Checks `lhs_ty` and dispatches to `codegen_int_binop`, `codegen_float_binop`, etc.
2. **TraitRegistry path** (type checker only): `infer_trait_binary_op` in `infer.rs` checks `trait_registry.has_impl()` but this information is lost before codegen.

**Unification approach:**
- Phase 18-03 exposes `TraitRegistry` in `TypeckResult`
- Phase 18-03 makes the MIR lowerer aware of trait dispatch so it can emit trait-based calls for user-defined operator impls
- The codegen binop path remains for now (it handles primitives efficiently), but the MIR lowerer will emit `MirExpr::Call` to mangled trait methods for user types instead of `MirExpr::BinOp`

**Key insight:** The unification happens at MIR lowering, not at LLVM codegen. When the MIR lowerer sees `BinOp::Add` on a struct type, it should emit a `Call` to `Add__add__MyStruct` instead of `BinOp { op: Add }`. The LLVM codegen for `BinOp` remains unchanged (it only handles primitives).

### Pattern 4: Storage Refactor for Structural Matching

**What:** The current `impls` field uses `FxHashMap<(String, String), ImplDef>` keyed by `(trait_name, type_key_string)`. With structural matching, string keys are no longer the lookup mechanism.

**Options:**
- **Option A:** Keep `FxHashMap` but use it as `FxHashMap<String, Vec<ImplDef>>` keyed by trait name, then iterate + unify
- **Option B:** Use `Vec<ImplDef>` and linear scan (simpler, fine for the number of impls in a typical program)

**Recommendation:** Option A -- `FxHashMap<String, Vec<ImplDef>>` keyed by trait name. This avoids scanning all impls across all traits, while allowing structural matching within a trait's impls.

### Anti-Patterns to Avoid

- **String-based type comparison for generics:** The existing `type_to_key` approach fails for any parameterized type. Never compare types by stringifying them.
- **Modifying the main `InferCtx` during trait lookup:** Temporary unification must use a fresh, throwaway context. Mutating the main context would corrupt ongoing inference.
- **Replacing codegen binop with trait calls for primitives:** Keep `codegen_int_binop`/`codegen_float_binop` for performance. Only route user types through trait dispatch.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Type structural comparison | String matching (`type_to_key`) | `InferCtx::unify()` with temporary context | Handles all type shapes including `App`, `Fun`, `Tuple` |
| Type variable freshening | Manual substitution | `InferCtx::fresh_var()` + substitution map | Already handles all `Ty` variants correctly |
| Error rendering | Custom format strings | Existing `diagnostics.rs` + ariadne | Consistent error output with existing errors |
| Type key generation | New hashing scheme | Structural unification (replaces keys entirely) | String keys fundamentally can't handle generic impls |

**Key insight:** The unification engine in `unify.rs` already does exactly what structural type matching needs. The only new code is creating a temporary `InferCtx` and freshening type parameters in the impl type before attempting unification.

## Common Pitfalls

### Pitfall 1: type_to_key Cannot Match Generic Impls
**What goes wrong:** `impl Display for List<T>` stores key `"List<T>"`. Query for `List<Int>` generates key `"List<Int>"`. Exact string match fails.
**Why it happens:** `type_to_key` serializes the type as-is, treating type variables as literal text rather than as placeholders to be unified.
**How to avoid:** Replace all `type_to_key` usage with structural matching via temporary unification.
**Warning signs:** Any test involving `impl Trait for GenericType<T>` will fail silently (no impl found, no error produced -- just wrong behavior).

### Pitfall 2: HashMap::insert Silently Overwrites Duplicate Impls
**What goes wrong:** Two `impl Display for Int` blocks: the second silently replaces the first. No error is produced.
**Why it happens:** `register_impl` (line 128) does `self.impls.insert(key, impl_def)` -- HashMap insert is a replace operation.
**How to avoid:** Check for existing impl before insertion. Emit `TypeError::DuplicateImpl` error with both source locations.
**Warning signs:** Tests with duplicate impls pass without errors instead of reporting the conflict.

### Pitfall 3: Method Name Collision Between Traits
**What goes wrong:** `interface A do fn foo(self) end` and `interface B do fn foo(self) end`. If a type implements both, calling `foo(x)` is ambiguous -- which trait's method should be called?
**Why it happens:** `resolve_trait_method` (line 154-168) scans all impls linearly and returns the first match. With multiple traits defining the same method name, resolution is nondeterministic (depends on HashMap iteration order).
**How to avoid:** When resolving a method name, check if multiple traits provide it for the same type. If so, emit an ambiguity error. For v1.3, qualified syntax (`Trait.method(x)`) can be deferred -- an error is sufficient.
**Warning signs:** Tests for method resolution succeed but produce different results depending on HashMap seed.

### Pitfall 4: TraitRegistry Not Available to Codegen
**What goes wrong:** `TraitRegistry` is created locally in `infer()` and dropped when inference completes. The MIR lowerer and LLVM codegen have no access to trait information.
**Why it happens:** `TypeckResult` only includes `types`, `errors`, `warnings`, `result_type`, and `type_registry`. `TraitRegistry` was not included because codegen originally skipped all trait-related nodes.
**How to avoid:** Add `trait_registry: TraitRegistry` to `TypeckResult`. Return it from `infer()`.
**Warning signs:** Plan 18-03 (dispatch unification) is impossible without this. Plan 19 (trait codegen) is also blocked.

### Pitfall 5: Freshening Type Parameters in Impl Types
**What goes wrong:** When doing structural matching, the impl type `List<T>` where `T` is a named type parameter (stored as `Ty::Con(TyCon { name: "T" })`) needs to be replaced with fresh type variables (`Ty::Var(fresh)`) before unification.
**Why it happens:** Named type parameters in impl types are stored as `Ty::Con` nodes (same as concrete types like `Int`), not as `Ty::Var`. Unifying `Ty::Con("T")` with `Ty::Con("Int")` fails (different constructors).
**How to avoid:** Track which names are type parameters in the impl's generic params. Before unification, substitute all `Ty::Con(name)` where `name` is a type param with fresh `Ty::Var`.
**Warning signs:** Structural matching works for concrete impls (`impl Display for Int`) but fails for generic impls (`impl Display for List<T>`).

### Pitfall 6: Mutating InferCtx During Lookup
**What goes wrong:** If structural matching uses the main `InferCtx` for unification, it will bind type variables in the main context, corrupting ongoing inference.
**Why it happens:** `InferCtx::unify` is stateful -- it modifies the union-find table.
**How to avoid:** Always create a temporary `InferCtx` for structural matching. Discard it after the match check.
**Warning signs:** Type inference produces wrong results after trait lookups. Variables get unexpectedly bound.

### Pitfall 7: name_to_type Only Handles Simple Types
**What goes wrong:** `name_to_type` in `infer.rs:5102-5110` converts type name strings to `Ty`. It only handles `"Int"`, `"Float"`, `"String"`, `"Bool"`, and falls through to `Ty::Con(other)`. This means `impl Display for List<Int>` would store `Ty::Con("List")` not `Ty::App(Con("List"), [Con("Int")])`.
**Why it happens:** The parser provides the impl type as path nodes, but `infer_impl_def` only extracts the base identifier, not type arguments.
**How to avoid:** Enhance `infer_impl_def` to parse type arguments from the AST (the second PATH node may include generic args). For now, check if the type name corresponds to a struct/sum in the type registry and construct the appropriate `Ty::App`.
**Warning signs:** Generic impl types are always stored as bare constructors without type parameters.

## Code Examples

### Current type_to_key implementation (to be replaced)

```rust
// Source: snow-typeck/src/traits.rs:199-213
fn type_to_key(ty: &Ty) -> String {
    match ty {
        Ty::Con(c) => c.name.clone(),
        Ty::App(con, args) => {
            let con_key = type_to_key(con);
            let arg_keys: Vec<String> = args.iter().map(type_to_key).collect();
            format!("{}<{}>", con_key, arg_keys.join(", "))
        }
        Ty::Tuple(elems) => {
            let elem_keys: Vec<String> = elems.iter().map(type_to_key).collect();
            format!("({})", elem_keys.join(", "))
        }
        _ => format!("{}", ty),
    }
}
```

### Current register_impl (overwrites without checking)

```rust
// Source: snow-typeck/src/traits.rs:92-132
pub fn register_impl(&mut self, impl_def: ImplDef) -> Vec<TypeError> {
    // ... validation ...
    let type_key = type_to_key(&impl_def.impl_type);
    self.impls.insert((impl_def.trait_name.clone(), type_key), impl_def);  // Silent overwrite!
    errors
}
```

### Current codegen binop (hardcoded type dispatch)

```rust
// Source: snow-codegen/src/codegen/expr.rs:209-244
fn codegen_binop(&mut self, op: &BinOp, lhs: &MirExpr, rhs: &MirExpr, _ty: &MirType)
    -> Result<BasicValueEnum<'ctx>, String>
{
    // ... short circuit for And/Or ...
    match lhs_ty {
        MirType::Int => self.codegen_int_binop(op, lhs_val, rhs_val),
        MirType::Float => self.codegen_float_binop(op, lhs_val, rhs_val),
        MirType::Bool => self.codegen_bool_binop(op, lhs_val, rhs_val),
        _ => Err(format!("Unsupported binop type: {:?}", lhs_ty)),  // <-- User types fail here
    }
}
```

### Current MIR lowering of impl methods (uses fn_def, no mangling)

```rust
// Source: snow-codegen/src/mir/lower.rs:424-429
Item::ImplDef(impl_def) => {
    // Lower impl methods as standalone functions.
    for method in impl_def.methods() {
        self.lower_fn_def(&method);  // Uses method's own name, no Trait__Method__Type mangling
    }
}
```

### How TypeckResult is constructed (missing TraitRegistry)

```rust
// Source: snow-typeck/src/infer.rs:629-635
TypeckResult {
    types: resolved_types,
    errors: ctx.errors,
    warnings: ctx.warnings,
    result_type: resolved_result,
    type_registry,
    // NOTE: trait_registry is NOT included -- it's dropped here
}
```

### Existing mangle_type_name (for reference -- used in generic type monomorphization)

```rust
// Source: snow-codegen/src/mir/types.rs:143-150
pub fn mangle_type_name(base: &str, args: &[Ty], registry: &TypeRegistry) -> String {
    let mut name = base.to_string();
    for arg in args {
        name.push('_');
        name.push_str(&mir_type_suffix(&resolve_type(arg, registry, false)));
    }
    name
}
// For traits, the convention will be: Trait__Method__Type (double underscore separators)
// This is different from the single-underscore used in generic type mangling
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| String-based type keys | Structural matching via unification | Phase 18-01 | Generic impls work correctly |
| Silent impl overwrite | Check-before-insert with error | Phase 18-02 | Duplicate impls detected |
| No method collision detection | Ambiguity error for shared names | Phase 18-02 | Deterministic resolution |
| Separate codegen + typeck dispatch | Unified dispatch through TraitRegistry | Phase 18-03 | User operator overloading works |
| TraitRegistry local to infer() | TraitRegistry in TypeckResult | Phase 18-03 | Codegen can access trait info |

## Open Questions

1. **ImplDef span tracking**
   - What we know: `ImplDef` in `traits.rs` has no `span` field. Error messages for duplicate impls need to point to both locations.
   - What's unclear: Should the span be a `TextRange` (from rowan) or a simpler representation?
   - Recommendation: Add `span: Option<TextRange>` to `ImplDef`. Compiler-known impls from builtins get `None`. User-defined impls get the `impl` keyword's `TextRange`.

2. **Generic impl type parsing**
   - What we know: `infer_impl_def` extracts only the base type name from the second PATH node. It does not parse generic arguments (e.g., `List<T>` in `impl Display for List<T>`).
   - What's unclear: Does the parser already produce generic args in the PATH node, or does additional parsing work need to happen?
   - Recommendation: Investigate the parser's PATH node structure for impl types during plan 18-01 implementation. The parser may already provide type arguments that `infer_impl_def` ignores.

3. **Qualified syntax for method name collision resolution**
   - What we know: The success criteria say "produce a compile-time ambiguity error or resolve via qualified syntax." Qualified syntax like `Trait.method(x)` is more work.
   - What's unclear: Is qualified syntax needed for v1.3 or can it be deferred?
   - Recommendation: For Phase 18-02, implement the ambiguity error only. Qualified syntax can be a follow-up if needed.

4. **ImplDef storage refactor timing**
   - What we know: Changing from `FxHashMap<(String, String), ImplDef>` to `FxHashMap<String, Vec<ImplDef>>` changes the API surface.
   - What's unclear: Should this be done in plan 18-01 (structural matching) or plan 18-02 (duplicate detection)?
   - Recommendation: Do it in plan 18-01 since structural matching requires iterating over impls anyway. The HashMap key change is a prerequisite for the new lookup approach.

5. **MIR lowerer dispatch for user operator types**
   - What we know: `lower_binary_expr` (line 1216) always emits `MirExpr::BinOp`. Codegen for `BinOp` only handles Int/Float/Bool/String.
   - What's unclear: Should plan 18-03 modify the MIR lowerer to emit `MirExpr::Call` for user types, or should plan 19 handle this?
   - Recommendation: Plan 18-03 should focus on exposing `TraitRegistry` and making the dispatch mechanism available. The actual call-site resolution (emitting `MirExpr::Call` instead of `MirExpr::BinOp`) is plan 19-02 territory, since it requires trait method bodies to be lowered first (plan 19-01).

## Sources

### Primary (HIGH confidence)
- `snow-typeck/src/traits.rs` -- Full TraitRegistry implementation, type_to_key function
- `snow-typeck/src/unify.rs` -- InferCtx unification engine with ena
- `snow-typeck/src/ty.rs` -- Ty enum definition with all variants
- `snow-typeck/src/builtins.rs` -- Compiler-known trait registration (Add, Sub, Mul, Div, Mod, Eq, Ord, Not)
- `snow-typeck/src/infer.rs` -- infer_interface_def, infer_impl_def, infer_binary, infer_trait_binary_op
- `snow-typeck/src/error.rs` -- TypeError variants for trait errors
- `snow-typeck/src/lib.rs` -- TypeckResult struct (missing TraitRegistry field)
- `snow-codegen/src/mir/lower.rs` -- MIR lowering, current impl method lowering, binary expr lowering
- `snow-codegen/src/codegen/expr.rs` -- LLVM codegen for binops (hardcoded type dispatch)
- `snow-codegen/src/mir/types.rs` -- resolve_type, mangle_type_name
- `snow-codegen/src/mir/mod.rs` -- MIR type definitions, BinOp enum

### Secondary (MEDIUM confidence)
- `snow-parser/src/ast/item.rs` -- ImplDef.methods() and InterfaceDef API
- `snow-typeck/tests/traits.rs` -- Existing trait test suite (13 tests)
- `.planning/milestones/v1.3-ROADMAP.md` -- Phase plan details and pitfall tracking

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all code is internal, fully readable, zero new dependencies
- Architecture: HIGH -- all patterns derived from reading the actual codebase, not external sources
- Pitfalls: HIGH -- all 7 pitfalls identified from reading actual code and tracing execution paths

**Key file locations for implementation:**
- `crates/snow-typeck/src/traits.rs` -- Primary file for plans 18-01 and 18-02
- `crates/snow-typeck/src/infer.rs` -- `infer_impl_def` needs type arg parsing (18-01), `infer()` needs to return trait_registry (18-03)
- `crates/snow-typeck/src/error.rs` -- New error variants: DuplicateImpl, AmbiguousMethod (18-02)
- `crates/snow-typeck/src/diagnostics.rs` -- Render new error variants (18-02)
- `crates/snow-typeck/src/lib.rs` -- Add TraitRegistry to TypeckResult (18-03)
- `crates/snow-codegen/src/mir/lower.rs` -- Accept TraitRegistry, prep dispatch unification (18-03)

**Line counts for key files:**
- `traits.rs`: 294 lines
- `unify.rs`: 626 lines
- `infer.rs`: ~5115 lines
- `lower.rs`: ~2000+ lines
- `error.rs`: 447 lines

**Research date:** 2026-02-07
**Valid until:** 60 days (internal codebase, stable)
