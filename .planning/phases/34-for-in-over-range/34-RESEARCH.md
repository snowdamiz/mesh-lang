# Phase 34: For-In over Range - Research

**Researched:** 2026-02-08
**Domain:** For-in loop syntax, range iteration, zero-allocation integer loop codegen, loop variable scoping, MIR desugaring
**Confidence:** HIGH

## Summary

This phase adds `for i in 0..10 do body end` syntax to the Snow language. The core challenge is implementing this with zero heap allocation for the range counter (FORIN-07), which means the for-in loop over a range must desugar into pure integer arithmetic -- an alloca counter that increments from start to end, never creating a runtime Range object.

The existing codebase provides all the building blocks. Phase 33 established the loop infrastructure: `loop_stack` on `CodeGen` for break/continue targets, `emit_reduction_check()` at back-edges, `loop_depth` tracking on `InferCtx` for break/continue validation, and the `MirExpr::While`/`Break`/`Continue` variants. The `for` and `in` keywords already exist as `TokenKind::For` and `TokenKind::In` (added in the original 48-keyword set). The `..` operator already parses as a `BinaryExpr` with `DOT_DOT` operator at binding power 13/14. The `MirExpr::Let` pattern already handles scoped variable bindings with save/restore of previous bindings in codegen. All the mechanical pieces exist -- this phase wires them together.

The recommended approach is a MIR desugaring strategy: the parser produces a `FOR_IN_EXPR` CST node, the AST provides a `ForInExpr` wrapper with accessors for binding name, iterable, and body, the typeck infers the range's element type and scopes the loop variable, but at the MIR level the for-in is desugared into `MirExpr::ForInRange` (a dedicated MIR variant) rather than desugaring all the way to `While`. This preserves the semantic information (start, end, binding name, body) through to codegen, where a dedicated `codegen_for_in_range` function emits the optimal four-block LLVM structure (header/body/latch/merge) with the counter as an alloca.

**Primary recommendation:** Add `FOR_IN_EXPR` to parser, `ForInExpr` to AST, desugar in typeck (check range bounds are Int, scope loop var to body), add `MirExpr::ForInRange` to MIR, and emit a four-block LLVM loop in codegen with an alloca counter -- no Range heap allocation, no runtime calls.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| inkwell | 0.8.0 | LLVM alloca, basic blocks, conditional/unconditional branches | Already in workspace; used for while loops |
| snow-common | workspace | TokenKind::For, TokenKind::In, TokenKind::DotDot already exist | No new keywords needed |
| snow-parser | workspace | New FOR_IN_EXPR CST node kind, parser function | Extends existing Pratt parser |
| snow-typeck | workspace | Type inference for for-in, loop_depth reuse from Phase 33 | enter_loop/exit_loop already exist |
| snow-codegen | workspace | MIR variant, MIR lowering, LLVM codegen | loop_stack reuse from Phase 33 |
| snow-fmt | workspace | Formatter support for for-in syntax | walk_for_in_expr |

### Not Needed
No new external dependencies. No runtime library changes needed (the zero-allocation design specifically avoids `snow_range_new`).

## Architecture Patterns

### Pipeline Flow for For-In over Range

```
Source: for i in 0..10 do println(i) end

1. Lexer:    [For, Ident(i), In, IntLit(0), DotDot, IntLit(10), Do, ..., End]
2. Parser:   FOR_IN_EXPR { binding: NAME, iterable: BINARY_EXPR(0..10), body: BLOCK }
3. AST:      ForInExpr { binding_name(), iterable(), body() }
4. Typeck:   - Iterable is BinaryExpr with DotDot op: check both sides are Int
             - Bind loop var `i` as Int in body scope, scoped to body only
             - Loop result type: Unit (for Phase 34 -- Phase 35 adds List<T>)
             - enter_loop()/exit_loop() for break/continue
5. MIR:      MirExpr::ForInRange { var: "i", start: 0, end: 10, body: ..., ty: Unit }
6. Codegen:  alloca %i; header: %i_val < end? -> body/merge; body: ...; latch: %i++ + reduction; merge
```

### LLVM Basic Block Structure for For-In Range

```
entry:
  %start = <codegen start_expr>       ; e.g., 0
  %end = <codegen end_expr>           ; e.g., 10
  %i = alloca i64                     ; loop counter (NO heap allocation)
  store i64 %start, %i
  br forin_header

forin_header:                          ; <-- break targets merge, continue targets latch
  %i_val = load i64, %i
  %cond = icmp slt i64 %i_val, %end   ; i < end (half-open range)
  br i1 %cond, forin_body, forin_merge

forin_body:
  ; Loop body with %i_val available as the loop variable
  <codegen body>
  ; If not terminated by break/continue:
  br forin_latch

forin_latch:                           ; <-- continue jumps here (not header)
  %i_next = load i64, %i
  %i_inc = add i64 %i_next, 1
  store i64 %i_inc, %i
  call void @snow_reduction_check()   ; reduction check at back-edge
  br forin_header                      ; the back-edge

forin_merge:
  ; for-in returns Unit
```

**Key difference from while loop:** Four blocks instead of three (header/body/latch/merge vs cond/body/merge). The extra latch block is critical because `continue` must jump to the latch (which increments the counter) NOT the header (which would re-check the condition with the same counter value, causing an infinite loop). This is the standard loop structure used by LLVM's own loop optimizations.

### Pattern: MIR Desugaring vs Direct Codegen

Two options were considered:

1. **Full desugaring to MirExpr::While** -- Transform `for i in 0..10 do body end` into `Let { i_counter = 0, While { cond: i_counter < 10, body: Let { i = i_counter, Block[body, i_counter = i_counter + 1] } } }` at the MIR level.
   - Pro: Reuses existing While codegen completely
   - Con: Loses semantic information, makes `continue` handling complex (need to ensure increment happens before jumping), complicates the MIR output for debugging

2. **Dedicated MirExpr::ForInRange** (RECOMMENDED) -- Keep the for-in as a first-class MIR node with `var`, `start`, `end`, `body`, and emit the four-block structure directly in codegen.
   - Pro: Clean semantic preservation, correct continue behavior (jumps to latch), clear codegen, easier to extend for Phase 35
   - Con: New MIR variant and codegen function (but minimal -- follows While pattern exactly)

The dedicated variant is recommended because it naturally handles the continue-targets-latch requirement without any contortions.

### Pattern: Loop Variable Scoping (FORIN-08)

The loop variable must be:
1. **Scoped to the body** -- not visible after `end`
2. **Fresh per iteration** -- each iteration gets its own binding

This is achieved naturally through the existing scoping patterns:

**In typeck:** Push a new scope before inferring the body, bind `i: Int` in that scope, pop after. The variable `i` is not visible after `end`.

**In MIR lowering:** The for-in body is lowered with the loop variable added to the scope. `MirExpr::ForInRange` wraps the body -- the variable is referenced by name inside the body.

**In codegen:** The counter alloca is registered in `self.locals` before codegen-ing the body, and removed/restored after. Each iteration loads the current counter value into the loop variable alloca. The body sees a consistent value per iteration (load from the counter alloca at the start of each iteration body, stored into a separate loop-variable alloca).

Actually, for zero-allocation, the simplest approach is: the loop counter IS the loop variable alloca. At the start of each body execution, the counter holds the current iteration value. The body reads from it. At the latch, it gets incremented. This is clean because:
- The body sees `i` as the current value
- If the body mutates `i` (which Snow does not currently support for loop variables), it would be overwritten at the latch anyway
- The alloca+mem2reg pattern will promote this to a phi node at -O1+

### Pattern: Reusing Loop Infrastructure from Phase 33

The for-in loop reuses ALL of Phase 33's loop infrastructure:

| Feature | How Reused |
|---------|------------|
| `loop_stack` on CodeGen | Push `(forin_header, forin_merge)` -- break jumps to merge |
| `emit_reduction_check()` | Called at forin_latch (the back-edge) |
| `ctx.enter_loop()` / `ctx.exit_loop()` | Used in typeck for break/continue validation |
| `MirExpr::Break` | Unchanged -- codegen_break reads from loop_stack.last().merge_bb |
| `MirExpr::Continue` | Changed -- continue must jump to LATCH, not HEADER |

**CRITICAL: Continue target for for-in differs from while.** In a while loop, continue jumps to `cond_bb` (the condition check). In a for-in range loop, continue must jump to the latch (which increments the counter then jumps to header). The `loop_stack` entry format needs to change to accommodate this. Two options:

1. Change `loop_stack` from `Vec<(cond_bb, merge_bb)>` to `Vec<(continue_target_bb, merge_bb)>` -- for while loops, continue_target = cond_bb; for for-in, continue_target = latch_bb.
2. Add a separate `continue_target` to the tuple: `Vec<(cond_bb, merge_bb, continue_bb)>`.

Option 1 is cleaner: rename the first field to "continue_target" semantically. The while loop sets continue_target = cond_bb, the for-in loop sets continue_target = latch_bb. The `codegen_continue` function already jumps to the first element of the tuple, so no code changes needed in `codegen_continue` -- just the semantic meaning changes.

Wait -- reviewing Phase 33's codegen_continue: it jumps to `cond_bb` (first element). And codegen_break jumps to `merge_bb` (second element). So if we make the first element the "continue target", while's continues would still go to cond_bb and for-in's continues would go to latch_bb. This is exactly right. The only change needed is that for-in pushes `(latch_bb, merge_bb)` instead of `(cond_bb, merge_bb)`.

### Anti-Patterns to Avoid

- **Do NOT create a Range object:** `for i in 0..10` must NOT call `snow_range_new`. The `..` in this context is recognized at MIR lowering time as range syntax and desugared to start/end integers. This satisfies FORIN-07.
- **Do NOT desugar for-in to while in the MIR:** A direct while desugaring loses the natural continue-to-latch semantics. Keep ForInRange as a first-class MIR node.
- **Do NOT make continue jump to the header in for-in:** This would skip the increment, causing an infinite loop on `for i in 0..10 do continue end`.
- **Do NOT leak the loop variable:** `i` must not be visible after `end`. The codegen must save/restore `self.locals` entries.
- **Do NOT handle DotDot in infer_binary for this phase:** The `..` operator in typeck already falls to the `_ => fresh_var()` case. For Phase 34, the `..` in `for i in 0..10` is recognized by the for-in type inference, NOT by general binary expression inference. The `..` as a standalone expression (`let r = 0..10`) remains handled by `Range.new()` via the module system. This phase does NOT need to fix the general `..` operator.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Loop counter management | Manual phi nodes | alloca + mem2reg pattern | Consistent with existing if/while codegen; LLVM -O1 promotes to phis |
| Reduction counting | Per-loop counter | `emit_reduction_check()` | Already handles thread-local counter, coroutine yielding |
| Break/continue tracking | Separate for-in stack | Existing `loop_stack` | Same push/pop pattern, just different continue target |
| Variable scoping | Manual scope tracking | `locals.insert/remove` pattern from `codegen_let` | Existing save/restore pattern handles nested scopes |

## Common Pitfalls

### Pitfall 1: Continue in For-In Causes Infinite Loop
**What goes wrong:** `for i in 0..10 do continue end` enters an infinite loop because continue jumps back to the header without incrementing the counter.
**Why it happens:** Using the while loop's `(cond_bb, merge_bb)` pattern where continue targets the condition check.
**How to avoid:** Use a four-block structure with a separate latch block. For-in pushes `(latch_bb, merge_bb)` onto loop_stack. Continue jumps to latch (which increments then re-checks condition).
**Warning signs:** Programs with `continue` inside for-in loops hang or run forever.

### Pitfall 2: Loop Variable Leaks into Surrounding Scope
**What goes wrong:** After `for i in 0..10 do end`, the variable `i` is still accessible and holds value 10.
**Why it happens:** The loop variable alloca is registered in `self.locals` but not cleaned up.
**How to avoid:** Save the previous value of `self.locals[var_name]` before the loop, restore it after. Follow the `codegen_let` pattern exactly: `let old = self.locals.insert(name, alloca); ... self.locals.remove/restore`.
**Warning signs:** Tests showing the loop variable is accessible after `end`, or tests where a variable named `i` from an outer scope is clobbered.

### Pitfall 3: DotDot Operator Recognized Outside For-In Context
**What goes wrong:** `let r = 0..10` no longer creates a Range because the parser/lowerer now treats `..` specially.
**Why it happens:** Over-eagerly handling `..` at the binary expression level instead of only in the for-in context.
**How to avoid:** Only recognize the `0..10` range pattern INSIDE `ForInExpr`'s iterable. The general `BinaryExpr` with `DotDot` operator continues to be handled as before (currently falls to fresh_var in typeck, used via `Range.new()` in practice). The for-in parser captures the `..` expression as the iterable, and the MIR lowerer extracts start/end from the binary expression when it sees a for-in with a DotDot iterable.
**Warning signs:** Existing Range module tests break.

### Pitfall 4: Off-by-One in Range Bounds
**What goes wrong:** `for i in 0..10` iterates 11 times instead of 10 (or 9 instead of 10).
**Why it happens:** Using `<=` instead of `<` for the condition, or starting at 1 instead of 0.
**How to avoid:** Use `icmp slt` (signed less-than, NOT less-than-or-equal). The range is half-open: `[start, end)` -- this matches the existing `snow_range_new` semantics where `Range.new(1, 4)` has length 3.
**Warning signs:** Loop body executes wrong number of times.

### Pitfall 5: Break After Body Expressions in For-In
**What goes wrong:** After `break` or `continue` in the body, subsequent expressions emit code into a terminated basic block, causing LLVM verification failure.
**Why it happens:** The body codegen continues past break/continue.
**How to avoid:** After codegen-ing the body, check `get_terminator().is_none()` before emitting the branch to latch. This is the exact same pattern used in `codegen_while`. The body may contain `break` or `continue` which terminate the block early.
**Warning signs:** LLVM verification error "Instruction does not dominate all uses" or "terminator in middle of block."

### Pitfall 6: Reduction Check Placement
**What goes wrong:** The reduction check is in the body instead of the latch, causing `continue` to skip the check.
**Why it happens:** Copy-pasting from while loop where the reduction check is at the end of the body.
**How to avoid:** Place the `emit_reduction_check()` call in the latch block, not the body block. The latch is always reached (both from body fall-through and from continue), so the reduction check happens every iteration.
**Warning signs:** Actor scheduler starvation in tight for-in loops with continue.

## Code Examples

### Parser: parse_for_in_expr

```rust
// Source: modeled on parse_while_expr
fn parse_for_in_expr(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // FOR_KW

    // Parse binding name (just an identifier)
    if p.at(SyntaxKind::IDENT) {
        let name_m = p.open();
        p.advance(); // IDENT
        p.close(name_m, SyntaxKind::NAME);
    } else {
        p.error("expected loop variable name after `for`");
    }

    // Expect `in`
    p.expect(SyntaxKind::IN_KW);

    // Parse iterable expression (e.g., 0..10, or a variable)
    expr(p);

    // Expect `do`
    let do_span = p.current_span();
    p.expect(SyntaxKind::DO_KW);

    // Parse body block
    parse_block_body(p);

    // Expect `end`
    if !p.at(SyntaxKind::END_KW) {
        p.error_with_related(
            "expected `end` to close `for` block",
            do_span,
            "`do` block started here",
        );
    } else {
        p.advance(); // END_KW
    }

    p.close(m, SyntaxKind::FOR_IN_EXPR)
}
```

### AST: ForInExpr

```rust
ast_node!(ForInExpr, FOR_IN_EXPR);

impl ForInExpr {
    /// The loop variable name (NAME child).
    pub fn binding_name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The iterable expression (e.g., 0..10).
    pub fn iterable(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The loop body block.
    pub fn body(&self) -> Option<Block> {
        child_node(&self.syntax)
    }
}
```

### Typeck: infer_for_in

```rust
fn infer_for_in(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    for_in: &ForInExpr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    // Infer the iterable expression.
    if let Some(iterable) = for_in.iterable() {
        let iter_ty = infer_expr(ctx, env, &iterable, types,
            type_registry, trait_registry, fn_constraints)?;

        // For Phase 34: the iterable must be a range (BinaryExpr with DotDot).
        // Check that both sides of the range are Int.
        // (The iterable type itself may be a fresh var from the _ => fresh_var() case;
        // we validate the range structure in the iterable expression instead.)
        if let Expr::BinaryExpr(bin) = &iterable {
            if let Some(op) = bin.op() {
                if op.kind() == SyntaxKind::DOT_DOT {
                    // Both sides must be Int.
                    if let Some(lhs) = bin.lhs() {
                        let lhs_ty = infer_expr(ctx, env, &lhs, types,
                            type_registry, trait_registry, fn_constraints)?;
                        ctx.unify(lhs_ty, Ty::int(), ConstraintOrigin::Builtin)?;
                    }
                    if let Some(rhs) = bin.rhs() {
                        let rhs_ty = infer_expr(ctx, env, &rhs, types,
                            type_registry, trait_registry, fn_constraints)?;
                        ctx.unify(rhs_ty, Ty::int(), ConstraintOrigin::Builtin)?;
                    }
                }
            }
        }
    }

    // Bind loop variable in a new scope, infer body with loop depth.
    let var_name = for_in.binding_name()
        .and_then(|n| n.text())
        .unwrap_or_else(|| "_".to_string());

    let saved = env.push_scope();
    env.bind(var_name, Scheme::mono(Ty::int())); // loop var is Int for range iteration
    ctx.enter_loop();

    if let Some(body) = for_in.body() {
        let _ = infer_block(ctx, env, &body, types,
            type_registry, trait_registry, fn_constraints)?;
    }

    ctx.exit_loop();
    env.pop_scope(saved);

    // Phase 34: for-in over range returns Unit.
    // Phase 35 will change this to List<T>.
    Ok(Ty::Tuple(vec![]))
}
```

### MIR: ForInRange variant

```rust
// In MirExpr enum, in the Loop primitives section:

/// For-in loop over an integer range: `for var in start..end do body end`.
/// Desugared to integer counter iteration with no heap allocation.
ForInRange {
    /// Loop variable name.
    var: String,
    /// Start value (inclusive).
    start: Box<MirExpr>,
    /// End value (exclusive).
    end: Box<MirExpr>,
    /// Loop body.
    body: Box<MirExpr>,
    /// Result type (Unit for Phase 34).
    ty: MirType,
},
```

### MIR Lowering: lower_for_in_expr

```rust
fn lower_for_in_expr(&mut self, for_in: &ForInExpr) -> MirExpr {
    let var_name = for_in.binding_name()
        .and_then(|n| n.text())
        .unwrap_or_else(|| "_".to_string());

    // Extract start and end from the DotDot binary expression.
    let (start, end) = if let Some(iterable) = for_in.iterable() {
        if let Expr::BinaryExpr(bin) = &iterable {
            let start = bin.lhs()
                .map(|e| self.lower_expr(&e))
                .unwrap_or(MirExpr::IntLit(0, MirType::Int));
            let end = bin.rhs()
                .map(|e| self.lower_expr(&e))
                .unwrap_or(MirExpr::IntLit(0, MirType::Int));
            (start, end)
        } else {
            // Fallback: non-range iterable (Phase 35 will handle this)
            (MirExpr::IntLit(0, MirType::Int), MirExpr::IntLit(0, MirType::Int))
        }
    } else {
        (MirExpr::IntLit(0, MirType::Int), MirExpr::IntLit(0, MirType::Int))
    };

    // Lower body with loop variable in scope.
    self.push_scope();
    self.insert_var(var_name.clone(), MirType::Int);
    let body = for_in.body()
        .map(|b| self.lower_block(&b))
        .unwrap_or(MirExpr::Unit);
    self.pop_scope();

    MirExpr::ForInRange {
        var: var_name,
        start: Box::new(start),
        end: Box::new(end),
        body: Box::new(body),
        ty: MirType::Unit,
    }
}
```

### Codegen: codegen_for_in_range

```rust
fn codegen_for_in_range(
    &mut self,
    var: &str,
    start: &MirExpr,
    end: &MirExpr,
    body: &MirExpr,
    _ty: &MirType,
) -> Result<BasicValueEnum<'ctx>, String> {
    let fn_val = self.current_function();

    // Evaluate start and end expressions.
    let start_val = self.codegen_expr(start)?.into_int_value();
    let end_val = self.codegen_expr(end)?.into_int_value();

    // Alloca for the loop counter (zero heap allocation).
    let i64_ty = self.context.i64_type();
    let counter = self.builder.build_alloca(i64_ty, var)
        .map_err(|e| e.to_string())?;
    self.builder.build_store(counter, start_val)
        .map_err(|e| e.to_string())?;

    // Create basic blocks.
    let header_bb = self.context.append_basic_block(fn_val, "forin_header");
    let body_bb = self.context.append_basic_block(fn_val, "forin_body");
    let latch_bb = self.context.append_basic_block(fn_val, "forin_latch");
    let merge_bb = self.context.append_basic_block(fn_val, "forin_merge");

    // Push loop context: continue -> latch, break -> merge.
    self.loop_stack.push((latch_bb, merge_bb));

    // Branch to header.
    self.builder.build_unconditional_branch(header_bb)
        .map_err(|e| e.to_string())?;

    // -- Header: check counter < end --
    self.builder.position_at_end(header_bb);
    let current_val = self.builder.build_load(i64_ty, counter, "i_val")
        .map_err(|e| e.to_string())?.into_int_value();
    let cond = self.builder.build_int_compare(
        IntPredicate::SLT, current_val, end_val, "forin_cond")
        .map_err(|e| e.to_string())?;
    self.builder.build_conditional_branch(cond, body_bb, merge_bb)
        .map_err(|e| e.to_string())?;

    // -- Body: register loop variable, codegen body --
    self.builder.position_at_end(body_bb);

    // Register the loop variable (save previous binding).
    let old_alloca = self.locals.insert(var.to_string(), counter);
    let old_type = self.local_types.insert(var.to_string(), MirType::Int);

    let _body_val = self.codegen_expr(body)?;

    // If body did not terminate (no break/continue), fall through to latch.
    if let Some(bb) = self.builder.get_insert_block() {
        if bb.get_terminator().is_none() {
            self.builder.build_unconditional_branch(latch_bb)
                .map_err(|e| e.to_string())?;
        }
    }

    // -- Latch: increment counter, reduction check, back-edge to header --
    self.builder.position_at_end(latch_bb);
    let cur = self.builder.build_load(i64_ty, counter, "i_cur")
        .map_err(|e| e.to_string())?.into_int_value();
    let next = self.builder.build_int_add(cur, i64_ty.const_int(1, false), "i_next")
        .map_err(|e| e.to_string())?;
    self.builder.build_store(counter, next)
        .map_err(|e| e.to_string())?;
    self.emit_reduction_check();
    self.builder.build_unconditional_branch(header_bb)
        .map_err(|e| e.to_string())?;

    // Pop loop context.
    self.loop_stack.pop();

    // Restore previous variable binding.
    if let Some(prev) = old_alloca {
        self.locals.insert(var.to_string(), prev);
    } else {
        self.locals.remove(var);
    }
    if let Some(prev_ty) = old_type {
        self.local_types.insert(var.to_string(), prev_ty);
    } else {
        self.local_types.remove(var);
    }

    // Position at merge block.
    self.builder.position_at_end(merge_bb);

    // For-in returns Unit.
    Ok(self.context.struct_type(&[], false).const_zero().into())
}
```

### Formatter: walk_for_in_expr

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
                    _ => {
                        // Iterable expression (e.g., 0..10).
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    ir::concat(parts)
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `Range.new(1, 10) \|> Range.map(fn i -> ...)` | `for i in 0..10 do ... end` | Phase 34 | Natural iteration syntax, zero allocation for integer ranges |
| No loop syntax | `while cond do ... end` | Phase 33 | Foundation for all loops |
| Recursive functions for iteration | for-in loops | Phase 34 | Idiomatic iteration |

## Key Design Decisions

### 1. For-In over Range Returns Unit (Phase 34 Only)
The for-in loop over a range returns Unit in Phase 34. Phase 35 will add `List<T>` comprehension semantics where `for x in collection do expr end` returns `List<T>`. For Phase 34, the range variant returns Unit to keep the implementation focused. This is analogous to how while loops return Unit (WHILE-03).

### 2. Half-Open Range Semantics: [start, end)
The range `0..10` iterates from 0 to 9 (inclusive start, exclusive end). This matches:
- The existing `snow_range_new(start, end)` semantics where `Range.new(1, 4)` has length 3
- Rust's `0..10` semantics
- Python's `range(0, 10)` semantics
- The most common convention in programming languages

### 3. Loop Stack Reuse with Continue-to-Latch
Rather than changing the `loop_stack` type from `Vec<(BasicBlock, BasicBlock)>`, we reuse the same type but change the semantic meaning of the first element:
- For while loops: `(cond_bb, merge_bb)` -- continue goes to cond, break goes to merge
- For for-in: `(latch_bb, merge_bb)` -- continue goes to latch, break goes to merge

The codegen_continue and codegen_break functions work unchanged because they already jump to the first and second elements respectively. This is a zero-change integration.

### 4. No New Keywords Required
`for` is already `TokenKind::For` (keyword #14 of 48). `in` is already `TokenKind::In` (keyword #20). `do` and `end` are already keywords. Only a new CST node kind (`FOR_IN_EXPR`) and AST wrapper (`ForInExpr`) are needed.

### 5. DotDot Not Changed at General Level
The `..` binary operator continues to work as before in general expressions. Only inside `for ... in ... do ... end` is the `..` recognized as range syntax and desugared to start/end integers. This means `Range.new(0, 10)` continues to work via the module system. Phase 35 may later formalize `..` as a first-class operator that creates Range values.

## Open Questions

1. **Should `for i in expr do body end` work when expr is a Range variable (not literal `..`)?**
   - What we know: Phase 34 only requires `for i in 0..10 do body end` (literal range syntax). But a user might write `let r = Range.new(0, 10); for i in r do body end`.
   - What's unclear: Whether to support this in Phase 34 or defer to Phase 35.
   - Recommendation: Defer to Phase 35. Phase 34 only handles the `for i in start..end` pattern where the iterable is a BinaryExpr with DotDot. For other iterables, emit a compile error "for-in over non-range iterables not yet supported". This keeps Phase 34 focused on zero-allocation integer arithmetic.

2. **Empty range behavior: `for i in 10..0 do body end`**
   - What we know: The condition `i < end` where start=10 and end=0 is immediately false, so the body executes zero times. This matches `Range.new(10, 0)` which has length 0.
   - Recommendation: No special handling needed. The `icmp slt` check naturally handles this.

3. **Negative ranges: `for i in -5..5 do body end`**
   - What we know: Using `icmp slt` (signed less-than) handles negative values correctly.
   - Recommendation: Works automatically with signed comparison. Verify in tests.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `snow-common/src/token.rs` -- For (line 44), In (line 49) already exist as keywords
- Codebase analysis: `snow-parser/src/parser/expressions.rs` -- DotDot has binding power 13/14 (line 40)
- Codebase analysis: `snow-parser/src/syntax_kind.rs` -- FOR_KW (line 38), IN_KW (line 43), DOT_DOT (line 90) all exist
- Codebase analysis: `snow-typeck/src/infer.rs` -- infer_binary has no DotDot case (falls to fresh_var at line 2610)
- Codebase analysis: `snow-typeck/src/unify.rs` -- InferCtx has loop_depth, enter_loop, exit_loop, enter_closure, in_loop (lines 32-73)
- Codebase analysis: `snow-codegen/src/mir/mod.rs` -- MirExpr::While, Break, Continue at lines 293-302
- Codebase analysis: `snow-codegen/src/mir/lower.rs` -- lower_while_expr at line 3956, lower_binary_expr at 3172 (DotDot falls to BinOp::Add at line 3193)
- Codebase analysis: `snow-codegen/src/codegen/mod.rs` -- loop_stack at line 86, CodeGen::new initializes at line 164
- Codebase analysis: `snow-codegen/src/codegen/expr.rs` -- codegen_while at line 1667, codegen_break at 1720, codegen_continue at 1735, codegen_let at 924 (save/restore pattern)
- Codebase analysis: `snow-fmt/src/walker.rs` -- walk_while_expr at line 404 (model for walk_for_in_expr)
- Codebase analysis: `snow-rt/src/collections/range.rs` -- snow_range_new creates heap-allocated 16-byte range, confirming we must NOT use it for FORIN-07
- Codebase analysis: Phase 33 verification report -- all 11 truths verified, loop infrastructure complete

### Secondary (MEDIUM confidence)
- LLVM Language Reference: four-block loop structure (header/body/latch/merge) is the standard representation that LLVM's LoopInfo pass recognizes
- Phase 33 research: established patterns for keyword addition, CST node kinds, AST wrappers, typeck inference, MIR lowering, and LLVM codegen

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all libraries already in workspace, no new dependencies, keywords already exist
- Architecture: HIGH -- follows established patterns from Phase 33 exactly, all pipeline stages inspected, four-block loop structure is standard LLVM
- Pitfalls: HIGH -- each pitfall identified from actual code patterns and LLVM semantics
- Code examples: HIGH -- modeled on actual existing code (codegen_while, parse_while_expr, codegen_let save/restore)
- Continue-to-latch: HIGH -- verified by inspecting codegen_continue (jumps to first element of loop_stack.last())

**Research date:** 2026-02-08
**Valid until:** indefinite (codebase-specific research, not library-version-dependent)
