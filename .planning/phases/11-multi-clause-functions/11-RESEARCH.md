# Phase 11: Multi-Clause Functions - Research

**Researched:** 2026-02-07
**Domain:** Compiler implementation -- parser, type checker, MIR lowering, codegen
**Confidence:** HIGH

## Summary

This research investigates how to add multi-clause function definitions to the Snow compiler. The existing codebase already has all the foundational infrastructure needed: pattern matching in case expressions with exhaustiveness checking (Maranget's algorithm), guard clause support with `when` keyword in match arms, type unification across case arm bodies, and a decision tree pattern compiler for code generation.

The core strategy is: **desugar multi-clause functions into a single function with an internal case expression**. This approach reuses the existing pattern matching infrastructure completely -- the parser collects multiple `fn name(...)` declarations with the same name and arity, groups them into a single function definition with a synthesized case expression over a tuple of the parameters, and everything downstream (type checking, exhaustiveness, MIR lowering, codegen) works without modification.

**Primary recommendation:** Implement multi-clause functions as a parser-level grouping + AST-level desugaring into case expressions. This minimizes changes to the type checker, MIR lowerer, and codegen while providing all required semantics (exhaustiveness checking, type unification, guard clauses).

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions
- Guard clauses supported with `when` keyword (e.g., `fn abs(n) when n < 0 = -n`)
- Guard expressions can be arbitrary expressions that return Bool, including function calls -- not limited to simple comparisons
- First-match wins -- clauses tried top-to-bottom, first matching clause executes
- Wildcard/catch-all clause must be last -- compiler error if a catch-all appears before other clauses
- Different arities are separate functions -- `fn foo(x)` and `fn foo(x, y)` are `foo/1` and `foo/2`, not conflicting clauses
- All parameters in a multi-parameter function support patterns (not just the first)

### Claude's Discretion
- Whether to support both `= expr` (single-expression) and `do/end` (block body) forms, or one form only
- How clauses are grouped syntactically (consecutive standalone declarations vs single block)
- Exhaustiveness warning format and detail level
- Whether unreachable clauses produce a warning or error
- Return type mismatch error verbosity
- Runtime behavior when no clause matches (panic vs error tuple)
- Whether zero-arg multi-clause functions are supported (only meaningful with guards)
- Where multi-clause functions are valid (top level only vs everywhere functions work)
- Whether single-clause and multi-clause are distinct or seamlessly unified

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Standard Stack

No external libraries needed. This is entirely an internal compiler feature using existing Rust crates in the workspace.

### Core Crates Modified

| Crate | Purpose | Changes Needed |
|-------|---------|----------------|
| `snow-parser` | Parsing `fn` declarations | New syntax for `= expr` form, pattern params, guard clauses, clause grouping |
| `snow-typeck` | Type checking multi-clause fns | Clause grouping, desugaring to case expr, return type unification |
| `snow-codegen` | MIR lowering and LLVM codegen | Minimal -- desugaring produces standard case expressions |
| `snow-common` | Shared token types | Possibly no changes (WHEN_KW and EQ already exist) |
| `snow-lexer` | Tokenization | No changes needed |

### Existing Infrastructure to Reuse

| Feature | Location | How It's Reused |
|---------|----------|-----------------|
| Pattern matching | `snow-parser/src/parser/patterns.rs` | Patterns in function parameters |
| Exhaustiveness checking | `snow-typeck/src/exhaustiveness.rs` | Applied to multi-clause patterns |
| Guard expressions | `snow-typeck/src/infer.rs` (validate_guard_expr) | Guards on function clauses |
| Pattern compilation | `snow-codegen/src/pattern/compile.rs` | Decision tree compilation |
| Decision tree codegen | `snow-codegen/src/codegen/pattern.rs` | LLVM IR for pattern branches |

## Architecture Patterns

### Recommended Implementation Strategy: Desugaring

The most robust approach is to desugar multi-clause functions into standard single functions with case expressions. This is the approach used by Erlang/OTP (BEAM compiler), Elixir, and Haskell (GHC).

**Why desugaring wins:**
1. Reuses all existing pattern matching infrastructure (exhaustiveness, redundancy, codegen)
2. Type checker already handles case expressions with guard clauses
3. No changes needed to MIR or LLVM codegen
4. Semantics are identical to hand-written case expressions

**Where the desugaring happens:**
The desugaring should happen at the boundary between parsing and type checking. Two viable approaches:

#### Option A: Parser-level grouping + Early desugaring (RECOMMENDED)

The parser produces individual `FnDef` nodes for each clause. A post-parse grouping pass (or the type checker's first pass) collects consecutive `FnDef` nodes with the same name and arity, then desugars them into a single function with a case expression.

```
Source:
  fn fib(0) = 0
  fn fib(1) = 1
  fn fib(n) = fib(n-1) + fib(n-2)

After grouping + desugaring (conceptual):
  fn fib(__arg0) do
    case __arg0 do
      0 -> 0
      1 -> 1
      n -> fib(n-1) + fib(n-2)
    end
  end
```

For multi-parameter functions, the scrutinee is a tuple:
```
Source:
  fn add(0, y) = y
  fn add(x, 0) = x
  fn add(x, y) = x + y

After desugaring (conceptual):
  fn add(__arg0, __arg1) do
    case (__arg0, __arg1) do
      (0, y) -> y
      (x, 0) -> x
      (x, y) -> x + y
    end
  end
```

#### Option B: New AST node for multi-clause functions

Add a new `MultiClauseFnDef` AST node that the parser produces directly. The type checker then processes this node type specifically. This requires more changes but keeps the AST closer to the source.

**Recommendation: Option A** is better because it keeps downstream passes (typeck, MIR, codegen) unchanged. The desugaring can happen in the type checker's pre-pass, which already scans for function definitions.

### Syntax Design Decisions (Claude's Discretion)

#### Both `= expr` and `do/end` forms: SUPPORT BOTH

Supporting both forms is valuable:
- `= expr` for concise single-expression clauses (the common case for multi-clause functions)
- `do/end` for when a clause needs multiple statements

This matches the roadmap examples (`fn fib(0) = 0`) and is consistent with how Elixir handles this (`def fib(0), do: 0` for single-expression, `def fib(n) do ... end` for blocks).

**Parser impact:** The parser checks for `=` after the parameter list (and optional guard). If `=` is found, parse a single expression. If `do` is found, parse a block body as today.

#### Consecutive standalone declarations: USE THIS

Clauses are consecutive `fn name(...)` declarations at the same scope level. No wrapping block needed. This is the Elixir/Erlang style and matches the roadmap example:

```snow
fn fib(0) = 0
fn fib(1) = 1
fn fib(n) = fib(n-1) + fib(n-2)
```

The grouping happens at the type checker level, not the parser level. The parser produces individual `FnDef` nodes. The type checker groups consecutive same-name, same-arity function definitions.

**Key rule:** Clauses must be consecutive. If `fn fib(...)` appears, then a non-`fib` declaration appears, then `fn fib(...)` appears again, the second group is a **redefinition error**, not a continuation.

#### Clause parameter parsing: Patterns instead of names

Currently, `Param` nodes contain an IDENT (name) and optional type annotation. For multi-clause functions, parameters need to support patterns (literals, wildcards, constructors, tuples).

**Two approaches:**
1. **Change Param to hold patterns** -- Modify the parser to allow patterns in parameter position. Simple identifiers are still valid patterns (IDENT_PAT).
2. **New FnClause node type** -- Add a separate node for clause parameters.

**Recommendation:** Approach 1 is cleaner. When parsing `fn fib(0)`, the parser detects that the parameter is a literal pattern (not just an IDENT), and wraps it in the appropriate pattern node. The `= expr` body form signals this is a multi-clause function clause.

#### Single-clause and multi-clause: SEAMLESSLY UNIFIED

A function with one clause using `do/end` body (the current form) is just a degenerate case of a multi-clause function with one clause. The existing `fn name(x) do ... end` syntax remains unchanged. A function is "multi-clause" only when multiple `fn name(...)` declarations with the same name and arity appear consecutively.

This means: no breaking changes to existing code. Every existing function definition continues to work exactly as before.

#### Zero-arg multi-clause functions: SUPPORT WITH GUARDS

Zero-arg functions with multiple clauses only make sense with guards:
```snow
fn status() when connected() = "online"
fn status() = "offline"
```
This is useful and should be supported. Without guards, multiple zero-arg clauses of the same name would be redundant/conflicting.

#### Where multi-clause functions are valid: EVERYWHERE FUNCTIONS WORK

Multi-clause functions should work at top level, inside modules, inside impl blocks -- anywhere `fn` definitions are currently valid. The grouping logic applies at every scope level.

#### Unreachable clauses: WARNING (not error)

Unreachable (redundant) clauses should produce a warning, not an error. This matches the existing behavior for redundant match arms (`ctx.warnings.push(...)` in `infer_case`). The code is technically valid, just wasteful.

#### Runtime behavior when no clause matches: PANIC

When no clause matches at runtime, the function should panic with a clear error message: `"no matching clause for function 'name/arity'"`. This is consistent with Erlang/Elixir behavior (`FunctionClauseError`) and simpler than error tuples. The panic infrastructure (`MirExpr::Panic`) already exists.

### Project Structure Changes

```
snow-parser/src/
├── parser/
│   ├── items.rs        # Modified: parse_fn_def supports = expr form and pattern params
│   ├── patterns.rs     # Unchanged (already handles all pattern types)
│   └── expressions.rs  # Possibly minor changes for = expr body parsing
├── ast/
│   ├── item.rs         # Modified: FnDef gets guard() accessor, body/expr_body distinction
│   └── pat.rs          # Unchanged
└── syntax_kind.rs      # Possibly add FN_CLAUSE or GUARD_CLAUSE usage

snow-typeck/src/
├── infer.rs            # Modified: clause grouping, desugaring, guard validation expansion
├── exhaustiveness.rs   # Unchanged (already works for case expressions)
└── error.rs            # Modified: new error variants for clause-related issues

snow-codegen/src/
├── mir/
│   └── lower.rs        # Minor: handle desugared case in function bodies
└── pattern/
    └── compile.rs      # Unchanged (already compiles case expressions)
```

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Pattern matching in params | Custom param matching logic | Desugar to `case` expression | Existing case expression handles all pattern types, guards, exhaustiveness |
| Exhaustiveness checking | New exhaustiveness algorithm | Existing `check_exhaustiveness` | Maranget's algorithm already handles all pattern types |
| Decision tree compilation | New pattern compiler | Existing `compile_match` | Already compiles patterns to efficient decision trees |
| Guard evaluation | Custom guard handling | Existing guard infrastructure | `validate_guard_expr` and guard codegen already work |
| Type unification across clauses | Custom unifier | Existing case arm unification in `infer_case` | Already unifies return types across case arms |

## Common Pitfalls

### Pitfall 1: Clause Grouping Across Scope Boundaries
**What goes wrong:** If the grouping logic naively collects all `fn name(...)` definitions in a scope, it might group functions from different modules or impl blocks.
**Why it happens:** Functions at different nesting levels could share names.
**How to avoid:** Group only consecutive declarations at the same scope level. A non-function item between two `fn name(...)` definitions breaks the group.
**Warning signs:** Tests where `fn foo` in module A and `fn foo` in module B interfere.

### Pitfall 2: Guard Validation Too Restrictive
**What goes wrong:** The user decision says guards allow "arbitrary expressions that return Bool, including function calls." But the existing `validate_guard_expr` in `infer_case` is restrictive -- it only allows comparisons, boolean ops, literals, name refs, and named function calls. It disallows many expression types.
**Why it happens:** The existing guard validation was designed to be conservative for case expressions.
**How to avoid:** For multi-clause function guards, relax the validation to match the user decision: any expression that type-checks to Bool is valid. Alternatively, update `validate_guard_expr` to be more permissive.
**Warning signs:** User writes `fn foo(x) when is_valid(x) and x.field > 0 = ...` and gets rejected.

### Pitfall 3: Catch-All Detection Across All Parameters
**What goes wrong:** The "catch-all must be last" rule is straightforward for single-parameter functions (literal pattern vs wildcard/variable). For multi-parameter functions, a clause is catch-all only if ALL parameters are wildcards/variables.
**Why it happens:** Checking only the first parameter misses cases like `fn add(_, 0) = ...` which is not catch-all even though the first param is wildcard.
**How to avoid:** A clause is catch-all if every parameter pattern is a wildcard or variable binding (no constructors, no literals). Check all parameters.
**Warning signs:** False positive catch-all errors on clauses with mixed wildcard/specific patterns.

### Pitfall 4: Name Binding Across Clause Parameters
**What goes wrong:** In `fn add(x, y) = x + y`, `x` and `y` are bound by the patterns. The desugared case arm body must be able to reference these bindings.
**Why it happens:** When desugaring to a case expression over a tuple, variable patterns in the tuple elements need to be correctly scoped so the body can access them.
**How to avoid:** The tuple pattern `(x, y) -> x + y` naturally binds `x` and `y` in the case arm body scope. The existing pattern binding mechanism in the type checker handles this.
**Warning signs:** "unbound variable" errors when referencing pattern-bound names in the body.

### Pitfall 5: Existing Single-Clause Functions Must Not Break
**What goes wrong:** The parser changes to support pattern parameters accidentally break existing `fn name(param) do ... end` syntax.
**Why it happens:** An identifier parameter like `x` in `fn foo(x)` needs to be parsed as an identifier pattern, but the existing code parses it as a `Param` with a `name` token.
**How to avoid:** Only trigger pattern parameter parsing when the function uses `= expr` body form or when a non-identifier token appears in parameter position (literal, wildcard `_`, constructor, tuple). For `do/end` functions, keep existing parsing unchanged. Or: make the change backward compatible by treating `IDENT` as both a valid param name and a valid pattern.
**Warning signs:** Existing test suite (`functions.snow`, `pattern_match.snow`, etc.) starts failing.

### Pitfall 6: Type Annotation Interaction with Patterns
**What goes wrong:** In `fn foo(x :: Int)`, the parameter has both a name and a type annotation. But in `fn foo(0)`, the parameter is a literal pattern with no type annotation. How do these interact in multi-clause functions?
**Why it happens:** The first clause might have `fn foo(0)` (no type annotation) and a later clause `fn foo(n)` (variable pattern, also no annotation). Type inference handles this through unification with the scrutinee type.
**How to avoid:** In multi-clause functions, parameter type annotations should be optional. The type is inferred from the pattern and the usage context. If annotations are present, they must be consistent across all clauses (or only allowed on the first clause). Simplest: annotations are on the overall function (via the first clause or a separate signature), not on individual pattern clauses.
**Warning signs:** Conflicting type annotations between clauses causing confusing errors.

### Pitfall 7: Visibility and Generic Parameters on Multi-Clause Functions
**What goes wrong:** Only the first clause should have `pub`, generic parameters `<T>`, and return type annotation `-> Type`. If they appear on subsequent clauses, that's an error.
**Why it happens:** Each clause is parsed as a standalone `FnDef`, so the parser allows all these annotations on every clause.
**How to avoid:** During clause grouping, validate that `pub`, generic params, return type, and where clause appear only on the first clause. If duplicated, emit a diagnostic.
**Warning signs:** Generic parameters getting confused when they appear on multiple clauses.

## Code Examples

### Example 1: How the parser currently handles `fn` definitions

```rust
// From snow-parser/src/parser/items.rs, line 26
pub(crate) fn parse_fn_def(p: &mut Parser) {
    let m = p.open();
    parse_optional_visibility(p);
    p.advance(); // FN_KW or DEF_KW

    // Function name
    if p.at(SyntaxKind::IDENT) {
        let name = p.open();
        p.advance();
        p.close(name, SyntaxKind::NAME);
    }

    // Optional type parameters: <T, U>
    if p.at(SyntaxKind::LT) { parse_generic_param_list(p); }

    // Parameter list
    if p.at(SyntaxKind::L_PAREN) { parse_param_list(p); }

    // Return type: -> Type
    if p.at(SyntaxKind::ARROW) { /* ... */ }

    // Where clause
    if p.at(SyntaxKind::WHERE_KW) { /* ... */ }

    // Body: do ... end
    p.expect(SyntaxKind::DO_KW);
    parse_item_block_body(p);
    p.expect(SyntaxKind::END_KW);

    p.close(m, SyntaxKind::FN_DEF);
}
```

**What changes:** After parameter list and optional guard, check for `= expr` (single-expression body) as alternative to `do ... end` (block body). Parameters may contain patterns instead of just identifiers.

### Example 2: How case expressions handle guards and exhaustiveness

```rust
// From snow-typeck/src/infer.rs, line 2355
fn infer_case(ctx, env, case, types, ...) -> Result<Ty, TypeError> {
    let scrutinee_ty = /* infer scrutinee */;
    let mut arm_patterns = Vec::new();
    let mut arm_has_guard = Vec::new();

    for arm in case.arms() {
        // 1. Infer pattern type, unify with scrutinee
        let pat_ty = infer_pattern(ctx, env, &pat, types, type_registry)?;
        ctx.unify(pat_ty, scrutinee_ty.clone(), ...)?;

        // 2. Validate and type-check guard
        if let Some(guard_expr) = arm.guard() {
            validate_guard_expr(&guard_expr)?;
            let guard_ty = infer_expr(ctx, env, &guard_expr, ...)?;
            ctx.unify(guard_ty, Ty::bool(), ...);
        }

        // 3. Unify body types across arms
        let body_ty = infer_expr(ctx, env, &body, ...)?;
        if let Some(ref prev_ty) = result_ty {
            ctx.unify(prev_ty.clone(), body_ty.clone(), ...)?;
        }
    }

    // 4. Exhaustiveness check (excluding guarded arms)
    check_exhaustiveness(&unguarded_patterns, &scrutinee_type_info, &registry);

    // 5. Redundancy check (all arms)
    check_redundancy(&arm_patterns, &scrutinee_type_info, &registry);
}
```

**Key insight:** This exact flow applies to multi-clause functions after desugaring. Each clause becomes a case arm. The desugared function's body IS a case expression, so all of this code runs automatically.

### Example 3: Desugaring transformation (pseudocode)

```rust
// Input: multiple FnDef AST nodes with same name/arity
// fn fib(0) = 0
// fn fib(1) = 1
// fn fib(n) when n > 1 = fib(n-1) + fib(n-2)

// Output: single function with case expression
// Conceptually:
fn desugar_multi_clause(clauses: &[FnClause]) -> FnDef {
    let name = clauses[0].name;
    let arity = clauses[0].params.len();

    // Generate parameter names: __p0, __p1, ...
    let params: Vec<String> = (0..arity).map(|i| format!("__p{}", i)).collect();

    // Build case arms from clauses
    let arms: Vec<CaseArm> = clauses.iter().map(|clause| {
        let pattern = if arity == 1 {
            clause.params[0].pattern.clone()
        } else {
            TuplePattern(clause.params.iter().map(|p| p.pattern.clone()).collect())
        };
        CaseArm {
            pattern,
            guard: clause.guard.clone(),
            body: clause.body.clone(),
        }
    }).collect();

    // Build scrutinee
    let scrutinee = if arity == 1 {
        Var("__p0")
    } else {
        Tuple(params.iter().map(|p| Var(p)).collect())
    };

    FnDef {
        name,
        params: params.iter().map(|p| Param { name: p, type_ann: None }).collect(),
        body: CaseExpr { scrutinee, arms },
    }
}
```

### Example 4: Pattern parameter parsing (new parser logic)

```rust
// New: parse function parameter that may be a pattern
fn parse_fn_clause_param(p: &mut Parser) -> Option<MarkClosed> {
    // Try to parse as a pattern
    match p.current() {
        // Literal: fn fib(0) = ...
        SyntaxKind::INT_LITERAL | SyntaxKind::FLOAT_LITERAL
        | SyntaxKind::TRUE_KW | SyntaxKind::FALSE_KW | SyntaxKind::NIL_KW => {
            parse_pattern(p)
        }
        // Negative literal: fn foo(-1) = ...
        SyntaxKind::MINUS if matches!(p.nth(1), SyntaxKind::INT_LITERAL | SyntaxKind::FLOAT_LITERAL) => {
            parse_pattern(p)
        }
        // Wildcard: fn foo(_) = ...
        SyntaxKind::IDENT if p.current_text() == "_" => {
            parse_pattern(p)
        }
        // Tuple: fn foo((a, b)) = ...
        SyntaxKind::L_PAREN => {
            parse_pattern(p)
        }
        // Constructor: fn foo(Some(x)) = ...
        SyntaxKind::IDENT if p.current_text().starts_with(|c: char| c.is_uppercase()) && p.nth(1) == SyntaxKind::L_PAREN => {
            parse_pattern(p)
        }
        // Regular identifier with optional type annotation: fn foo(x :: Int) = ...
        SyntaxKind::IDENT => {
            // Parse as regular param (existing logic) OR as ident pattern
            let m = p.open();
            p.advance(); // ident
            if p.at(SyntaxKind::COLON_COLON) {
                // Type annotation -- regular param
                let ann = p.open();
                p.advance(); // ::
                parse_type(p);
                p.close(ann, SyntaxKind::TYPE_ANNOTATION);
            }
            Some(p.close(m, SyntaxKind::PARAM))
        }
        _ => None,
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Wrap in case expressions | Multi-clause function syntax | Phase 11 | Ergonomic improvement, no semantic change |

**How other languages implement this:**

| Language | Syntax | Implementation | Guards |
|----------|--------|----------------|--------|
| Erlang | `fib(0) -> 0; fib(N) -> ...` | Compiled to pattern match in BEAM | `when` with restricted expressions |
| Elixir | `def fib(0), do: 0` | Desugars to Erlang clauses | `when` with guard-safe functions |
| Haskell | `fib 0 = 0; fib n = ...` | Desugars to case expression in Core | `\| guard` with Bool expressions |
| OCaml | `let rec fib = function 0 -> 0 \| n -> ...` | Pattern match in lambda | `when` with Bool expressions |
| Rust | N/A (no multi-clause) | Uses `match` expression | `if` guards in match |
| Scala | Uses `match` + partial functions | Pattern match | `if` guards |

**Snow's approach** is most similar to Elixir/Haskell: consecutive `fn` declarations with the same name are grouped into a single function with pattern-matched clauses. The `when` guard syntax and first-match-wins semantics match Elixir. Allowing arbitrary Bool expressions in guards (not a restricted subset) goes beyond Erlang/Elixir but matches Haskell.

## Open Questions

1. **Type annotations on multi-clause functions**
   - What we know: Individual clause parameters use patterns, not typed params. The function's type comes from inference or from a return type annotation.
   - What's unclear: Should type annotations be allowed on individual clause parameters (e.g., `fn foo(x :: Int) = x + 1`)? Or only on the first clause?
   - Recommendation: Allow type annotations on the first clause's parameters as a type signature, disallow on subsequent clauses. For `= expr` clauses with simple variable patterns, allow `:: Type` annotation. For literal/constructor patterns, type annotation is meaningless (the literal already constrains the type).

2. **Interaction with existing `do/end` functions**
   - What we know: Existing functions use `fn name(x) do ... end`. Multi-clause functions use `fn name(0) = 0`.
   - What's unclear: Can a multi-clause function mix `= expr` and `do/end` bodies across clauses?
   - Recommendation: Yes, allow mixing. `fn fib(0) = 0` followed by `fn fib(n) do complex_logic end` should work. Both are just different body forms for a clause.

3. **Interaction with impl blocks**
   - What we know: Impl blocks contain method definitions as `FnDef` nodes.
   - What's unclear: Should multi-clause methods work inside `impl` blocks?
   - Recommendation: Yes, support everywhere. The grouping logic is the same regardless of context.

4. **Error recovery for interleaved clauses**
   - What we know: Clauses must be consecutive.
   - What's unclear: What error message for: `fn foo(0) = 0; fn bar() = 1; fn foo(n) = n`?
   - Recommendation: The second `fn foo` is a redefinition error: "function `foo/1` already defined at line X. Multi-clause functions must have consecutive clauses."

## Sources

### Primary (HIGH confidence)
- Snow codebase exploration: `snow-parser/src/parser/items.rs` -- current function parsing
- Snow codebase exploration: `snow-parser/src/ast/item.rs` -- FnDef AST node
- Snow codebase exploration: `snow-parser/src/ast/pat.rs` -- Pattern types
- Snow codebase exploration: `snow-parser/src/parser/patterns.rs` -- Pattern parsing
- Snow codebase exploration: `snow-typeck/src/infer.rs` -- infer_fn_def, infer_case, validate_guard_expr
- Snow codebase exploration: `snow-typeck/src/exhaustiveness.rs` -- Maranget's algorithm
- Snow codebase exploration: `snow-codegen/src/mir/mod.rs` -- MIR types (MirFunction, MirMatchArm, MirPattern)
- Snow codebase exploration: `snow-codegen/src/mir/lower.rs` -- lower_fn_def, lower_case_expr
- Snow codebase exploration: `snow-codegen/src/pattern/compile.rs` -- compile_match
- Snow codebase exploration: `snow-codegen/src/codegen/pattern.rs` -- codegen_decision_tree
- Snow codebase exploration: `snow-parser/src/syntax_kind.rs` -- WHEN_KW, GUARD_CLAUSE already exist

### Secondary (MEDIUM confidence)
- Erlang/Elixir compiler approach (training knowledge verified against codebase patterns)
- Haskell/GHC desugaring approach (training knowledge)
- Maranget's "Warnings for Pattern Matching" (2007) -- referenced in exhaustiveness.rs comments

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- verified by reading every relevant file in the codebase
- Architecture: HIGH -- desugaring approach proven in multiple production compilers; codebase infrastructure confirmed to support it
- Pitfalls: HIGH -- identified from reading actual code patterns and understanding the parser/typeck interaction
- Code examples: HIGH -- based on reading actual source code, not hypothetical

**Research date:** 2026-02-07
**Valid until:** 2026-03-07 (stable -- compiler internals don't change without our changes)
