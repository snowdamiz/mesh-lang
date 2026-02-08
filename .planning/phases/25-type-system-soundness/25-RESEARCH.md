# Phase 25: Type System Soundness - Research

**Researched:** 2026-02-08
**Domain:** Constrained polymorphism, where-clause propagation in Hindley-Milner type inference
**Confidence:** HIGH

## Summary

Phase 25 fixes a type system soundness bug where higher-order constrained functions lose their trait constraints when captured as values. The root cause is that Snow's where-clause enforcement is name-based: the `fn_constraints` map stores constraints keyed by the original function name (e.g., `"show"`), but when a constrained function is captured via `let f = show`, subsequent calls use `f(...)` which looks up `"f"` in `fn_constraints` and finds nothing. The constraint silently disappears, allowing unsound calls.

The fix requires propagating constraints through the type system itself -- attaching them to the `Scheme` or `Ty` representation -- rather than relying on a side-channel name-based lookup. This is a well-understood problem in type system theory, formalized by Mark P. Jones as "qualified types" (1994). The key insight is that trait constraints should travel with the type scheme, so when `let f = show` generalizes `show`'s type into a scheme for `f`, the constraints are preserved and checked at `f`'s call site regardless of the name.

The implementation requires changes in three areas: (1) extend `Scheme` (or introduce a parallel structure) to carry where-clause constraints alongside type variables, (2) modify `infer_call` to check constraints from the callee's resolved origin rather than just looking up the callee name in `fn_constraints`, and (3) handle the indirect case where constrained functions are passed as arguments to higher-order functions.

**Primary recommendation:** Extend `fn_constraints` propagation so that when a name-ref resolves to a known constrained function (via the env), the constraints transfer to the new binding. This is the minimal-change approach that avoids restructuring `Ty`/`Scheme` while still being correct.

## Standard Stack

This phase uses zero new dependencies. All work is within existing crates.

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| snow-typeck | internal | Type inference, where-clause enforcement, Scheme/Ty types | Contains all code being modified |
| ena | existing dep | Union-find for HM unification | Already used by `InferCtx` |
| rustc-hash | existing dep | `FxHashMap` used throughout | `fn_constraints` map, `TypeEnv` |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| snow-codegen | internal | MIR lowering, defense-in-depth warning | Only if MIR needs updates |
| snow-parser | internal | AST node types (FnDef, LetBinding, CallExpr) | Read-only, for AST traversal |
| ariadne | existing dep | Error diagnostic rendering | TraitNotSatisfied error messages |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Extending fn_constraints propagation | Adding constraints to Ty::Fun itself | Cleaner theoretically (qualified types) but much larger change; modifies Ty enum used everywhere |
| Side-channel constraint map | Wrapping Scheme with ConstrainedScheme | Requires changing TypeEnv storage type; more invasive |

## Architecture Patterns

### Current Architecture (The Bug)

```
infer.rs flow for constrained functions:

1. fn show<T>(x :: T) -> String where T: Display
   - Parsed: where_constraints = [("T", "Display")]
   - Stored in: fn_constraints["show"] = FnConstraints { where_constraints, type_params, param_type_param_names }
   - Type env: env["show"] = Scheme { vars: [a], ty: Fun([Var(a)], String) }

2. let f = show
   - infer_let_binding: init_ty = infer_expr(NameRef("show")) = ctx.instantiate(env["show"])
   - binding_ty = Fun([Var(fresh_a)], String)
   - scheme = ctx.generalize(binding_ty)
   - env["f"] = Scheme { vars: [...], ty: Fun([...], String) }
   - ** NO CONSTRAINT PROPAGATION ** -- fn_constraints["f"] is NEVER set

3. f(some_non_display_value)
   - infer_call: callee = NameRef("f") -> fn_name = "f"
   - fn_constraints.get("f") -> None  <-- BUG: constraints lost!
   - Call proceeds without checking Display bound
```

### Recommended Fix: Constraint Propagation Through Let Bindings

```
Two-part fix:

Part A: Track which function a let binding aliases
  When `let f = show`:
  - Detect that init_expr is a NameRef pointing to a constrained function
  - Copy fn_constraints["show"] into fn_constraints["f"]

Part B: Track constraints through call-site indirection
  When f(...) is called:
  - fn_constraints.get("f") now finds the propagated constraints
  - Constraint checking proceeds normally
```

### Pattern 1: Direct Alias Propagation in infer_let_binding

**What:** When a let binding's initializer is a simple name reference to a constrained function, propagate the constraints to the new name.

**When to use:** `let f = show` where `show` has where-clause constraints.

**Location:** `infer.rs`, `infer_let_binding()` function (around line 2128).

**Approach:**
```rust
// After inferring init_ty in infer_let_binding:
if let Some(name) = let_.name() {
    if let Some(name_text) = name.text() {
        // Check if initializer is a NameRef to a constrained function
        if let Some(init_name) = extract_name_ref(&init_expr) {
            if let Some(source_constraints) = fn_constraints.get(&init_name) {
                // Propagate constraints to the new binding name
                fn_constraints.insert(name_text.clone(), source_constraints.clone());
            }
        }
        env.insert(name_text, scheme);
    }
}
```

**Key detail:** `fn_constraints` is currently passed as `&FxHashMap` (immutable reference) to `infer_let_binding`. It needs to become `&mut FxHashMap` for this fix.

### Pattern 2: Higher-Order Function Constraint Propagation

**What:** When a constrained function is passed as an argument to a higher-order function, the constraint must propagate to the call site where the function parameter is invoked.

**When to use:** `apply(show, value)` where `apply` calls its first argument.

**Approach:** This is harder because the constraint lives on the argument, not on the higher-order function itself. Two sub-approaches:

**Sub-approach A (simpler, covers most cases):** At the call site of the higher-order function, check if any argument is a NameRef to a constrained function. If so, when that argument's type is unified with the parameter type, the parameter type inherits the constraints. Then at the inner call site, check constraints.

**Sub-approach B (full solution):** Extend the type representation to carry constraints. This is the "qualified types" approach -- `Ty::Fun` gains an optional constraint set. This is theoretically cleaner but much more invasive.

**Recommendation:** Start with Pattern 1 (direct alias) which covers `let f = show; f(x)`. Pattern 2 can be deferred as it requires significantly more architecture changes and is a less common use case in practice.

### Pattern 3: Multi-Level Alias Chain

**What:** Handle chains like `let f = show; let g = f; g(x)`.

**Approach:** Pattern 1 already handles this naturally because when `let g = f` is processed, `fn_constraints["f"]` already exists (from the first propagation), so it propagates to `"g"`.

### Anti-Patterns to Avoid
- **Do NOT modify the Ty enum:** Adding constraints to `Ty::Fun` would require touching every function that pattern-matches on `Ty`, which is essentially the entire type checker. The Ty enum is used in hundreds of match arms across infer.rs, unify.rs, traits.rs.
- **Do NOT try to make Scheme carry constraints as a first approach:** `Scheme` is used for ALL bindings (not just constrained functions), and modifying it would affect generalization/instantiation everywhere.
- **Do NOT check constraints at generalization time:** The constraint must be checked at the CALL site when concrete types are known, not when the function is captured.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Constraint storage | New data structure for constraints | Existing `FnConstraints` struct + `fn_constraints` map | Already has where_constraints, type_params, param_type_param_names |
| Type param resolution | Custom type param tracker | Existing `param_type_param_names` in FnConstraints | Already maps arg positions to type param names |
| Constraint checking | Custom trait impl verification | Existing `trait_registry.check_where_constraints()` | Already does structural matching via temp unification |
| Name resolution | Custom function name tracker | Existing `env.lookup()` + `ctx.instantiate()` | Type environment already tracks all bindings |

**Key insight:** All the machinery for checking constraints already works correctly for direct calls (`show(42)` correctly errors when Int lacks Displayable). The only missing piece is propagating the constraint association when the function is aliased.

## Common Pitfalls

### Pitfall 1: Modifying fn_constraints Mutability

**What goes wrong:** `infer_let_binding` currently receives `fn_constraints` as `&FxHashMap` (immutable). Changing it to `&mut FxHashMap` requires updating the function signature and all callers.

**Why it happens:** The original design only needed to READ constraints at call sites, never WRITE them outside of function definition.

**How to avoid:** Trace all callers of `infer_let_binding` and update signatures. The call chain is: `infer_item` -> `infer_let_binding`. Check that `infer_item` also has `&mut` access.

**Warning signs:** Compiler errors about borrowing `fn_constraints` as mutable when already borrowed as immutable.

### Pitfall 2: Constraint Identity Under Instantiation

**What goes wrong:** When `show`'s scheme is instantiated for `let f = show`, the type params get fresh variables. The constraints reference the ORIGINAL type param names (e.g., "T"), not the fresh variables. If the constraint checking relies on matching type param names to argument positions, the param_type_param_names must also be propagated correctly.

**Why it happens:** FnConstraints stores both `type_params: FxHashMap<String, Ty>` (mapping param names to their Ty vars at definition time) and `param_type_param_names: Vec<Option<String>>` (mapping arg positions to param names). When propagated, the type_params map points to stale type variables from the original function's scope.

**How to avoid:** The constraint checking at call sites (lines 2662-2700 in infer.rs) resolves type params from the call-site argument types, not from the stored type_params. The `param_type_param_names` tells it WHICH argument positions correspond to WHICH type param names, and then it resolves the concrete types from the actual arguments. This should work correctly for aliases as long as the param_type_param_names is propagated.

**Warning signs:** Constraints appearing to be checked but always passing (because the type params resolve to unbound variables instead of concrete types).

### Pitfall 3: Closure Capture vs. Function Alias

**What goes wrong:** Confusing `let f = show` (function alias) with `let f = fn(x) do show(x) end` (closure wrapping). The closure case already works because the inner `show(x)` call checks constraints directly. Only the alias case is broken.

**Why it happens:** Both produce a value of function type, but only the alias drops constraints.

**How to avoid:** Focus the fix on detecting NameRef initializers in let bindings. Closure initializers don't need special handling.

**Warning signs:** Tests passing for closure wrappers but failing for direct aliases.

### Pitfall 4: Recursive Constraint Propagation

**What goes wrong:** If `let f = show` and then `f` is used as an argument in a higher-order call, the constraints on `f` must be checked when `f` is eventually called.

**Why it happens:** Higher-order functions receive `f` as a parameter with type `Fun([?a], String)`, but the parameter name inside the higher-order function is different (e.g., `callback`), so `fn_constraints["callback"]` doesn't exist.

**How to avoid:** For v1 (this phase), focus on the direct alias case (`let f = show; f(x)`). The higher-order case (`apply(show, x)`) is significantly harder and may require the qualified-types approach. Document this as a known limitation.

**Warning signs:** Tests for `let f = show; f(non_display)` pass, but `apply(show, non_display)` does not error.

### Pitfall 5: Breaking Existing Where-Clause Tests

**What goes wrong:** Changing `fn_constraints` from immutable to mutable reference could affect existing constraint checking if any code path reads constraints while they're being modified.

**Why it happens:** Rust's borrow checker will prevent this at compile time, but the logical flow must still be correct.

**How to avoid:** Run the full test suite after each change. The existing e2e_where_clause_enforcement test in lower.rs verifies direct call constraint checking.

**Warning signs:** Existing tests regressing after the signature change.

## Code Examples

### Example 1: The Bug (Current Behavior)

```snow
# Source: Current Snow behavior (from phase description)
interface Displayable do
  fn display(self) -> String
end

fn show<T>(x :: T) -> String where T: Displayable do
  Displayable.display(x)
end

fn main() do
  let f = show       # f gets type Fun([?a], String) -- NO constraints
  f(42)              # Should error: Int does not implement Displayable
                     # Currently: No error (unsound!)
end
```

### Example 2: The Fix (Desired Behavior)

```snow
# After fix: same source produces compile error
fn main() do
  let f = show       # fn_constraints["f"] = clone of fn_constraints["show"]
  f(42)              # infer_call checks fn_constraints["f"]
                     # Finds where_constraints: [("T", "Displayable")]
                     # Resolves T = Int (from arg type)
                     # Checks: has_impl("Displayable", &Int) -> false
                     # Error: TraitNotSatisfied { ty: Int, trait_name: "Displayable" }
end
```

### Example 3: Current infer_call Constraint Check (infer.rs:2662-2700)

```rust
// Source: crates/snow-typeck/src/infer.rs lines 2662-2700
// This is the existing constraint check that works for direct calls
// but fails for aliases because it uses the callee NAME for lookup.

// Check where-clause constraints at the call site.
if let Expr::NameRef(name_ref) = &callee_expr {
    if let Some(fn_name) = name_ref.text() {
        if let Some(constraints) = fn_constraints.get(&fn_name) {
            // ^^ This lookup fails when fn_name = "f" and constraints
            //    were only stored under "show"
            if !constraints.where_constraints.is_empty() {
                let mut resolved_type_args: FxHashMap<String, Ty> = FxHashMap::default();
                for (i, tp_name_opt) in constraints.param_type_param_names.iter().enumerate() {
                    if let Some(tp_name) = tp_name_opt {
                        if i < arg_types.len() {
                            let resolved = ctx.resolve(arg_types[i].clone());
                            resolved_type_args.insert(tp_name.clone(), resolved);
                        }
                    }
                }
                // ... fallback resolution, then:
                let errors = trait_registry.check_where_constraints(
                    &constraints.where_constraints,
                    &resolved_type_args,
                    origin,
                );
                // Report errors
            }
        }
    }
}
```

### Example 4: Proposed Fix in infer_let_binding

```rust
// Source: Proposed change to crates/snow-typeck/src/infer.rs
// In infer_let_binding, after computing scheme:

fn infer_let_binding(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    let_: &LetBinding,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &mut FxHashMap<String, FnConstraints>,  // <-- was &FxHashMap
) -> Result<Ty, TypeError> {
    // ... existing code ...

    if let Some(name) = let_.name() {
        if let Some(name_text) = name.text() {
            // Propagate where-clause constraints if RHS is a constrained function ref
            if let Some(source_name) = try_extract_name_ref(&init_expr) {
                if let Some(source_constraints) = fn_constraints.get(&source_name).cloned() {
                    fn_constraints.insert(name_text.clone(), source_constraints);
                }
            }
            env.insert(name_text, scheme);
        }
    }
    // ...
}
```

### Example 5: Helper to Extract NameRef from Expression

```rust
// Source: New helper function for crates/snow-typeck/src/infer.rs

/// Try to extract a simple name reference from an expression.
/// Returns Some(name) if the expression is a bare NameRef, None otherwise.
fn try_extract_name_ref(expr: &Expr) -> Option<String> {
    match expr {
        Expr::NameRef(name_ref) => name_ref.text(),
        _ => None,
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| String-based type_to_key for trait lookup | Structural matching via temp unification | Phase 18 (v1.3) | Generic impls resolve correctly |
| No constraint checking | fn_constraints map checked at direct call sites | Phase 18/19 (v1.3) | Direct calls enforce where-clauses |
| Name-based constraint lookup only | Need: Constraint propagation through aliases | Phase 25 (this phase) | Captured constrained functions remain sound |

**Theoretical background:**
- Mark P. Jones, "Qualified Types: Theory and Practice" (1994) -- formalizes carrying constraints with type schemes
- Haskell's approach: type class constraints are part of the type scheme itself (`forall a. Eq a => a -> a -> Bool`)
- Rust's approach: where-clause bounds are part of the function signature and propagate through trait objects / fn pointers
- Snow's current approach: constraints stored in a side-channel map keyed by function name -- simple but doesn't survive renaming

## Open Questions

1. **Higher-order function constraint propagation**
   - What we know: `let f = show; f(x)` can be fixed with alias propagation in fn_constraints
   - What's unclear: `apply(show, value)` where `apply = fn(f, x) do f(x) end` -- the constraint on `show` needs to be checked when `f(x)` is called inside `apply`, but `f` is a parameter name with no fn_constraints entry
   - Recommendation: Fix the direct alias case first. The higher-order case is rare in practice and requires deeper type system changes (qualified types or constraint-carrying function types). Document as a known limitation.

2. **Method reference constraint propagation**
   - What we know: `let f = show` (bare function name) is the common case
   - What's unclear: `let f = SomeModule.show` or `let f = Trait.method` -- do these paths also need constraint propagation?
   - Recommendation: Check if module-qualified or trait-qualified references go through the same NameRef path. If so, the fix covers them automatically.

3. **Constraint propagation through data structures**
   - What we know: Functions can be stored in lists, maps, structs
   - What's unclear: `let funcs = [show, display]; funcs[0](x)` -- can constraints be preserved through collection access?
   - Recommendation: Out of scope for this phase. Collection-stored functions lose constraint info in any practical type system without dependent types.

## Sources

### Primary (HIGH confidence)
- `crates/snow-typeck/src/infer.rs` -- Direct source code analysis of FnConstraints, infer_let_binding, infer_call
- `crates/snow-typeck/src/ty.rs` -- Ty enum and Scheme struct (lines 204-225)
- `crates/snow-typeck/src/traits.rs` -- TraitRegistry.check_where_constraints (lines 268-287)
- `crates/snow-typeck/src/unify.rs` -- InferCtx generalize/instantiate (lines 306-428)
- `crates/snow-typeck/src/env.rs` -- TypeEnv scope management
- `crates/snow-codegen/src/mir/lower.rs:7544` -- Existing e2e_where_clause_enforcement test

### Secondary (MEDIUM confidence)
- Mark P. Jones, ["Qualified Types: Theory and Practice"](http://web.cecs.pdx.edu/~mpj/pubs/thesis.html) (1994) -- formal foundation for constraint propagation through type schemes
- [Wikipedia: Hindley-Milner type system](https://en.wikipedia.org/wiki/Hindley%E2%80%93Milner_type_system) -- background on let-polymorphism and generalization
- [Wikipedia: Type class](https://en.wikipedia.org/wiki/Type_class) -- type class constraints in Haskell
- [Wikipedia: Bounded quantification](https://en.wikipedia.org/wiki/Bounded_quantification) -- theoretical framework for constrained polymorphism

### Tertiary (LOW confidence)
- [Kwang's Haskell Blog: HM inference with constraints](https://kseo.github.io/posts/2017-01-02-hindley-milner-inference-with-constraints.html) -- general overview

## Metadata

**Confidence breakdown:**
- Bug diagnosis: HIGH -- direct source code analysis confirms the name-based lookup is the root cause
- Fix approach (alias propagation): HIGH -- straightforward code change, same mechanism already works for direct calls
- Fix approach (higher-order): MEDIUM -- theoretically understood but implementation scope unclear
- Pitfalls: HIGH -- based on direct analysis of data flow in infer.rs

**Research date:** 2026-02-08
**Valid until:** Indefinite (stable internal codebase, no external dependencies)
