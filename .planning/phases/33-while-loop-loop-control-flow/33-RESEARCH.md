# Phase 33: While Loop + Loop Control Flow - Research

**Researched:** 2026-02-08
**Domain:** While loop syntax, break/continue control flow, reduction checks at loop back-edges, LLVM basic block structure for loops
**Confidence:** HIGH

## Summary

This phase introduces while loops, break, and continue to the Snow language. The implementation touches every layer of the compiler pipeline: lexer (3 new keywords), parser (new CST node kinds and expression parsing), type checker (new expression inference, loop context tracking for break/continue validation), MIR (new expression variants), LLVM codegen (loop basic block structure with back-edge reduction checks), formatter, and LSP.

The existing codebase provides clear patterns to follow. The if-expression implementation (`if cond do body end` -> `IF_EXPR` CST node -> `IfExpr` AST wrapper -> `infer_if` in typeck -> `MirExpr::If` -> `codegen_if` with alloca+branch+merge) establishes the exact pattern for while loops. The while loop is structurally simpler than if-expression (no else branch) but introduces new concepts: back-edge jumps, loop context tracking for break/continue, and closure boundary detection.

The runtime already has `snow_reduction_check()` callable from codegen, currently emitted after function/closure calls. For while loops, the same function must be called at the loop's back-edge (the unconditional branch from end-of-body back to the condition check). This ensures tight loops without function calls still yield to the actor scheduler. The `emit_reduction_check()` helper in `CodeGen` already exists and handles the "only emit if block is not terminated" guard.

**Primary recommendation:** Follow the if-expression pipeline pattern exactly. Add `while`/`break`/`continue` as new keywords in `TokenKind`, new CST node kinds (`WHILE_EXPR`, `BREAK_EXPR`, `CONTINUE_EXPR`), new AST wrappers, new typeck inference with loop-depth tracking, new `MirExpr` variants (`While`, `Break`, `Continue`), and LLVM codegen using a three-block structure (cond_check / body / merge) with `snow_reduction_check()` at the back-edge.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| inkwell | 0.8.0 | LLVM basic blocks, conditional branch, alloca for loop state | Already in workspace; provides safe LLVM wrappers |
| snow-common | workspace | TokenKind, Span | Keyword definitions live here |
| snow-parser | workspace | CST node kinds, AST wrappers | Parser changes for while/break/continue syntax |
| snow-typeck | workspace | Type inference for while loops, loop-depth tracking | Break/continue validation |
| snow-codegen | workspace | MIR lowering and LLVM IR emission | Loop codegen |
| snow-fmt | workspace | Formatter support for new syntax | Walk while/break/continue nodes |
| snow-rt | workspace | `snow_reduction_check()` | Already exists; called at back-edges |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| ariadne | 0.6 | Diagnostic rendering for break/continue errors | Already in workspace |
| insta | 1.46 | Snapshot tests for parser, MIR | Already in workspace |

### Not Needed
No new external dependencies are required. Everything is internal to the existing workspace.

## Architecture Patterns

### Pipeline Flow for While Loop

```
Source: while x > 0 do x = x - 1 end

1. Lexer:    [While, Ident(x), Gt, IntLit(0), Do, ..., End]
2. Parser:   WHILE_EXPR { condition: BinaryExpr, body: Block }
3. AST:      WhileExpr { condition(), body() }
4. Typeck:   infer_while -> condition must be Bool, body is Unit, result is Unit
5. MIR:      MirExpr::While { cond, body, ty: Unit }
6. Codegen:  cond_bb -> body_bb -> (reduction_check + br cond_bb) | merge_bb
```

### LLVM Basic Block Structure for While

```
entry:
  br cond_check

cond_check:                          ; <-- back-edge target
  %cond = <codegen condition>
  br i1 %cond, body, merge

body:
  <codegen body expressions>
  call void @snow_reduction_check()  ; <-- reduction check at back-edge
  br cond_check                      ; <-- the back-edge

merge:
  <continue with rest of function>
  ; while returns Unit (empty struct)
```

### LLVM Basic Block Structure for Break/Continue

```
cond_check:
  %cond = <codegen condition>
  br i1 %cond, body, merge

body:
  <codegen body, which may contain:>
  ; break  -> br merge
  ; continue -> br cond_check (with reduction check first)
  call void @snow_reduction_check()
  br cond_check

merge:
  ; while expression result = Unit
```

### Pattern: Loop Context Stack for Break/Continue Validation

The type checker needs a loop context stack to:
1. Know whether break/continue are inside a loop (BRKC-04)
2. Know whether a closure boundary was crossed (BRKC-05)
3. Know the target basic blocks for codegen (codegen needs to know which `cond_check` and `merge` to jump to)

**Typeck approach:** Add a `loop_depth: u32` and `closure_depth_at_loop: Vec<u32>` to the inference context (or pass as parameters). When entering a while loop body, increment loop_depth. When entering a closure, increment closure_depth. Break/continue are valid only when `loop_depth > 0` AND the closure_depth hasn't increased since the enclosing loop.

**Simpler approach (recommended):** Pass a `loop_depth: u32` parameter through the inference functions. When entering a while body, pass `loop_depth + 1`. When entering a closure, reset to 0. Break/continue check `loop_depth > 0`.

**Codegen approach:** Maintain a stack of `(cond_bb, merge_bb)` pairs. Push when entering a while loop, pop when exiting. Break jumps to top-of-stack merge_bb. Continue jumps to top-of-stack cond_bb (with reduction check).

### Pattern: New Keyword Addition

Based on existing pattern (45 keywords currently):

```
1. snow-common/src/token.rs:
   - Add While, Break, Continue to TokenKind enum
   - Add "while" => Some(TokenKind::While), etc. to keyword_from_str
   - Update keyword count in tests (45 -> 48)

2. snow-parser/src/syntax_kind.rs:
   - Add WHILE_KW, BREAK_KW, CONTINUE_KW to SyntaxKind
   - Add WHILE_EXPR, BREAK_EXPR, CONTINUE_EXPR node kinds
   - Add mapping in From<TokenKind> impl
   - Update test counts

3. snow-lexer: No changes needed (lex_ident already delegates to keyword_from_str)
```

### Pattern: New Expression Addition

Based on the if-expression pattern:

```
1. Parser (expressions.rs):
   - Add SyntaxKind::WHILE_KW => Some(parse_while_expr(p)) to lhs()
   - Add SyntaxKind::BREAK_KW => Some(parse_break_expr(p)) to lhs()
   - Add SyntaxKind::CONTINUE_KW => Some(parse_continue_expr(p)) to lhs()

2. AST (ast/expr.rs):
   - Add WhileExpr, BreakExpr, ContinueExpr variants to Expr enum
   - Add ast_node! macros and accessor methods
   - Add cast arms in Expr::cast()
   - Add syntax() arms in Expr::syntax()

3. Typeck (infer.rs):
   - Add Expr::WhileExpr => infer_while(...)
   - Add Expr::BreakExpr => infer_break(...)
   - Add Expr::ContinueExpr => infer_continue(...)

4. MIR (mir/mod.rs):
   - Add MirExpr::While { cond, body, ty }
   - Add MirExpr::Break
   - Add MirExpr::Continue

5. MIR lowering (mir/lower.rs):
   - Add Expr::WhileExpr => lower_while_expr(...)
   - Add Expr::BreakExpr => lower_break()
   - Add Expr::ContinueExpr => lower_continue()

6. Codegen (codegen/expr.rs):
   - Add MirExpr::While => codegen_while(...)
   - Add MirExpr::Break => codegen_break()
   - Add MirExpr::Continue => codegen_continue()
```

### Anti-Patterns to Avoid

- **Do NOT use phi nodes for the while loop result:** While returns Unit, so there is no value to merge. Use the alloca pattern only if while-with-value is added later (deferred to LOOP-01).
- **Do NOT check break/continue validity in the parser:** The parser should accept them anywhere (producing a valid CST) and the type checker should emit the errors. This matches Snow's pattern where semantic errors come from typeck, not the parser.
- **Do NOT insert reduction checks inside the condition evaluation:** Only at the back-edge (unconditional branch from body end to condition check). Checking inside the condition would cause double-yields per iteration.
- **Do NOT hand-roll loop detection for closure boundaries:** Use the existing scope-pushing pattern. When inferring a closure body, reset the loop context to zero.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Reduction counting | Custom per-loop counter | `snow_reduction_check()` | Already handles thread-local counter, coroutine yielding, and no-op when not in actor context |
| Basic block management | Manual block insertion | Inkwell `context.append_basic_block()` | Existing pattern in `codegen_if` shows exactly how |
| Error diagnostics | Custom error rendering | `TypeError` variants + ariadne | Existing diagnostic pipeline handles span-based errors |

## Common Pitfalls

### Pitfall 1: Break/Continue Without Loop Context in Codegen
**What goes wrong:** Codegen encounters `MirExpr::Break` but has no target basic block to jump to.
**Why it happens:** The MIR lowering doesn't prevent break/continue outside loops; that validation happens in typeck. But if typeck has a bug, codegen crashes.
**How to avoid:** The typeck must validate break/continue context, AND codegen should have a defensive check (return error, not panic). The codegen should maintain a loop context stack and return `Err("break outside loop")` if the stack is empty.
**Warning signs:** Panic in codegen when processing break/continue.

### Pitfall 2: Unterminated Blocks After Break/Continue
**What goes wrong:** After `break` or `continue`, the LLVM block has a terminator (unconditional branch). But the while loop body codegen continues to emit instructions, which LLVM rejects (instructions after terminator).
**Why it happens:** Break/continue emit `br` instructions. Any code after them in the same block is unreachable.
**How to avoid:** After emitting a break or continue, check if the current block has a terminator before emitting more code. The existing `codegen_if` already uses this pattern: `if bb.get_terminator().is_none() { build_branch(...) }`.
**Warning signs:** LLVM verification failure with "terminator in middle of basic block."

### Pitfall 3: Closure Boundary for Break/Continue
**What goes wrong:** `while true do fn -> break end end` -- break inside a closure inside a loop. The break would try to jump to the loop's merge block, but the closure is a separate LLVM function. Cross-function jumps are illegal in LLVM IR.
**Why it happens:** Closures are lifted to top-level functions in MIR. Break/continue refer to LLVM basic blocks that exist only in the enclosing function.
**How to avoid:** The typeck MUST reject break/continue inside closures within loops (BRKC-05). This is the only correct solution -- there is no reasonable way to make cross-function jumps work.
**Warning signs:** LLVM segfault or verification failure.

### Pitfall 4: Infinite Loop Without Reduction Check
**What goes wrong:** `while true do end` -- an empty body with no function calls. Without a reduction check at the back-edge, this loop runs forever and starves the actor scheduler.
**Why it happens:** The existing reduction checks are only emitted after function calls. An empty while body has no function calls.
**How to avoid:** Always emit `snow_reduction_check()` at the while loop's back-edge, regardless of whether the body contains function calls.
**Warning signs:** Actor system hangs when one actor runs a tight loop.

### Pitfall 5: While Loop Type Must Be Unit
**What goes wrong:** Type inference assigns the body's type to the while expression, causing type errors when the body isn't Unit.
**Why it happens:** Copy-pasting from if-expression pattern where the type is the branch type.
**How to avoid:** WHILE-03 specifies that while loops return Unit. The typeck must always return `Ty::Tuple(vec![])` (Unit) for while expressions, regardless of the body's type.
**Warning signs:** Type error when using while in a let-binding or as the last expression.

### Pitfall 6: parse_block_body Stops at Wrong Tokens
**What goes wrong:** The while loop body parser stops at `ELSE_KW` (which is a block terminator in `parse_block_body`), causing parse errors if the while body contains an if-else.
**Why it happens:** `parse_block_body` treats `ELSE_KW` as a block terminator because it's designed for if-expression blocks.
**How to avoid:** The while loop body should use `parse_block_body` as-is, because the `ELSE_KW` terminator only applies to the innermost block. An `if` inside a while body creates its own block scope, so the `else` is consumed by the inner if-expression's parsing, not by the while's block. This is actually fine -- the existing `parse_block_body` already handles nested if-else correctly because the if parser consumes the else before the block parser sees it.
**Warning signs:** None expected -- this is a false alarm worth verifying during implementation.

## Code Examples

### Parser: parse_while_expr (modeled on parse_if_expr)

```rust
// Source: existing parse_if_expr in expressions.rs
fn parse_while_expr(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // WHILE_KW

    // Parse condition expression
    expr(p);

    // Expect `do`
    let do_span = p.current_span();
    p.expect(SyntaxKind::DO_KW);

    // Parse body block
    parse_block_body(p);

    // Expect `end`
    if !p.at(SyntaxKind::END_KW) {
        p.error_with_related(
            "expected `end` to close `while` block",
            do_span,
            "`do` block started here",
        );
    } else {
        p.advance(); // END_KW
    }

    p.close(m, SyntaxKind::WHILE_EXPR)
}
```

### Parser: parse_break_expr and parse_continue_expr

```rust
fn parse_break_expr(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // BREAK_KW
    p.close(m, SyntaxKind::BREAK_EXPR)
}

fn parse_continue_expr(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // CONTINUE_KW
    p.close(m, SyntaxKind::CONTINUE_EXPR)
}
```

### Typeck: infer_while with loop context

```rust
fn infer_while(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    while_: &WhileExpr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
    loop_depth: u32,  // <-- new parameter threaded through
) -> Result<Ty, TypeError> {
    // Condition must be Bool
    if let Some(cond) = while_.condition() {
        let cond_ty = infer_expr_with_loop(ctx, env, &cond, types, type_registry,
            trait_registry, fn_constraints, loop_depth)?;
        ctx.unify(cond_ty, Ty::bool(), ConstraintOrigin::Builtin)?;
    }

    // Infer body with incremented loop depth
    if let Some(body) = while_.body() {
        infer_block_with_loop(ctx, env, &body, types, type_registry,
            trait_registry, fn_constraints, loop_depth + 1)?;
    }

    // While always returns Unit
    Ok(Ty::Tuple(vec![]))
}
```

### Typeck: break/continue validation

```rust
fn infer_break(
    ctx: &mut InferCtx,
    break_: &BreakExpr,
    loop_depth: u32,
) -> Result<Ty, TypeError> {
    if loop_depth == 0 {
        ctx.errors.push(TypeError::BreakOutsideLoop {
            span: break_.syntax().text_range(),
        });
    }
    // break has type Never (already exists as Ty::Never, unifies with anything)
    Ok(Ty::Never)
}
```

### MIR: New expression variants

```rust
// In MirExpr enum:
/// While loop: evaluates condition, if true executes body and repeats.
While {
    cond: Box<MirExpr>,
    body: Box<MirExpr>,
    ty: MirType, // Always MirType::Unit
},
/// Break: exit the innermost enclosing loop.
Break,
/// Continue: skip to the next iteration of the innermost enclosing loop.
Continue,
```

### Codegen: codegen_while (the core LLVM emission)

```rust
fn codegen_while(
    &mut self,
    cond: &MirExpr,
    body: &MirExpr,
    ty: &MirType,
) -> Result<BasicValueEnum<'ctx>, String> {
    let fn_val = self.current_function();

    let cond_bb = self.context.append_basic_block(fn_val, "while_cond");
    let body_bb = self.context.append_basic_block(fn_val, "while_body");
    let merge_bb = self.context.append_basic_block(fn_val, "while_merge");

    // Push loop context for break/continue
    self.loop_stack.push((cond_bb, merge_bb));

    // Branch to condition check
    self.builder.build_unconditional_branch(cond_bb)
        .map_err(|e| e.to_string())?;

    // Condition block
    self.builder.position_at_end(cond_bb);
    let cond_val = self.codegen_expr(cond)?.into_int_value();
    self.builder.build_conditional_branch(cond_val, body_bb, merge_bb)
        .map_err(|e| e.to_string())?;

    // Body block
    self.builder.position_at_end(body_bb);
    self.codegen_expr(body)?;

    // Back-edge: reduction check + jump back to condition
    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
        self.emit_reduction_check();
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| e.to_string())?;
    }

    // Pop loop context
    self.loop_stack.pop();

    // Merge block -- while returns Unit
    self.builder.position_at_end(merge_bb);
    Ok(self.context.struct_type(&[], false).const_zero().into())
}
```

### Codegen: codegen_break and codegen_continue

```rust
fn codegen_break(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
    let (_, merge_bb) = self.loop_stack.last()
        .ok_or("break outside loop (should have been caught by typeck)")?;
    let merge_bb = *merge_bb;
    self.builder.build_unconditional_branch(merge_bb)
        .map_err(|e| e.to_string())?;
    // Return a dummy value -- this code is unreachable
    Ok(self.context.struct_type(&[], false).const_zero().into())
}

fn codegen_continue(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
    let (cond_bb, _) = self.loop_stack.last()
        .ok_or("continue outside loop (should have been caught by typeck)")?;
    let cond_bb = *cond_bb;
    self.emit_reduction_check();
    self.builder.build_unconditional_branch(cond_bb)
        .map_err(|e| e.to_string())?;
    // Return a dummy value -- this code is unreachable
    Ok(self.context.struct_type(&[], false).const_zero().into())
}
```

### Formatter: walk_while_expr

```rust
// Modeled on walk_if_expr in walker.rs
fn walk_while_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    // while <cond> do
    //   <body>
    // end
    // Similar structure to walk_if_expr but simpler (no else branch)
    for child in node.children_with_tokens() {
        match child.kind() {
            SyntaxKind::WHILE_KW => parts.push(text("while")),
            SyntaxKind::DO_KW => { parts.push(sp()); parts.push(text("do")); }
            SyntaxKind::END_KW => { parts.push(nl()); parts.push(text("end")); }
            SyntaxKind::BLOCK => {
                parts.push(indent(walk_node(&child.into_node().unwrap())));
            }
            k if !k.is_trivia() => {
                if let Some(n) = child.as_node() {
                    parts.push(sp());
                    parts.push(walk_node(n));
                }
            }
            _ => {}
        }
    }
    group(parts)
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Recursive busy_loop for iteration | while loop (this phase) | Phase 33 | Users no longer need recursive workarounds for loops |
| Reduction checks only after calls | Reduction checks at loop back-edges too | Phase 33 | Tight loops without calls no longer starve scheduler |

**No deprecation needed:** This phase adds new features without changing existing behavior.

## Key Design Decisions

### 1. While Returns Unit (WHILE-03)
The while loop always returns Unit type. This is simpler than having it return the last body expression value (which would require tracking "what if the body never executes?"). Deferred requirement LOOP-01 would add `break value` for while-with-value semantics -- that is explicitly out of scope.

### 2. Break/Continue Have Type Never
Break and continue transfer control flow and never produce a value. They have type `Ty::Never` in the type system, which already unifies with any type (verified: `unify.rs` line 271 has `(Ty::Never, _) | (_, Ty::Never) => Ok(())`). This allows patterns like `if cond do break end` where the if-expression's then-branch is Never. The `return` expression already uses this pattern.

### 3. Loop Depth via Parameter Threading (not struct field)
Rather than adding a `loop_depth` field to `InferCtx` (which would require mutable access patterns different from the current code), thread `loop_depth` as a parameter through `infer_expr`, `infer_block`, etc. When entering a closure body, pass `loop_depth = 0` to reset the context. This approach is non-invasive -- it changes function signatures but not the overall architecture.

**Alternative considered:** Adding `loop_depth` to `InferCtx`. This would require fewer signature changes but mixes loop state into the unification context, which is conceptually wrong (loop depth is syntactic, not type-algebraic). The parameter approach is cleaner.

**Note on invasiveness:** The `infer_expr` function has many callers. Adding a `loop_depth` parameter means updating every call site. However, every call site currently passes 0 (not in a loop), and only `infer_while` passes a non-zero value. The alternative of using a side-channel (thread-local, struct field) would be less explicit.

### 4. Codegen Loop Stack (new field on CodeGen)
Add a `loop_stack: Vec<(BasicBlock, BasicBlock)>` field to the `CodeGen` struct. Each entry is `(cond_bb, merge_bb)`. Push when entering a while loop, pop when exiting. Break/continue read the top of the stack. This is the standard approach used by all LLVM-based compilers for nested loops.

### 5. Reduction Check Placement
The `snow_reduction_check()` call goes at the back-edge (just before `br cond_bb` in the body block). This ensures:
- Every iteration does exactly one reduction check
- The check happens even if the body is empty (`while true do end`)
- Continue also emits a reduction check before jumping to cond (since it is also a back-edge)

## Resolved Questions

1. **Never type in the type system** -- RESOLVED
   `Ty::Never` already exists in `snow-typeck/src/ty.rs` (line 59). It already unifies with any type via special-case in `unify.rs` line 271: `(Ty::Never, _) | (_, Ty::Never) => Ok(())`. The `return` expression already returns `Ty::Never`. Break and continue should do the same. No new type system changes needed.

2. **MirType::Never for break/continue** -- RESOLVED
   `MirType::Never` already exists in `snow-codegen/src/mir/mod.rs` (line 85). The `MirExpr::Return` already returns `&MirType::Never` from its `ty()` method. The MIR type resolution in `resolve_type()` handles `Ty::Never` -> `MirType::Never`. Break and continue can use the same pattern.

## Open Questions

1. **Threading loop_depth through all infer_expr callers**
   - What we know: `infer_expr` is called from many places. Adding a parameter is invasive but explicit.
   - What's unclear: Exact number of call sites that need updating.
   - Recommendation: Start with a default `loop_depth = 0` wrapper function (`infer_expr` calls `infer_expr_with_loop(... 0)`) to minimize churn. Only the while body path and recursive calls need the new parameter. This wrapper approach keeps backward compatibility.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `snow-common/src/token.rs` (TokenKind enum, keyword_from_str -- 45 keywords, no while/break/continue)
- Codebase analysis: `snow-parser/src/syntax_kind.rs` (SyntaxKind enum, 80+ node kinds)
- Codebase analysis: `snow-parser/src/ast/expr.rs` (Expr enum with 21 variants, IfExpr pattern)
- Codebase analysis: `snow-parser/src/parser/expressions.rs` (parse_if_expr lines 557-600, parse_block_body lines 445-488)
- Codebase analysis: `snow-typeck/src/infer.rs` (infer_if lines 3053-3096, loop_depth not yet present)
- Codebase analysis: `snow-typeck/src/ty.rs` (Ty::Never exists at line 59)
- Codebase analysis: `snow-typeck/src/unify.rs` (Never unifies with anything at line 271)
- Codebase analysis: `snow-codegen/src/mir/mod.rs` (MirExpr enum, MirType::Never at line 85)
- Codebase analysis: `snow-codegen/src/mir/lower.rs` (Lowerer struct with 158 lines of state, lower_if_expr at line 3916)
- Codebase analysis: `snow-codegen/src/codegen/expr.rs` (codegen_if lines 856-913, emit_reduction_check lines 1657-1663)
- Codebase analysis: `snow-rt/src/actor/mod.rs` (snow_reduction_check at line 160, thread-local counter)
- Codebase analysis: `snow-fmt/src/walker.rs` (walk_if_expr at line 311)

### Secondary (MEDIUM confidence)
- LLVM Language Reference: loop structure with basic blocks (standard knowledge, verified against codegen_if pattern)
- BEAM/Erlang reduction counting model (matches existing snow_reduction_check design)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - all libraries already in workspace, no new dependencies
- Architecture: HIGH - follows established if-expression pattern exactly, all pipeline stages inspected
- Pitfalls: HIGH - each pitfall identified from actual code patterns in the codebase
- Code examples: HIGH - modeled on actual existing code (codegen_if, parse_if_expr, infer_if)

**Research date:** 2026-02-08
**Valid until:** indefinite (codebase-specific research, not library-version-dependent)
