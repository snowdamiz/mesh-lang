# Stack Research: Method Dot-Syntax

**Domain:** Compiler feature addition -- method call resolution via dot syntax
**Researched:** 2026-02-08
**Confidence:** HIGH (based on direct codebase analysis + compiler design literature)

## Executive Summary

Method dot-syntax (`value.method(args)`) requires NO new external dependencies. The entire feature is implemented through changes to existing compiler passes. Snow already has all the infrastructure needed: the parser handles `expr.ident` (FIELD_ACCESS at binding power 25), the type checker has `TraitRegistry` with `resolve_trait_method` and `find_method_traits`, the MIR lowerer already rewrites bare `method(receiver, args)` calls to `Trait__Method__Type(receiver, args)`, and codegen handles `MirExpr::Call` with mangled names. The work is a **wiring exercise**, not a new-capability exercise.

## What Exists Today (DO NOT CHANGE)

These are existing capabilities that method dot-syntax builds on, not replaces.

### Parser (snow-parser)

| Component | Current State | Relevance |
|-----------|--------------|-----------|
| `FIELD_ACCESS` CST node | `expr.ident` parsed at postfix BP 25 | Reuse this parse; the disambiguation happens later |
| `CALL_EXPR` CST node | `expr(args)` parsed at postfix BP 25 | `expr.method(args)` currently parses as `CALL_EXPR(FIELD_ACCESS(expr, method), args)` |
| `PIPE_EXPR` CST node | `expr \|> func` at BP 3/4 | Do NOT change; pipe remains a separate mechanism |
| `PARAM` with `SELF_KW` | `self` keyword accepted in param lists | Already parsed for impl methods |

### Type Checker (snow-typeck)

| Component | Current State | Relevance |
|-----------|--------------|-----------|
| `TraitRegistry.resolve_trait_method(name, ty)` | Searches all impls for method matching type | Core lookup for method resolution |
| `TraitRegistry.find_method_traits(name, ty)` | Returns all traits providing method for type | Ambiguity detection |
| `TraitRegistry.find_impl(trait, ty)` | Structural unification-based impl lookup | Already handles generics via freshening |
| `infer_field_access()` | Resolves struct fields, stdlib modules, service modules | Must be extended to try method resolution |
| `infer_call()` | Resolves function calls with where-clause checking | Will receive method calls after desugaring |
| `ImplMethodSig.has_self` | Tracks whether method takes self | Used to distinguish methods from static functions |

### MIR Lowerer (snow-codegen/mir)

| Component | Current State | Relevance |
|-----------|--------------|-----------|
| `lower_call_expr()` | Rewrites bare `method(receiver)` to `Trait__Method__Type(receiver)` | Already does trait method dispatch |
| `lower_field_access()` | Handles struct fields, stdlib modules, service modules | Must be extended or a new path added |
| `mir_type_to_ty()` | Converts MirType back to Ty for TraitRegistry lookups | Already exists for dispatch |
| `mir_type_to_impl_name()` | Extracts type name for mangled names | Already exists |

### Codegen (snow-codegen/codegen)

| Component | Current State | Relevance |
|-----------|--------------|-----------|
| `codegen_expr()` for `MirExpr::Call` | Emits LLVM call instruction | No change needed -- method calls lower to regular `MirExpr::Call` |
| `codegen_field_access()` | Emits GEP for struct fields | No change needed -- field access remains unchanged |

## Recommended Stack Changes

### Change 1: New AST Node -- METHOD_CALL (Parser Layer)

**What:** Add `METHOD_CALL` SyntaxKind and `MethodCallExpr` AST node.
**Why:** The parser currently produces `CALL_EXPR(FIELD_ACCESS(base, method), args)` for `value.method(args)`. However, this structure is ambiguous -- the type checker cannot distinguish "call a function returned by field access" from "call a method on the receiver." A dedicated CST node eliminates this ambiguity at parse time.

| Technology | Change | Purpose | Why |
|------------|--------|---------|-----|
| `SyntaxKind` enum | Add `METHOD_CALL` variant | Distinct CST node for `expr.method(args)` | Disambiguates from `(expr.field)(args)` at parse time |
| `expressions.rs` postfix loop | Detect `DOT IDENT L_PAREN` sequence | Emit `METHOD_CALL` instead of `CALL_EXPR(FIELD_ACCESS(...))` | Parser is the cheapest place to detect this pattern |
| `ast/expr.rs` | Add `MethodCallExpr` typed AST wrapper | Clean API: `.receiver()`, `.method_name()`, `.arg_list()` | Follows existing pattern (CallExpr, FieldAccess) |

**Parser detection logic:** In the postfix loop of `expr_bp`, after matching `DOT`:
1. Check if `DOT IDENT L_PAREN` (method call with args)
2. If yes: open before lhs, advance DOT, advance IDENT, parse arg list, close as `METHOD_CALL`
3. If no: fall through to existing FIELD_ACCESS handling

This is a 2-token lookahead (DOT + peek at IDENT then L_PAREN), which is trivial in the existing Pratt parser.

### Alternative Considered: No New CST Node (Desugar Later)

**Approach:** Keep `CALL_EXPR(FIELD_ACCESS(...))` and detect the pattern in the type checker.
**Why rejected:** The type checker would need to speculatively try "is this a field access returning a callable?" AND "is this a method call?" for every `CALL_EXPR` whose callee is a `FIELD_ACCESS`. This creates fragile disambiguation logic in the wrong layer. The parser already sees the syntactic structure and should encode it.

### Change 2: Type Checker Method Resolution (Type Checker Layer)

**What:** Add `infer_method_call()` function in `snow-typeck/src/infer.rs`.
**Why:** This is the core semantic logic. Given `receiver.method(args)`, resolve the receiver type, look up the method in the TraitRegistry, and type-check the call.

| Component | Change | Purpose | Why |
|-----------|--------|---------|-----|
| `infer.rs` match on `Expr` | Add `Expr::MethodCallExpr(mc)` arm | Route to new inference function | Follows existing pattern |
| `infer_method_call()` | New function (~50 lines) | Core method resolution algorithm | Heart of the feature |
| Error types | Add `NoSuchMethod { ty, method_name, span }` | Good diagnostics for failed resolution | Users need clear errors |
| Error types | Add `AmbiguousMethod { ty, method_name, candidates, span }` | Ambiguity detection | Multiple traits with same method name |

**Method resolution algorithm (simplified vs Rust):**

Snow does NOT need Rust's autoderef chain because Snow has no references, no `Deref` trait, and no auto-borrowing. The algorithm is:

```
infer_method_call(receiver_expr, method_name, args):
  1. receiver_ty = infer_expr(receiver_expr)
  2. resolved_ty = resolve(receiver_ty)  // follow unification links
  3. matching_traits = trait_registry.find_method_traits(method_name, resolved_ty)
  4. if matching_traits.is_empty():
       emit NoSuchMethod error
  5. if matching_traits.len() > 1:
       emit AmbiguousMethod error (list trait names)
  6. trait_name = matching_traits[0]
  7. impl_def = trait_registry.find_impl(trait_name, resolved_ty)
  8. method_sig = impl_def.methods[method_name]
  9. // Type check: unify (receiver_ty, arg_types...) against method signature
  10. // Return the method's return type
```

This is dramatically simpler than Rust's method resolution because:
- No autoderef chain (no `&T`, `&mut T`, `**T` candidates)
- No inherent methods (Snow uses trait impls for everything)
- No auto-borrowing (Snow is value-typed / GC-managed)
- Static dispatch only (no vtables)

### Change 3: MIR Lowering (MIR Layer)

**What:** Add `lower_method_call()` in `snow-codegen/src/mir/lower.rs`.
**Why:** Desugar `receiver.method(args)` into `Trait__Method__Type(receiver, args)` -- a regular `MirExpr::Call`.

| Component | Change | Purpose | Why |
|-----------|--------|---------|-----|
| `lower_expr()` match | Add `Expr::MethodCallExpr(mc)` arm | Route to new lowering function | Follows existing pattern |
| `lower_method_call()` | New function (~30 lines) | Desugar to mangled call | Reuse existing mangling logic |

**The desugaring is:**

```
lower_method_call(receiver, method_name, args):
  1. lower_receiver = lower_expr(receiver)
  2. lower_args = [lower_receiver] ++ map(lower_expr, args)
  3. receiver_mir_ty = lower_receiver.ty()
  4. ty_for_lookup = mir_type_to_ty(receiver_mir_ty)
  5. matching_traits = trait_registry.find_method_traits(method_name, ty_for_lookup)
  6. trait_name = matching_traits[0]  // already validated by typeck
  7. type_name = mir_type_to_impl_name(receiver_mir_ty)
  8. mangled = format!("{}__{}__{}", trait_name, method_name, type_name)
  9. // Handle primitive builtins (same as existing logic in lower_call_expr)
  10. return MirExpr::Call { func: Var(mangled), args: lower_args, ty: result_ty }
```

This reuses the EXACT same mangling and dispatch logic already in `lower_call_expr()` for bare `method(receiver)` calls. The key insight: **both calling conventions produce identical MIR**. The only difference is where the receiver comes from (first arg vs dot-syntax receiver).

### Change 4: No Codegen Changes Required

**Why:** Method calls desugar to `MirExpr::Call` in MIR, which codegen already handles. The mangled function names (`Trait__Method__Type`) are already emitted as LLVM functions. No new LLVM instructions, no new calling convention, no new MirExpr variant needed.

### Change 5: Formatter and LSP Updates

| Component | Change | Purpose | Why |
|-----------|--------|---------|-----|
| `snow-fmt` walker/ir | Handle `METHOD_CALL` SyntaxKind | Format `value.method(args)` correctly | Formatter must know about new node |
| `snow-lsp` definition.rs | Handle `METHOD_CALL` for go-to-definition | Navigate to impl method from call site | LSP should resolve method calls |

## What NOT to Change

| Component | Why Leave Alone |
|-----------|----------------|
| `FIELD_ACCESS` parsing/inference | Field access (`value.field`) remains exactly as-is. Only `value.field(args)` gets the new treatment. |
| `PIPE_EXPR` | Pipe (`\|>`) is a separate mechanism. Method syntax complements, does not replace, pipes. |
| `MirExpr` enum | No new variant needed. Method calls desugar to existing `MirExpr::Call`. |
| `codegen_expr()` | No changes. MIR already produces `Call` nodes that codegen handles. |
| `monomorphize()` | No changes. Monomorphization already handles `Call` nodes with mangled names. |
| Existing `lower_call_expr()` trait dispatch | Keep the existing bare `method(receiver)` calling convention working. Both styles should work. |
| `TraitRegistry` | No structural changes. Existing `find_method_traits` and `resolve_trait_method` are sufficient. |

## Stack Patterns by Feature Variant

### If only supporting trait methods via dot syntax (RECOMMENDED):

The minimal change set above is sufficient. `value.method(args)` resolves through TraitRegistry.

### If also supporting inherent methods (methods without a trait):

Would require a new registry for inherent impls (methods defined in `impl Type do ... end` without a trait). This is a future extension, NOT needed for the initial milestone. Snow currently requires all methods to belong to a trait. Inherent methods would add:
- `InherentImplRegistry` in typeck
- Priority ordering: inherent methods checked before trait methods (like Rust)
- Additional MIR lowering path

**Recommendation: Do NOT add inherent methods in this milestone.** Keep the scope to trait-dispatched method dot-syntax. Inherent methods can be added later without breaking changes.

### If also supporting UFCS (Uniform Function Call Syntax):

Would allow `free_function(receiver, args)` to also be called as `receiver.free_function(args)`. This is a separate, larger feature. Snow's pipe operator (`|>`) already provides this ergonomic. Do NOT conflate with method dot-syntax.

## Architecture of Method Resolution in Other Languages

### Rust (most complex)

Rust's method resolution involves autoderef chains, auto-borrowing (`&T`, `&mut T`), unsized coercion, inherent methods before trait methods, and probe phases. This complexity exists because Rust has references, ownership, and the `Deref` trait. Snow needs none of this.

**What to take from Rust:** The priority ordering concept (inherent before extension) is good design for future extensibility. The "first match wins" approach is simple and predictable.

### Swift (medium complexity)

Swift uses static dispatch for struct methods (direct function call), v-table dispatch for class methods, and witness tables for protocol methods. Swift's struct methods are essentially what Snow's trait methods would look like: the compiler embeds the function address directly.

**What to take from Swift:** Static dispatch via direct call is exactly what Snow already does with `Trait__Method__Type` mangling. Swift validates that this approach scales to real-world usage.

### Kotlin (medium complexity)

Kotlin resolves method calls on the declared type. Extension functions (which are syntactic sugar, not true methods) are resolved statically based on the declared type, not the runtime type.

**What to take from Kotlin:** Snow's approach of desugaring `value.method()` to `Trait__Method__Type(value)` is equivalent to Kotlin's extension function dispatch. It is predictable, fast, and well-understood.

### Summary: Snow's Approach is Correct

Snow's approach -- static dispatch via name mangling (`Trait__Method__Type`) with the receiver as the first argument -- is the standard approach for statically-typed languages without inheritance. It is used by Rust (for monomorphized calls), Swift (for struct methods), and Kotlin (for extension functions). No novel research is needed.

## Critical Integration Points

### 1. Parser: FIELD_ACCESS vs METHOD_CALL Disambiguation

The parser must distinguish `value.field` from `value.method(args)`. The key is the `L_PAREN` after the identifier:

```
value.field       -> FIELD_ACCESS(value, "field")
value.method()    -> METHOD_CALL(value, "method", ARG_LIST())
value.method(a)   -> METHOD_CALL(value, "method", ARG_LIST(a))
```

The postfix loop in `expr_bp` already handles this pattern -- it checks for `DOT` then `L_PAREN` in sequence. The modification is to check for `DOT IDENT L_PAREN` and emit METHOD_CALL instead of letting it fall through to `FIELD_ACCESS` followed by `CALL_EXPR`.

### 2. Type Checker: Separate Paths for Fields vs Methods

With the new `METHOD_CALL` CST node, field access and method calls are distinguished at parse time. `infer_field_access` does NOT need modification. The new `infer_method_call` handles method resolution independently.

### 3. MIR Lowerer: Reuse Existing Dispatch Logic

The `lower_call_expr()` function (lines 3527-3600) already contains the full trait method dispatch logic:
- Look up `find_method_traits(name, ty)`
- Mangle to `Trait__Method__Type`
- Handle primitive builtin short-circuits
- Fallback for monomorphized generic types

`lower_method_call()` should extract this logic into a shared helper function, then call it from both `lower_call_expr()` and `lower_method_call()`. This avoids duplicating the ~70 lines of dispatch logic.

### 4. Monomorphization: Already Handled

The `collect_function_refs()` function in `mono.rs` walks `MirExpr::Call` nodes and adds referenced function names to the reachable set. Since method calls produce `MirExpr::Call`, monomorphization works automatically.

## Version Compatibility

| Package | Version | Compatibility Notes |
|---------|---------|---------------------|
| rowan | Current | No changes to CST infrastructure needed; just add new SyntaxKind variant |
| inkwell | 0.8 | No changes; method calls produce same LLVM IR as regular calls |
| ariadne | Current | New error types need new diagnostic renderings |

No dependency additions, no version bumps, no new crates.

## Complexity Assessment

| Layer | Estimated Complexity | Lines of Code | Risk |
|-------|---------------------|---------------|------|
| Parser | Low | ~20 lines | Mechanical; well-understood pattern |
| AST | Low | ~30 lines | Add MethodCallExpr following existing patterns |
| Type Checker | Medium | ~60 lines | Method resolution logic; error handling |
| MIR Lowering | Low | ~40 lines | Reuse existing dispatch; extract helper |
| Codegen | None | 0 lines | No changes needed |
| Formatter | Low | ~10 lines | Handle new SyntaxKind |
| Tests | Medium | ~200 lines | Parser, typeck, MIR, e2e tests |

**Total estimated effort:** ~360 lines of new/modified code + ~200 lines of tests.

## Sources

- [Rust Method Call Expressions](https://doc.rust-lang.org/reference/expressions/method-call-expr.html) -- Official Rust Reference on method resolution algorithm (HIGH confidence)
- [Rust Method Lookup Internals](https://rustc-dev-guide.rust-lang.org/method-lookup.html) -- rustc-dev-guide probe phase documentation (HIGH confidence)
- [Swift Method Dispatch Mechanisms](https://nilcoalescing.com/blog/MethodDispatchMechanismsInSwift/) -- static vs dynamic dispatch in Swift (MEDIUM confidence)
- [Swift Method Dispatch Deep Dive](https://blog.jacobstechtavern.com/p/compiler-cocaine-the-swift-method) -- compiler-level dispatch analysis (MEDIUM confidence)
- [UFCS in Rust](https://doc.rust-lang.org/book/first-edition/ufcs.html) -- Rust's fully qualified syntax (HIGH confidence)
- [Uniform Function Call Syntax](https://en.wikipedia.org/wiki/Uniform_Function_Call_Syntax) -- UFCS concept overview (MEDIUM confidence)
- Direct codebase analysis of Snow compiler (66,521 lines) -- `snow-parser`, `snow-typeck`, `snow-codegen` (HIGH confidence)

---
*Stack research for: Snow compiler method dot-syntax feature*
*Researched: 2026-02-08*
