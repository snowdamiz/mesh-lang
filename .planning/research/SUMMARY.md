# Project Research Summary

**Project:** Method Dot-Syntax for Snow Language (v1.6)
**Domain:** Compiler feature addition — method call syntax for existing typed language
**Researched:** 2026-02-08
**Confidence:** HIGH

## Executive Summary

Method dot-syntax (`value.method(args)`) for the Snow compiler is a **wiring exercise**, not a new-capability exercise. The entire feature builds on existing infrastructure: the parser already handles `expr.method(args)` as `CALL_EXPR(FIELD_ACCESS(...), ARG_LIST)`, the type checker has `TraitRegistry` with `find_method_traits()` for impl lookup, MIR lowering already rewrites trait method calls to mangled names (`Trait__Method__Type`), and codegen handles function calls. No new dependencies, no new MIR nodes, no new runtime mechanisms.

The recommended approach is pure desugaring at two integration points: (1) type checker adds method resolution as a fallback in `infer_field_access` when struct field lookup fails, and (2) MIR lowering detects `CallExpr(FieldAccess(...))` pattern and rewrites to `method(receiver, args)` before trait dispatch. This produces identical MIR as bare-name calls (`to_string(point)` vs `point.to_string()`), ensuring consistency with the existing pipe operator and trait system. The implementation is ~150 lines in type checker, ~100 lines in MIR lowering, plus tests.

The critical risk is **resolution priority ordering** in `infer_field_access`. This function currently resolves `expr.ident` through a priority chain: module lookup → service lookup → variant constructor → struct field. Method resolution must come AFTER struct fields (fields win over methods) but handle the case where the base type has no struct definition (primitives like `Int`). Getting this ordering wrong breaks existing code — `self.x` in impl bodies must remain struct field access, and `String.length` must remain module-qualified calls. The mitigation is clear: method resolution only triggers when (a) struct field lookup fails, and (b) the FieldAccess is the callee of a CallExpr (has trailing parentheses).

## Key Findings

### Recommended Stack

**Summary from STACK.md:** No new dependencies required. Method dot-syntax is implemented entirely through changes to existing compiler passes. The parser handles the syntax, the type checker resolves methods via `TraitRegistry`, MIR lowering desugars to mangled function calls, and codegen remains unchanged.

**Core technologies (all existing):**
- **rowan CST library** — parser already produces correct tree structure for `expr.method(args)`
- **TraitRegistry (snow-typeck)** — `find_method_traits(name, ty)` performs impl lookup with structural unification for generics
- **MIR mangling (snow-codegen)** — `Trait__Method__Type` mangling already used for bare-name calls like `to_string(42)`
- **Monomorphization (snow-codegen)** — static dispatch via name mangling, no vtables, no runtime method resolution

**What NOT to add:**
- No new CST node (reuse `CALL_EXPR(FIELD_ACCESS(...), ARG_LIST)`)
- No new MIR node (method calls become `MirExpr::Call`)
- No new dependencies (rowan, inkwell, ariadne versions unchanged)

### Expected Features

**Summary from FEATURES.md:** The MVP focuses on trait method dispatch via dot-syntax, matching Rust's approach but simpler due to Snow's lack of references and autoderef. Method chaining falls out naturally from postfix parsing. Inherent methods (impl without trait) are explicitly deferred to post-MVP.

**Must have (table stakes):**
- Basic method call: `value.method(args)` resolves to trait impl method with receiver as first arg
- Field vs. method disambiguation: `point.x` (field) vs `point.x()` (method) — parentheses decide
- Method chaining: `list.filter(pred).map(f).length()` — left-to-right resolution
- Trait method resolution: given receiver type, search all impls for method name
- Self-parameter passing: `value.method(a, b)` desugars to `method(value, a, b)`
- Generic type support: methods on `List<Int>` resolve through existing unification
- Clear error messages: "type X has no method `foo`" with ambiguity detection

**Should have (competitive):**
- Ambiguity resolution: multiple traits with same method name produce clear error with disambiguation guidance
- Fully qualified syntax: `Display.to_string(point)` as explicit fallback (already works via module-qualified calls)
- Pipe operator coexistence: both `value.method(args)` and `value |> method(args)` work, user chooses

**Defer (v2+):**
- Inherent methods (`impl Type do ... end` without trait) — requires separate registry, new mangling, priority rules
- Method references / partial application (`let f = point.method`) — requires closure generation
- IDE autocomplete for methods — LSP enhancement, separate from core language

### Architecture Approach

**Summary from ARCHITECTURE.md:** Method dot-syntax is implemented as syntactic sugar via desugaring at two integration points. Type checker adds method resolution fallback in `infer_field_access` (after struct field lookup fails), returning the method's function type. MIR lowering detects `CallExpr(FieldAccess(...))` pattern in `lower_call_expr`, extracts receiver and method name, prepends receiver to args, and feeds into existing trait dispatch logic. Both paths converge to `MirExpr::Call { func: Var("Trait__Method__Type"), args: [receiver, ...] }` — identical to bare-name calls.

**Major components:**

1. **Type Checker (snow-typeck/infer.rs)** — `infer_field_access` gains method resolution fallback after struct field lookup fails; `infer_call` detects FieldAccess callee and prepends receiver type to arg list for unification.

2. **MIR Lowering (snow-codegen/mir/lower.rs)** — `lower_call_expr` intercepts `CallExpr(FieldAccess(...))` pattern at AST level, extracts receiver and method name, calls extracted `resolve_trait_method_callee` helper (shared with bare-name dispatch path).

3. **Trait Registry (snow-typeck/traits.rs)** — `find_method_traits(method_name, ty)` already exists; structural type matching via temporary unification handles generics; returns list of matching trait names for ambiguity detection.

**Resolution priority (critical invariant):**
1. Module-qualified (`String.length`) — existing
2. Service module (`Counter.get`) — existing
3. Sum type variant (`Shape.Circle`) — existing
4. Struct field (`point.x`) — existing
5. Method via trait impl (`point.to_string()`) — **NEW**, only when steps 1-4 fail
6. Error: NoSuchField/NoSuchMethod

**Build order:** (1) Extract trait dispatch helper in MIR lowering (refactor), (2) Add method resolution fallback in type checker, (3) Add receiver-type prepending in `infer_call`, (4) Add method call desugaring in MIR lowering, (5) End-to-end tests.

### Critical Pitfalls

From PITFALLS.md, top 5 that require explicit mitigation:

1. **Resolution Order Determines Semantics** — The priority chain in `infer_field_access` is fragile. Method resolution MUST come after struct fields, service modules, and variant constructors. Otherwise `self.x` in impl bodies breaks (fields shadow methods), or `String.length` breaks (module calls shadow methods). **Mitigation:** Method resolution only activates when (a) struct field lookup fails, AND (b) the FieldAccess is the callee of a CallExpr (has trailing parens). Test every ordering conflict explicitly.

2. **Type Variable Leak from Premature Method Resolution** — If the receiver type is still a type variable (`Ty::Var`) when method resolution runs, `find_method_traits` returns no results. The current code falls back to `fresh_var()`, causing cascading type errors. **Mitigation:** When receiver type resolves to `Ty::Var` and method lookup fails, emit clear error: "cannot call method on value of unknown type — add type annotation." Do NOT return fresh_var for method calls.

3. **Method Call vs. Pipe Operator Semantic Divergence** — Pipe works with any function (`x |> free_fn`), method dot-syntax only works with impl methods (`x.impl_method()`). These use different resolution paths (env lookup vs TraitRegistry). **Mitigation:** Decide on asymmetry and document it. Test that methods callable via dot are also callable via pipe (trait methods are registered in env at infer.rs:2139). Do NOT attempt UFCS (universal function call syntax).

4. **Breaking Codegen — FieldAccess MIR Node vs. Method Call** — The MIR lowerer's `lower_call_expr` calls `lower_expr` on the callee, which dispatches to `lower_field_access`, producing `MirExpr::FieldAccess`. This becomes the callee of a Call, and codegen crashes (tries to do struct GEP on a callee). **Mitigation:** Intercept the `CallExpr(FieldAccess(...))` pattern in `lower_call_expr` BEFORE calling `lower_field_access`. Extract receiver and method name from AST directly, desugar to `method(receiver, args)`, feed into trait dispatch. Method calls NEVER reach `lower_field_access`.

5. **Ambiguous Method Resolution Across Multiple Traits** — `find_method_traits` returns a `Vec<String>` of matching trait names. The current MIR lowerer takes `matching_traits[0]` — the first match from a HashMap iterator, which is nondeterministic. **Mitigation:** Type checker must detect `find_method_traits.len() > 1` and emit ambiguity error with all matching trait names. MIR lowering should sort `matching_traits` alphabetically as fallback for determinism.

## Implications for Roadmap

Based on research, suggested phase structure for method dot-syntax milestone:

### Phase 1: Core Type Checker Method Resolution
**Rationale:** Type checking is the semantic correctness layer. Getting method resolution right in the type checker ensures that `point.to_string()` is correctly typed before any code generation. This phase has no MIR/codegen changes, so failures are caught early with clear type errors.

**Delivers:**
- Method resolution fallback in `infer_field_access` (after struct field lookup fails)
- `find_method_traits` integration with proper priority ordering
- Clear error messages: "type X has no method `foo`"
- Ambiguity detection: multiple traits with same method name produce error
- Receiver-type prepending in `infer_call` for correct arity checking

**Addresses (from FEATURES.md):**
- Basic method call type checking
- Generic type support (via existing unification in TraitRegistry)
- Error messages for "no such method"

**Avoids (from PITFALLS.md):**
- Pitfall 1: Resolution order — method resolution comes AFTER struct fields
- Pitfall 2: Type variable leak — emit clear error when receiver type is unresolved
- Pitfall 5: Ambiguity detection — check `find_method_traits` result count

**Research flag:** Standard patterns. HM type inference + trait resolution is well-understood. No deep research needed.

### Phase 2: MIR Lowering Desugaring
**Rationale:** Depends on Phase 1 type checking being correct. MIR lowering trusts the types map from type checker. This phase reuses existing trait dispatch infrastructure (mangled names, builtin short-circuits, monomorphization).

**Delivers:**
- Intercept `CallExpr(FieldAccess(...))` in `lower_call_expr`
- Extract receiver and method name from AST
- Prepend receiver to arg list
- Feed into extracted `resolve_trait_method_callee` helper (shared with bare-name calls)
- Produce `MirExpr::Call { func: Var(mangled_name), args: [receiver, ...], ty }`

**Uses (from STACK.md):**
- Existing MIR Call node (no new variants)
- Trait__Method__Type mangling (already implemented)
- Builtin dispatch (Display__to_string__Int -> snow_int_to_string)

**Implements (from ARCHITECTURE.md):**
- Desugaring at MIR boundary (method calls become regular calls)
- Shared dispatch helper (bare-name and dot-syntax converge)

**Avoids (from PITFALLS.md):**
- Pitfall 4: Breaking codegen — intercept before `lower_field_access`, never produce FieldAccess MIR for method calls
- Pitfall 10: Duplicated dispatch logic — extract helper, call from both paths
- Pitfall 5: Monomorphization — method calls use standard Call nodes, automatically tracked

**Research flag:** Standard patterns. Trait dispatch via mangling matches Rust's monomorphized calls.

### Phase 3: Integration Testing and Edge Cases
**Rationale:** Validates interaction with existing features (field access, pipe operator, module-qualified calls, chaining). Tests priority rules and error handling comprehensively.

**Delivers:**
- End-to-end tests: `point.to_string()` compiles and runs
- Primitive receiver: `42.to_string()` returns "42"
- Chaining: `point.to_string().length()` works
- Generic receiver: `Box<Int>.to_string()` resolves through unification
- Pipe equivalence: `point.to_string()` == `point |> to_string()` (same result)
- Priority rules: struct field access still works when method has same name
- Error messages: clear diagnostics for no-such-method, ambiguity

**Addresses (from FEATURES.md):**
- Method chaining (falls out naturally from postfix parsing)
- Pipe operator coexistence (both syntaxes work)
- Fully qualified disambiguation (`Display.to_string(point)`)

**Avoids (from PITFALLS.md):**
- Pitfall 3: Pipe divergence — test both paths produce same results
- Pitfall 6: Self-binding leak — test method calls inside other method bodies
- Pitfall 9: Chained expressions — test nested method calls with clear error messages

**Research flag:** Standard testing patterns. No deep research needed.

### Phase Ordering Rationale

- **Phase 1 before Phase 2:** Type checking must be correct before MIR lowering can trust the types map. If typeck produces wrong types, MIR lowering propagates the error into codegen.
- **Phase 2 before Phase 3:** MIR lowering must produce valid Call nodes before integration tests can exercise the full pipeline.
- **No separate parser phase:** Parser already handles `expr.method(args)` correctly — verified via existing snapshot tests.
- **No codegen phase:** Codegen remains unchanged — method calls desugar to regular Call nodes at MIR level.

**Dependency structure:**
```
Existing Infrastructure (parser, TraitRegistry, MIR Call nodes)
    |
    v
Phase 1: Type Checker (method resolution + ambiguity detection)
    |
    v
Phase 2: MIR Lowering (desugaring to mangled calls)
    |
    v
Phase 3: Integration Tests (validate full pipeline)
```

### Research Flags

**Phases with standard patterns (skip `/gsd:research-phase`):**
- All three phases use well-established patterns from Rust compiler design
- HM type inference + trait resolution is textbook material
- Method desugaring to first-argument passing is standard UFCS implementation
- Mangling schemes for static dispatch are well-understood

**No phases require deep research.** The domain (compiler method resolution) is extremely well-documented, and Snow's existing infrastructure closely matches Rust's design.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Direct codebase analysis of 66,521 lines. All infrastructure exists. No new dependencies. |
| Features | HIGH | Extensive prior art from Rust, Swift, Kotlin. Method dot-syntax is well-established with clear expectations. |
| Architecture | HIGH | Two integration points identified with exact line numbers in codebase. Desugaring approach validated by existing pipe operator. |
| Pitfalls | HIGH | Codebase analysis reveals exact conflict points (infer_field_access resolution chain, lower_call_expr callee handling). Resolution order pitfall has explicit test plan. |

**Overall confidence:** HIGH

### Gaps to Address

**Gap 1: Single-letter vs multi-character type parameters**
- `freshen_type_params` (traits.rs:299) only freshens single-uppercase-letter constructors (`A`-`Z`)
- Multi-character type params (e.g., `Key`, `Value`) may not be freshened correctly
- **Handling:** Test with multi-character params during Phase 1 implementation. If broken, audit `freshen_type_params` to use impl's declared type param list instead of naming convention.

**Gap 2: Where-clause constraint checking for dot-syntax calls**
- `infer_call` (infer.rs:2713-2749) only checks constraints when callee is `NameRef`
- Method calls have `FieldAccess` callee
- **Handling:** Extract method name from FieldAccess, look up constraints, apply existing checking logic. Should be straightforward extension.

**Gap 3: LSP go-to-definition for methods**
- snow-lsp/src/definition.rs does not handle FieldAccess
- Clicking "go to definition" on method calls will not work initially
- **Handling:** Defer to post-MVP. Add `method_resolutions` map to TypeckResult for LSP queries. Not a blocker for language feature.

## Sources

### Primary (HIGH confidence)
- Direct codebase analysis: 66,521 lines of Snow compiler (parser, typeck, codegen, MIR, LSP)
- `snow-typeck/src/infer.rs` — infer_field_access (line 3879), resolution priority chain
- `snow-typeck/src/traits.rs` — TraitRegistry, find_method_traits (line 246), structural unification
- `snow-codegen/src/mir/lower.rs` — trait dispatch (lines 3527-3599), lower_call_expr (line 3362)
- `snow-parser/tests/snapshots/parser_tests__mixed_postfix.snap` — confirms `a.b(c)` parse tree structure

### Secondary (MEDIUM-HIGH confidence)
- [Rust Method Call Expressions Reference](https://doc.rust-lang.org/reference/expressions/method-call-expr.html) — official method resolution rules
- [Rust Compiler Dev Guide: Method Lookup](https://rustc-dev-guide.rust-lang.org/method-lookup.html) — probe phase internals
- [Swift Method Dispatch Mechanisms](https://nilcoalescing.com/blog/MethodDispatchMechanismsInSwift/) — static dispatch for structs
- [UFCS in Rust](https://doc.rust-lang.org/book/first-edition/ufcs.html) — fully qualified syntax
- [Uniform Function Call Syntax (Wikipedia)](https://en.wikipedia.org/wiki/Uniform_Function_Call_Syntax) — UFCS concept overview

### Tertiary (MEDIUM confidence)
- [C++ P3021 UFCS Proposal](https://open-std.org/JTC1/SC22/WG21/docs/papers/2023/p3021r0.pdf) — ambiguity problems with UFCS (why NOT to build it)
- [Swift Compiler Cocaine: Method Dispatch Deep Dive](https://blog.jacobstechtavern.com/p/compiler-cocaine-the-swift-method) — compiler-level dispatch analysis

---
*Research completed: 2026-02-08*
*Ready for roadmap: yes*
