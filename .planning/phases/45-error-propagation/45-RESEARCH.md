# Phase 45: Error Propagation - Research

**Researched:** 2026-02-09
**Domain:** Postfix `?` operator for Result/Option early-return desugaring in a compiled language
**Confidence:** HIGH

## Summary

Phase 45 adds the `?` (try/propagation) operator to Snow, enabling concise error propagation on `Result<T,E>` and `Option<T>` values. The operator `expr?` desugars to a pattern match: on `Ok(v)`/`Some(v)` it unwraps to `v`, on `Err(e)`/`None` it early-returns from the enclosing function. The type checker must validate that the enclosing function's return type is compatible (Result or Option respectively), and emit a clear diagnostic when it is not.

This is a purely compiler-side feature requiring changes across four crates (parser, typeck, MIR lowering, codegen) but **zero new runtime functions and zero new Rust dependencies**. The `?` token (`TokenKind::Question` / `SyntaxKind::QUESTION`) already exists in the lexer and parser -- it is currently used only for type annotation sugar (`Int?` -> `Option<Int>`). The key challenge is disambiguating `?` in expression position (postfix try operator) from `?` in type position (Option sugar), and threading the enclosing function's return type through the type checker so `?` can validate compatibility.

The existing `MirExpr::Return` and `MirExpr::Match` nodes are sufficient to represent the desugared form. No new MIR node type is strictly needed -- `expr?` can be lowered to `match expr do Ok(v) -> v; Err(e) -> return Err(e) end` (and similarly for Option). However, a dedicated `MirExpr::TryOp` variant makes the lowering cleaner and allows the codegen to emit more targeted code.

**Primary recommendation:** Add `TRY_EXPR` as a new postfix expression in the parser, add a `TryIncompatibleReturn` error to typeck, desugar to `Match + Return(ConstructVariant)` in MIR lowering, and reuse existing codegen for Match/Return unchanged.

## Standard Stack

### Core (existing crates, no new dependencies)

| Component | Location | Purpose | What Changes |
|-----------|----------|---------|--------------|
| snow-lexer | `crates/snow-lexer/src/lib.rs` | Tokenization | NONE -- `?` already tokenized as `TokenKind::Question` |
| snow-parser | `crates/snow-parser/src/` | CST construction | Add `TRY_EXPR` node kind + postfix parsing |
| snow-typeck | `crates/snow-typeck/src/` | Type inference | Add `?` type validation against fn return type |
| snow-codegen (MIR) | `crates/snow-codegen/src/mir/` | MIR lowering | Desugar `TryExpr` to `Match + Return` |
| snow-codegen (LLVM) | `crates/snow-codegen/src/codegen/` | LLVM IR generation | No changes needed -- uses existing Match/Return codegen |

### Supporting

| Component | Location | Purpose | What Changes |
|-----------|----------|---------|--------------|
| snow-fmt | `crates/snow-fmt/src/` | Formatter | Handle `TRY_EXPR` in walker |
| snow-lsp | `crates/snow-lsp/src/` | Language server | Potentially update analysis for new node |

## Architecture Patterns

### Pattern 1: Postfix Operator Parsing (Pratt Parser Extension)

**What:** The `?` operator is a postfix (suffix) operator with the highest binding power (same level as field access, call, indexing -- POSTFIX_BP = 25).

**When to use:** After any expression atom, check if `?` follows.

**How it works in the existing parser:**

The Pratt parser in `expressions.rs` already handles postfix operators in a loop at lines 87-133. The pattern is:
1. Parse LHS atom
2. Loop checking for postfix tokens: `L_BRACE` (struct literal), `L_PAREN` (call), `DOT` (field access), `L_BRACKET` (index)
3. Each postfix wraps the LHS in a new CST node

The `?` operator follows this exact pattern: check for `QUESTION` after any expression, wrap in `TRY_EXPR`.

**Critical disambiguation:** The `?` token is ALREADY used in type annotations (`Int?` -> `Option<Int>`). However, type annotations only appear after `::` in parameter/let positions, parsed by `parse_type` in `items.rs`. Expression parsing in `expressions.rs` is a completely separate code path. There is **no ambiguity** because:
- In type position: `parse_type` calls `apply_type_sugar` which consumes `?` after type names
- In expression position: `expr_bp` postfix loop sees `?` after expression atoms

These are different parser functions, different call sites, no overlap.

**Example (parser):**
```
// In expressions.rs, expr_bp function, postfix loop:
// After existing postfix checks (struct lit, call, field, index)...

// -- Postfix: try operator --
if current == SyntaxKind::QUESTION && POSTFIX_BP >= min_bp {
    let m = p.open_before(lhs);
    p.advance(); // consume ?
    lhs = p.close(m, SyntaxKind::TRY_EXPR);
    continue;
}
```

### Pattern 2: Type Checking with Enclosing Function Return Type

**What:** The `?` operator must validate that the enclosing function returns a compatible type (Result/Option).

**Current state:** The type checker (`infer.rs`) does NOT currently track the enclosing function's return type in `InferCtx`. Return expressions (`infer_return` at line 4389) simply infer the value's type and return `Ty::Never` -- they do NOT validate against the declared return type.

**Recommended approach:** Add a field to `InferCtx`:
```rust
/// Stack of enclosing function return types.
/// Pushed when entering a function body, popped when leaving.
/// Used by ? operator to validate compatibility.
pub fn_return_type_stack: Vec<Option<Ty>>,
```

The `?` operator inference then:
1. Infer the operand type
2. Check if operand is `Result<T, E>` or `Option<T>` (by examining `Ty::App` with base name "Result" or "Option")
3. Check if the enclosing function return type (top of stack) is compatible
4. For `Result<T, E>?`: operand must be `Result<T, E>`, fn must return `Result<_, E>` (same E), expression type is `T`
5. For `Option<T>?`: operand must be `Option<T>`, fn must return `Option<_>`, expression type is `T`
6. If no compatible return type: emit `TryIncompatibleReturn` error

**Why a stack:** Closures can nest inside functions. When entering a closure body, push the closure's return type. The `?` inside a closure propagates to the closure's return, not the outer function.

### Pattern 3: MIR Desugaring (Match + Return + ConstructVariant)

**What:** Lower `expr?` into a match expression with early return.

**For `Result<T, E>`:**
```
expr?
```
becomes:
```
case expr do
  Ok(__try_val) -> __try_val
  Err(__try_err) -> return Err(__try_err)
end
```

In MIR terms:
```rust
MirExpr::Match {
    scrutinee: Box::new(lowered_expr),
    arms: vec![
        MirMatchArm {
            pattern: MirPattern::Constructor {
                type_name: "Result_T_E",  // monomorphized name
                variant: "Ok",
                fields: vec![MirPattern::Var("__try_val", inner_ty)],
                bindings: vec![("__try_val", inner_ty)],
            },
            guard: None,
            body: MirExpr::Var("__try_val", inner_ty),
        },
        MirMatchArm {
            pattern: MirPattern::Constructor {
                type_name: "Result_T_E",
                variant: "Err",
                fields: vec![MirPattern::Var("__try_err", err_ty)],
                bindings: vec![("__try_err", err_ty)],
            },
            guard: None,
            body: MirExpr::Return(Box::new(MirExpr::ConstructVariant {
                type_name: "Result_T2_E",  // fn return type's monomorphized name
                variant: "Err",
                fields: vec![MirExpr::Var("__try_err", err_ty)],
                ty: fn_return_mir_type,
            })),
        },
    ],
    ty: inner_ty,  // T -- the unwrapped success type
}
```

**For `Option<T>`:**
```
expr?
```
becomes:
```
case expr do
  Some(__try_val) -> __try_val
  None -> return None
end
```

### Pattern 4: Type Extraction from Result/Option

**What:** Given a `Ty`, extract the inner type parameters.

**Implementation:** Add helper functions:
```rust
/// Extract T from Option<T>, returns None if not an Option type.
fn extract_option_inner(ty: &Ty) -> Option<Ty> {
    match ty {
        Ty::App(con, args) if matches!(con.as_ref(), Ty::Con(c) if c.name == "Option") => {
            args.first().cloned()
        }
        _ => None,
    }
}

/// Extract (T, E) from Result<T, E>, returns None if not a Result type.
fn extract_result_types(ty: &Ty) -> Option<(Ty, Ty)> {
    match ty {
        Ty::App(con, args) if matches!(con.as_ref(), Ty::Con(c) if c.name == "Result") => {
            if args.len() >= 2 {
                Some((args[0].clone(), args[1].clone()))
            } else {
                None
            }
        }
        _ => None,
    }
}
```

### Anti-Patterns to Avoid

- **Don't add a new MIR intrinsic for ?:** The existing `MirExpr::Match` + `MirExpr::Return` + `MirExpr::ConstructVariant` already handle the desugared form perfectly. Adding a `MirExpr::Try` would require changes to both MIR lowering AND codegen. Instead, desugar entirely during MIR lowering.

- **Don't try to make ? work in closures without tracking return types:** A `?` inside `fn(x) -> x? end` must early-return from the closure, not the outer function. The fn_return_type_stack handles this naturally.

- **Don't allow ? on non-Result/non-Option types:** Unlike Rust's `Try` trait, Snow has no general try mechanism. Only `Result<T,E>` and `Option<T>` are supported. Keep it simple.

- **Don't modify the lexer:** The `?` token already exists as `TokenKind::Question`. No lexer changes needed.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Sum type construction in MIR | Custom try-specific codegen | `MirExpr::ConstructVariant` | Already handles Option/Result variant construction with proper monomorphized type names |
| Early return | New return mechanism | `MirExpr::Return(Box::new(...))` | Already generates correct LLVM `ret` instruction |
| Pattern matching on Result/Option | Custom tag-check IR | `MirExpr::Match` with `MirPattern::Constructor` | Full pattern match compilation already handles sum types correctly |
| Type name mangling | Inline string formatting | `mangle_type_name()` from `mir/types.rs` | Already produces correct `Option_Int`, `Result_Int_String` etc. |

**Key insight:** The entire `?` operator can be implemented by composing existing MIR primitives. The only truly new code is in the parser (postfix recognition), typeck (validation), and MIR lowering (desugaring). Codegen needs zero changes.

## Common Pitfalls

### Pitfall 1: Forgetting to Track Function Return Type in Closures

**What goes wrong:** If the fn_return_type is stored as a single field instead of a stack, closures break. `fn(x) -> x? end` inside a function returning `Int` would incorrectly validate against `Int` instead of the closure's inferred return type.

**Why it happens:** The type checker processes closures by saving/restoring context (like `enter_closure`/`exit_closure` for loop_depth). Same pattern needed for return types.

**How to avoid:** Use a `Vec<Option<Ty>>` stack. Push when entering any function/closure body. Pop when leaving. The `?` operator reads the top of the stack.

**Warning signs:** Tests with `?` inside closures fail or give wrong error messages.

### Pitfall 2: Monomorphized Type Name Mismatch

**What goes wrong:** The `ConstructVariant` in the Err arm uses the wrong type name. For example, the expression has type `Result<Int, String>` (mangled: `Result_Int_String`), but the function returns `Result<Bool, String>` (mangled: `Result_Bool_String`). The Err early-return must construct `Result_Bool_String.Err`, not `Result_Int_String.Err`.

**Why it happens:** The `?` desugaring must use the *function's return type* for the early-return variant construction, not the *operand's type*.

**How to avoid:** In MIR lowering, resolve the function return type to get the correct monomorphized sum type name for the Err/None construction.

**Warning signs:** LLVM type mismatch errors, "unknown sum type" panics, or incorrect variant tag values at runtime.

### Pitfall 3: Type Variable Resolution Before ? Validation

**What goes wrong:** When the operand or function return type contains unresolved type variables at the point `?` is checked, the validation cannot determine if the types are compatible.

**Why it happens:** HM inference may not have resolved all type variables when `?` is first encountered. The operand might be `Result<?3, ?4>` at that point.

**How to avoid:** Defer full validation. During `infer_expr`, unify the operand with a fresh `Result<T, E>` and unify E with the function return's error type. Let unification handle the constraint propagation. The type checker should:
1. Create fresh vars: `let t = ctx.fresh_var(); let e = ctx.fresh_var();`
2. Unify operand with `Result<t, e>` or `Option<t>`
3. Unify function return type with `Result<_, e>` or `Option<_>`
4. Return `t` as the expression type

**Warning signs:** "type mismatch" errors on valid code where types should be inferrable.

### Pitfall 4: Missing Sum Type Definition for Monomorphized Result/Option

**What goes wrong:** The MIR lowering generates a `ConstructVariant` with type_name `Result_Bool_String` but this monomorphized sum type was never registered in the MIR module.

**Why it happens:** Monomorphized sum type defs are generated on demand during lowering. The `?` desugaring creates new monomorphized instances that might not have been seen before.

**How to avoid:** The existing monomorphization infrastructure in `mono.rs` and `lower.rs` already handles this -- `ConstructVariant` triggers sum type def generation. Verify by checking that `ensure_sum_type_def` (or equivalent) is called for the function return type's monomorphized name.

**Warning signs:** "Unknown sum type" errors during codegen.

### Pitfall 5: ? After Method Calls in Expression Chains

**What goes wrong:** `file.read()?.trim()` -- the `?` must bind tighter than method calls or at the same level.

**Why it happens:** If `?` has lower precedence than `.`, the parser would try to parse `file.read()?.trim()` as `file.read() ? (.trim())` which makes no sense.

**How to avoid:** Give `?` the same binding power as other postfix operators (POSTFIX_BP = 25). Parse it in the same postfix loop alongside `.`, `()`, `[]`.

**Warning signs:** Parse errors on `expr?.method()` chains.

## Code Examples

### Example 1: User-facing Snow code with ?

```snow
fn read_config(path :: String) :: Result<String, String> do
  let contents = File.read(path)?   # early-returns Err if read fails
  Ok(contents)
end

fn first_element(list :: List<Int>) :: Option<Int> do
  let head = List.head(list)?   # early-returns None if empty
  Some(head)
end

fn main() do
  case read_config("config.txt") do
    Ok(data) -> println(data)
    Err(msg) -> println("Error: ${msg}")
  end
end
```

### Example 2: ? in nested context (closure vs function)

```snow
fn process(items :: List<String>) :: Result<List<Int>, String> do
  # ? here returns from process(), not the closure
  let config = read_config("settings.txt")?

  # This ? would be inside the for-in body (no early return from outer fn)
  # It would need the for-in body to return Result, which is different
  Ok([1, 2, 3])
end
```

### Example 3: Error diagnostic when ? is used incorrectly

```snow
fn add(a :: Int, b :: Int) :: Int do
  let result = some_operation()?   # ERROR: ? requires return type Result or Option
  result + 1
end
```

Expected error:
```
error[E0036]: `?` operator requires function to return `Result` or `Option`
  --> main.snow:2:35
  |
2 |   let result = some_operation()?
  |                                ^ cannot use `?` here
  |
  = help: the enclosing function returns `Int`, but `?` requires `Result<_, _>` or `Option<_>`
  = help: consider wrapping the return type: `Result<Int, String>`
```

## State of the Art

| Aspect | Rust Approach | Snow Approach | Rationale |
|--------|--------------|---------------|-----------|
| `?` on Result | Desugar to Try trait impl | Desugar to Match + Return | Snow has no general Try trait; keep it simple |
| `?` on Option | Desugar to Try trait impl | Desugar to Match + Return | Same -- no From trait for error conversion |
| Error type conversion | `From<E1> into E2` via From trait | Must match exactly | Snow doesn't have From trait; add later if needed |
| `?` in main | Requires main to return Result | Only works if main returns Result/Option | Same restriction |

**Key difference from Rust:** Snow does NOT support automatic error type conversion via `From`. The error type `E` in the operand's `Result<T, E>` must unify with the error type in the function's return `Result<_, E>`. This is simpler and avoids the need for a `From` trait, but means users must have matching error types. This is a reasonable v1 limitation.

## Open Questions

1. **Should `?` work on `Option<T>` inside a function returning `Result<T, E>`?**
   - What we know: Rust supports this via `From<NoneError>`. Elixir's `with` is different.
   - What's unclear: Whether this conversion adds enough value for v1.9.
   - Recommendation: **No** for v1.9. Keep it strict: `Option?` only in `Option`-returning functions, `Result?` only in `Result`-returning functions. Simpler to implement, easier to understand, can relax later.

2. **Should `?` propagate across module boundaries?**
   - What we know: Since `?` desugars entirely at the call site, and Result/Option are built-in types visible everywhere, this should work automatically.
   - What's unclear: Whether monomorphized sum type defs from imported modules need special handling.
   - Recommendation: Should work automatically. The MIR merge codegen (Phase 41) already handles cross-module sum types. Verify with a cross-module test.

3. **Should `?` work in actor message handlers?**
   - What we know: Actor handlers have specific return type semantics (state transitions).
   - What's unclear: Whether `?` inside a cast/call handler should return from the handler or the actor.
   - Recommendation: **Yes**, `?` works in any function/closure body. It always early-returns from the immediately enclosing function scope. If the handler returns `Result<State, Error>`, `?` works naturally.

## Sources

### Primary (HIGH confidence)
- **Codebase analysis** -- Direct reading of all relevant source files:
  - `crates/snow-lexer/src/lib.rs` -- `?` tokenized as `TokenKind::Question` (line 123, 634)
  - `crates/snow-parser/src/syntax_kind.rs` -- `QUESTION` SyntaxKind exists (line 97)
  - `crates/snow-parser/src/parser/expressions.rs` -- Pratt parser postfix loop (lines 87-133, POSTFIX_BP=25)
  - `crates/snow-parser/src/parser/items.rs` -- `?` in type annotations (lines 464-469)
  - `crates/snow-parser/src/ast/expr.rs` -- Expr enum with all variants (lines 16-46)
  - `crates/snow-typeck/src/infer.rs` -- Result/Option registration (lines 751-830), return inference (lines 4389-4399)
  - `crates/snow-typeck/src/error.rs` -- TypeError enum (lines 57-282)
  - `crates/snow-typeck/src/diagnostics.rs` -- Error formatting pattern (E0032/E0033 as model)
  - `crates/snow-typeck/src/unify.rs` -- InferCtx fields (lines 18-43)
  - `crates/snow-codegen/src/mir/mod.rs` -- MirExpr enum (lines 143-377), Match/Return/ConstructVariant
  - `crates/snow-codegen/src/mir/lower.rs` -- Lowerer struct (lines 158-212), expr dispatch (lines 3190-3220)
  - `crates/snow-codegen/src/mir/types.rs` -- mangle_type_name (lines 141-148), resolve_app for Option/Result (lines 93-136)
  - `crates/snow-codegen/src/codegen/expr.rs` -- codegen_return (lines 1534-1544), codegen_construct_variant (lines 1360-1426)
  - `crates/snowc/tests/e2e.rs` -- Test harness pattern (compile_and_run, compile_expect_error)

### Secondary (MEDIUM confidence)
- **Prior phase decisions** (from project state): "Result<T,E> and Option<T> fully implemented; ? operator desugars to match+return in MIR"
- **Rust reference** (general knowledge): Rust's `?` operator design as proven prior art for the desugaring pattern

## Metadata

**Confidence breakdown:**
- Parser changes: HIGH -- `?` token exists, postfix loop is well-understood, disambiguation is clear
- Typeck changes: HIGH -- pattern follows existing `BreakOutsideLoop` model, InferCtx stack pattern proven by `loop_depth`
- MIR desugaring: HIGH -- all component MIR nodes (Match, Return, ConstructVariant) already exist and are well-tested
- Codegen changes: HIGH -- no changes needed, desugaring reuses existing codegen paths
- Pitfalls: HIGH -- identified from direct code analysis, cross-referenced with monomorphization and closure behavior

**Research date:** 2026-02-09
**Valid until:** 2026-03-11 (30 days -- stable domain, all findings from current codebase)
