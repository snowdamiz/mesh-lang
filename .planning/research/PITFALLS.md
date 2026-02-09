# Domain Pitfalls: Adding Method Dot-Syntax to Snow

**Domain:** Compiler extension -- method dot-syntax for an existing HM-inferred language
**Researched:** 2026-02-08
**Confidence:** HIGH (based on direct codebase analysis + established compiler engineering knowledge)

**Scope:** This document covers pitfalls specific to adding `expr.method(args)` dot-syntax to the Snow compiler, which already has field access (`point.x`), module-qualified calls (`String.length(s)`), pipe operator (`|>`), sum type variant constructors (`Shape.Circle`), HM type inference, and monomorphization-based static dispatch.

---

## Critical Pitfalls

Mistakes that cause rewrites, soundness holes, or regressions in existing features.

---

### Pitfall 1: Resolution Order Determines Semantics -- And You Will Get It Wrong First

**What goes wrong:** The `infer_field_access` function (infer.rs:3879) currently resolves `expr.ident` in this order:

1. Stdlib module lookup (`String.length`)
2. Service module method (`Counter.get_count`)
3. Sum type variant constructor (`Shape.Circle`)
4. Struct field access (`point.x`)
5. Fallback: fresh type variable (silently succeed with unknown type)

Adding method resolution means inserting a new step into this chain. **Every ordering choice creates a different class of bugs:**

- **Methods before fields:** A struct with a field `x` and an impl method `x()` becomes ambiguous. `point.x` could mean field access (no parens) or a method reference (if Snow supports first-class method references). The parser currently produces the same `FIELD_ACCESS` node for both `point.x` and the base of `point.x()`.

- **Methods after fields:** A struct field named `length` on a type with `impl Display` providing `length()` shadows the method. Users write `s.length` expecting the method but get the field.

- **Methods before variant constructors:** `Shape.Circle` could resolve as a method on the `Shape` type if someone adds an impl method named `Circle`. This would silently change the meaning of existing code.

**Why it happens:** The parser produces `FIELD_ACCESS` for `point.x` regardless of whether `x` is a field, a method, a variant, or a module member. All disambiguation is deferred to the type checker, which runs a linear if/else chain. Adding method resolution means choosing where in this chain it goes.

**Consequences:** Silent semantic changes to existing programs. A program that compiled and did one thing will now do a different thing with no error message.

**Prevention:**
- The parser already handles `expr.ident` as `FIELD_ACCESS` and `expr.ident(args)` as `CallExpr(FieldAccess, ArgList)`. Method resolution should ONLY trigger on the latter pattern -- when `FieldAccess` is the callee of a `CallExpr`. Pure `FieldAccess` without trailing parens should NEVER resolve to a method. This matches the "impl-method-only" constraint.
- Within `infer_field_access`, method resolution must come AFTER struct field lookup, not before. Fields are lexically visible; methods require type-directed lookup. Fields win.
- Within `infer_field_access`, method resolution must come AFTER sum type variant lookup and service module lookup. These are name-based lookups on the base expression's text; methods are type-directed lookups on the base expression's inferred type.
- Write explicit tests for every ordering conflict: struct field vs method with same name, module member vs method with same name, sum variant vs method with same name.

**Detection:** Any test where `point.x` changes meaning after adding an impl method named `x` to the type of `point`.

---

### Pitfall 2: Type Variable Leak from Premature Method Resolution

**What goes wrong:** In Snow's HM inference, `infer_field_access` infers the base expression type, then resolves the type to determine what `.ident` means. But if the base type is still a type variable (`Ty::Var`) at resolution time, method lookup cannot proceed because we do not know which type's impl to search.

The current code (infer.rs:3947-3993) calls `ctx.resolve(base_ty)` and then checks if it is `Ty::App` or `Ty::Con` to determine the struct name. If the type is still `Ty::Var`, it falls through to the fallback `Ok(ctx.fresh_var())` at line 3992 -- silently returning a fresh variable instead of reporting an error.

With method resolution added, this fallback becomes dangerous: `x.method()` where `x` has an unresolved type variable will silently produce a fresh variable for the entire call, causing downstream type errors that are impossible to diagnose.

**Why it happens:** HM inference processes expressions left-to-right within the AST. When a method call `x.foo()` appears before `x`'s type is constrained, the base expression type is still `?N`. The `TraitRegistry::resolve_trait_method` requires a concrete type to search impls. Passing `Ty::Var` to it produces no results, even if later unification would have revealed `x : Point`.

**Consequences:**
- Method calls on variables whose type is inferred later in the expression/block produce incorrect fresh-var types instead of the method's return type.
- Downstream unification errors blame the wrong expression.
- The error messages say things like "expected Int, found ?47" which are meaningless to users.

**Prevention:**
- When the resolved base type is `Ty::Var` and we are attempting method resolution, do NOT fall through to fresh_var. Instead, defer method resolution: create a fresh return-type variable, record a "pending method resolution" constraint, and revisit after more unification has occurred. Alternatively, emit a specific error: "cannot call method `foo` on a value of unknown type -- add a type annotation."
- In practice, the simpler approach for Snow (which does not have full constraint solving) is: resolve the base type, and if it is still a variable, check if unification has bound it. If not, emit a clear error directing the user to annotate the type.
- Test: `let x = some_function(); x.method()` where `some_function` returns a generic/polymorphic type.

**Detection:** Any test where method call returns `?N` type variable instead of the method's declared return type.

---

### Pitfall 3: Method Call vs. Pipe Operator Semantic Divergence

**What goes wrong:** Snow already has `|>` (pipe operator). `x |> foo` desugars to `foo(x)`. Method dot-syntax `x.foo()` also desugars to `foo(x)` at the call level. These two features have ALMOST the same semantics but DIFFERENT resolution paths, leading to cases where one works and the other does not, or where they produce different types.

The pipe operator (infer.rs:2817) infers the LHS, infers the RHS callee, prepends LHS type to the arg list, and unifies with a function type. It goes through `infer_expr` for the callee, which resolves names through the normal scope chain.

Method dot-syntax, by contrast, must resolve through the `TraitRegistry`, searching impl blocks for the method name on the base type. These are fundamentally different resolution paths.

**Specific divergence scenarios:**

1. **`x |> to_string` works but `x.to_string()` does not:** The pipe version finds `to_string` as a regular function in the env (registered at infer.rs:2139-2141). The method version searches impl blocks. If the method is registered in the env but not properly in the trait registry, pipe works and dot does not.

2. **`x.length()` works but `x |> length` does not:** The method version finds `length` in the impl block for `x`'s type. The pipe version tries to find a standalone function `length` in scope, which may not exist (methods are not standalone functions).

3. **Different type inference behavior:** The pipe infers callee type independently and then unifies. Method resolution resolves the return type by looking up the impl's return type annotation. If the impl's return type uses a type variable while the standalone function uses a concrete type, the results differ.

**Why it happens:** The pipe operator was designed for free-standing functions. Methods live in impl blocks. These are separate namespaces with separate resolution rules.

**Consequences:** Users expect `x.foo(a)` and `x |> foo(a)` to be interchangeable. When they are not, the resulting confusion erodes trust in the type system.

**Prevention:**
- Design decision: are methods also callable via pipe? If `impl Display for Point` defines `to_string(self)`, does `point |> to_string` work? If yes, method registration must also insert the method as a callable function in the env (which is already happening at infer.rs:2139-2141). Verify this path is not broken.
- Design decision: are free-standing functions also callable via dot? If `fn length(s: String) -> Int` is defined, does `s.length()` work? For "impl-method-only" dot syntax, the answer is NO -- only methods defined in `impl` blocks are callable via dot.
- Test: for every method callable via dot, verify it is also callable via pipe, and vice versa (or explicitly document the asymmetry).
- The MIR lowerer already handles pipe desugaring in `lower_pipe_expr` (lower.rs:3658). Method call lowering must produce the same MIR structure (a `MirExpr::Call` with self prepended to args) to ensure codegen consistency.

**Detection:** Any test where `x.method()` and `x |> method` produce different types or one errors while the other succeeds.

---

### Pitfall 4: Breaking Codegen -- FieldAccess MIR Node vs. Method Call MIR Node

**What goes wrong:** The MIR has a `FieldAccess` variant (mir/mod.rs:208) that takes `{ object, field, ty }` and generates LLVM struct GEP instructions (codegen/expr.rs:1096). It also has a `Call` variant for function calls. Currently, `lower_field_access` (lower.rs:3705) always produces `MirExpr::FieldAccess`.

When adding method dot-syntax, `expr.method(args)` must NOT lower to `MirExpr::FieldAccess` -- it must lower to `MirExpr::Call` with the method's mangled name and `expr` prepended to the arg list. But the parser produces `CallExpr(FieldAccess(expr, method), ArgList(args))`. The MIR lowerer's `lower_call_expr` calls `self.lower_expr(&callee)` on the callee, which dispatches to `lower_field_access`, which produces a `MirExpr::FieldAccess`. This `FieldAccess` MIR node then appears as the callee of a `Call`, which codegen does not know how to handle -- it tries to do a struct GEP on the "callee" and crashes.

**Why it happens:** The existing MIR lowerer was designed when `FieldAccess` always meant struct field access. It does not have a `MethodCall` MIR variant. The call lowering code (lower.rs:3362-3654) does trait method rewriting based on the callee being a `MirExpr::Var` -- but a method call from dot-syntax arrives as `MirExpr::FieldAccess`, not `MirExpr::Var`.

**Consequences:** Compiler crash (LLVM codegen panic) or incorrect code generation. The codegen at expr.rs:1107 explicitly errors on non-struct types: `"Field access on non-struct type"`. A method call on `Int` or `String` would hit this path and crash.

**Prevention:**
- The MIR lowerer must intercept the `CallExpr(FieldAccess(...), ArgList(...))` pattern BEFORE calling `lower_field_access` on the callee. In `lower_call_expr`, check if the callee is an `Expr::FieldAccess`. If so, extract the base and method name, resolve the method through the trait registry, produce a `MirExpr::Call` with the mangled function name and self-prepended args. Do NOT call `lower_field_access` for method calls.
- Alternatively, add a new MIR variant `MirExpr::MethodCall { receiver, method, trait_name, args, ty }` that codegen translates to the mangled call. This is cleaner but requires changes to mir/mod.rs, mono.rs, and codegen/expr.rs.
- The simpler approach: rewrite to `MirExpr::Call { func: MirExpr::Var(mangled_name, fn_ty), args: [self, ...args], ty }` in the lowerer, reusing the existing trait method dispatch path that already exists for `to_string(42)` style calls (lower.rs:3527-3600).

**Detection:** Any test where `expr.method()` compiles to MIR and passes through codegen without crashing.

---

### Pitfall 5: Monomorphization Does Not Know About Method Calls

**What goes wrong:** The monomorphization pass (mono.rs) collects reachable functions by walking MIR expressions and collecting function names from `MirExpr::Var` and `MirExpr::MakeClosure`. If method calls are lowered to `MirExpr::Call { func: MirExpr::Var(mangled_name, ...) }`, this works automatically. But if they are lowered to a new `MethodCall` MIR variant, the monomorphization pass will not see them, and the method function will be pruned as unreachable.

**Why it happens:** `collect_function_refs` (mono.rs:86) has explicit match arms for every MIR variant. A new variant without a corresponding match arm will be silently ignored, causing the method's impl function to be dropped.

**Consequences:** Linker errors ("undefined symbol") or runtime crashes from calling pruned functions.

**Prevention:**
- If adding a new MIR variant, add a corresponding arm to `collect_function_refs` immediately.
- Prefer the simpler approach of rewriting method calls to standard `MirExpr::Call` with mangled names, so the existing `collect_function_refs` arm for `Call` handles them automatically.
- Test: verify that a method called only via dot-syntax is present in the final binary.

**Detection:** Linker error on a program that uses method dot-syntax.

---

### Pitfall 6: The `self` Binding Escapes Its Scope

**What goes wrong:** In `infer_impl_def` (infer.rs:2084-2129), the type checker pushes a new scope, inserts `self` bound to the impl type, type-checks the method body, then pops the scope. This works for the body of the method definition.

But when type-checking a method CALL (`x.method()`), the type checker must NOT insert `self` into the caller's scope. If the method call resolution naively reuses the impl's type signature (which has a `self` parameter), and the resolution code accidentally inserts `self` into the calling scope, it will shadow any existing `self` binding (e.g., inside another method body or an actor's `receive` block where `self` means the actor's PID).

**Why it happens:** The method's type in the env is stored as `Ty::Fun([impl_type, ...params], ret)` (infer.rs:2132-2136). The first parameter corresponds to `self` but is stored as the impl type, not as a named binding. If the call-site resolution code instantiates the method scheme and then tries to create a `self` binding for type-checking, it leaks into the calling scope.

**Consequences:** Silent semantic change: `self` in the calling context (e.g., an actor method) gets shadowed. The caller's `self` reference now has the wrong type. Actor self-references break.

**Prevention:**
- Method call resolution must NEVER push `self` into the calling scope. It should only unify the first argument type with the receiver's type.
- The call-site pattern for `x.method(a, b)` should be: infer type of `x`, look up `method` in impls for that type, get the method's function type `(Self, A, B) -> R`, unify `x`'s type with `Self`, unify `a` with `A`, unify `b` with `B`, return `R`. No scope manipulation needed.
- Test: `x.method()` inside an actor `receive` block where `self` is the actor PID. Verify `self` still refers to the PID after the method call.

**Detection:** `self` reference after a method call resolves to the wrong type.

---

## Moderate Pitfalls

Mistakes that cause technical debt, confusing errors, or delayed regressions.

---

### Pitfall 7: Ambiguous Method Resolution Across Multiple Traits

**What goes wrong:** The `TraitRegistry::find_method_traits` (traits.rs:246) already handles the case where multiple traits provide the same method name for the same type. The MIR lowerer (lower.rs:3537) takes the FIRST match: `matching_traits[0]`. This is nondeterministic because `self.impls` is a `FxHashMap` (which does not guarantee iteration order).

With method dot-syntax, this ambiguity becomes user-facing. When writing `x.to_string()`, if both `Display` and `Debug` provide a `to_string` method for `x`'s type, the result depends on HashMap iteration order. This is a correctness bug that manifests as nondeterministic behavior across compilations.

**Why it happens:** `FxHashMap` iteration order is not stable. The existing code at lower.rs:3537 silently picks whichever trait comes first in the hash map's iterator.

**Prevention:**
- In the type checker: when resolving `x.method()`, call `find_method_traits` and if it returns more than one trait, emit an ambiguity error: "method `method` is provided by multiple traits for type `T`: Trait1, Trait2. Use explicit qualified syntax: `Trait1.method(x)`."
- In the MIR lowerer: sort `matching_traits` alphabetically before picking the first one, to ensure deterministic behavior even without the type checker's ambiguity check.
- Test: define two traits with the same method name, impl both for the same type, call via dot-syntax, assert ambiguity error.

**Detection:** A program that compiles differently on different runs.

---

### Pitfall 8: Generic Type Parameter Confusion in Method Return Types

**What goes wrong:** When a method is defined in an `impl` block for a generic type (e.g., `impl Display for List<T>`), the method's return type may reference `T`. The current `resolve_trait_method` (traits.rs:212-238) freshens type parameters and resolves through a temporary `InferCtx`. But this temporary context is discarded after the lookup -- the resolved return type's variables are from the temporary context, not the caller's `InferCtx`.

If a method call `my_list.first()` returns `T`, and `my_list` is `List<Int>`, the temporary context correctly resolves `T` to `Int`. But if the return type contains multiple type parameters or nested generics, the freshening and resolution may not correctly propagate all bindings.

**Why it happens:** The `freshen_type_params` function (traits.rs:299) only freshens single-uppercase-letter type constructors (`A`-`Z`). Type parameters named with multiple characters (e.g., `Key`, `Value`) are not freshened, causing them to be treated as concrete types.

**Consequences:** Methods on types with multi-character type parameters return incorrect types. The return type contains unresolved type parameter names as if they were concrete types.

**Prevention:**
- Audit `freshen_type_params` to ensure it handles ALL type parameters from the impl, not just single-letter ones.
- Store type parameter names in the `ImplDef` structure and use them for freshening, rather than relying on the naming convention.
- Test: define a generic struct with multi-character type params, impl a method, call via dot-syntax, verify the return type is correctly instantiated.

**Detection:** Method on `Map<Key, Value>` returns `Key` as a concrete type name instead of the actual key type.

---

### Pitfall 9: Method Calls in Chained Expressions

**What goes wrong:** The parser handles `a.b.c.d` as nested `FIELD_ACCESS` nodes: `FieldAccess(FieldAccess(FieldAccess(a, b), c), d)`. When method calls are mixed in, `a.b().c.d()` becomes `CallExpr(FieldAccess(CallExpr(FieldAccess(a, b), ArgList()), d), ArgList())` with `FieldAccess(CallExpr(...), c)` in the middle.

The type checker must resolve each step left-to-right: infer `a`, resolve `b` as a method call, get the return type, use that as the base for field access `c`, then resolve `d` as another method call. If any step in the chain fails to produce a concrete type (see Pitfall 2 about type variables), the entire chain breaks with an inscrutable error.

**Why it happens:** Chained expressions are deeply nested in the CST. Each `infer_field_access` call must fully resolve its base before proceeding. If method resolution returns a fresh variable instead of the concrete return type, the next step in the chain sees an unresolved type and cannot proceed.

**Prevention:**
- Ensure method resolution ALWAYS returns the method's declared return type (instantiated with the receiver's type parameters), never a fresh variable.
- Error messages for chained expressions should identify WHICH step in the chain failed: "in expression `a.b().c`, could not resolve method `b` on type `A`."
- Test: `point.to_string().length()` -- method call on `Point` returning `String`, then field access or method call on `String`.

**Detection:** Chained method calls producing cascading type errors that blame the wrong subexpression.

---

### Pitfall 10: The MIR Lowerer's Trait Dispatch Duplicates Type Checker Logic

**What goes wrong:** The MIR lowerer (lower.rs:3527-3600) already has its own trait method dispatch: it checks if a callee name is a known function, and if not, searches the trait registry for matching traits and rewrites to mangled names. Adding method dot-syntax introduces a SECOND dispatch path: the lowerer must also handle `CallExpr(FieldAccess(...))` by looking up methods.

If these two paths use slightly different logic (e.g., different mangling schemes, different trait lookup order, different handling of builtin vs. user-defined types), the same method call will produce different MIR depending on how it is written.

**Why it happens:** The existing `lower_call_expr` was designed for `to_string(42)` style calls (function-name-first). Method dot-syntax introduces `42.to_string()` style calls (receiver-first). The lowerer must handle both and produce identical MIR.

**Prevention:**
- Both call paths should converge to a single helper function: `resolve_method_call(receiver_type, method_name, args) -> MirExpr::Call`. This helper handles trait lookup, mangling, and builtin short-circuits (like `Display__to_string__String` -> identity) in one place.
- Test: verify that `to_string(42)` and `42.to_string()` produce identical MIR.

**Detection:** Two syntactically equivalent method calls produce different runtime behavior.

---

### Pitfall 11: LSP Go-to-Definition Does Not Know About Methods

**What goes wrong:** The LSP server's definition handler (snow-lsp/src/definition.rs) currently does not handle `FieldAccess` at all (confirmed by grep -- no matches for FieldAccess in definition.rs). When method dot-syntax is added, clicking "go to definition" on `.method()` will do nothing or jump to the wrong location.

**Why it happens:** The LSP was built when `FieldAccess` only meant struct fields and module members, which are resolved by name, not by type. Method resolution requires type information that the LSP may not have readily available for the cursor position.

**Prevention:**
- This is a follow-up feature, not a blocker. But plan for it: the type checker should record method resolution results (which trait, which impl, which method) in the `TypeckResult` so the LSP can query them.
- Add a `method_resolutions: FxHashMap<TextRange, MethodResolution>` to `TypeckResult` where `MethodResolution` contains the trait name, impl type, and source range of the method body.

**Detection:** Clicking "go to definition" on a method call in the LSP does nothing.

---

## Minor Pitfalls

Mistakes that cause annoyance but are fixable without major rework.

---

### Pitfall 12: Error Messages Reference "Field" When User Wrote a Method Call

**What goes wrong:** The `TypeError::NoSuchField` error (error.rs:115) says `"type 'X' has no field 'y'"`. When the user writes `x.y()` intending a method call, the error message talks about "field" instead of "method", which is confusing.

**Prevention:**
- When method resolution fails and the expression is the callee of a `CallExpr`, use a different error: `"type 'X' has no method 'y'"`. Provide a suggestion: "did you mean to define a method `y` in an impl block for `X`?"
- Add a new `TypeError::NoSuchMethod` variant that includes the type, method name, span, and optionally a list of available methods for that type.

---

### Pitfall 13: Method Name Collides with Builtin Runtime Function

**What goes wrong:** The MIR lowerer maps method names to mangled names like `Display__to_string__Int`, and some of these are further mapped to runtime names like `snow_int_to_string` (lower.rs:3544-3560). If a user defines an impl method whose mangled name collides with a runtime function name, the lowerer silently redirects to the runtime function.

**Prevention:**
- The mangling scheme (`Trait__Method__Type`) should be designed to never collide with the `snow_*` runtime prefix. This is already the case. But verify that user-defined trait names cannot start with `snow_`.
- Test: define a trait named `snow_internal` and verify it does not cause name collisions.

---

### Pitfall 14: Formatter Does Not Preserve Method Call Syntax

**What goes wrong:** The Snow formatter (snow-fmt) uses a CST walker to pretty-print code. If it does not have specific handling for `CallExpr(FieldAccess(...), ArgList(...))`, it may format `x.method(a, b)` with unexpected whitespace or line breaking.

**Prevention:**
- The formatter handles `FIELD_ACCESS` and `CALL_EXPR` separately. Since method calls are syntactically just `CallExpr` wrapping `FieldAccess`, the existing formatting should mostly work. But test edge cases: `very_long_expression.method(very_long_arg1, very_long_arg2)` should break sensibly.

---

## The Central Integration Challenge

**The single most likely thing to go wrong is the resolution order in `infer_field_access` (Pitfall 1).** This is the integration point where all existing features collide with the new feature. The current function is a carefully ordered chain of if/else blocks handling five different meanings of `expr.ident`. Adding a sixth meaning (method call) requires getting the ordering exactly right, AND requires distinguishing "bare field access" from "method call" (which depends on whether the FieldAccess is the callee of a CallExpr).

The type checker currently processes `FieldAccess` in isolation -- it does not know whether the FieldAccess is being used as a callee. The fix requires either:

(a) Passing context from the parent `CallExpr` inference down to `infer_field_access` (e.g., a flag `is_callee: bool`), or

(b) Handling method calls at the `CallExpr` level BEFORE dispatching to `infer_field_access` -- detect when the callee is a `FieldAccess`, extract the base and method name, and handle method resolution in `infer_call` rather than `infer_field_access`.

Option (b) is cleaner because it keeps `infer_field_access` focused on field access and avoids threading flags through the call chain.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| Parser changes (if any needed) | No parser changes needed -- `expr.ident(args)` already parses as `CallExpr(FieldAccess(...), ArgList(...))` | Verify with parser tests that this CST structure is correct |
| Type checker: method resolution in `infer_field_access` | Pitfalls 1, 2, 6: resolution order, type variables, self-binding leak | Add method resolution AFTER all existing checks; only trigger when FieldAccess is callee of CallExpr; or handle at CallExpr level instead |
| Type checker: return type inference | Pitfall 8: generic type params in return types | Audit `freshen_type_params` for multi-character params |
| Type checker: ambiguity detection | Pitfall 7: multiple traits providing same method | Check `find_method_traits` result count and emit error if > 1 |
| MIR lowering | Pitfalls 4, 10: FieldAccess vs Call MIR node, duplicated dispatch logic | Intercept method calls in `lower_call_expr` before `lower_field_access`; converge dispatch paths |
| Monomorphization | Pitfall 5: method functions pruned as unreachable | Use standard `MirExpr::Call` with mangled names, or add match arm in `collect_function_refs` |
| Codegen | Pitfall 4 continued: codegen crashes on non-struct FieldAccess | Method calls must never reach `codegen_field_access` |
| Error reporting | Pitfalls 12, 9: confusing error messages, chain failures | Add `NoSuchMethod` error variant; improve chain error provenance |
| Pipe operator interaction | Pitfall 3: semantic divergence between `.method()` and `\|> method` | Decide on asymmetry; test both paths |
| LSP | Pitfall 11: go-to-definition broken for methods | Plan `method_resolutions` map in TypeckResult |

---

## Sources

### Snow Codebase Analysis (HIGH confidence -- direct code reading)
- `crates/snow-typeck/src/infer.rs` -- `infer_field_access` (line 3879), `infer_pipe` (line 2817), `infer_impl_def` (line 2003), `infer_call` resolution chain
- `crates/snow-typeck/src/traits.rs` -- `TraitRegistry`, `resolve_trait_method` (line 212), `find_method_traits` (line 246), `freshen_type_params` (line 299)
- `crates/snow-typeck/src/unify.rs` -- `InferCtx`, `resolve`, `unify`, `generalize`, `instantiate`
- `crates/snow-typeck/src/ty.rs` -- `Ty` enum, `Scheme` struct
- `crates/snow-typeck/src/env.rs` -- `TypeEnv` scope stack
- `crates/snow-parser/src/parser/expressions.rs` -- postfix `FIELD_ACCESS` parsing (line 117), `CALL_EXPR` (line 105)
- `crates/snow-parser/src/ast/expr.rs` -- `FieldAccess`, `CallExpr` AST nodes
- `crates/snow-codegen/src/mir/lower.rs` -- `lower_field_access` (line 3705), `lower_call_expr` (line 3362), trait method dispatch (line 3527)
- `crates/snow-codegen/src/mir/mod.rs` -- `MirExpr::FieldAccess` (line 208), `MirExpr::Call` (line 168)
- `crates/snow-codegen/src/mir/mono.rs` -- `collect_function_refs` (line 86), `MirExpr::FieldAccess` arm (line 155)
- `crates/snow-codegen/src/codegen/expr.rs` -- `codegen_field_access` (line 1096), struct GEP (line 1128)
- `crates/snow-lsp/src/definition.rs` -- no FieldAccess handling

### Compiler Engineering Domain Knowledge (HIGH confidence)
- [Rust method resolution: inherent vs trait methods (issue #51402)](https://github.com/rust-lang/rust/issues/51402) -- method resolution does not check parameter types in probe phase; inherent methods priority breaks for trait objects
- [Rust RFC 0048 on traits](https://rust-lang.github.io/rfcs/0048-traits.html) -- UFCS and inherent vs trait method precedence design
- [Deref confusion and method resolution](https://www.fuzzypixelz.com/blog/deref-confusion/) -- method resolution ambiguity with smart pointer types
- [OCaml record field disambiguation](https://www.lexifi.com/blog/ocaml/type-based-selection-label-and-constructors/) -- type-based selection of labels and constructors; "last record in scope" pitfall
- [OCaml record field ambiguity](https://dev.realworldocaml.org/records.html) -- reusing field names leads to ambiguity; ordering determines resolution
- [Hindley-Milner and overloading](https://en.wikipedia.org/wiki/Hindley%E2%80%93Milner_type_system) -- overloading complications with HM inference
- [Type inference for overloading](https://link.springer.com/chapter/10.1007/10705424_3) -- challenges of overloading without restrictions in HM systems
- [C# Overload Resolution Priority (C# 13)](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/proposals/csharp-13.0/overload-resolution-priority) -- API evolution pitfalls with method overloading

---
*Pitfalls research for: Snow method dot-syntax milestone*
*Researched: 2026-02-08*
