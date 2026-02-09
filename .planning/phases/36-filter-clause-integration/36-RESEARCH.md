# Phase 36: Filter Clause + Integration - Research

**Researched:** 2026-02-09
**Domain:** For-in filter clause (`when` condition), list builder conditional push, nested loops, closures in loops, pipe chains with loops, formatter/LSP support
**Confidence:** HIGH

## Summary

This phase adds a `when` filter clause to for-in loops (`for x in list when condition do body end`) so that only elements satisfying the condition are processed and included in the collected result list. It also requires verifying that all loop forms (range, list, map, set -- with and without filter) work correctly with closures, nesting, pipe chains, and tooling (formatter + LSP).

The implementation is a natural extension of Phase 35's comprehension machinery. The `when` keyword already exists in the lexer/parser infrastructure (used for match arm guards and closure guards), and the four-block loop pattern (header/body/latch/merge) provides a clear insertion point for the filter condition: after element extraction in the body block but before executing the user's body expression, emit a conditional branch that skips to the latch (incrementing the counter without pushing to the result list) when the condition is false. The result list uses the existing `snow_list_builder_new`/`push` pattern, but pre-allocation capacity becomes an upper bound since filtered elements reduce actual pushes -- this is harmless because the list builder tracks actual length via its `len` field.

There are two distinct sub-goals: (1) FILT-01 adds the `when` clause syntax and semantics across all pipeline stages (parser, AST, typeck, MIR, lowering, codegen, formatter), and (2) FILT-02 ensures integration correctness with nested loops, closures, and pipe chains. FILT-02 is primarily a testing and verification concern since the existing infrastructure should handle these cases if each layer is correctly extended.

**Primary recommendation:** Add an optional `filter` field (`Option<Box<MirExpr>>`) to all four ForIn MIR variants. In the parser, insert `when` clause parsing between the iterable and `do` keyword. In codegen, add a `filter_bb` basic block between the element extraction (body_bb start) and the user's body expression, branching to latch on false. In typeck, infer the filter expression in the loop variable scope and unify with Bool. The return type remains `List<body_ty>` but the list may be shorter than the collection length.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| snow-parser | workspace | Parse `when condition` between iterable and `do` in for-in | WHEN_KW token already exists; match arm guards use same pattern |
| snow-typeck | workspace | Infer filter expression type (must be Bool) in loop variable scope | Same scope mechanism as body inference |
| snow-codegen | workspace | MIR filter field + LLVM conditional branch in body block | Extends existing four-block pattern |
| snow-fmt | workspace | Format `when` keyword with surrounding spaces in walk_for_in_expr | Same pattern as fn def/closure guard formatting |
| inkwell | 0.8.0 | Build conditional branch instruction for filter | Already used for if/while/loop condition branches |
| snow-rt | workspace | No new runtime functions needed | List builder handles under-capacity naturally |

### Not Needed
No new external dependencies. No new runtime functions. The existing `snow_list_builder_new(capacity)` + `snow_list_builder_push(list, elem)` work correctly when fewer elements are pushed than the pre-allocated capacity -- the `len` field tracks actual pushes, and excess capacity is harmless.

## Architecture Patterns

### Pipeline Flow for Filter Clause

```
Source: for x in [1, 2, 3, 4, 5] when x > 2 do x * 10 end

1. Lexer:    [For, Ident(x), In, [...], When, Ident(x), Gt, IntLiteral(2), Do, ..., End]
2. Parser:   FOR_IN_EXPR {
               binding: NAME(x),
               iterable: LIST_LITERAL([1,2,3,4,5]),
               filter: BINARY_EXPR(x > 2),       <-- NEW: child expr after WHEN_KW
               body: BLOCK(x * 10)
             }
3. AST:      ForInExpr { binding_name(), iterable(), filter(), body() }
4. Typeck:   - Iterable type: List<Int>
             - Bind x as Int in loop scope
             - Filter expr: x > 2 -> Bool (unify with Bool)
             - Body type: Int (x * 10)
             - For-in result: List<Int> (comprehension)
5. MIR:      MirExpr::ForInList {
               var: "x", collection: ListLit([1..5]),
               filter: Some(BinOp(Gt, Var("x"), IntLit(2))),   <-- NEW
               body: BinOp(Mul, Var("x"), IntLit(10)),
               elem_ty: Int, body_ty: Int, ty: Ptr
             }
6. Codegen:  header -> body_bb:
               extract element, bind x
               codegen filter (x > 2)
               cond_br filter_val, do_body_bb, latch_bb   <-- NEW: skip to latch on false
             do_body_bb:
               body_val = codegen body (x * 10)
               list_builder_push(result, body_val)
               br latch_bb
             latch_bb:
               counter++, reduction_check, br header
             merge_bb:
               return result_list   (contains [30, 40, 50])
```

### LLVM Basic Block Structure with Filter

```
entry:
  %collection = <codegen collection>
  %len = call i64 @snow_list_length(%collection)
  %result = call ptr @snow_list_builder_new(%len)   ; capacity = upper bound
  %result_alloca = alloca ptr
  store ptr %result, %result_alloca
  %counter = alloca i64
  store i64 0, %counter
  br forin_header

forin_header:
  %i = load i64, %counter
  %cond = icmp slt i64 %i, %len
  br i1 %cond, forin_body, forin_merge

forin_body:                                    ; Element extraction + filter check
  %elem_raw = call i64 @snow_list_get(%collection, %i)
  %elem = <convert_from_list_element>
  ; Bind loop variable
  store %elem, %var_alloca

  ; -- FILTER CHECK (NEW) --
  %filter_val = <codegen filter expression>     ; e.g., x > 2
  br i1 %filter_val, forin_do_body, forin_latch ; skip body+push on false

forin_do_body:                                  ; User body + push (NEW block)
  %body_val = <codegen body>
  %body_u64 = <convert_to_list_element body_val>
  %cur_result = load ptr, %result_alloca
  call void @snow_list_builder_push(%cur_result, %body_u64)
  br forin_latch

forin_latch:
  %i_cur = load i64, %counter
  %i_next = add i64 %i_cur, 1
  store i64 %i_next, %counter
  call void @snow_reduction_check()
  br forin_header

forin_merge:
  %final_list = load ptr, %result_alloca
  ; len = number of elements actually pushed (may be < capacity)
```

### Key Design: Five-Block Pattern with Filter

When a filter is present, the body block splits into two:
1. **forin_body**: Extract element, bind variable, evaluate filter condition, conditional branch
2. **forin_do_body**: Execute user body, push to result list, branch to latch

When no filter is present, the existing four-block pattern is preserved (no behavioral change).

This design means `continue` still works correctly: it jumps to `latch_bb`, which is the same target regardless of whether a filter is present. `break` still works correctly: it jumps to `merge_bb`. The loop variable is bound before the filter is evaluated, so the filter expression can reference the loop variable.

### Pattern: Filter in Type Checker

The filter expression must:
1. Be inferred AFTER the loop variable(s) are bound in scope (so `x` is available)
2. Be inferred BEFORE the body (logically: bind vars, check filter, then check body)
3. Unify with `Bool` (filter must be a boolean expression)

```rust
// In infer_for_in, after binding loop variables in scope:
if let Some(filter_expr) = for_in.filter() {
    let filter_ty = infer_expr(ctx, env, &filter_expr, types, type_registry, trait_registry, fn_constraints)?;
    ctx.unify(filter_ty, Ty::bool(), ConstraintOrigin::BinOp {
        op_span: filter_expr.syntax().text_range(),
    })?;
}
```

### Pattern: Parser Extension

The existing `parse_for_in_expr` has the structure:
```
for binding in iterable do body end
```

With filter, it becomes:
```
for binding in iterable [when filter_expr] do body end
```

The `when` keyword is already recognized by the lexer (`TokenKind::When` -> `SyntaxKind::WHEN_KW`). After parsing the iterable expression and before expecting `do`, check for `WHEN_KW`:

```rust
// After: expr(p);  // iterable
// Before: p.expect(SyntaxKind::DO_KW);  // do

if p.at(SyntaxKind::WHEN_KW) {
    p.advance(); // WHEN_KW
    expr(p);     // filter expression
}
```

The filter expression becomes a direct child expr of `FOR_IN_EXPR`, positioned after the iterable expression and WHEN_KW token. The AST accessor finds it by looking for WHEN_KW then the next Expr child (same pattern as `MatchArm::guard()`).

### Pattern: AST Filter Accessor

```rust
impl ForInExpr {
    /// The filter expression (after `when`), if present.
    pub fn filter(&self) -> Option<Expr> {
        let has_when = self.syntax
            .children_with_tokens()
            .any(|it| it.kind() == SyntaxKind::WHEN_KW);
        if has_when {
            // With when: iterable is first expr, filter is second expr
            self.syntax.children().filter_map(Expr::cast).nth(1)
        } else {
            None
        }
    }

    /// The iterable expression -- always the first Expr child.
    pub fn iterable(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The loop body block -- always the BLOCK child.
    pub fn body(&self) -> Option<Block> {
        child_node(&self.syntax)
    }
}
```

**Important:** The existing `iterable()` accessor returns the first `Expr` child, which works correctly regardless of filter presence because the iterable is always the first expression child. The `body()` accessor returns the `Block` child, which is also unaffected. Only `filter()` is new.

### Pattern: MIR Filter Field

Add `filter: Option<Box<MirExpr>>` to all four ForIn variants:

```rust
ForInRange {
    var: String,
    start: Box<MirExpr>,
    end: Box<MirExpr>,
    filter: Option<Box<MirExpr>>,    // NEW
    body: Box<MirExpr>,
    ty: MirType,
},

ForInList {
    var: String,
    collection: Box<MirExpr>,
    filter: Option<Box<MirExpr>>,    // NEW
    body: Box<MirExpr>,
    elem_ty: MirType,
    body_ty: MirType,
    ty: MirType,
},

ForInMap {
    key_var: String,
    val_var: String,
    collection: Box<MirExpr>,
    filter: Option<Box<MirExpr>>,    // NEW
    body: Box<MirExpr>,
    key_ty: MirType,
    val_ty: MirType,
    body_ty: MirType,
    ty: MirType,
},

ForInSet {
    var: String,
    collection: Box<MirExpr>,
    filter: Option<Box<MirExpr>>,    // NEW
    body: Box<MirExpr>,
    elem_ty: MirType,
    body_ty: MirType,
    ty: MirType,
},
```

### Pattern: Formatter When Clause

The formatter's `walk_for_in_expr` currently handles: FOR_KW, NAME/DESTRUCTURE_BINDING, IN_KW, iterable expr, DO_KW, BLOCK, END_KW.

Add a case for `WHEN_KW` in the token match:

```rust
SyntaxKind::WHEN_KW => {
    parts.push(sp());
    parts.push(ir::text("when"));
    parts.push(sp());
}
```

This matches the existing pattern used for `when` in `walk_fn_def` (line 200-203) and `walk_closure_clause` (line 614-616).

### Anti-Patterns to Avoid

- **Do NOT create a separate CST node for the filter clause.** The `when` keyword and filter expression are direct children of `FOR_IN_EXPR`, just like how `when` guard works in `MATCH_ARM`. No separate `FILTER_CLAUSE` or `GUARD_CLAUSE` node is needed.
- **Do NOT change the pre-allocation capacity calculation.** The list builder is pre-allocated to the full collection length (upper bound). With filtering, fewer elements are pushed, but the builder handles this correctly -- `len` tracks actual pushes, excess capacity is harmless.
- **Do NOT create a new basic block name like "forin_filter".** The filter check belongs in the existing `forin_body` block. Only add a new `forin_do_body` block for the portion after the filter check (body + push). This minimizes changes to the block structure.
- **Do NOT change `codegen_break` or `codegen_continue`.** Break still jumps to merge_bb, continue still jumps to latch_bb. The filter block is between body_bb and latch_bb, so continue correctly skips both the filter and body. Break correctly exits the entire loop.
- **Do NOT add filter support to MIR lowering without also reading it from the AST.** The pipeline must be complete: parser -> AST accessor -> typeck -> MIR lowering -> codegen.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Filter condition branching | Manual phi nodes for filter result | Simple conditional branch (br i1) | Filter is a Bool -> direct conditional branch, no phi needed |
| Result list capacity with filter | Dynamic capacity adjustment | Pre-allocate full capacity, push fewer elements | List builder handles len < capacity correctly; no reallocation needed |
| Filter expression in scope | Separate scope for filter | Same scope as body (push_scope happens before filter) | Filter needs access to loop variable, same as body |
| When keyword parsing | New token/keyword | Existing WHEN_KW / TokenKind::When | Already in lexer, parser, and SyntaxKind |
| Filter AST accessor | Separate filter child node type | Expr child after WHEN_KW token | Same pattern as MatchArm::guard() |

**Key insight:** The filter clause is structurally identical to a match arm guard -- a `when` keyword followed by an expression that must evaluate to Bool. The infrastructure for this pattern already exists throughout the codebase.

## Common Pitfalls

### Pitfall 1: Filter Expression Parsed Before Iterable
**What goes wrong:** Filter expression is parsed in the wrong position, causing the iterable to be misidentified.
**Why it happens:** The parser inserts `when` parsing at the wrong point in the for-in parsing sequence.
**How to avoid:** Parse in exact order: `for` -> binding -> `in` -> iterable_expr -> [`when` filter_expr] -> `do` -> body -> `end`. The `when` check comes after `expr(p)` for the iterable and before `p.expect(DO_KW)`.
**Warning signs:** Parse errors on valid filter syntax, or iterable expression absorbing the `when` keyword.

### Pitfall 2: Filter Not Having Access to Loop Variable
**What goes wrong:** Filter expression references the loop variable but it's not in scope.
**Why it happens:** Typeck evaluates the filter before pushing the loop variable scope.
**How to avoid:** In `infer_for_in`, the order must be: (1) infer iterable, (2) push scope, (3) bind loop variable(s), (4) infer filter, (5) infer body, (6) exit loop, (7) pop scope. Steps 4 and 5 are both inside the pushed scope.
**Warning signs:** "Undefined variable" errors in filter expressions.

### Pitfall 3: Continue Inside Filter vs. Continue Inside Body
**What goes wrong:** `continue` behaves differently depending on whether it's in the filter or body context.
**Why it happens:** The filter is evaluated in the same loop context as the body.
**How to avoid:** The filter is a simple expression (not a block), so `continue`/`break` cannot appear inside it. The parser parses the filter with `expr(p)` which produces an expression tree, not a block with statements. If someone writes `for x in list when (do continue end) do body end`, the `continue` would be inside a nested block expression, but this is a degenerate case. The normal case is `for x in list when x > 0 do body end` where the filter is a simple boolean expression.
**Warning signs:** None expected in practice. The filter is an expression, not a statement sequence.

### Pitfall 4: MIR Lowering Forgets to Lower the Filter Expression
**What goes wrong:** Filter is parsed and type-checked but not lowered to MIR, so codegen never sees it.
**Why it happens:** The four `lower_for_in_*` methods need to read and lower the filter from the AST.
**How to avoid:** In each `lower_for_in_*` method, after pushing scope and binding loop variables, lower the filter expression if present: `let filter = for_in.filter().map(|f| self.lower_expr(&f));`. Pass it as `filter: filter.map(Box::new)` in the MIR construction.
**Warning signs:** Filter syntax parses and type-checks but has no runtime effect.

### Pitfall 5: Monomorphization and Free-Var Collection Miss the Filter
**What goes wrong:** Closures inside filter expressions don't get their free variables captured, or monomorphization misses function references in the filter.
**Why it happens:** The `collect_function_refs` and `collect_free_vars` walkers in `mono.rs` and `lower.rs` need to traverse the filter expression too.
**How to avoid:** When adding the `filter` field to MIR ForIn variants, also update `collect_function_refs` (mono.rs ~line 227-243) and `collect_free_vars` (lower.rs ~line 7436-7462) to traverse `filter` if `Some`.
**Warning signs:** Linker errors for uncollected function references, or missing captures in closures within filter expressions.

### Pitfall 6: Formatter Drops When Clause
**What goes wrong:** Formatted output of `for x in list when x > 0 do body end` omits the `when x > 0` part.
**Why it happens:** `walk_for_in_expr` doesn't handle WHEN_KW token.
**How to avoid:** Add `SyntaxKind::WHEN_KW` case to the token match in `walk_for_in_expr`, emitting `sp() + "when" + sp()`. The filter expression (a child Node) will be handled by the existing `_ => walk_node(&n)` fallback for non-BLOCK, non-NAME nodes.
**Warning signs:** Formatted code loses the filter clause.

### Pitfall 7: Integration Test Gaps -- Nested Loops, Closures, Pipes
**What goes wrong:** Phase passes but nested for-in loops, closures inside for-in, or for-in inside pipe chains have subtle bugs.
**Why it happens:** These combinations weren't explicitly tested.
**How to avoid:** Write explicit e2e tests for: (1) nested for-in with filters at both levels, (2) closure defined inside a for-in body that captures the loop variable, (3) for-in expression piped to another function (`for x in list when x > 0 do x end |> List.length`), (4) for-in with break/continue inside filtered loop.
**Warning signs:** Tests pass in isolation but fail in complex combinations.

## Code Examples

### Parser: parse_for_in_expr with Filter

```rust
fn parse_for_in_expr(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // FOR_KW

    // Parse binding (unchanged).
    if p.at(SyntaxKind::L_BRACE) {
        // Destructuring binding: {k, v}
        // ... existing code ...
    } else if p.at(SyntaxKind::IDENT) {
        let name = p.open();
        p.advance();
        p.close(name, SyntaxKind::NAME);
    } else {
        p.error("expected loop variable name or {key, value} after `for`");
    }

    p.expect(SyntaxKind::IN_KW);

    // Parse iterable expression.
    if !p.has_error() {
        expr(p);
    }

    // Optional filter clause: `when condition`
    if p.at(SyntaxKind::WHEN_KW) {
        p.advance(); // WHEN_KW
        expr(p);     // filter expression
    }

    // Expect `do`.
    let do_span = p.current_span();
    p.expect(SyntaxKind::DO_KW);

    // Parse body.
    parse_block_body(p);

    // Expect `end`.
    if !p.at(SyntaxKind::END_KW) {
        p.error_with_related(
            "expected `end` to close `for` block",
            do_span,
            "`do` block started here",
        );
    } else {
        p.advance();
    }

    p.close(m, SyntaxKind::FOR_IN_EXPR)
}
```

### AST: ForInExpr with Filter Accessor

```rust
impl ForInExpr {
    /// The filter expression (after `when`), if present.
    pub fn filter(&self) -> Option<Expr> {
        let has_when = self.syntax
            .children_with_tokens()
            .any(|it| it.kind() == SyntaxKind::WHEN_KW);
        if has_when {
            // With `when`: first expr = iterable, second expr = filter
            self.syntax.children().filter_map(Expr::cast).nth(1)
        } else {
            None
        }
    }
}
```

### Typeck: Filter Condition Inference

```rust
// Inside infer_for_in, after push_scope and binding loop variables:

// Infer filter condition if present (FILT-01).
if let Some(filter_expr) = for_in.filter() {
    let filter_ty = infer_expr(ctx, env, &filter_expr, types, type_registry, trait_registry, fn_constraints)?;
    let origin = ConstraintOrigin::BinOp {
        op_span: filter_expr.syntax().text_range(),
    };
    ctx.unify(filter_ty, Ty::bool(), origin)?;
}
```

### MIR Lowering: Lower Filter Expression

```rust
// Inside lower_for_in_list (and similar for range/map/set):

self.push_scope();
self.insert_var(var_name.clone(), elem_mir_ty.clone());

// Lower filter if present.
let filter = for_in.filter().map(|f| Box::new(self.lower_expr(&f)));

let body = for_in
    .body()
    .map(|b| self.lower_block(&b))
    .unwrap_or(MirExpr::Unit);
let body_ty = body.ty().clone();
self.pop_scope();

MirExpr::ForInList {
    var: var_name,
    collection: Box::new(collection),
    filter,                              // NEW
    body: Box::new(body),
    elem_ty: elem_mir_ty,
    body_ty,
    ty: MirType::Ptr,
}
```

### Codegen: Filter Branch in Body Block

```rust
// In codegen_for_in_list, after binding loop variable in body_bb:

// If filter present, add conditional branch.
if let Some(filter_expr) = filter {
    let filter_val = self.codegen_expr(filter_expr)?
        .into_int_value();
    // Create the "do_body" block for the actual body + push.
    let do_body_bb = self.context.append_basic_block(fn_val, "forin_do_body");
    self.builder.build_conditional_branch(filter_val, do_body_bb, latch_bb)
        .map_err(|e| e.to_string())?;
    self.builder.position_at_end(do_body_bb);
}

// Codegen body (unchanged).
let body_val = self.codegen_expr(body_expr)?;

// Push to result list (unchanged).
if let Some(bb) = self.builder.get_insert_block() {
    if bb.get_terminator().is_none() {
        let body_as_i64 = self.convert_to_list_element(body_val, body_ty)?;
        // ... push to result list ...
        self.builder.build_unconditional_branch(latch_bb)?;
    }
}
```

### Formatter: walk_for_in_expr with When

```rust
fn walk_for_in_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::FOR_KW => {
                        parts.push(ir::text("for"));
                        parts.push(sp());
                    }
                    SyntaxKind::IN_KW => {
                        parts.push(sp());
                        parts.push(ir::text("in"));
                        parts.push(sp());
                    }
                    SyntaxKind::WHEN_KW => {          // NEW
                        parts.push(sp());
                        parts.push(ir::text("when"));
                        parts.push(sp());
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                    }
                    SyntaxKind::END_KW => {
                        parts.push(ir::hardline());
                        parts.push(ir::text("end"));
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        let body = walk_block_body(&n);
                        parts.push(ir::indent(ir::concat(vec![ir::hardline(), body])));
                    }
                    SyntaxKind::NAME => {
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::DESTRUCTURE_BINDING => {
                        parts.push(walk_destructure_binding(&n));
                    }
                    _ => {
                        // Iterable expr and filter expr both handled here.
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }
    ir::concat(parts)
}
```

## Files That Must Change

### FILT-01: Filter Clause Syntax and Semantics

| File | Change | Complexity |
|------|--------|-----------|
| `crates/snow-parser/src/parser/expressions.rs` | Add `when` check in `parse_for_in_expr` (3 lines) | LOW |
| `crates/snow-parser/src/ast/expr.rs` | Add `filter()` accessor to `ForInExpr` (12 lines) | LOW |
| `crates/snow-typeck/src/infer.rs` | Add filter type inference + Bool unification in `infer_for_in` (6 lines) | LOW |
| `crates/snow-codegen/src/mir/mod.rs` | Add `filter: Option<Box<MirExpr>>` to 4 ForIn variants + update ty() match | MEDIUM |
| `crates/snow-codegen/src/mir/lower.rs` | Lower filter expr in 4 `lower_for_in_*` methods | MEDIUM |
| `crates/snow-codegen/src/codegen/expr.rs` | Add filter branch in 4 `codegen_for_in_*` methods | MEDIUM |
| `crates/snow-codegen/src/mir/mono.rs` | Add filter traversal in `collect_function_refs` for 4 variants | LOW |
| `crates/snow-codegen/src/pattern/compile.rs` | Add filter traversal in `compile_expr_patterns` for 4 variants | LOW |
| `crates/snow-codegen/src/mir/lower.rs` (free vars) | Add filter traversal in `collect_free_vars` for 4 variants | LOW |
| `crates/snow-fmt/src/walker.rs` | Add WHEN_KW case in `walk_for_in_expr` (4 lines) | LOW |

### FILT-02: Integration Testing

| File | Change | Complexity |
|------|--------|-----------|
| `tests/e2e/for_in_filter.snow` | New fixture: filter with range, list, map, set, continue, break | LOW |
| `crates/snowc/tests/e2e.rs` | E2E test entries for filter, nesting, closures, pipes | LOW |
| `crates/snow-fmt/src/walker.rs` (tests) | Formatter tests for `when` clause idempotency | LOW |
| `crates/snow-parser/tests/parser_tests.rs` | Parser test for for-in with when clause | LOW |

## Interactions with Existing Code

### Continue + Filter
`continue` jumps to `latch_bb`. With filter, the body block has: extract element -> bind var -> filter check -> [do_body_bb: body + push -> latch] or [latch directly]. Continue inside the body (do_body_bb) correctly jumps to latch, skipping the push. Continue inside the filter is not possible (filter is an expression, not a block). The behavior is consistent.

### Break + Filter
`break` jumps to `merge_bb`. The result list alloca contains whatever was pushed before break. With filter, some iterations may have been skipped (filter false), so the result list has `n_pushed` elements where `n_pushed <= iterations_completed`. This is correct.

### Nested Loops with Filter
Each for-in loop has its own result_alloca, counter, and loop_stack entry. Nesting works because `loop_stack.push()` and `loop_stack.pop()` correctly scope break/continue targets. The filter in each loop level is independent. No special handling needed.

### Closures Inside Filtered For-In
A closure inside the body of a filtered for-in captures the loop variable via the existing capture mechanism. The `collect_free_vars` walker must traverse the filter expression too (in case the filter itself references outer variables). The loop variable is locally bound (excluded from captures) in both the filter and body.

### Pipe Chains
For-in expressions are parsed as atoms in the Pratt parser. They can appear in pipe chains: `for x in list when x > 0 do x end |> List.length`. The parser handles this because `parse_for_in_expr` is called from `lhs()` and the result participates in the infix parsing loop (pipe has binding power (3, 4)).

## Open Questions

1. **Should filter expressions support complex expressions with side effects?**
   - What we know: The filter is parsed with `expr(p)` which allows any expression, including function calls. `for x in list when expensive_check(x) do body end` would call `expensive_check` for every element.
   - What's unclear: Whether side effects in filter expressions are intentional or should be restricted.
   - Recommendation: Allow any expression. This is consistent with `when` guards in match arms, which allow arbitrary expressions including function calls. The user is responsible for performance implications.

2. **Should the filter clause work with map destructuring?**
   - What we know: `for {k, v} in map when v > 0 do k end` would need both `k` and `v` in scope for the filter. Since the scope push happens before the filter, both variables are available.
   - What's unclear: Nothing -- this should work naturally.
   - Recommendation: Support it. Test explicitly.

3. **Should `for x in 0..10 when x % 2 == 0 do x end` produce `[0, 2, 4, 6, 8]`?**
   - What we know: Range for-in binds `x` as Int, filter checks `x % 2 == 0`, body evaluates `x`, push only happens when filter is true.
   - Recommendation: Yes, this is the expected semantics. Test explicitly.

## Sources

### Primary (HIGH confidence)
- Codebase: `crates/snow-parser/src/parser/expressions.rs` -- `parse_for_in_expr` (line 1195-1256), `parse_match_arm` with `when` guard (line 651-672)
- Codebase: `crates/snow-parser/src/ast/expr.rs` -- `ForInExpr` (line 583-605), `MatchArm::guard()` pattern (line 360-391)
- Codebase: `crates/snow-common/src/token.rs` -- `TokenKind::When` (line 73), `keyword_from_str("when")` (line 238)
- Codebase: `crates/snow-parser/src/syntax_kind.rs` -- `WHEN_KW` (line 66), `FOR_IN_EXPR` (line 289)
- Codebase: `crates/snow-typeck/src/infer.rs` -- `infer_for_in` (line 3176-3296), scope push/bind/body/pop pattern
- Codebase: `crates/snow-codegen/src/mir/mod.rs` -- ForInRange/List/Map/Set variants (line 306-352), `MirMatchArm::guard` pattern (line 436)
- Codebase: `crates/snow-codegen/src/mir/lower.rs` -- `lower_for_in_expr` + 4 helper methods (line 4015-4183), `collect_free_vars` for ForIn (line 7436-7462)
- Codebase: `crates/snow-codegen/src/codegen/expr.rs` -- `codegen_for_in_range` (line 1738-1872), `codegen_for_in_list` (line 2842-2992), `codegen_break`/`codegen_continue` (line 1874-1905)
- Codebase: `crates/snow-codegen/src/mir/mono.rs` -- `collect_function_refs` for ForIn (line 227-243)
- Codebase: `crates/snow-codegen/src/pattern/compile.rs` -- `compile_expr_patterns` for ForIn (line 1251-1267)
- Codebase: `crates/snow-fmt/src/walker.rs` -- `walk_for_in_expr` (line 451-503), WHEN_KW formatting in fn_def (line 200-203)
- Codebase: `crates/snowc/tests/e2e.rs` -- existing for-in tests (line 1155-1309)
- Codebase: Phase 35 research and verification -- established list builder, comprehension semantics, four/five-block pattern

### Secondary (MEDIUM confidence)
- LLVM conditional branch pattern: verified by existing codegen for if-expressions, while-loops, and match arm guards
- Filter + list builder interaction: capacity as upper bound is standard practice; runtime `snow_list_builder_push` only advances `len`, never reads `cap`

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- no new dependencies, all infrastructure exists
- Architecture: HIGH -- filter is a direct extension of existing four-block loop pattern
- Parser changes: HIGH -- follows exact same `when` pattern as match arm guards
- Typeck changes: HIGH -- 6 lines of code in existing function, Bool unification is standard
- Codegen changes: HIGH -- conditional branch is the simplest LLVM construct, five-block pattern is well-understood
- Formatter changes: HIGH -- 4 lines following established WHEN_KW formatting pattern
- Integration (nesting/closures/pipes): MEDIUM -- requires explicit testing but infrastructure supports it by design
- Pitfalls: HIGH -- identified from code inspection, each with concrete prevention strategy

**Research date:** 2026-02-09
**Valid until:** indefinite (codebase-specific research, not library-version-dependent)
