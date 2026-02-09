# Phase 30: Core Method Resolution - Research

**Researched:** 2026-02-08
**Domain:** Compiler feature -- method dot-syntax desugaring (type checker + MIR lowering)
**Confidence:** HIGH

## Summary

Phase 30 adds `value.method(args)` syntax that resolves to trait impl methods, with the receiver automatically passed as the first argument. The implementation is pure desugaring at two integration points -- the type checker (`infer_field_access` / `infer_call` in `snow-typeck/src/infer.rs`) and MIR lowering (`lower_call_expr` in `snow-codegen/src/mir/lower.rs`). No new CST nodes, MIR nodes, or runtime mechanisms are needed. The parser already produces the correct CST structure: `CALL_EXPR { FIELD_ACCESS { base, ".", method_name }, ARG_LIST { args } }`, confirmed by the `parser_tests__mixed_postfix` snapshot test.

The core work involves two changes: (1) In the type checker, when `infer_call` encounters a `CallExpr` whose callee is a `FieldAccess`, detect this as a potential method call. After existing resolution steps (module, service, variant, struct field) fail in `infer_field_access`, fall back to trait method resolution via `TraitRegistry.resolve_trait_method(method_name, receiver_type)`. Return the method's function type so `infer_call` can prepend the receiver type and unify correctly. (2) In MIR lowering, `lower_call_expr` intercepts the `CallExpr(FieldAccess(...))` pattern BEFORE calling `lower_field_access` on the callee. It extracts the receiver expression and method name from the AST, lowers the receiver, prepends it to the args, and feeds the resulting `(method_name, [receiver, ...args])` into the existing trait dispatch logic (which already handles `find_method_traits` + mangling to `Trait__Method__Type`). The method call becomes a standard `MirExpr::Call` -- identical to what bare-name calls like `to_string(point)` produce today.

Key infrastructure already exists: `TraitRegistry.find_method_traits(method_name, ty)` returns all traits providing a method for a type (using structural unification for generics), `TraitRegistry.resolve_trait_method(method_name, ty)` returns the method's return type, `AmbiguousMethod` error variant (E0027) is defined but not yet wired, and the `Trait__Method__Type` mangling scheme with primitive builtin short-circuits is fully operational. The `NoSuchField` error (E0009) needs to be replaced with a new `NoSuchMethod` error when the context is a method call. An estimated ~150 lines in the type checker and ~100 lines in MIR lowering, plus tests.

**Primary recommendation:** Implement method resolution as a fallback in `infer_field_access` (after struct field lookup fails) and intercept `CallExpr(FieldAccess(...))` in `lower_call_expr` before it reaches `lower_field_access`. Both paths converge to existing trait dispatch infrastructure. No new MIR nodes.

## Standard Stack

### Core

This phase modifies existing compiler crates only. No new dependencies.

| Crate | File | Purpose | Why Modified |
|-------|------|---------|--------------|
| `snow-typeck` | `infer.rs` | Type inference engine | Add method resolution fallback in `infer_field_access`, detect FieldAccess callee in `infer_call` |
| `snow-typeck` | `traits.rs` | Trait registry | Already has `find_method_traits` and `resolve_trait_method` -- no changes needed |
| `snow-typeck` | `error.rs` | Type error definitions | Add `NoSuchMethod` error variant |
| `snow-typeck` | `diagnostics.rs` | Error rendering | Add diagnostic rendering for `NoSuchMethod` |
| `snow-codegen` | `mir/lower.rs` | MIR lowering | Intercept method call pattern in `lower_call_expr` |

### Supporting

| Crate | File | Purpose | Status |
|-------|------|---------|--------|
| `snow-parser` | `parser/expressions.rs` | Pratt parser postfix loop | No changes -- already produces correct CST |
| `snow-parser` | `ast/expr.rs` | AST node types | No changes -- `FieldAccess` and `CallExpr` already have needed accessors |
| `snow-codegen` | `mir/mod.rs` | MIR node definitions | No changes -- `MirExpr::Call` reused for method calls |
| `snow-codegen` | `mir/mono.rs` | Monomorphization | No changes -- method calls use standard `Call` nodes, automatically tracked |
| `snow-codegen` | `codegen/` | LLVM codegen | No changes -- receives desugared `MirExpr::Call` |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Desugaring in `infer_call` | New `MirExpr::MethodCall` variant | Would require changes to `mir/mod.rs`, `mono.rs`, `codegen/expr.rs` -- much more invasive for zero semantic benefit |
| Method resolution in `infer_field_access` | Method resolution in `infer_call` only (option b from PITFALLS.md) | Cleaner separation but requires `infer_field_access` to still return useful info when it fails on a method name; the fallback approach is simpler |
| `NoSuchMethod` error variant | Reuse `NoSuchField` with modified message | Less clear diagnostics; separate variant better supports DIAG-01 requirement |

## Architecture Patterns

### CST Structure for Method Calls (No Parser Changes)

The parser already produces the correct CST for `point.to_string()`:

```
CALL_EXPR
  FIELD_ACCESS         <-- callee is a FieldAccess
    NAME_REF "point"   <-- receiver expression
    DOT "."
    IDENT "to_string"  <-- method name
  ARG_LIST
    L_PAREN R_PAREN    <-- explicit args (none in this case)
```

Source: `crates/snow-parser/tests/snapshots/parser_tests__mixed_postfix.snap` -- confirmed with `a.b(c)` producing exactly `CALL_EXPR(FIELD_ACCESS(a, b), ARG_LIST(c))`.

### Pattern 1: Method Resolution Fallback in Type Checker

**What:** In `infer_field_access`, after struct field lookup fails and the expression is a method call context, try trait method resolution.

**When to use:** When `infer_call` detects its callee is a `FieldAccess` and the field name is not a struct field on the resolved base type.

**Resolution Priority (critical invariant -- must be preserved):**
1. Module-qualified (`String.length`) -- line 3903 in infer.rs (existing)
2. Service module (`Counter.get`) -- line 3917 in infer.rs (existing)
3. Sum type variant (`Shape.Circle`) -- line 3934 in infer.rs (existing)
4. Struct field (`point.x`) -- line 3962 in infer.rs (existing)
5. **NEW: Trait method (`point.to_string()`)** -- only when steps 1-4 fail
6. Error: `NoSuchMethod` (not `NoSuchField`) when in method-call context

**Implementation approach:**

```rust
// In infer_field_access, after struct field lookup fails (around line 3982):
// Instead of immediately returning NoSuchField, check trait registry.

// Option A: Pass is_callee flag from infer_call
// Option B: Always try method resolution as fallback, return function type

// When method resolution succeeds:
// - Return the method's function type: Ty::Fun([receiver_type, ...params], return_type)
// - infer_call will handle unification with actual args

// When method resolution fails:
// - If is_callee context: emit NoSuchMethod error
// - If not callee context: emit NoSuchField error (existing behavior)
```

### Pattern 2: Method Call Desugaring in MIR Lowering

**What:** In `lower_call_expr`, detect when the callee is a `FieldAccess` AST node, extract receiver + method name, prepend receiver to args, feed into existing trait dispatch.

**When to use:** Every time `lower_call_expr` processes a `CallExpr` whose AST callee is an `Expr::FieldAccess`.

**Implementation approach:**

```rust
// In lower_call_expr (around line 3362), BEFORE lowering the callee:
fn lower_call_expr(&mut self, call: &CallExpr) -> MirExpr {
    // Check if callee is a FieldAccess (method call pattern)
    if let Some(Expr::FieldAccess(fa)) = call.callee() {
        // Check if this is NOT a module/service/variant (those are handled by lower_field_access)
        if !self.is_module_or_variant_access(&fa) {
            let method_name = fa.field().map(|t| t.text().to_string()).unwrap_or_default();
            let receiver = fa.base().map(|e| self.lower_expr(&e)).unwrap_or(MirExpr::Unit);

            let mut args = vec![receiver];
            if let Some(arg_list) = call.arg_list() {
                for arg in arg_list.args() {
                    args.push(self.lower_expr(&arg));
                }
            }

            let ty = self.resolve_range(call.syntax().text_range());

            // Feed into existing trait dispatch logic
            // (same path as bare-name calls like `to_string(point)`)
            let callee = MirExpr::Var(method_name, /* fn type */);
            // ... existing trait dispatch code handles mangling ...
        }
    }
    // ... existing lower_call_expr logic ...
}
```

### Pattern 3: Shared Trait Dispatch Helper

**What:** Extract the existing trait method dispatch logic (lower.rs lines 3527-3600) into a helper function that both bare-name calls and method dot-syntax calls use.

**Why:** Prevents duplicated dispatch logic (Pitfall 10 from PITFALLS.md). Ensures `to_string(point)` and `point.to_string()` produce identical MIR.

```rust
// Extract from lower_call_expr:
fn resolve_trait_method_callee(
    &self,
    method_name: &str,
    first_arg_ty: &MirType,
    var_ty: &MirType,
) -> MirExpr {
    let ty_for_lookup = mir_type_to_ty(first_arg_ty);
    let matching_traits = self.trait_registry.find_method_traits(method_name, &ty_for_lookup);
    if !matching_traits.is_empty() {
        let trait_name = &matching_traits[0]; // typeck already checked ambiguity
        let type_name = mir_type_to_impl_name(first_arg_ty);
        let mangled = format!("{}__{}__{}", trait_name, method_name, type_name);
        // Handle primitive builtin redirects...
        MirExpr::Var(resolved_name, var_ty.clone())
    } else {
        // Fallback for monomorphized generics...
    }
}
```

### Anti-Patterns to Avoid

- **Do not add a new MIR node.** Method calls must desugar to `MirExpr::Call` so monomorphization, codegen, and all downstream passes work unchanged. Adding `MirExpr::MethodCall` would require updating `collect_function_refs` in mono.rs, `codegen_expr` in expr.rs, and the `ty()` method on MirExpr.

- **Do not call `lower_field_access` for method calls.** This produces `MirExpr::FieldAccess` (struct GEP), which codegen cannot use as a callee. Method calls must be intercepted in `lower_call_expr` BEFORE the callee is lowered.

- **Do not insert `self` into the calling scope.** Method call resolution must only unify the receiver's type with the method's first parameter type. Pushing `self` into the env would shadow actor `self` references.

- **Do not attempt UFCS (Universal Function Call Syntax).** Only trait impl methods are callable via dot-syntax. Free functions are not. The pipe operator handles free-function chaining.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Trait method lookup | Custom impl search | `TraitRegistry.find_method_traits(name, ty)` | Already handles structural unification for generics, parametric impls, multi-trait search |
| Method return type resolution | Manual type construction | `TraitRegistry.resolve_trait_method(name, ty)` | Already handles freshening, unification, and return type variable resolution |
| Ambiguity detection | Custom multi-match logic | `find_method_traits` returning `Vec<String>` with length check | Already collects all matching traits; check `len() > 1` |
| Name mangling | New mangling scheme | Existing `Trait__Method__Type` pattern | Already used by all trait dispatch; primitive redirects already implemented |
| Structural type matching | String-based type comparison | `freshen_type_params` + `InferCtx.unify` in temporary context | Handles generic impls like `impl Display for List<T>` matching `List<Int>` |

**Key insight:** The TraitRegistry was designed with this exact use case in mind. `find_method_traits` and `resolve_trait_method` already exist and handle all the hard cases (generics, parametric impls, multi-trait disambiguation). The phase is wiring, not invention.

## Common Pitfalls

### Pitfall 1: Resolution Priority Regression

**What goes wrong:** Method resolution inserted at the wrong point in `infer_field_access`'s priority chain breaks existing syntax: `String.length` becomes a method call instead of a module-qualified function, or `Shape.Circle` becomes a method call instead of a variant constructor.

**Why it happens:** `infer_field_access` resolves `expr.ident` through a linear if/else chain. All five existing cases (module, service, variant, struct field, fallback) must remain in their current order. Method resolution must come after ALL of them.

**How to avoid:** Method resolution is step 5 (after struct field at step 4). Only triggers when: (a) base type is resolved to a concrete type, (b) struct field lookup failed, (c) we are in a method-call context (callee of a CallExpr). Test every priority conflict: module member vs method, variant vs method, struct field vs method.

**Warning signs:** Any existing test involving `String.length`, `Shape.Circle`, `Counter.get`, or `point.x` starts failing.

### Pitfall 2: FieldAccess MIR Node Instead of Call

**What goes wrong:** In MIR lowering, `lower_call_expr` calls `lower_expr` on the callee, which dispatches to `lower_field_access`, producing `MirExpr::FieldAccess`. This struct-GEP node appears as the callee of a Call, and codegen crashes trying to do a struct field access on a non-struct type.

**Why it happens:** The existing `lower_call_expr` (line 3362) calls `self.lower_expr(&callee_expr)` unconditionally. For method calls, the callee is `Expr::FieldAccess`, which routes to `lower_field_access` (line 3705), which produces `MirExpr::FieldAccess { object, field, ty }`. The trait dispatch logic at line 3530 only fires when callee is `MirExpr::Var`.

**How to avoid:** Intercept the `CallExpr(FieldAccess(...))` pattern in `lower_call_expr` BEFORE calling `lower_expr` on the callee. Extract receiver and method name from the AST `FieldAccess` node directly, build `MirExpr::Var(method_name, ...)` as the callee, and feed into existing trait dispatch.

**Warning signs:** LLVM codegen panics with "Field access on non-struct type" or "expected struct pointer for GEP".

### Pitfall 3: Type Variable Receiver

**What goes wrong:** When the receiver's type is still `Ty::Var` (unresolved type variable) at method resolution time, `find_method_traits` returns empty -- no traits match an unresolved variable. The current fallback at line 3992 returns `Ok(ctx.fresh_var())`, causing cascading type errors downstream.

**Why it happens:** HM inference processes expressions left-to-right. A method call on a variable whose type is constrained later will have an unresolved receiver type.

**How to avoid:** When receiver type resolves to `Ty::Var` in method-call context, emit a specific error: "cannot call method on value of unknown type -- add type annotation." Do NOT return a fresh variable for method calls. For Phase 30 (concrete types only), this is acceptable. Phase 31 handles generics.

**Warning signs:** Method calls producing `?N` type variables instead of concrete return types; cascading "expected X, found ?N" errors.

### Pitfall 4: Duplicated Dispatch Logic

**What goes wrong:** The MIR lowerer has trait dispatch at lines 3527-3600 for bare-name calls (`to_string(42)`). Adding a second dispatch path for dot-syntax calls (`42.to_string()`) with slightly different logic causes semantic divergence: the same method produces different MIR depending on how it is called.

**Why it happens:** Copy-paste of dispatch logic with subtle differences (different primitive builtin handling, different mangling, different fallback behavior).

**How to avoid:** Extract the trait dispatch logic (lines 3527-3600) into a helper function. Both bare-name calls and method dot-syntax calls feed into this single helper. Test that `to_string(point)` and `point.to_string()` produce identical MIR.

**Warning signs:** A method callable via bare name produces different runtime results when called via dot-syntax.

### Pitfall 5: NoSuchField Error for Method Calls (DIAG-01)

**What goes wrong:** When a user writes `point.nonexistent()`, the type checker currently produces `"type Point has no field nonexistent"` (TypeError::NoSuchField, E0009). This is confusing because the user wrote a method call, not a field access.

**Why it happens:** `infer_field_access` does not distinguish between `point.nonexistent` (field access) and `point.nonexistent()` (method call). It produces `NoSuchField` for both.

**How to avoid:** Add a `TypeError::NoSuchMethod` variant. When method resolution fails and the FieldAccess is the callee of a CallExpr, emit `NoSuchMethod { ty, method_name, span }` instead of `NoSuchField`. This directly addresses requirement DIAG-01.

**Warning signs:** Users seeing "no field" errors when they wrote method calls.

## Code Examples

### Existing: TraitRegistry Method Resolution (Already Works)

```rust
// Source: crates/snow-typeck/src/traits.rs, line 212
pub fn resolve_trait_method(
    &self,
    method_name: &str,
    arg_ty: &Ty,
) -> Option<Ty> {
    for impl_list in self.impls.values() {
        for impl_def in impl_list {
            if let Some(method_sig) = impl_def.methods.get(method_name) {
                let mut ctx = InferCtx::new();
                let freshened = freshen_type_params(&impl_def.impl_type, &mut ctx);
                if ctx.unify(freshened, arg_ty.clone(), ConstraintOrigin::Builtin).is_ok() {
                    return match &method_sig.return_type {
                        Some(ret_ty) => Some(ctx.resolve(ret_ty.clone())),
                        None => None,
                    };
                }
            }
        }
    }
    None
}
```

### Existing: Ambiguity Detection (Already Works)

```rust
// Source: crates/snow-typeck/src/traits.rs, line 246
pub fn find_method_traits(&self, method_name: &str, ty: &Ty) -> Vec<String> {
    let mut trait_names = Vec::new();
    for (trait_name, impl_list) in &self.impls {
        for impl_def in impl_list {
            if impl_def.methods.contains_key(method_name) {
                let mut ctx = InferCtx::new();
                let freshened = freshen_type_params(&impl_def.impl_type, &mut ctx);
                if ctx.unify(freshened, ty.clone(), ConstraintOrigin::Builtin).is_ok() {
                    trait_names.push(trait_name.clone());
                    break;
                }
            }
        }
    }
    trait_names
}
```

### Existing: CST Parse Tree for `a.b(c)` (Parser Needs No Changes)

```
CALL_EXPR@0..6
  FIELD_ACCESS@0..3
    NAME_REF@0..1
      IDENT@0..1 "a"
    DOT@1..2 "."
    IDENT@2..3 "b"
  ARG_LIST@3..6
    L_PAREN@3..4 "("
    NAME_REF@4..5
      IDENT@4..5 "c"
    R_PAREN@5..6 ")"
```
Source: `crates/snow-parser/tests/snapshots/parser_tests__mixed_postfix.snap`

### Existing: MIR Lowering Trait Dispatch for Bare-Name Calls

```rust
// Source: crates/snow-codegen/src/mir/lower.rs, line 3527-3540
// Trait method call rewriting: if the callee is a bare method name
// (not in known_functions), check if it's a trait method for the first
// arg's type. If so, rewrite to the mangled name (Trait__Method__Type).
let callee = if let MirExpr::Var(ref name, ref var_ty) = callee {
    if !self.known_functions.contains_key(name) && !args.is_empty() {
        let first_arg_ty = args[0].ty().clone();
        let ty_for_lookup = mir_type_to_ty(&first_arg_ty);
        let matching_traits =
            self.trait_registry.find_method_traits(name, &ty_for_lookup);
        if !matching_traits.is_empty() {
            let trait_name = &matching_traits[0];
            let type_name = mir_type_to_impl_name(&first_arg_ty);
            let mangled = format!("{}__{}__{}", trait_name, name, type_name);
            // ... primitive builtin redirects ...
            MirExpr::Var(resolved, var_ty.clone())
        }
    }
};
```

### Existing: AmbiguousMethod Error Variant (Defined But Not Yet Wired)

```rust
// Source: crates/snow-typeck/src/error.rs, line 229
AmbiguousMethod {
    method_name: String,
    candidate_traits: Vec<String>,
    ty: Ty,
}
// Error code E0027, diagnostic rendering already exists in diagnostics.rs line 1284
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Bare-name calls only: `to_string(point)` | Adding dot-syntax: `point.to_string()` | Phase 30 (this phase) | Desugars to identical MIR -- both become `Display__to_string__Point(point)` |
| `NoSuchField` for all field access failures | `NoSuchMethod` when in method-call context | Phase 30 (this phase) | Better diagnostics for users writing `point.nonexistent()` |
| `AmbiguousMethod` error unused | Wired into method resolution when `find_method_traits` returns > 1 | Phase 30 (this phase) | Replaces silent first-match behavior with explicit error |

**Key design principle (established in project research):** Method dot-syntax is pure desugaring. `point.to_string()` and `to_string(point)` produce identical MIR and runtime behavior. The dot is syntactic sugar for "prepend receiver as first arg, resolve via trait registry."

## Open Questions

1. **How to pass method-call context to `infer_field_access`?**
   - What we know: `infer_field_access` does not currently know if the FieldAccess is the callee of a CallExpr. It needs this context to (a) try method resolution as fallback, and (b) emit `NoSuchMethod` instead of `NoSuchField`.
   - What's unclear: Whether to add an `is_callee: bool` parameter, or handle method calls entirely in `infer_call` (option b from PITFALLS.md).
   - Recommendation: Add an `is_method_call: bool` parameter to `infer_field_access`. When true and struct field lookup fails, try `trait_registry.resolve_trait_method`. When false, preserve existing `NoSuchField` behavior. This is the minimal change that satisfies both METH-03 and DIAG-01.

2. **Should method calls check where-clause constraints?**
   - What we know: `infer_call` (line 2713-2749) checks where-clause constraints only when callee is `NameRef`. Method calls have `FieldAccess` callee.
   - What's unclear: Whether any trait methods in the current codebase have where-clause constraints that would need checking.
   - Recommendation: Defer to Phase 31/32. Core method resolution (Phase 30) focuses on concrete types with no where-clauses. Constraint checking can be added when generic method calls are supported.

3. **Should `infer_field_access` return the full function type or just the return type for methods?**
   - What we know: For struct fields, `infer_field_access` returns the field type. For methods, the caller (`infer_call`) needs the full function type to check arity and arg types.
   - What's unclear: Whether `infer_field_access` should return `Ty::Fun([self_type, ...params], ret)` for methods, or just the return type (with `infer_call` doing arity checking separately).
   - Recommendation: Return the full function type `Ty::Fun(...)` from `infer_field_access` when resolving a method. This lets `infer_call`'s existing unification logic (`ctx.unify(callee_ty, expected_fn_ty, ...)`) handle arity checking automatically. The method's function type includes the self parameter, so `infer_call` must know to prepend the receiver to its expected function type.

## Sources

### Primary (HIGH confidence)

- **Direct codebase analysis** -- all findings verified by reading Snow compiler source code (66,521 lines of Rust)
  - `crates/snow-typeck/src/infer.rs` -- `infer_field_access` (line 3879), resolution priority chain (lines 3903-3993), `infer_call` (line 2671)
  - `crates/snow-typeck/src/traits.rs` -- `TraitRegistry`, `find_method_traits` (line 246), `resolve_trait_method` (line 212), `freshen_type_params` (line 299)
  - `crates/snow-typeck/src/error.rs` -- `TypeError::NoSuchField` (line 115), `TypeError::AmbiguousMethod` (line 229)
  - `crates/snow-typeck/src/diagnostics.rs` -- `AmbiguousMethod` rendering (line 1284), error codes E0009 (NoSuchField), E0027 (AmbiguousMethod)
  - `crates/snow-codegen/src/mir/lower.rs` -- `lower_call_expr` (line 3362), `lower_field_access` (line 3705), trait dispatch (lines 3527-3600), `lower_pipe_expr` (line 3658)
  - `crates/snow-codegen/src/mir/mod.rs` -- `MirExpr::FieldAccess` (line 208), `MirExpr::Call` (line 168)
  - `crates/snow-codegen/src/mir/types.rs` -- `mir_type_to_ty` (line 189), `mir_type_to_impl_name` (line 205)
  - `crates/snow-parser/src/parser/expressions.rs` -- postfix `FIELD_ACCESS` at BP 25 (line 117), `CALL_EXPR` at BP 25 (line 105)
  - `crates/snow-parser/src/ast/expr.rs` -- `FieldAccess.base()` (line 252), `FieldAccess.field()` (line 257), `CallExpr.callee()` (line 211)
  - `crates/snow-parser/tests/snapshots/parser_tests__mixed_postfix.snap` -- CST structure confirmation

- **Project-level research** -- `.planning/research/SUMMARY.md`, `ARCHITECTURE.md`, `FEATURES.md`, `PITFALLS.md` (all researched 2026-02-08, HIGH confidence)

### Secondary (MEDIUM-HIGH confidence)

- Rust Method Call Expressions Reference -- established method resolution patterns
- Snow compiler ROADMAP.md -- phase dependencies and prior decisions

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- direct codebase analysis, all files read and verified
- Architecture: HIGH -- two integration points identified with exact line numbers; approach validated by existing pipe operator desugaring pattern
- Pitfalls: HIGH -- all five critical pitfalls identified with exact code locations and prevention strategies, cross-referenced with project-level PITFALLS.md

**Research date:** 2026-02-08
**Valid until:** 90 days (compiler internals, stable codebase with slow-moving architecture)
