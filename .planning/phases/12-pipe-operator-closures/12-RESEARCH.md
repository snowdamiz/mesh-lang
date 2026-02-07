# Phase 12: Pipe Operator Closures - Research

**Researched:** 2026-02-07
**Domain:** Parser extension for inline closure syntax in pipe chains
**Confidence:** HIGH

## Summary

This phase extends the Snow parser's `parse_closure` function to support the full closure syntax specified in CONTEXT.md -- bare parameters (no parens required), `do/end` block bodies, multi-clause closures with `|` separator, guard clauses, and full pattern matching in parameters. The existing closure infrastructure (type checker's `infer_closure`, MIR lowering's `lower_closure_expr` with lambda lifting, LLVM codegen's closure calling convention) already works correctly. The problem is strictly a parser limitation: `parse_closure` currently requires parenthesized parameter lists and only supports the `-> block end` body form.

The pipe operator parsing (`expr_bp` with Pratt parsing, binding power 3/4) and argument list parsing (`parse_arg_list`) already correctly handle closures inside pipe chains -- the existing test `full_program_with_imports_pipes_closures` proves that `nums |> filter(fn(x) -> x > 2 end) |> map(fn(x) -> x * 2 end)` parses and executes correctly. The issue is only that the new syntax forms (bare params, do/end body, multi-clause, guards) are not yet parsed.

**Primary recommendation:** Rewrite `parse_closure` in `expressions.rs` to support all specified syntax forms, reusing `parse_fn_clause_param` for pattern parameters and the existing guard/block infrastructure from Phase 11. No changes needed to type checker, MIR lowering, or LLVM codegen (except minor AST accessor additions for multi-clause closure bodies).

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Closure syntax
- Use `fn params -> body end` syntax (Elixir-style)
- Support multi-clause closures with `|` separator: `fn 0 -> "zero" | n -> to_string(n) end`
- Full pattern matching in closure parameters (destructuring tuples, structs, etc.)
- Two body forms: `fn x -> expr end` for one-liners, `fn x do ... end` for multi-line blocks

#### Nesting behavior
- No limit on nesting depth -- arbitrary closure nesting inside pipe chains is legal
- Balanced matching for `end` tokens -- each `fn`/`do` gets its own `end`, parser counts nesting depth
- `do/end` blocks inside closures support full statements (let, case, if, etc.) -- same as named function bodies

#### Error messages
- Missing `end` errors point back to the opening `fn` token: "unclosed closure starting at line X -- expected `end`"
- Bail on first closure parse error in a pipe chain (no recovery, user fixes one at a time)
- When user writes bare closure as pipe target (`|> fn x -> x end`), suggest the fix: "unexpected closure as pipe target -- did you mean `|> Func(fn x -> x end)`?"

#### Edge cases
- Closures support any number of parameters (comma-separated): `fn acc, x -> acc + x end`
- Guard clauses supported in closure clauses: `fn x when x > 0 -> x | x -> -x end`
- Pipe operator precedence follows Elixir's rules -- pipe has low precedence, closure body extends to `end`

### Claude's Discretion

- Whether pipes are allowed inside closure bodies that are themselves pipe arguments (e.g., `|> Enum.map(fn x -> x |> transform() end)`) -- likely yes for consistency
- Whether closures can be the source (left-hand side) of a pipe chain
- Terminology in error messages ("closure" vs "anonymous function")

### Deferred Ideas (OUT OF SCOPE)

None -- discussion stayed within phase scope.
</user_constraints>

## Standard Stack

This phase involves only internal Snow compiler modifications. No external libraries needed.

### Core Files to Modify

| File | Purpose | Changes Required |
|------|---------|-----------------|
| `crates/snow-parser/src/parser/expressions.rs` | Pratt expression parser | Rewrite `parse_closure` for new syntax forms |
| `crates/snow-parser/src/ast/expr.rs` | Closure AST node | Add accessors for multi-clause body, guards |
| `crates/snow-parser/src/syntax_kind.rs` | Syntax node kinds | May need CLOSURE_CLAUSE kind for multi-clause |
| `crates/snow-fmt/src/walker.rs` | Formatter | Update `walk_closure_expr` for new syntax |
| `crates/snow-parser/tests/parser_tests.rs` | Parser snapshot tests | New tests for all syntax forms |

### Files That Should NOT Need Changes

| File | Reason Already Works |
|------|---------------------|
| `crates/snow-typeck/src/infer.rs` | `infer_closure` reads param_list + body from AST; works with any parser output |
| `crates/snow-codegen/src/mir/lower.rs` | `lower_closure_expr` reads param_list + body; lambda lifting works unchanged |
| `crates/snow-codegen/src/codegen/expr.rs` | Closure calling convention (fn_ptr + env_ptr) independent of syntax |
| `crates/snow-parser/src/parser/mod.rs` | Pipe-related code (Pratt parsing loop, binding power) already correct |

## Architecture Patterns

### Current Closure Parser Structure

```
parse_closure(p):
  advance FN_KW
  if at(L_PAREN): parse_param_list(p)    // parenthesized params only
  expect(ARROW)                           // single -> body form only
  parse_block_body(p)
  expect(END_KW)
  close(m, CLOSURE_EXPR)
```

### Required New Structure

```
parse_closure(p):
  fn_span = current_span()
  advance FN_KW

  // Multi-clause check: parse first clause, then check for | separator
  parse_closure_clause(p, fn_span)

  while at(BAR):                          // multi-clause: | separator
    advance BAR
    parse_closure_clause(p, fn_span)

  expect(END_KW)  // single end for entire closure
  close(m, CLOSURE_EXPR)

parse_closure_clause(p, fn_span):
  open CLOSURE_CLAUSE marker
  // Parameters: bare (comma-separated) or parenthesized
  if at(L_PAREN):
    parse_fn_clause_param_list(p)         // reuse Phase 11 pattern param parsing
  else if looks_like_closure_params(p):
    parse_bare_closure_params(p)          // new: comma-separated bare params
  // Optional guard: when <expr>
  if at(WHEN_KW):
    parse guard clause
  // Body: -> expr or do ... end
  if at(ARROW):
    advance -> ; expr(p)                  // single expression body
  else if at(DO_KW):
    advance do ; parse_block_body(p) ; expect(END_KW) // block body (inner end)
  else:
    error
```

### Key Design Decision: BAR vs END Disambiguation

The `|` token (BAR) is used for:
1. Multi-clause closure separator: `fn 0 -> "zero" | n -> n end`
2. Or-patterns inside patterns: `Circle(_) | Point`
3. Trailing closure params: `do |x| ... end`

Inside `parse_closure`, after a clause body expression finishes, the parser needs to distinguish:
- `|` as clause separator (next clause follows)
- `end` as closure terminator

This is straightforward because after an expression in a `-> expr` clause body, `|` can only be the clause separator (not or-pattern, since the body is an expression, not a pattern).

For `do/end` block body closures, the block has its own `end`, so the `|` after the block's `end` is unambiguously a clause separator.

### Bare Parameter Parsing

The new `fn x, y -> ...` syntax requires distinguishing bare closure params from other expression forms. The key insight: after `fn`, if the next tokens are identifiers (or patterns) followed by either `->`, `when`, `do`, or `,`, they are parameters.

```
parse_bare_closure_params(p):
  open PARAM_LIST
  parse_fn_clause_param(p)    // reuse Phase 11 pattern param parser
  while eat(COMMA):
    parse_fn_clause_param(p)
  close PARAM_LIST
```

Lookahead disambiguation after `fn`:
- `fn(` -> parenthesized params (existing code path)
- `fn IDENT ,` -> bare params (new)
- `fn IDENT ->` -> bare single param (new)
- `fn IDENT when` -> bare single param with guard (new)
- `fn IDENT do` -> bare single param with do/end body (new)
- `fn LITERAL ->` -> pattern param (literal) clause (new)
- `fn (` -> could be paren params OR tuple pattern; existing paren path handles both

### End Token Matching (Already Correct)

The recursive-descent parser naturally handles `end` matching. Each `parse_block_body` call terminates at the first `END_KW` it sees, and the caller consumes it. For nested structures:

```
fn x do            <- outer closure starts, parse_block_body for outer
  if cond do       <- if_expr starts, parse_block_body for if-then
    body
  end              <- if_expr's parse_block_body stops, if_expr consumes end
  more_body
end                <- outer closure's parse_block_body stops, closure consumes end
```

No nesting depth counter is needed because the call stack handles it.

### Pipe Precedence (Already Correct)

Pipe `|>` has binding power (3, 4) -- the lowest expression precedence. When parsing `list |> Enum.map(fn x -> x * 2 end)`:

1. Pipe RHS is parsed at `min_bp=4`
2. `Enum.map(...)` becomes CALL_EXPR (postfix BP=25, higher than 4)
3. Inside arg list, `fn x -> x * 2 end` is parsed with `min_bp=0` (reset inside parens)
4. The closure body `x * 2` is parsed by `parse_block_body`, which stops at `end`
5. Closure consumes `end`, arg list consumes `)`, call expr closes, pipe expr closes

No precedence changes needed.

### Anti-Patterns to Avoid

- **Do NOT add a nesting depth counter for `end` matching.** The recursive descent call stack already handles this correctly. Adding a counter would be redundant and error-prone.
- **Do NOT modify `parse_block_body` to handle `BAR` as a terminator.** Instead, the single-expression closure body form (`-> expr`) naturally terminates when `expr_bp` encounters `BAR` or `END_KW` (neither is an infix operator, so the Pratt loop exits).
- **Do NOT change pipe binding power.** Closures work inside pipes because they are inside parenthesized argument lists, where the binding power resets to 0.
- **Do NOT modify the type checker or MIR lowering for single-clause closures.** The existing infrastructure reads `param_list()` and `body()` from the AST; as long as the parser produces the same CLOSURE_EXPR node shape, downstream works unchanged.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Pattern matching in closure params | Custom param parser | `parse_fn_clause_param` from Phase 11 | Already handles literals, wildcards, constructors, tuples, regular idents |
| Guard clause parsing | Custom guard parser | Existing `WHEN_KW` + `expr()` + `GUARD_CLAUSE` node pattern | Same pattern used in `parse_fn_def` and `parse_match_arm` |
| Block body parsing | Custom block parser | `parse_block_body` | Already handles let, case, if, nested fn, etc. |
| Error messages with related spans | Custom error tracking | `error_with_related(msg, fn_span, "closure started here")` | Existing `ParseError::with_related` infrastructure |

## Common Pitfalls

### Pitfall 1: BAR Ambiguity with Or-Patterns in Closure Params

**What goes wrong:** A multi-clause closure like `fn Some(x) | None -> ... end` could be misinterpreted as an or-pattern `Some(x) | None` in a single-clause closure.

**Why it happens:** Both multi-clause separator and or-pattern use `|` (BAR).

**How to avoid:** In the closure body form `-> expr`, after the expression, check for BAR to start a new clause. In the parameter position, parse patterns normally -- or-patterns only occur within a single pattern context (e.g., `fn Some(x) | None -> ...` is a multi-clause, NOT an or-pattern, because `Some(x)` is a complete pattern followed by `|`). The rule: after `->` and its body expression, `|` starts a new clause. Inside a parameter list (before `->` or `when` or `do`), `|` is ambiguous and should be treated as clause separator (not or-pattern) at the top level. Note: This is consistent with Elixir's behavior.

**Warning signs:** Test case `fn Some(x) | None -> "something" end` should be parsed as two clauses, not one clause with an or-pattern.

### Pitfall 2: Expression Body vs Block Body Termination

**What goes wrong:** In `fn x -> x + 1 end`, the expression `x + 1` must stop before `end`. If the parser tries to include `end` as part of the expression, parsing fails.

**Why it happens:** `parse_block_body` (used for `-> expr` form) terminates at `END_KW`, which is correct. But if we use `expr(p)` directly instead of `parse_block_body`, the expression parser won't know to stop at `end`.

**How to avoid:** Use `parse_block_body` for single-expression `-> body` form (same as current implementation). `parse_block_body` treats the expression as a single-statement block and stops at `END_KW`. For multi-clause closures, `parse_block_body` also stops at `BAR` would be needed, OR use `expr(p)` and rely on the Pratt parser's binding power. Since `BAR` is not an infix operator in `infix_binding_power`, the Pratt loop will exit at `BAR`, which is correct.

**Decision:** For multi-clause `-> expr` bodies, call `expr(p)` instead of `parse_block_body` so the expression terminates at `|` (BAR) or `end` (END_KW). For `do/end` block bodies, use `parse_block_body` as usual.

### Pitfall 3: Bare Closure as Pipe Target

**What goes wrong:** User writes `|> fn x -> x end` instead of `|> Func(fn x -> x end)`.

**Why it happens:** The pipe RHS is parsed by `expr_bp(p, 4)`, which calls `lhs()`, which dispatches `FN_KW` to `parse_closure`. The closure parses fine, but the resulting PIPE_EXPR has a closure as its RHS instead of a function call.

**How to avoid:** After pipe parsing, in type checking, `infer_pipe` unifies the RHS type with `Fun(vec![lhs_ty], ret_var)`. If the RHS is a closure, this would mean the closure must take a single argument that is itself a function -- probably not what the user intended. The user decision says to produce a specific error message. This could be a post-parse validation: detect PIPE_EXPR where RHS is CLOSURE_EXPR and emit the suggestion.

**Implementation:** Add a check in `expr_bp` after closing a PIPE_EXPR: if the RHS is a CLOSURE_EXPR, emit the diagnostic. OR handle in a post-parse validation pass.

### Pitfall 4: Multi-Clause Closures in Type Checker

**What goes wrong:** The current `infer_closure` reads a single `param_list()` and `body()` from the ClosureExpr AST node. Multi-clause closures need multiple clause bodies with different parameter patterns.

**Why it happens:** The AST accessors don't yet support multi-clause closure structure.

**How to avoid:** Two approaches:
1. **Desugar in parser/AST:** Multi-clause closures produce a single-clause closure that wraps a case/match expression internally. Similar to how Phase 11 handles multi-clause named functions.
2. **Extend AST:** Add CLOSURE_CLAUSE children to CLOSURE_EXPR, add accessors, extend type checker.

**Recommendation:** Use approach 1 (desugar to match). The type checker already handles case expressions with pattern matching and guards. Multi-clause `fn 0 -> "zero" | n -> to_string(n) end` desugars to `fn (__arg) -> case __arg do 0 -> "zero"; n -> to_string(n) end end`. This approach requires NO type checker changes.

### Pitfall 5: do/end Inside -> Body Closures Creates Extra end

**What goes wrong:** `fn x -> if x > 0 do x else -x end end` has TWO `end` tokens. The first belongs to the `if` expression, the second to the closure.

**Why it happens:** The `-> body` form uses `parse_block_body` which stops at the first `END_KW` -- the `if`'s `end`. The closure then expects its own `end`.

**Why it's actually fine:** `parse_block_body` calls `parse_item_or_stmt` which dispatches `if` to `parse_if_expr`. The if parser calls `parse_block_body` for its own then-branch, which stops at the if's `end`. The if parser consumes that `end`. Then the closure's `parse_block_body` continues and stops at the closure's `end`. The closure parser consumes that `end`. The recursive descent structure naturally handles this.

## Code Examples

### Example 1: Bare Single-Param Closure in Pipe

Snow source:
```
list |> Enum.map(fn x -> x * 2 end)
```

Expected CST structure:
```
PIPE_EXPR
  NAME_REF "list"
  PIPE "|>"
  CALL_EXPR
    FIELD_ACCESS
      NAME_REF "Enum"
      DOT "."
      IDENT "map"
    ARG_LIST
      L_PAREN "("
      CLOSURE_EXPR
        FN_KW "fn"
        PARAM_LIST           // no parens, bare param
          PARAM
            IDENT "x"
        ARROW "->"
        BLOCK
          BINARY_EXPR
            NAME_REF "x"
            STAR "*"
            LITERAL 2
        END_KW "end"
      R_PAREN ")"
```

### Example 2: Multi-Clause Closure

Snow source:
```
fn 0 -> "zero" | n -> to_string(n) end
```

If desugared to match:
```
CLOSURE_EXPR
  FN_KW "fn"
  PARAM_LIST
    PARAM
      IDENT "__arg"
  ARROW "->"
  BLOCK
    CASE_EXPR
      NAME_REF "__arg"
      DO_KW "do"
      MATCH_ARM
        LITERAL_PAT 0
        ARROW "->"
        STRING_EXPR "zero"
      MATCH_ARM
        IDENT_PAT "n"
        ARROW "->"
        CALL_EXPR ...
      END_KW "end"
  END_KW "end"
```

Alternatively, without desugaring (keeping multi-clause in CST):
```
CLOSURE_EXPR
  FN_KW "fn"
  CLOSURE_CLAUSE              // new node kind
    PARAM_LIST
      PARAM
        LITERAL_PAT 0
    ARROW "->"
    BLOCK "zero"
  BAR "|"
  CLOSURE_CLAUSE
    PARAM_LIST
      PARAM
        IDENT "n"
    ARROW "->"
    BLOCK (call to_string)
  END_KW "end"
```

### Example 3: Guard Clause in Closure

Snow source:
```
fn x when x > 0 -> x | x -> -x end
```

### Example 4: do/end Body Closure

Snow source:
```
|> Enum.map(fn x do
  let doubled = x * 2
  doubled + 1
end)
```

Expected CST:
```
CLOSURE_EXPR
  FN_KW "fn"
  PARAM_LIST
    PARAM
      IDENT "x"
  DO_KW "do"
  BLOCK
    LET_BINDING ...
    BINARY_EXPR ...
  END_KW "end"
```

### Example 5: Nested Pipes Inside Closure Body

Snow source:
```
list |> Enum.map(fn x -> x |> transform() end)
```

This works naturally because the closure body is parsed by `parse_block_body` -> `parse_item_or_stmt` -> `expr` -> `expr_bp(0)`. Inside `expr_bp`, the inner `|>` is an infix operator with BP (3,4), so `x |> transform()` parses as a PIPE_EXPR. The `end` keyword is not an operator, so `expr_bp` exits, `parse_block_body` stops.

## Discretion Recommendations

### 1. Pipes Inside Closure Bodies: YES

**Recommendation:** Allow pipes inside closure bodies unconditionally.

**Rationale:** The parser already handles this correctly. When parsing `fn x -> x |> transform() end`, the closure body is parsed by `parse_block_body` which calls `expr(p)` which calls `expr_bp(p, 0)`. The inner pipe `|>` is handled by the Pratt parser at binding power (3,4). The `end` keyword exits the Pratt loop (not a recognized operator). No special code needed.

**Confidence:** HIGH -- verified by tracing through the parser code. The existing test `full_chained_pipes_and_field_access` already tests `filter(fn (x) -> x.active end)` inside pipes.

### 2. Closures as Pipe Source (LHS): YES, but defer error to type checker

**Recommendation:** Allow closures as pipe LHS syntactically. The type checker will catch nonsensical types.

**Rationale:** `(fn x -> x * 2 end) |> apply(5)` is syntactically valid. The parser already handles this -- `parse_closure` returns a MarkClosed, and the Pratt loop sees `|>` as the next infix operator. The type checker would need to unify the closure type `Fun(params, ret)` with the expected pipe input type. If the user writes something that doesn't type-check, the type error message is clear enough.

**Confidence:** HIGH -- the parser code already supports this naturally.

### 3. Error Message Terminology: Use "closure"

**Recommendation:** Use "closure" in error messages, not "anonymous function."

**Rationale:** Snow's codebase consistently uses "closure" in comments, AST node names (CLOSURE_EXPR, ClosureExpr), and code. The CONTEXT.md also uses "closure." Using "anonymous function" would be inconsistent with the codebase and longer.

**Error message examples:**
- `"unclosed closure starting at line X -- expected 'end'"`
- `"unexpected closure as pipe target -- did you mean '|> Func(fn x -> x end)'?"`
- `"expected '->' or 'do' after closure parameters"`

**Confidence:** HIGH -- consistent with existing codebase naming.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `fn(x) -> body end` (paren-only params) | `fn x -> body end` (bare params) | Phase 12 | More ergonomic pipe chains |
| Single-clause closures only | Multi-clause `fn 0 -> "zero" \| n -> n end` | Phase 12 | Pattern matching in closures |
| `-> body` only body form | `-> expr` and `do ... end` body forms | Phase 12 | Multi-statement closure bodies |

## Open Questions

1. **Multi-clause desugaring vs CST representation**
   - What we know: Phase 11 desugars multi-clause named functions in the TYPE CHECKER (grouping consecutive FnDef nodes and generating match bodies in MIR lowering). Closures are different -- they are a single expression, not consecutive top-level items.
   - What's unclear: Should multi-clause closures be desugared in the PARSER (producing a single-clause closure with case/match body in the CST) or preserved in the CST (new CLOSURE_CLAUSE node kind) and desugared in type checker/MIR?
   - Recommendation: **Desugar in the parser.** This minimizes downstream changes -- `infer_closure` and `lower_closure_expr` work unchanged. The parser produces a standard CLOSURE_EXPR with a single param and a CASE_EXPR body. The cost is that the CST doesn't perfectly represent the source, but for Phase 12 this is the pragmatic path. The formatter can be updated to recognize the desugared pattern and format it back to multi-clause syntax if needed.

2. **Comma-separated bare params: disambiguation from tuple expression**
   - What we know: `fn x, y -> x + y end` has bare comma-separated params. After `fn`, we need to know where params end and the body begins.
   - What's clear: `->`, `when`, and `do` all unambiguously terminate the parameter list.
   - Recommendation: Parse identifiers/patterns separated by commas until hitting `->`, `when`, or `do`. This is unambiguous because those tokens cannot appear in a parameter name or pattern.

## Sources

### Primary (HIGH confidence)
- `/Users/sn0w/Documents/dev/snow/crates/snow-parser/src/parser/expressions.rs` -- Current parse_closure, expr_bp, infix_binding_power, parse_arg_list implementations
- `/Users/sn0w/Documents/dev/snow/crates/snow-parser/src/parser/mod.rs` -- Parser infrastructure (events, markers, delimiter depth tracking)
- `/Users/sn0w/Documents/dev/snow/crates/snow-parser/src/syntax_kind.rs` -- All syntax kinds including CLOSURE_EXPR, PIPE_EXPR, PIPE, BAR
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/infer.rs` -- infer_closure, infer_pipe implementations
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/lower.rs` -- lower_closure_expr (lambda lifting), lower_pipe_expr (desugaring)
- `/Users/sn0w/Documents/dev/snow/crates/snow-parser/tests/snapshots/parser_tests__full_program_with_imports_pipes_closures.snap` -- Proof that closures inside pipe chains already parse
- `/Users/sn0w/Documents/dev/snow/.planning/phases/11-multi-clause-functions/11-01-SUMMARY.md` -- Phase 11 pattern param infrastructure

### Secondary (MEDIUM confidence)
- `/Users/sn0w/Documents/dev/snow/tests/e2e/closures.snow` -- E2E test proving closure lambda lifting works
- `/Users/sn0w/Documents/dev/snow/tests/e2e/pipe.snow` -- E2E test proving pipe desugaring works
- `/Users/sn0w/Documents/dev/snow/tests/e2e/stdlib_list_pipe_chain.snow` -- E2E test proving closures as function arguments work

## Metadata

**Confidence breakdown:**
- Parser changes: HIGH -- direct code analysis of the exact functions to modify
- Architecture: HIGH -- traced complete execution paths through parser, type checker, and codegen
- Pitfalls: HIGH -- identified by analyzing actual code interactions, not speculation
- Discretion recommendations: HIGH -- verified against existing code behavior

**Research date:** 2026-02-07
**Valid until:** Indefinite (internal compiler code, no external dependency drift)
