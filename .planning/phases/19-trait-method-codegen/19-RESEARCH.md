# Phase 19: Trait Method Codegen - Research

**Researched:** 2026-02-08
**Domain:** MIR lowering of trait impl method bodies, call-site resolution, where-clause codegen enforcement
**Confidence:** HIGH

## Summary

Phase 19 takes the trait infrastructure built in Phase 18 (structural matching, duplicate detection, unified dispatch, TraitRegistry in Lowerer) and makes it produce executable code. The three core tasks are: (1) lowering `impl` block method bodies to `MirFunction`s with mangled names (`Trait__Method__Type`), (2) resolving call sites that reference trait methods to those mangled names via TraitRegistry lookup, and (3) threading where-clause constraints through MIR lowering so unmet bounds are rejected at codegen time, not just at type-check time.

The current state: `Item::ImplDef` in `lower.rs:427-432` iterates methods and calls `self.lower_fn_def(&method)` on each, which produces a `MirFunction` named by the method's own name (e.g., `greet`) -- no trait name, no type name, no mangling. Similarly, ImplDef methods are NOT pre-registered in `known_functions` during the first pass (line 187 has `_ => {}`). Call sites resolve the method name as a regular function via the typeck environment (line 1800-1808 of `infer.rs` registers methods as plain functions). There is no monomorphization depth limit anywhere in the codebase.

**Primary recommendation:** Modify `lower_item` for `ImplDef` to extract trait name and type name from PATH nodes, generate `Trait__Method__Type` mangled names, handle `self` as the first concrete-typed parameter, and register these in `known_functions`. Then modify `lower_call_expr` to detect trait method calls via TraitRegistry and rewrite them to use mangled names. Where-clause enforcement at MIR lowering is defense-in-depth (typeck already rejects, but codegen should also verify). Add a configurable recursion depth counter.

## Standard Stack

This phase uses zero new dependencies (locked decision). All work is within existing crates.

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| snow-codegen | internal | MIR lowering (lower.rs), codegen (expr.rs) | Primary modification target |
| snow-typeck | internal | TraitRegistry, TypeckResult, Ty types | Provides trait resolution data |
| snow-parser | internal | ImplDef, FnDef, Path AST nodes | Provides AST for lowering |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| rustc-hash | existing dep | FxHashMap for efficient lookups | Already used throughout |
| rowan | existing dep | TextRange for type lookups | Already used in lowerer |

### Alternatives Considered
None. Zero new Rust crate dependencies is a locked decision.

## Architecture Patterns

### Current Architecture (What Exists After Phase 18)

```
snow-typeck/src/
  traits.rs          # TraitRegistry with structural matching, ImplDef, TraitDef
  infer.rs           # infer_impl_def() registers methods as plain functions in env
  lib.rs             # TypeckResult with trait_registry field

snow-codegen/src/
  mir/lower.rs       # Lowerer with trait_registry: &'a TraitRegistry
                     # lower_item() for ImplDef calls lower_fn_def() with bare method name
                     # lower_call_expr() does NOT use TraitRegistry
  mir/types.rs       # resolve_type(), mangle_type_name() (single underscore for generics)
  mir/mono.rs        # Reachability-based monomorphization (no depth limit)
  mir/mod.rs         # MirFunction, MirExpr::Call, MirExpr::BinOp
  codegen/expr.rs    # codegen_call() looks up functions by name, codegen_binop() hardcoded
```

### Pattern 1: ImplDef Method Lowering with Mangled Names

**What:** When encountering `Item::ImplDef`, extract the trait name and impl type name from the AST PATH nodes (same approach as `infer_impl_def` in `infer.rs:1682-1707`), then for each method, produce a `MirFunction` with name `Trait__Method__Type` (double underscore separators).

**When to use:** Every ImplDef item during lowering.

**Key mechanics:**
```rust
// In lower_item(), replacing the current ImplDef arm (lower.rs:427-432):
Item::ImplDef(impl_def) => {
    // Extract trait name and type name from PATH children (same as infer_impl_def)
    let paths: Vec<_> = impl_def.syntax().children()
        .filter(|n| n.kind() == SyntaxKind::PATH)
        .collect();

    let trait_name = paths.first()
        .and_then(|path| path.children_with_tokens()
            .filter_map(|t| t.into_token())
            .find(|t| t.kind() == SyntaxKind::IDENT)
            .map(|t| t.text().to_string()))
        .unwrap_or_else(|| "<unknown>".to_string());

    let type_name = paths.get(1)
        .and_then(|path| path.children_with_tokens()
            .filter_map(|t| t.into_token())
            .find(|t| t.kind() == SyntaxKind::IDENT)
            .map(|t| t.text().to_string()))
        .unwrap_or_else(|| "<unknown>".to_string());

    for method in impl_def.methods() {
        let method_name = method.name().and_then(|n| n.text())
            .unwrap_or_else(|| "<unnamed>".to_string());
        let mangled = format!("{}__{}__{}", trait_name, method_name, type_name);
        self.lower_impl_method(&method, &mangled, &type_name);
    }
}
```

**Critical detail -- `self` parameter handling:**
The `self` parameter in impl methods should become the first parameter with the concrete type of the implementing struct. The type checker stores the impl type as `Ty::Con("MyStruct")` (via `name_to_type` at `infer.rs:1709`). The lowerer must:
1. Detect which parameter is `self` (has `SELF_KW` token, not an IDENT)
2. Replace it with a named parameter (e.g., `self`) typed as `MirType::Struct("MyStruct")`
3. Register `self` in the scope so the method body can reference it

### Pattern 2: Call-Site Resolution via TraitRegistry

**What:** When lowering a `CallExpr` whose callee resolves to a trait method, rewrite the call target to use the mangled name.

**When to use:** In `lower_call_expr` when the callee is a trait method name.

**Key mechanics:**
```rust
// In lower_call_expr, after lowering args but before generating MirExpr::Call:
if let MirExpr::Var(ref name, _) = callee {
    if !self.known_functions.contains_key(name) {
        // Not a regular function -- check if it's a trait method.
        // Use the first argument's type to find the impl.
        if !args.is_empty() {
            let first_arg_ty = args[0].ty();
            let type_name = mir_type_to_impl_name(first_arg_ty);
            let trait_names = self.trait_registry
                .find_method_traits(name, &self.mir_type_to_ty(first_arg_ty));
            if let Some(trait_name) = trait_names.first() {
                let mangled = format!("{}__{}__{}", trait_name, name, type_name);
                // Rewrite callee to use mangled name
                callee = MirExpr::Var(mangled.clone(), /* appropriate fn type */);
            }
        }
    }
}
```

**Critical consideration:** The type checker registers method names as plain functions in the type environment (`infer.rs:1806-1808`). At the MIR level, when we see `greet(my_struct)`, the callee is a `MirExpr::Var("greet", ...)`. We need to detect this is a trait method (not a standalone function) and rewrite it.

**Detection strategy:** A function name that:
1. Is NOT in `known_functions` (not a top-level fn, actor, service, etc.)
2. Has args, and the first arg's type matches an impl in TraitRegistry for a trait that provides this method name
3. TraitRegistry's `find_method_traits(method_name, first_arg_type)` returns a non-empty result

### Pattern 3: Pre-Registration of Mangled Method Names

**What:** During the first pass (pre-registration loop at line 146-188), when encountering an `ImplDef`, compute the mangled names and register them in `known_functions`.

**Why critical:** Without pre-registration, calls to trait methods appear as "unknown functions" and get lowered as closure calls instead of direct calls. The `is_known_fn` check at line 1339-1342 determines whether a call becomes `MirExpr::Call` (direct) or `MirExpr::ClosureCall` (indirect). Trait methods must be direct calls.

**Key mechanics:**
```rust
// In the pre-registration loop (lower.rs ~line 149):
Item::ImplDef(impl_def) => {
    let (trait_name, type_name) = extract_trait_and_type_names(&impl_def);
    for method in impl_def.methods() {
        if let Some(method_name) = method.name().and_then(|n| n.text()) {
            let mangled = format!("{}__{}__{}", trait_name, method_name, type_name);
            let fn_ty = self.resolve_range(method.syntax().text_range());
            self.known_functions.insert(mangled.clone(), fn_ty.clone());
            // Also insert the bare method name mapping for call-site rewriting
        }
    }
}
```

### Pattern 4: Where-Clause Enforcement at MIR Lowering

**What:** At call sites in the MIR lowerer, verify trait bounds are satisfied. This is defense-in-depth -- the type checker already rejects invalid calls, but codegen should not silently produce wrong code if a bound is violated.

**Implementation approach:** The type checker already stores `FnConstraints` per function (including `where_constraints: Vec<(String, String)>`). However, this data is NOT currently in `TypeckResult` -- it is local to the `infer()` function. Two options:

1. **Option A (minimal):** Trust that typeck already validated bounds. The MIR lowerer does not re-check. This is simpler and sufficient if typeck is always run before codegen (which it is in the current pipeline).

2. **Option B (defense-in-depth):** Thread `FnConstraints` through `TypeckResult` and check at MIR call sites. This adds safety but requires modifying `TypeckResult` and `Lowerer`.

**Recommendation:** Option A for the initial implementation. Where-clause enforcement is already comprehensive in the type checker (`infer_call` at `infer.rs:2364-2405`). The MIR lowerer should focus on correct name resolution. If typeck passes without errors, the bounds are satisfied. Add an assertion/panic if a trait method call cannot be resolved (indicating a typeck bug, not a user error).

### Pattern 5: Monomorphization Depth Limit

**What:** Add a configurable recursion depth counter to prevent stack overflow from infinitely recursive trait method instantiation.

**Where:** The depth limit applies during MIR lowering, specifically when lowering function bodies that may trigger further function lowering. In practice, Snow uses static dispatch via monomorphization, so deeply nested generic trait method calls could theoretically cause unbounded recursion.

**Implementation:**
```rust
// In Lowerer struct:
mono_depth: u32,
max_mono_depth: u32,  // Default: 64

// When entering a function body:
fn lower_fn_body_with_depth_check(&mut self, ...) -> MirExpr {
    self.mono_depth += 1;
    if self.mono_depth > self.max_mono_depth {
        // Emit a panic/error instead of continuing
        return MirExpr::Panic {
            message: format!("monomorphization depth limit ({}) exceeded", self.max_mono_depth),
            file: "<compiler>".to_string(),
            line: 0,
        };
    }
    let result = self.lower_block(body);
    self.mono_depth -= 1;
    result
}
```

**Note:** In the current architecture, MIR lowering is a single pass over the AST -- it does not recursively instantiate functions. The depth limit is primarily a safety net for future monomorphization work. For Phase 19, the simple counter is sufficient.

### Anti-Patterns to Avoid

- **Modifying codegen/expr.rs for trait dispatch:** All trait dispatch resolution happens at MIR lowering (lower.rs), NOT at LLVM codegen (expr.rs). By the time codegen sees a `MirExpr::Call`, the mangled name is already resolved. Codegen just calls the function by name.

- **Creating new MirExpr variants for trait calls:** Trait method calls are just `MirExpr::Call` with a mangled function name. No new variants needed. The existing `codegen_call` in `expr.rs` already handles `MirExpr::Call { func: MirExpr::Var(name, _), ... }` by looking up the function in `self.functions`.

- **Duplicating PATH extraction logic:** The pattern for extracting trait name and type name from ImplDef PATH nodes already exists in `infer_impl_def` (`infer.rs:1682-1707`). The lowerer should use the same approach (filter SyntaxKind::PATH children, extract first IDENT from each).

- **Changing TraitRegistry API:** The existing `find_method_traits`, `find_impl`, `has_impl`, `resolve_trait_method` methods are sufficient. No new methods needed on TraitRegistry.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Trait name extraction from AST | New parser method | PATH children + IDENT filter (same as infer_impl_def) | Pattern already proven in typeck |
| Type matching for call resolution | Custom type comparison | `TraitRegistry::find_method_traits()` | Structural unification already handles all cases |
| Self parameter detection | Name-based check | `SyntaxKind::SELF_KW` token check (same as infer_impl_def) | Correct parser token detection |
| Name mangling | Ad-hoc string concatenation | `format!("{}__{}__{}",  trait, method, type)` | Locked decision: double underscore separators |
| Ty to type name conversion | New mapping function | Existing `resolve_type()` + MirType Display | Already handles all type shapes |

**Key insight:** The type checker already does 90% of the work. The MIR lowerer's job is to translate the type checker's trait knowledge into mangled function names. The TraitRegistry is already threaded to the Lowerer (Phase 18-03). The lowerer just needs to USE it.

## Common Pitfalls

### Pitfall 1: ImplDef Methods Not Pre-Registered as Known Functions

**What goes wrong:** Calls to trait methods get lowered as `MirExpr::ClosureCall` instead of `MirExpr::Call`, because the mangled function names are not in `known_functions`. LLVM codegen then tries to call them as closures (extract fn_ptr/env_ptr from a struct), which crashes.

**Why it happens:** The pre-registration pass at `lower.rs:146-188` has `_ => {}` for `Item::ImplDef`. Methods inside impl blocks are invisible to the pre-registration loop.

**How to avoid:** Add an `Item::ImplDef` arm to the pre-registration loop that computes mangled names and inserts them into `known_functions`.

**Warning signs:** Functions named `Trait__Method__Type` appear in MIR but calls to trait methods go through `ClosureCall`. Test failures with "closure call to non-closure" errors.

### Pitfall 2: Bare Method Name Collision

**What goes wrong:** If two traits both define a method named `greet`, and both have impls for different types, the type checker registers `greet` in the environment for the FIRST impl seen. The second impl's `greet` overwrites it (env.insert only if not exists -- `infer.rs:1806`). At MIR lowering, `greet(x)` always resolves to whichever impl was registered first.

**Why it happens:** The type checker registers methods as bare names (`greet`) not mangled names. The MIR lowerer inherits this ambiguity.

**How to avoid:** At the MIR lowering call site, don't rely on the bare method name lookup. Instead, use `TraitRegistry::find_method_traits(method_name, first_arg_type)` to find which trait provides this method for the given argument type, then construct the mangled name.

**Warning signs:** Trait method calls produce the wrong result when multiple traits define methods with the same name. Tests pass with single traits but fail with multiple.

### Pitfall 3: `self` Parameter Not Getting Concrete Type

**What goes wrong:** The `self` parameter in impl method bodies gets lowered as `MirType::Unit` or `MirType::Struct("self")` instead of the concrete implementing type (e.g., `MirType::Struct("MyStruct")`).

**Why it happens:** `lower_fn_def` extracts parameter types from the function's `Ty::Fun` stored in `types` by the type checker. But for impl methods, the type checker stores `self`'s type as the impl type (via `env.insert("self", Scheme::mono(impl_type))` at `infer.rs:1754`), which should propagate to the function type. If the function type is missing or only has non-self params, the self type is lost.

**How to avoid:** In the new `lower_impl_method`, explicitly handle the `self` parameter: detect it via `SELF_KW`, resolve the impl type name to a `MirType`, and use that as the first parameter's type. Don't rely solely on the `Ty::Fun` from typeck.

**Warning signs:** Method bodies that reference `self.field` fail with "undefined variable self" or produce wrong types for self access.

### Pitfall 4: Ty to MirType Conversion for Call-Site Resolution

**What goes wrong:** The TraitRegistry stores types as `Ty` (typeck's representation), but at MIR lowering call sites, we have `MirType` (codegen's representation). To use `find_method_traits`, we need to convert from `MirType` back to `Ty`.

**Why it happens:** The Lowerer has fully resolved `MirType`s for arguments, but TraitRegistry methods take `&Ty`. Going from `MirType::Struct("MyStruct")` back to `Ty::Con(TyCon::new("MyStruct"))` is straightforward for simple types but needs care for primitives and complex types.

**How to avoid:** Write a `mir_type_to_ty(mir_type: &MirType) -> Ty` helper that handles:
- `MirType::Int` -> `Ty::int()`
- `MirType::Float` -> `Ty::float()`
- `MirType::String` -> `Ty::string()`
- `MirType::Bool` -> `Ty::bool()`
- `MirType::Struct(name)` -> `Ty::Con(TyCon::new(name))`
- Other types -> best effort mapping

**Warning signs:** Trait method calls work for primitive types (Int, Float) but fail for user-defined struct types, or vice versa.

### Pitfall 5: Monomorphization Pass Removes Trait Methods

**What goes wrong:** The monomorphization pass (`mono.rs`) removes "unreachable" functions. If trait methods are lowered with mangled names but call sites still reference bare names (before rewriting), the mangled functions appear unreachable and get removed.

**Why it happens:** `collect_function_refs` in `mono.rs` scans `MirExpr::Call { func: Var(name) }` for the function name. If the call site has `Var("greet")` but the function is named `Greetable__greet__MyStruct`, the function appears unused.

**How to avoid:** Ensure call-site rewriting happens BEFORE monomorphization. Since lowering happens before mono, this should be automatic if call sites are correctly rewritten during lowering. But verify by checking that both the mangled function AND the rewritten call site reference the same name.

**Warning signs:** MIR dump shows mangled functions but they disappear after monomorphization. Runtime crashes with "undefined function".

### Pitfall 6: Typeck Registers Method With Self's Type in Function Type

**What goes wrong:** The type checker creates the method's function type as `Ty::Fun([impl_type], ret)` (`infer.rs:1802-1804`), where the first parameter IS the impl type. But `lower_fn_def` uses `param_list.params()` which includes the `self` keyword, and the zip of params+types could misalign if self is counted differently.

**Why it happens:** The parser's `param_list.params()` includes the `self` token as a parameter, and the typeck's `Ty::Fun` includes the impl type as the first param type. These SHOULD align, but if the lowerer treats `self` specially (skipping it), the alignment breaks.

**How to avoid:** In `lower_impl_method`, treat `self` as a regular parameter with a known name and type. Don't skip it -- zip it normally with the Ty::Fun parameter types. The type checker already put impl_type as the first param type.

**Warning signs:** Parameter count mismatches between MirFunction params and the body's variable references.

## Code Examples

### Current ImplDef Lowering (to be replaced)

```rust
// Source: snow-codegen/src/mir/lower.rs:427-432
Item::ImplDef(impl_def) => {
    // Lower impl methods as standalone functions.
    for method in impl_def.methods() {
        self.lower_fn_def(&method);
    }
}
```

### Current Pre-Registration (missing ImplDef)

```rust
// Source: snow-codegen/src/mir/lower.rs:146-188
// The pre-registration loop handles FnDef, ActorDef, SupervisorDef, ServiceDef
// but ImplDef falls through to `_ => {}`
for item in sf.items() {
    match &item {
        Item::FnDef(fn_def) => { /* registers in known_functions */ }
        Item::ActorDef(actor_def) => { /* registers in known_functions */ }
        // ...
        _ => {} // <-- ImplDef falls through here!
    }
}
```

### Type Checker's Method Registration (bare name, no mangling)

```rust
// Source: snow-typeck/src/infer.rs:1800-1808
// Register the method as a callable function so `to_string(42)` works.
let fn_ty = {
    let params = vec![impl_type.clone()];
    let ret = return_type.clone().unwrap_or_else(|| Ty::Tuple(vec![]));
    Ty::Fun(params, Box::new(ret))
};
if env.lookup(&method_name).is_none() {
    env.insert(method_name.clone(), Scheme::mono(fn_ty));
}
```

### TraitRegistry PATH Extraction Pattern (from infer.rs, to reuse)

```rust
// Source: snow-typeck/src/infer.rs:1682-1707
// This exact pattern should be used in the lowerer for extracting
// trait_name and type_name from ImplDef's PATH children.
let paths: Vec<_> = impl_
    .syntax()
    .children()
    .filter(|n| n.kind() == SyntaxKind::PATH)
    .collect();

let trait_name = paths
    .first()
    .and_then(|path| {
        path.children_with_tokens()
            .filter_map(|t| t.into_token())
            .find(|t| t.kind() == SyntaxKind::IDENT)
            .map(|t| t.text().to_string())
    })
    .unwrap_or_else(|| "<unknown>".to_string());

let impl_type_name = paths
    .get(1)
    .and_then(|path| {
        path.children_with_tokens()
            .filter_map(|t| t.into_token())
            .find(|t| t.kind() == SyntaxKind::IDENT)
            .map(|t| t.text().to_string())
    })
    .unwrap_or_else(|| "<unknown>".to_string());
```

### Existing Mangling Convention Reference

```rust
// Source: snow-codegen/src/mir/types.rs:143-150
// Generic type mangling uses SINGLE underscore: Option_Int, Result_Int_String
pub fn mangle_type_name(base: &str, args: &[Ty], registry: &TypeRegistry) -> String {
    let mut name = base.to_string();
    for arg in args {
        name.push('_');
        name.push_str(&mir_type_suffix(&resolve_type(arg, registry, false)));
    }
    name
}
// Trait method mangling uses DOUBLE underscore: Greetable__greet__MyStruct
// This avoids collision with generic type mangling.
```

### TraitRegistry API Available in Lowerer

```rust
// Source: snow-typeck/src/traits.rs
// All of these are available via self.trait_registry in the Lowerer:

// Check if a type implements a trait
fn has_impl(&self, trait_name: &str, ty: &Ty) -> bool

// Find the impl for a trait+type pair
fn find_impl(&self, trait_name: &str, ty: &Ty) -> Option<&ImplDef>

// Resolve a trait method's return type for a concrete type
fn resolve_trait_method(&self, method_name: &str, arg_ty: &Ty) -> Option<Ty>

// Find all traits providing a method for a type (ambiguity detection)
fn find_method_traits(&self, method_name: &str, ty: &Ty) -> Vec<String>

// Check where-clause constraints
fn check_where_constraints(&self, constraints: &[(String, String)],
    type_args: &FxHashMap<String, Ty>, origin: ConstraintOrigin) -> Vec<TypeError>
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| ImplDef methods lowered with bare names | Will use `Trait__Method__Type` mangling | Phase 19-01 | Unique function names, no collisions |
| Call sites use bare method name | Will resolve to mangled name via TraitRegistry | Phase 19-02 | Correct dispatch for all types |
| No where-clause enforcement in codegen | Will verify bounds at MIR lowering (or trust typeck) | Phase 19-03 | Defense-in-depth |
| No monomorphization depth limit | Configurable depth counter (default 64) | Phase 19-04 | Prevents compiler stack overflow |

## Open Questions

1. **Should where-clause enforcement be re-checked at MIR lowering?**
   - What we know: The type checker (`infer_call` at `infer.rs:2364-2405`) already comprehensively checks where-clause constraints at every call site. If typeck reports zero errors, all bounds are satisfied.
   - What's unclear: Is defense-in-depth worth the implementation cost of threading `FnConstraints` into `TypeckResult`?
   - Recommendation: Trust typeck. Add an assertion/panic in the lowerer if a trait method call cannot be resolved, which would indicate a typeck bug. This gives safety without the complexity of duplicating where-clause checking.

2. **How to handle method name ambiguity at MIR lowering?**
   - What we know: `find_method_traits` can return multiple trait names if multiple traits provide the same method for the same type. The type checker detects this and reports `AmbiguousMethod` errors.
   - What's unclear: If typeck detects ambiguity and continues (error recovery), the lowerer will see the ambiguous call.
   - Recommendation: If `find_method_traits` returns multiple results, use the FIRST one (same as resolve_trait_method does). The error was already reported by typeck. This matches the error-recovery strategy in 18-02 (push impl even on duplicate).

3. **Operator dispatch (binop) for user types**
   - What we know: `lower_binary_expr` always emits `MirExpr::BinOp`. `codegen_binop` only handles Int/Float/Bool/String. User-defined `impl Add for MyStruct` would need `a + b` to become `MirExpr::Call` to `Add__add__MyStruct`.
   - What's unclear: Is this in scope for Phase 19 or deferred?
   - Recommendation: Include a basic version in Phase 19-02. When lowering a binary expr, check if the operand type has an impl for the corresponding trait (Add, Sub, Mul, etc.) via TraitRegistry. If it's a user type (not Int/Float/Bool/String), emit `MirExpr::Call` instead of `MirExpr::BinOp`. Keep the existing `MirExpr::BinOp` path for primitives.

4. **MirType to Ty reverse mapping for TraitRegistry lookups**
   - What we know: TraitRegistry methods take `&Ty` but the lowerer works with `MirType`. Need a reverse mapping.
   - What's unclear: How complete does this mapping need to be? Only simple types (Int, Float, String, Bool, Struct) or also complex types (Tuple, FnPtr, etc.)?
   - Recommendation: Start with simple types only. Trait impls for tuples, function types, etc. are extremely unlikely in v1.3. A `mir_type_to_ty` helper covering Int, Float, String, Bool, Struct, SumType is sufficient.

## Sources

### Primary (HIGH confidence)
- `crates/snow-codegen/src/mir/lower.rs` -- Full MIR lowering implementation, ImplDef handling (line 427-432), call expr lowering (line 1282-1365), pre-registration (line 146-188), known_functions usage
- `crates/snow-typeck/src/traits.rs` -- TraitRegistry with all resolution methods (has_impl, find_impl, resolve_trait_method, find_method_traits, check_where_constraints)
- `crates/snow-typeck/src/infer.rs` -- infer_impl_def (line 1672-1820) showing PATH extraction and method registration, extract_where_constraints (line 4862-4886), FnConstraints struct (line 188-199)
- `crates/snow-codegen/src/mir/mod.rs` -- MirFunction, MirExpr variants, MirType enum
- `crates/snow-codegen/src/mir/types.rs` -- resolve_type, mangle_type_name, mir_type_suffix
- `crates/snow-codegen/src/mir/mono.rs` -- monomorphize, collect_reachable_functions
- `crates/snow-codegen/src/codegen/expr.rs` -- codegen_call (line 498+), codegen_binop (line 209-244)
- `crates/snow-parser/src/ast/item.rs` -- ImplDef.methods(), ImplDef.trait_path(), Path.segments()
- `.planning/phases/18-trait-infrastructure/18-VERIFICATION.md` -- Phase 18 completion proof
- `.planning/phases/18-trait-infrastructure/18-03-SUMMARY.md` -- TraitRegistry threading confirmation

### Secondary (MEDIUM confidence)
- `crates/snow-typeck/tests/integration.rs` -- Where-clause enforcement tests (line 176-195)
- `crates/snow-typeck/tests/traits.rs` -- Trait system integration tests
- `.planning/milestones/v1.3-REQUIREMENTS.md` -- CODEGEN-01 through CODEGEN-05 requirement definitions

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- zero new dependencies, all internal crates, fully readable
- Architecture: HIGH -- all patterns derived from reading actual codebase (lower.rs, traits.rs, infer.rs), not external sources
- Pitfalls: HIGH -- all 6 pitfalls identified from tracing actual execution paths through the lowerer and type checker

**Key file locations for implementation:**
- `crates/snow-codegen/src/mir/lower.rs` -- Primary modification target (ImplDef lowering, call-site resolution, pre-registration)
- `crates/snow-codegen/src/mir/types.rs` -- May need `mir_type_to_ty` helper
- `crates/snow-codegen/src/mir/mono.rs` -- Monomorphization depth limit (simple counter)
- `crates/snow-typeck/src/traits.rs` -- Read-only (API already sufficient)

**Line counts for key files:**
- `lower.rs`: ~4250 lines (with tests)
- `traits.rs`: 783 lines (with tests)
- `types.rs`: 345 lines (with tests)
- `mono.rs`: 305 lines (with tests)
- `mod.rs`: 495 lines

**Research date:** 2026-02-08
**Valid until:** 60 days (internal codebase, stable)
