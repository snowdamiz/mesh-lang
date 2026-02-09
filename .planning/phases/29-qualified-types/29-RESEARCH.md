# Phase 29: Qualified Types - Research

**Researched:** 2026-02-08
**Domain:** Constraint propagation through higher-order function arguments in HM type inference
**Confidence:** HIGH

## Summary

Phase 29 fixes the last remaining known limitation in Snow's type system: trait constraints are lost when constrained functions are passed as arguments to higher-order functions. Phase 25 fixed the direct alias case (`let f = show; f(42)`) by propagating `fn_constraints` entries through let bindings. Phase 29 must fix the higher-order passing case: `apply(show, 42)` where `apply = fn(f, x) do f(x) end` and `show` requires `Display`.

The root cause is the same as Phase 25's bug, but manifests through a different code path. In `infer_call`, constraint checking only happens when the callee is a `NameRef` found in `fn_constraints`. When `show` is passed as an argument to `apply`, the parameter `f` inside `apply`'s body has no `fn_constraints` entry, so when `f(x)` is called inside `apply`, no constraint is checked. The constraint on `show` silently disappears.

The recommended approach is a **call-site argument constraint check**: in `infer_call`, after unifying all arguments with parameter types, iterate over arguments to detect any that are NameRefs pointing to constrained functions. For each such argument, resolve the constrained function's type parameter variables (which are now unified with the actual argument types from the call) and check the trait constraints. This works because HM unification makes type variables flow through the higher-order function's body, so by the time we check at the outer call site, the type variables from the constrained function are bound to the concrete argument types. For nested propagation (QUAL-02), the same mechanism applies recursively at each call site level.

**Primary recommendation:** Add an argument-constraint-check pass in `infer_call` that detects constrained function arguments and verifies their constraints against the resolved types of other arguments at the call site, after unification.

## Standard Stack

This phase uses zero new dependencies. All work is within existing crates.

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| snow-typeck | internal | Type inference, constraint checking, FnConstraints | Contains all code being modified |
| ena | existing dep | Union-find for HM unification | Already used by InferCtx |
| rustc-hash | existing dep | FxHashMap used throughout | fn_constraints map, TypeEnv |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| snow-codegen | internal | MIR lowering, e2e test location | Test file for constraint propagation tests |
| snow-parser | internal | AST node types (CallExpr, NameRef, Expr) | Read-only, for AST traversal in infer_call |
| ariadne | existing dep | Error diagnostic rendering | TraitNotSatisfied error messages |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Call-site argument check | Propagate fn_constraints to parameter names | Would require modifying infer_fn_def to accept parameter-level constraints from call sites; creates coupling between call-site and callee-body inference |
| Call-site argument check | Extend Ty::Fun with optional constraint set | Theoretically cleaner (qualified types) but requires changing every match on Ty::Fun across infer.rs, unify.rs, traits.rs -- hundreds of match arms |
| Call-site argument check | Extend Scheme with constraints | Requires changing TypeEnv storage, generalization, instantiation -- more invasive than needed for this phase |

## Architecture Patterns

### Current Architecture (The Bug)

```
Scenario: apply(show, 42) where show requires Display

1. fn show<T>(x :: T) -> String where T: Display
   - fn_constraints["show"] = FnConstraints {
       where_constraints: [("T", "Display")],
       type_params: {"T" -> ?show_t},
       param_type_param_names: [Some("T")],
     }

2. fn apply(f, x) do f(x) end
   - No where_constraints on apply
   - Parameters: f :: ?apply_f, x :: ?apply_x
   - Body: f(x) -> callee is NameRef("f")
   - fn_constraints.get("f") -> None  <-- BUG: no constraints on "f"

3. apply(show, 42) call site:
   - show type instantiated to Fun([?fresh_t], String)
   - Unification: ?apply_f ~ Fun([?fresh_t], String)
   - Unification: ?apply_x ~ Int
   - Inside apply's body: f(x) -> ?fresh_t ~ Int (via unification)
   - BUT: fn_constraints.get("f") -> None -> no constraint check
   - Result: no TraitNotSatisfied error (unsound!)
```

### Recommended Fix: Call-Site Argument Constraint Check

```
Approach: Check constraints of constrained-function ARGUMENTS at the OUTER call site

When processing apply(show, 42) in infer_call:

1. Infer callee type: apply :: Fun([?f, ?x], ?ret)
2. Infer argument types: [show :: Fun([?fresh_t], String), 42 :: Int]
3. Unify callee with expected: ?f ~ Fun([?fresh_t], String), ?x ~ Int, ?ret ~ ?body
4. ** NEW STEP: Argument constraint check **
   For each argument:
     - Is it a NameRef?
     - Does fn_constraints have entry for that name?
     - If yes: the constrained function's type params have been instantiated
       with fresh vars (?fresh_t). After unification in step 3, some of
       these fresh vars may now be bound.
     - Resolve: look at the OTHER arguments to find what ?fresh_t resolved to.

   Key insight: After unification, ?fresh_t is part of the same
   equivalence class as ?apply_x (because inside apply's body, f(x)
   will unify ?fresh_t with ?apply_x's type). But at THIS point in
   inference, apply's BODY hasn't been type-checked yet for this
   specific call. So ?fresh_t is NOT yet unified with Int.

   ** REVISED APPROACH: The argument types ARE the resolution **
   When show is called with argument x inside apply, the type of x
   is ?apply_x. At the call site apply(show, 42), ?apply_x is unified
   with Int. But inside apply's body, f(x) just unifies ?fresh_t with
   ?apply_x -- both remain type variables.

   The constraint check must happen at a point where we know the
   CONCRETE type. Two viable approaches:

   A) Check at call site using argument position correlation
   B) Propagate constraints through parameters
```

### Pattern 1: Call-Site Argument Constraint Check (Recommended)

**What:** When calling `apply(show, 42)`, detect that `show` (an argument) has constraints, then check those constraints against the concrete types of OTHER arguments that correspond to `show`'s constrained parameters.

**When to use:** Any call where an argument is a NameRef to a constrained function.

**How it works:**

The key insight is that for `apply(show, 42)` to work, the call-site arguments must provide enough information to resolve the constrained function's type parameters. When `show :: Fun([T], String)` is passed alongside `42 :: Int`, the higher-order function's signature creates a type relationship between them. After unification at the call site:

- `show`'s instantiated type `Fun([?fresh_t], String)` is unified with `apply`'s first parameter type
- `42 :: Int` is unified with `apply`'s second parameter type
- `apply`'s signature links these: if `apply` is typed as `Fun([Fun([?a], ?b), ?a], ?b)`, then `?fresh_t ~ ?a ~ Int`

After the `ctx.unify(callee_ty, expected_fn_ty, ...)` call in `infer_call`, the type variables are connected. We can then resolve the constrained function's type params:

```rust
// In infer_call, after unification (after line 2707):
// Check constraints on function-typed arguments
if let Some(arg_list) = call.arg_list() {
    for (i, arg) in arg_list.args().enumerate() {
        if let Expr::NameRef(ref name_ref) = arg {
            if let Some(fn_name) = name_ref.text() {
                if let Some(constraints) = fn_constraints.get(&fn_name) {
                    if !constraints.where_constraints.is_empty() {
                        // Resolve type params from the constrained function's
                        // definition-time type_params map. After unification,
                        // these vars may now be bound to concrete types.
                        let mut resolved_type_args: FxHashMap<String, Ty> =
                            FxHashMap::default();

                        for (param_name, param_ty) in &constraints.type_params {
                            let resolved = ctx.resolve(param_ty.clone());
                            resolved_type_args.insert(param_name.clone(), resolved);
                        }

                        let errors = trait_registry.check_where_constraints(
                            &constraints.where_constraints,
                            &resolved_type_args,
                            origin.clone(),
                        );
                        ctx.errors.extend(errors.clone());
                        if let Some(first_err) = errors.into_iter().next() {
                            return Err(first_err);
                        }
                    }
                }
            }
        }
    }
}
```

**Critical detail about type_params:** The `constraints.type_params` maps type param names to the type variables created at function DEFINITION time (e.g., `"T" -> Var(5)`). When `show`'s scheme is instantiated at the call site, `ctx.instantiate()` creates fresh variables. But the FnConstraints stored in `fn_constraints["show"]` still references the ORIGINAL definition-time variables (e.g., `Var(5)`).

This is the same issue Phase 25's research identified in Pitfall 2. The solution there was to use `param_type_param_names` to map argument positions to type param names, then resolve from call-site argument types. For the higher-order case, we need the same approach but adapted:

- The constrained function's `param_type_param_names` tells us which argument positions correspond to which type params
- The constrained function's argument types are unified (via the higher-order function's type) with other call-site arguments
- After unification, we resolve these type variables to get concrete types

**HOWEVER:** There is a subtlety. When `show` is instantiated for the call `apply(show, 42)`, fresh variables are created by `ctx.instantiate()`. These fresh variables are NOT the same as the ones in `fn_constraints["show"].type_params`. So resolving `type_params` directly won't work -- those variables are from definition time, not instantiation time.

**REVISED APPROACH: Use the instantiated argument types.**

When `show` is inferred as an argument expression, `infer_expr` instantiates its scheme, creating `Fun([?fresh_t], String)`. The `?fresh_t` is a brand new variable. After the callee unification (`callee_ty ~ expected_fn_ty`), type variables flow through. But `?fresh_t` is part of `arg_types[0]` (the type of the `show` argument), specifically inside its `Fun` wrapper.

To check constraints, we need to:
1. Extract the parameter types from the constrained function's resolved type
2. Map them to the type param names using `param_type_param_names`
3. Resolve the extracted types to get concrete types
4. Check constraints

```rust
// After unification in infer_call:
for (i, arg) in arg_list.args().enumerate() {
    if let Expr::NameRef(ref name_ref) = arg {
        if let Some(fn_name) = name_ref.text() {
            if let Some(constraints) = fn_constraints.get(&fn_name) {
                if !constraints.where_constraints.is_empty() {
                    // Resolve the argument's type (a function type)
                    let resolved_arg_ty = ctx.resolve(arg_types[i].clone());

                    // Extract parameter types from the resolved function type
                    if let Ty::Fun(param_tys, _) = &resolved_arg_ty {
                        let mut resolved_type_args: FxHashMap<String, Ty> =
                            FxHashMap::default();

                        for (j, tp_name_opt) in
                            constraints.param_type_param_names.iter().enumerate()
                        {
                            if let Some(tp_name) = tp_name_opt {
                                if j < param_tys.len() {
                                    let resolved = ctx.resolve(param_tys[j].clone());
                                    resolved_type_args
                                        .insert(tp_name.clone(), resolved);
                                }
                            }
                        }

                        let errors = trait_registry.check_where_constraints(
                            &constraints.where_constraints,
                            &resolved_type_args,
                            origin.clone(),
                        );
                        ctx.errors.extend(errors.clone());
                    }
                }
            }
        }
    }
}
```

**Problem:** After `ctx.unify(callee_ty, expected_fn_ty, ...)`, the parameter types of the constrained function ARE unified with other argument types -- but only if the higher-order function's type SIGNATURE links them. The higher-order function `apply` has type `Fun([Fun([?a], ?b), ?a], ?b)`. When we unify:
- `show :: Fun([?fresh_t], String)` with parameter `Fun([?a], ?b)` => `?fresh_t ~ ?a`, `?b ~ String`
- `42 :: Int` with parameter `?a` => `?a ~ Int`
- Therefore: `?fresh_t ~ ?a ~ Int`

After this, resolving `?fresh_t` gives `Int`, so the constraint check can find `T -> Int` and check `Display` for `Int`.

But there's a timing issue: when does the body of `apply` get type-checked relative to the call site? In Snow's architecture:
- `apply` is defined as a top-level function and type-checked BEFORE the call site in `main`
- When `apply(show, 42)` is processed in `infer_call`, `apply`'s type is already known as a scheme
- The scheme is instantiated, creating fresh variables
- Arguments are unified with the instantiated parameter types
- This is when the type variables get connected

So the sequence is:
1. `apply` is type-checked: its type becomes `forall a b. Fun([Fun([a], b), a], b)`
2. `apply(show, 42)` is processed: `apply`'s scheme is instantiated to `Fun([Fun([?a], ?b), ?a], ?b)` with fresh `?a, ?b`
3. `show`'s scheme is instantiated to `Fun([?fresh_t], String)` with fresh `?fresh_t`
4. Unify `apply`'s instantiated type with `Fun([show_ty, Int], ?ret)`:
   - `Fun([?a], ?b) ~ Fun([?fresh_t], String)` => `?a ~ ?fresh_t`, `?b ~ String`
   - `?a ~ Int` => `?fresh_t ~ Int` (via `?a`)
5. NOW: `?fresh_t` resolves to `Int`
6. Check `show`'s constraints: `T` mapped to `?fresh_t` -> resolves to `Int`
7. Check `Display` for `Int`

This works. The key is that step 4 happens BEFORE we need to check constraints in step 6.

**BUT:** Step 4 is `ctx.unify(callee_ty, expected_fn_ty, origin)` on line 2707 of infer.rs. After this line, all the type variables are connected. Then we can check constraints on arguments.

**The timing is correct.** The constraint check on arguments can be placed right after the existing callee-name constraint check (around line 2750), before returning `ret_var`.

### Pattern 2: Nested Higher-Order (QUAL-02)

**What:** `wrap(apply, show, value)` where `wrap = fn(f, g, x) do f(g, x) end`.

**How it works:** The same mechanism applies recursively. When `wrap(apply, show, value)` is called:
- `apply` has no constraints on its own (it's a plain higher-order function)
- `show` has constraints
- `value` is concrete

At the call site of `wrap(apply, show, value)`:
- `apply`'s type is instantiated and unified with `wrap`'s first param
- `show`'s type is instantiated and unified with `wrap`'s second param
- `value`'s type is unified with `wrap`'s third param
- The argument constraint check finds `show` has constraints
- After unification, `show`'s type params are connected through `wrap`'s type to `value`'s type
- Constraints are checked

This requires that `wrap`'s type signature correctly links the type variables. If `wrap :: Fun([Fun([Fun([?a], ?b), ?a], ?b), Fun([?a], ?b), ?a], ?b)`, then `show`'s `?fresh_t` connects through to `value`'s type.

**Key requirement:** The higher-order functions must have types that correctly propagate the type parameter connections. This happens naturally through HM inference -- `wrap`'s inferred type correctly links all the parameters.

### Pattern 3: Handling the Pipe Operator Case

**What:** `42 |> show` where `show` requires Display and the pipe desugars to a function call.

**How it works:** The `infer_pipe` function (line 2756) already has a similar constraint-checking block that mirrors `infer_call`. It needs the same argument-level constraint check added. However, for pipes, the constrained function is the CALLEE (RHS), not an argument -- so the existing callee-name check already handles this case. The pipe case only matters if the RHS is a higher-order function receiving a constrained function, which would be unusual in pipe chains.

### Anti-Patterns to Avoid

- **Do NOT modify the Ty enum:** Adding constraints to `Ty::Fun` would require touching every match on `Ty` across the codebase. The `Ty` enum is used in hundreds of match arms in infer.rs, unify.rs, traits.rs, and codegen.
- **Do NOT try to propagate constraints into function bodies:** The body of `apply` is type-checked once at definition time, not re-checked for each call. Trying to inject constraints into the body's scope at call time would break the fundamental architecture.
- **Do NOT change Scheme to carry constraints:** This would affect generalization/instantiation for ALL bindings, not just constrained functions. Over-broad change.
- **Do NOT rely on definition-time type_params for resolution:** After instantiation, the definition-time type variables are stale. Use the instantiated argument types from the call site.
- **Do NOT check constraints only when types are fully resolved:** Some type variables may remain unresolved (e.g., when the result type is still polymorphic). Check what IS resolved and skip what is not. An unresolved type variable means the constraint cannot be verified yet and may need deferred checking or can be safely skipped if the variable is never bound to a concrete type.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Constraint storage | New constraint type | Existing FnConstraints struct | Already has where_constraints, type_params, param_type_param_names |
| Type param resolution | Custom resolution logic | ctx.resolve() on argument types | Union-find already tracks all unifications |
| Constraint checking | Custom trait verification | trait_registry.check_where_constraints() | Already does structural matching |
| Function type extraction | Custom type walker | Pattern match on Ty::Fun | Simple destructuring gives param types |
| Error reporting | Custom error type | Existing TypeError::TraitNotSatisfied | Already has ty, trait_name, origin fields |

**Key insight:** All constraint-checking machinery already works correctly. The existing `check_where_constraints` function, `TraitNotSatisfied` error, and `FnConstraints` structure are sufficient. The only missing piece is WHEN and WHERE to invoke the check -- specifically, on function-typed arguments at call sites.

## Common Pitfalls

### Pitfall 1: Stale Definition-Time Type Variables

**What goes wrong:** Using `constraints.type_params` (which maps type param names to definition-time Ty::Var values) after instantiation. These variables are from the original function definition's scope and are not connected to the call-site's fresh variables.

**Why it happens:** FnConstraints is stored once at function definition time. When the function is instantiated at a call site, `ctx.instantiate()` creates fresh variables that are substituted into the type, but FnConstraints is NOT updated.

**How to avoid:** Extract type parameters from the instantiated/resolved argument type at the call site. After unification, `ctx.resolve(arg_types[i].clone())` gives the resolved function type with its parameter types connected to the call-site's type variables. Destructure `Ty::Fun(param_tys, ret)` and use `param_type_param_names` to map positions to names.

**Warning signs:** Constraints appearing to be checked but always passing because type params resolve to unbound variables instead of concrete types.

### Pitfall 2: Unresolved Type Variables at Check Time

**What goes wrong:** When checking constraints, some type parameters may still be unresolved type variables (e.g., `?42` instead of `Int`). Calling `trait_registry.has_impl("Display", &Ty::Var(?42))` will fail because there's no impl registered for type variables.

**Why it happens:** The higher-order function's type signature may not fully constrain all type parameters at the call site. For example, if `apply(show, x)` where `x` has an unresolved type.

**How to avoid:** Only check constraints when the resolved type is a concrete type (not a `Ty::Var`). Skip constraint checks for unresolved type variables -- they will be caught later when the variable is bound, or remain polymorphic (which is valid).

**Warning signs:** False TraitNotSatisfied errors for polymorphic code that is actually correct.

### Pitfall 3: Double Error Reporting

**What goes wrong:** A constraint violation produces errors BOTH from the argument-level check AND from the existing callee-level check if the constrained function is also called directly somewhere.

**Why it happens:** The argument-level check fires at the outer call site, while the callee-level check fires inside the function body (if the function calls the parameter).

**How to avoid:** This is actually not a problem in practice because the callee-level check inside `apply`'s body will NOT fire (since `f` has no fn_constraints entry -- that's the whole bug). The argument-level check at the outer call site is the ONLY check that fires. No double reporting.

**Warning signs:** Multiple identical TraitNotSatisfied errors in the error list.

### Pitfall 4: Non-NameRef Constrained Arguments

**What goes wrong:** If a constrained function is passed through a let binding first (`let f = show; apply(f, 42)`), the argument is still a NameRef, and Phase 25's alias propagation ensures `fn_constraints["f"]` exists. So the argument-level check finds it. But if the argument is a more complex expression (e.g., `apply(get_show(), 42)`), the argument is not a NameRef and constraints cannot be found.

**Why it happens:** The constraint lookup is name-based, not type-based.

**How to avoid:** For Phase 29, handle only NameRef arguments (which covers `show`, `f`, `g`, etc.). Complex expressions that return constrained functions are extremely rare in practice and would require constraint-carrying types to handle -- document as a known limitation.

**Warning signs:** Tests passing for `apply(show, 42)` but failing for `apply(get_constrained_fn(), 42)`.

### Pitfall 5: Pipe Operator Mirror

**What goes wrong:** The `infer_pipe` function has its own constraint checking logic that mirrors `infer_call`. If the argument-level check is added to `infer_call` but not `infer_pipe`, pipes involving higher-order constrained functions won't be checked.

**Why it happens:** Code duplication between `infer_call` and `infer_pipe`.

**How to avoid:** Add the same argument-level constraint check to `infer_pipe` after its argument unification step. The pipe case is: `value |> apply(show)` which desugars differently but has the same structure.

**Warning signs:** Tests passing for `apply(show, 42)` but failing for `42 |> apply(show)`.

## Code Examples

### Example 1: QUAL-01 -- Basic Higher-Order Constrained Function

```snow
# Source: Snow program demonstrating the desired behavior
interface Displayable do
  fn display(self) -> String
end

impl Displayable for Int do
  fn display(self) -> String do
    "${self}"
  end
end

fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn apply(f, x) do
  f(x)
end

fn main() do
  # This should work: Int implements Displayable
  let result = apply(show, 42)
  println(result)
end
```

### Example 2: QUAL-02 -- Nested Higher-Order Passing

```snow
# Source: Snow program demonstrating nested constraint propagation
fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn apply(f, x) do
  f(x)
end

fn wrap(f, g, x) do
  f(g, x)
end

fn main() do
  # Constraints propagate through wrap -> apply -> show
  let result = wrap(apply, show, 42)
  println(result)
end
```

### Example 3: QUAL-03 -- Constraint Violation Error

```snow
# Source: Snow program that should produce a compile error
interface Greetable do
  fn greet(self) -> String
end

fn say_hello<T>(x :: T) -> String where T: Greetable do
  greet(x)
end

fn apply(f, x) do
  f(x)
end

fn main() do
  # ERROR: Int does not implement Greetable
  apply(say_hello, 42)
end
```

Expected error: `type 'Int' does not satisfy trait 'Greetable'`

### Example 4: Proposed Implementation in infer_call

```rust
// Source: Proposed addition to crates/snow-typeck/src/infer.rs
// In infer_call, after the existing callee-name constraint check (after line 2750):

// Check constraints on constrained-function ARGUMENTS passed to higher-order functions.
// When apply(show, 42) is called, show's constraints must be checked against 42's type.
if let Some(arg_list) = call.arg_list() {
    for (arg_idx, arg) in arg_list.args().enumerate() {
        // Only handle NameRef arguments (covers direct names and let aliases).
        if let Expr::NameRef(ref name_ref) = arg {
            if let Some(arg_fn_name) = name_ref.text() {
                if let Some(arg_constraints) = fn_constraints.get(&arg_fn_name) {
                    if !arg_constraints.where_constraints.is_empty() {
                        // Resolve the argument's type -- after unification with
                        // the callee's parameter type, this is a function type
                        // whose parameter types are connected to other arguments.
                        if arg_idx < arg_types.len() {
                            let resolved_arg_ty = ctx.resolve(arg_types[arg_idx].clone());

                            if let Ty::Fun(ref param_tys, _) = resolved_arg_ty {
                                let mut resolved_type_args: FxHashMap<String, Ty> =
                                    FxHashMap::default();

                                // Map from parameter position to type param name,
                                // then resolve the parameter type to get the concrete type.
                                for (j, tp_name_opt) in
                                    arg_constraints.param_type_param_names.iter().enumerate()
                                {
                                    if let Some(tp_name) = tp_name_opt {
                                        if j < param_tys.len() {
                                            let resolved = ctx.resolve(param_tys[j].clone());
                                            resolved_type_args
                                                .insert(tp_name.clone(), resolved);
                                        }
                                    }
                                }

                                // Fallback: try definition-time vars (they may be
                                // connected via unification to call-site types).
                                for (param_name, param_ty) in &arg_constraints.type_params {
                                    if !resolved_type_args.contains_key(param_name) {
                                        let resolved = ctx.resolve(param_ty.clone());
                                        resolved_type_args
                                            .insert(param_name.clone(), resolved);
                                    }
                                }

                                // Only check constraints where the type param resolved
                                // to a concrete type (not still a Ty::Var).
                                let checkable_constraints: Vec<(String, String)> =
                                    arg_constraints.where_constraints
                                        .iter()
                                        .filter(|(param_name, _)| {
                                            resolved_type_args.get(param_name)
                                                .map(|ty| !matches!(ty, Ty::Var(_)))
                                                .unwrap_or(false)
                                        })
                                        .cloned()
                                        .collect();

                                if !checkable_constraints.is_empty() {
                                    let errors = trait_registry.check_where_constraints(
                                        &checkable_constraints,
                                        &resolved_type_args,
                                        origin.clone(),
                                    );
                                    ctx.errors.extend(errors);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

### Example 5: Test Pattern (following existing e2e tests)

```rust
// Source: Test pattern for crates/snow-codegen/src/mir/lower.rs
#[test]
fn e2e_qualified_type_higher_order_apply() {
    let source = r#"
interface Displayable do
  fn display(self) -> String
end

impl Displayable for Int do
  fn display(self) -> String do
    "${self}"
  end
end

fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn apply(f, x) do
  f(x)
end

fn main() do
  apply(show, 42)
end
"#;
    let parse = snow_parser::parse(source);
    let typeck = snow_typeck::check(&parse);

    // Should NOT error: Int implements Displayable
    let has_trait_error = typeck.errors.iter().any(|e| {
        matches!(e, snow_typeck::error::TypeError::TraitNotSatisfied { .. })
    });
    assert!(
        !has_trait_error,
        "Should NOT get TraitNotSatisfied when passing show to apply with conforming type. Errors: {:?}",
        typeck.errors
    );
}

#[test]
fn e2e_qualified_type_higher_order_violation() {
    let source = r#"
interface Greetable do
  fn greet(self) -> String
end

fn say_hello<T>(x :: T) -> String where T: Greetable do
  greet(x)
end

fn apply(f, x) do
  f(x)
end

fn main() do
  apply(say_hello, 42)
end
"#;
    let parse = snow_parser::parse(source);
    let typeck = snow_typeck::check(&parse);

    let has_trait_error = typeck.errors.iter().any(|e| {
        matches!(e, snow_typeck::error::TypeError::TraitNotSatisfied { .. })
    });
    assert!(
        has_trait_error,
        "Expected TraitNotSatisfied when passing constrained function to apply with non-conforming type. Errors: {:?}",
        typeck.errors
    );
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No constraint checking | fn_constraints checked at direct call sites | Phase 18/19 (v1.3) | Direct calls enforce where-clauses |
| Name-based lookup only | Alias propagation through let bindings | Phase 25 (v1.4) | `let f = show; f(42)` correctly checked |
| No higher-order check | Need: argument-level constraint check | Phase 29 (this phase) | `apply(show, 42)` correctly checked |

**Theoretical background:**
- Mark P. Jones, "Qualified Types: Theory and Practice" (1994) -- formalizes the idea that constraints should propagate with type schemes
- Haskell's approach: type class constraints are part of the type scheme and propagate through instantiation; the constraint is always checked at the point of use
- Snow's pragmatic approach: side-channel `fn_constraints` map, checked at call sites where constrained functions are identified by name
- The call-site argument check extends the pragmatic approach to cover the higher-order case without restructuring the type representation

## Open Questions

1. **Complex argument expressions**
   - What we know: NameRef arguments (`show`, `f`, `g`) are handled by the argument constraint check
   - What's unclear: Complex expressions like `if cond do show else other end` as arguments -- constraints cannot be detected
   - Recommendation: Out of scope. NameRef covers all practical cases. Complex constraint-carrying expressions would require constraint-carrying types.

2. **Definition-time vs instantiation-time type variables**
   - What we know: After unification at the call site, the instantiated type variables (from the constrained function's argument type) are connected to the concrete argument types
   - What's unclear: Whether `ctx.resolve()` on the function type's parameter types will always correctly resolve through the unification chain
   - Recommendation: Add defensive checks -- if resolution still yields `Ty::Var`, skip the constraint check for that parameter. Test with multiple scenarios.

3. **MIR/Codegen impact**
   - What we know: This is purely a type-checker change. MIR lowering and codegen are unaffected.
   - What's unclear: Whether any MIR-level constraint checking (the "defense-in-depth" warning) needs updating
   - Recommendation: Check if MIR has any constraint checking logic. If so, it should not need changes since the new constraint check is in typeck.

4. **Performance of argument iteration**
   - What we know: The argument iteration adds one pass over call arguments
   - What's unclear: Whether this measurably impacts compile times for large programs
   - Recommendation: The iteration is O(n) where n is the number of arguments, and involves only HashMap lookups and type resolution. Should be negligible. No optimization needed.

## Sources

### Primary (HIGH confidence)
- `crates/snow-typeck/src/infer.rs` -- Direct source analysis of FnConstraints, infer_call, infer_let_binding, infer_fn_def, infer_block
- `crates/snow-typeck/src/traits.rs` -- TraitRegistry.check_where_constraints (lines 268-287)
- `crates/snow-typeck/src/unify.rs` -- InferCtx.resolve, unify, instantiate
- `crates/snow-typeck/src/ty.rs` -- Ty enum, Scheme struct
- `crates/snow-codegen/src/mir/lower.rs` -- Existing e2e_where_clause tests (lines 8058-8228)
- `.planning/phases/25-type-system-soundness/25-RESEARCH.md` -- Prior research on constraint propagation
- `.planning/phases/25-type-system-soundness/25-01-SUMMARY.md` -- Phase 25 implementation details

### Secondary (MEDIUM confidence)
- [Northwestern Qualified Types lecture notes](https://users.cs.northwestern.edu/~jesse/course/type-systems-wi18/type-notes/Qualified_types.html) -- Qualified types theory: predicate contexts propagate through application and abstraction rules
- [Mark P. Jones, "Functional Programming with Overloading and Higher-Order Polymorphism"](https://web.cecs.pdx.edu/~mpj/pubs/springschool95.pdf) -- Formal foundation for constraint propagation through type schemes
- [Wikipedia: Hindley-Milner type system](https://en.wikipedia.org/wiki/Hindley%E2%80%93Milner_type_system) -- Background on let-polymorphism and instantiation

### Tertiary (LOW confidence)
- [TypeScript PR #30215: Higher order function type inference](https://github.com/Microsoft/TypeScript/pull/30215) -- TypeScript's approach to higher-order generic function inference (different language, different type system)
- [Omnidirectional type inference for ML](https://inria.hal.science/hal-05438544v1/document) -- Recent research on dynamic constraint solving order

## Metadata

**Confidence breakdown:**
- Bug diagnosis: HIGH -- direct source code analysis confirms the name-based lookup failure path for higher-order arguments
- Fix approach (call-site argument check): HIGH -- unification semantics guarantee type variables are connected after `ctx.unify(callee_ty, expected_fn_ty)`, making resolution possible
- Nested propagation (QUAL-02): MEDIUM -- depends on the higher-order function's inferred type correctly linking all type variables, which HM inference should guarantee, but needs testing
- Pitfalls: HIGH -- based on direct analysis of FnConstraints, instantiation, and resolution data flow

**Research date:** 2026-02-08
**Valid until:** Indefinite (stable internal codebase, no external dependencies)
