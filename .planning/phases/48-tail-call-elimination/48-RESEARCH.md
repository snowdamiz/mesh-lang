# Phase 48: Tail-Call Elimination - Research

**Researched:** 2026-02-10
**Domain:** Self-recursive tail-call optimization via MIR loop transformation in a compiled language
**Confidence:** HIGH

## Summary

Phase 48 adds tail-call elimination (TCE) for self-recursive functions in the Snow compiler. The goal is to ensure that actor receive loops and other self-recursive patterns execute in constant stack space, preventing stack overflow even after millions of iterations. The decided approach is MIR-level loop transformation rather than relying on LLVM's `musttail` attribute, which has platform and calling-convention limitations.

The implementation requires changes in two compiler stages: MIR lowering and LLVM codegen. In MIR lowering, the compiler must (1) track the name of the currently-being-lowered function, (2) detect calls to that same function in tail position, and (3) emit a new `MirExpr::TailCall` node instead of `MirExpr::Call`. In codegen, the compiler must (1) wrap the function body in a loop when the function contains tail calls, (2) create mutable parameter allocas at function entry, and (3) compile `TailCall` as parameter reassignment followed by a branch to the loop header. This is exactly how languages like Erlang, Scala, and Kotlin implement guaranteed self-recursive TCE.

The tail position detection must propagate through: if/else branches (both arms), case/match arms (all arm bodies), receive arms (all arm bodies), blocks (last expression only), and let-chains (body of the innermost let). Expressions that are NOT in tail position include: function arguments, if/else conditions, let binding values, scrutinee expressions, and any expression followed by another expression in a block.

**Primary recommendation:** Add a `current_fn_name` field to `Lowerer`, add a `MirExpr::TailCall` variant, implement `is_tail_position` detection during MIR lowering or as a post-lowering rewrite pass, and generate loop-wrapped function bodies in codegen for functions that contain `TailCall` nodes.

## Standard Stack

### Core (existing crates, no new dependencies)

| Component | Location | Purpose | What Changes |
|-----------|----------|---------|--------------|
| snow-codegen (MIR defs) | `crates/snow-codegen/src/mir/mod.rs` | MIR node definitions | Add `MirExpr::TailCall` variant |
| snow-codegen (MIR lower) | `crates/snow-codegen/src/mir/lower.rs` | AST-to-MIR lowering | Track current function name, detect + emit tail calls |
| snow-codegen (codegen) | `crates/snow-codegen/src/codegen/mod.rs` | Function compilation | Wrap body in loop for tail-recursive functions |
| snow-codegen (codegen expr) | `crates/snow-codegen/src/codegen/expr.rs` | Expression codegen | Handle `TailCall` -> parameter reassign + continue |

### Supporting (no changes expected)

| Component | Location | Purpose | Why No Changes |
|-----------|----------|---------|----------------|
| snow-parser | `crates/snow-parser/src/` | Parsing | No syntax changes -- TCE is transparent to the user |
| snow-typeck | `crates/snow-typeck/src/` | Type checking | No type-level changes needed |
| snow-rt | `crates/snow-rt/src/` | Runtime | No runtime changes needed |
| snow-fmt | `crates/snow-fmt/src/` | Formatter | No formatting changes needed |

### No New Dependencies

This feature is entirely compiler-internal. No new Rust crates are needed.

## Architecture Patterns

### Pattern 1: MIR TailCall Node

**What:** A new `MirExpr::TailCall` variant that represents a self-recursive call in tail position.

**When to use:** When MIR lowering detects a call to the currently-being-compiled function where the call result is the return value of the function (tail position).

**Design:**
```rust
// In mir/mod.rs, add to MirExpr enum:
MirExpr::TailCall {
    /// Arguments for the recursive call (will be assigned to parameters).
    args: Vec<MirExpr>,
    /// Result type (matches function return type -- used for ty() method).
    ty: MirType,
}
```

The `TailCall` node does NOT carry the function name because it is only valid within the function being compiled. The codegen knows which function it's compiling.

**How `ty()` works:** `TailCall` returns `MirType::Never` because it never produces a value -- it branches to the loop header. This is consistent with how `Break`, `Continue`, and `Return` work.

### Pattern 2: Tail Position Detection

**What:** Determine whether an expression is in "tail position" -- i.e., its value becomes the function's return value with no further computation.

**Approach: Post-lowering rewrite pass on MirExpr.**

After the function body is lowered to MIR, walk the expression tree and rewrite `MirExpr::Call` nodes that call the current function AND are in tail position into `MirExpr::TailCall` nodes. This is cleaner than threading a `tail_position` boolean through every `lower_*` method.

**Tail position rules:**

1. **Function body:** The entire function body expression is in tail position.

2. **Block:** In `Block(exprs, ty)`, only the LAST expression is in tail position. Earlier expressions are NOT.

3. **Let:** In `Let { value, body, .. }`, the `body` is in tail position. The `value` is NOT.

4. **If/else:** In `If { then_body, else_body, .. }`, BOTH `then_body` and `else_body` are in tail position. The `cond` is NOT.

5. **Match/Case:** In `Match { arms, .. }`, all arm bodies are in tail position. The `scrutinee` is NOT.

6. **ActorReceive:** In `ActorReceive { arms, .. }`, all arm bodies are in tail position. The timeout_body is also in tail position.

7. **Return:** In `Return(inner)`, the `inner` is in tail position (it IS the value being returned -- if `inner` is a self-call, it's a tail call).

8. **NOT in tail position:** Arguments to function calls, operands of binary/unary ops, struct literal fields, variant constructor fields, conditions, scrutinees, let binding values.

**Implementation approach -- recursive rewrite function:**

```rust
fn rewrite_tail_calls(expr: &mut MirExpr, current_fn_name: &str) {
    match expr {
        MirExpr::Call { func, args, ty } => {
            if let MirExpr::Var(name, _) = func.as_ref() {
                if name == current_fn_name {
                    // Replace with TailCall
                    let args = std::mem::take(args);
                    let ty = ty.clone();
                    *expr = MirExpr::TailCall { args, ty };
                }
            }
        }
        MirExpr::Block(exprs, _) => {
            if let Some(last) = exprs.last_mut() {
                rewrite_tail_calls(last, current_fn_name);
            }
        }
        MirExpr::Let { body, .. } => {
            rewrite_tail_calls(body, current_fn_name);
        }
        MirExpr::If { then_body, else_body, .. } => {
            rewrite_tail_calls(then_body, current_fn_name);
            rewrite_tail_calls(else_body, current_fn_name);
        }
        MirExpr::Match { arms, .. } => {
            for arm in arms {
                rewrite_tail_calls(&mut arm.body, current_fn_name);
            }
        }
        MirExpr::ActorReceive { arms, timeout_body, .. } => {
            for arm in arms {
                rewrite_tail_calls(&mut arm.body, current_fn_name);
            }
            if let Some(tb) = timeout_body {
                rewrite_tail_calls(tb, current_fn_name);
            }
        }
        MirExpr::Return(inner) => {
            rewrite_tail_calls(inner, current_fn_name);
        }
        _ => {} // Not a tail context -- don't recurse
    }
}
```

### Pattern 3: Codegen Loop Wrapping

**What:** When compiling a function that contains `TailCall` nodes, wrap the function body in a loop.

**Pre-scan:** Before compiling the function body, scan the MIR tree for any `TailCall` nodes. If found, the function is tail-recursive and needs loop wrapping.

**Codegen structure for a tail-recursive function:**

```
entry:
    ; Alloca mutable parameter slots
    %p0 = alloca i64
    store i64 %arg0, i64* %p0
    %p1 = alloca i64
    store i64 %arg1, i64* %p1
    br label %tce_loop

tce_loop:
    ; Load current parameter values
    %p0_val = load i64, i64* %p0
    %p1_val = load i64, i64* %p1
    ; ... function body ...
    ; Normal return:
    ret i64 %result
    ; Tail call (replaces actual call):
    ;   store new_arg0 -> %p0
    ;   store new_arg1 -> %p1
    ;   br label %tce_loop
```

**Key detail:** The parameter allocas already exist in `compile_function` (line 360-376 in `codegen/mod.rs`). The function body code uses `self.locals` to load parameters by name. The `TailCall` codegen simply stores new values into these same allocas and branches to the loop header. This means the existing parameter setup already works correctly for TCE.

**The existing codegen already creates allocas for parameters and stores incoming values.** This is the `alloca + mem2reg` pattern. For TCE, these allocas become loop-carried variables naturally -- we just need to add the loop header basic block and branch back to it on tail calls.

**Codegen implementation for TailCall:**

```rust
// In codegen_expr, handle MirExpr::TailCall:
MirExpr::TailCall { args, .. } => {
    // Evaluate each new argument
    let new_vals: Vec<BasicValueEnum> = args.iter()
        .map(|a| self.codegen_expr(a))
        .collect::<Result<_, _>>()?;

    // Store new values into parameter allocas
    for (i, (param_name, _)) in current_fn_params.iter().enumerate() {
        let alloca = self.locals[param_name];
        self.builder.build_store(alloca, new_vals[i]);
    }

    // Emit reduction check (for preemptive scheduling)
    self.emit_reduction_check();

    // Branch to loop header
    self.builder.build_unconditional_branch(tce_loop_bb);

    // Return dummy (block is terminated)
    Ok(unit_val)
}
```

### Pattern 4: Function-Level Metadata for Loop Wrapping

**What:** Pass information about whether a function needs loop wrapping from the MIR level to codegen.

**Option A -- Scan MIR at codegen time:** Before compiling a function body, recursively scan the `MirExpr` tree for `TailCall` nodes. If any exist, create the loop header block. This is simple and doesn't require MIR-level metadata.

**Option B -- Add a flag to MirFunction:** Add `pub has_tail_calls: bool` to `MirFunction`. Set it during the rewrite pass. Codegen checks the flag.

**Recommendation:** Use Option B (flag on `MirFunction`). It's one extra field and avoids re-scanning the expression tree. The rewrite pass already walks the tree.

### Anti-Patterns to Avoid

- **DO NOT thread `is_tail_position: bool` through every `lower_*` method:** This would require modifying every expression lowering function's signature, creating massive churn. The post-lowering rewrite pass is far cleaner.

- **DO NOT use LLVM `musttail`:** The project decision explicitly chose MIR-level transformation. `musttail` has platform restrictions (requires same calling convention, same return type, specific ABIs) and cannot handle parameter type mismatches.

- **DO NOT attempt mutual tail-call elimination:** This is explicitly deferred to future requirements. Only SELF-recursive calls (function calls itself) are in scope.

- **DO NOT evaluate tail call arguments left-to-right into parameter allocas directly:** If argument expressions reference parameters (`fn f(n) = f(n - 1)`), they must all be evaluated FIRST, then all stored. Otherwise `f(a, b) -> f(b, a)` would be broken because storing `b` into `a`'s alloca would corrupt `a` before it's read for `b`'s alloca.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Tail position detection | Custom control flow analysis pass | Simple recursive MIR tree walk | MIR is a tree (not CFG), so tail position is structurally obvious |
| Loop transformation | New loop MIR nodes | Existing codegen basic block + branch infrastructure | LLVM builder already supports creating basic blocks and branches |
| Parameter mutation | New mutable variable infrastructure | Existing `alloca + store` pattern in compile_function | Parameters already have allocas -- just store new values |

**Key insight:** The MIR is still a tree structure (not a CFG/SSA graph), which makes tail position detection trivial -- just follow the "last expression" path through blocks, let bodies, if/else branches, and match arms. The codegen already uses the `alloca + mem2reg` pattern for parameters, which naturally supports parameter reassignment.

## Common Pitfalls

### Pitfall 1: Argument Evaluation Order in TailCall

**What goes wrong:** If tail call arguments reference the function's parameters, and arguments are evaluated and stored one at a time, earlier stores corrupt parameter values needed by later argument evaluations.

**Example:**
```
fn swap(a, b) do
  if done then {a, b}
  else swap(b, a)  # TailCall args = [Var("b"), Var("a")]
  end
end
```
If we store `b` into `a`'s alloca, then try to read `a` for `b`'s alloca, we get `b`'s value (already overwritten).

**How to avoid:** Evaluate ALL argument expressions FIRST into temporary values, THEN store all temporaries into parameter allocas. This two-phase approach is standard in TCE implementations.

**Warning signs:** Tests where parameters are swapped or reused in tail call arguments produce wrong results.

### Pitfall 2: Tail Position After ? Operator

**What goes wrong:** The `?` operator desugars to `Match + Return` in MIR. A call that follows `?` is NOT in tail position because the `?` introduces a match/return that wraps the subsequent code. However, a call that IS the body of a `?` match arm is potentially in tail position.

**How to avoid:** The rewrite pass handles this naturally because `Return(inner)` propagates tail position to `inner`. The desugared `?` creates `Match { Ok_arm: body, Err_arm: Return(...) }` -- the `Ok_arm`'s body continues normally, so if it ends in a self-call, the rewrite pass finds it through the block/let chain.

**Warning signs:** Programs using `?` before a tail call work correctly because `?` desugars before TCE rewriting.

### Pitfall 3: Module-Qualified Function Names

**What goes wrong:** The lowerer applies module name prefixing to private functions via `qualify_name`. A function `foo` in module `bar` becomes `bar__foo`. The function body still contains calls using the unqualified name `foo`, but the MirFunction has the qualified name `bar__foo`. The tail call detector must compare against the qualified name that appears in the `MirExpr::Call` callee.

**How to avoid:** Track the qualified function name (the one stored in `MirFunction.name`) for comparison. The lowerer already resolves call targets to their qualified names through `map_builtin_name` and `qualify_name` -- the MIR `Call` node contains the qualified name by the time it's generated.

**Warning signs:** TCE doesn't trigger in multi-module builds because function names don't match.

### Pitfall 4: Closure Functions and Actor Definitions

**What goes wrong:** Closures are lifted to top-level functions with generated names like `__closure_3`. Actor definitions also produce functions. If TCE runs on these, it could incorrectly detect self-recursion. However, this is actually correct behavior -- if a closure calls itself by name, that IS self-recursion. The issue is that closures don't typically call themselves by name (they'd need to capture their own reference).

**How to avoid:** The detection is name-based. Closures have generated names that don't appear in user code, so they won't match any user call expression. Actor bodies that call their own actor function by name (like `actor_loop(state)` recursion patterns) SHOULD be detected and optimized -- this is the primary use case.

**Warning signs:** None -- this works correctly with name-based matching.

### Pitfall 5: Reduction Check Placement

**What goes wrong:** The Snow runtime uses `snow_reduction_check()` for preemptive scheduling -- actors must yield periodically. In a TCE-transformed loop, the tail call becomes a branch, so the reduction check that would normally happen after the call is lost.

**How to avoid:** Emit `self.emit_reduction_check()` in the `TailCall` codegen, BEFORE branching to the loop header. This ensures actors using tail-recursive loops still yield to the scheduler.

**Warning signs:** Actor programs with tail-recursive loops that never yield, causing starvation of other actors.

### Pitfall 6: Impl Method Self-Recursion

**What goes wrong:** Trait impl methods have mangled names like `Trait__method__Type`. If a method calls itself recursively (e.g., a recursive `to_string` on a tree structure), the call in the source uses `self.method()` which is lowered to `Trait__method__Type(self, ...)`. The function name is `Trait__method__Type`. This should be detected as self-recursive if the call target matches the function being compiled.

**How to avoid:** Use the final mangled name from `MirFunction.name` for comparison. The call target resolution already produces the mangled name.

**Warning signs:** Recursive trait methods don't get TCE even when they should.

## Code Examples

### Example 1: Snow Program with Self-Recursive Tail Call

```snow
fn countdown(n) do
  if n <= 0 then
    println("done")
  else
    countdown(n - 1)
  end
end

fn main() do
  countdown(1000000)
end
```

### Example 2: Expected MIR Before TCE Rewrite

```
MirFunction {
    name: "countdown",
    params: [("n", Int)],
    return_type: Unit,
    body: If {
        cond: BinOp(LtEq, Var("n"), IntLit(0)),
        then_body: Call { func: Var("snow_println"), args: [StringLit("done")], ty: Unit },
        else_body: Call { func: Var("countdown"), args: [BinOp(Sub, Var("n"), IntLit(1))], ty: Unit },
        ty: Unit,
    },
    has_tail_calls: false,
}
```

### Example 3: Expected MIR After TCE Rewrite

```
MirFunction {
    name: "countdown",
    params: [("n", Int)],
    return_type: Unit,
    body: If {
        cond: BinOp(LtEq, Var("n"), IntLit(0)),
        then_body: Call { func: Var("snow_println"), args: [StringLit("done")], ty: Unit },
        else_body: TailCall { args: [BinOp(Sub, Var("n"), IntLit(1))], ty: Unit },
        ty: Unit,
    },
    has_tail_calls: true,
}
```

### Example 4: Actor Receive Loop Pattern

```snow
fn actor_loop(state) do
  receive do
    msg ->
      let new_state = process(state, msg)
      actor_loop(new_state)
  end
end
```

This is the critical pattern for actors. After TCE:
- `actor_loop(new_state)` becomes `TailCall { args: [new_state] }`
- The codegen wraps the function body in a loop
- The actor runs indefinitely without growing the stack

### Example 5: Case Expression with Tail Calls

```snow
fn process(cmd, state) do
  case cmd do
    "inc" -> process("idle", state + 1)
    "dec" -> process("idle", state - 1)
    "get" -> state
    _ -> process("idle", state)
  end
end
```

All case arms ending with `process(...)` calls are in tail position and get rewritten to `TailCall`.

## State of the Art

| Approach | Used By | How It Works | Limitations |
|----------|---------|--------------|-------------|
| MIR/IR loop transformation | Erlang/BEAM, Kotlin, Scala | Detect self-recursive tail calls, rewrite to loops | Self-recursion only |
| `musttail` LLVM attribute | Zig, some Clang extensions | Compiler hint for guaranteed tail call | Platform-specific, same calling convention required |
| Trampoline | Many FP languages | Return continuation instead of calling | Heap allocation overhead, indirect calls |
| CPS transform | Scheme, some Haskell backends | Convert all calls to CPS | Pervasive transformation, complexity |

**Snow's approach (MIR loop transformation)** is the most practical for self-recursive elimination. It is:
- Guaranteed to work on all platforms (no ABI restrictions)
- Zero runtime overhead (compiles to a simple loop + branch)
- Simple to implement (tree walk + basic block manipulation)
- Sufficient for the primary use case (actor receive loops)

Mutual tail-call elimination is explicitly deferred to future work and would require either `musttail` or trampolines.

## Implementation Strategy

### Recommended Plan Split: 2 Plans

**Plan TCE-01:** MIR infrastructure and rewrite pass
- Add `MirExpr::TailCall` variant to `mir/mod.rs`
- Add `has_tail_calls: bool` field to `MirFunction`
- Add `current_fn_name: Option<String>` to `Lowerer`
- Implement `rewrite_tail_calls` function
- Call rewrite pass after lowering each function body in `lower_fn_def`, `lower_actor_def`, and `lower_impl_method`
- Unit tests verifying tail position detection through all expression types

**Plan TCE-02:** Codegen loop wrapping and e2e tests
- Modify `compile_function` in `codegen/mod.rs` to detect `has_tail_calls` and create loop header
- Store `tce_loop_bb` on CodeGen (similar to `loop_stack`)
- Handle `MirExpr::TailCall` in `codegen_expr` -- evaluate args, store to param allocas, emit reduction check, branch to loop header
- Add `ty()` match arm for `TailCall`
- E2e tests: countdown 1M iterations, parameter swap recursion, actor receive loop, case-arm tail calls

### Key Implementation Details

1. **Where to store tce_loop_bb:** Add `tce_loop_header: Option<BasicBlock<'ctx>>` to `CodeGen`. Set it in `compile_function` when `func.has_tail_calls` is true. Clear it after compiling the function.

2. **Where to store current function's param info for TailCall codegen:** The param names and allocas are already in `self.locals` and the param order is in `func.params`. Pass the param names list down or store it on `CodeGen` alongside `tce_loop_header`.

3. **Handling the body type:** The function body type doesn't change. If the body is `If { ty: Unit }` and one branch is `TailCall { ty: Never }`, the if's type is still `Unit`. The `ty()` method on `TailCall` should return `MirType::Never` (same as `Break`/`Continue`/`Return`).

4. **Interaction with existing While/Break/Continue:** TCE uses a separate mechanism from `loop_stack`. The `tce_loop_header` is NOT pushed onto `loop_stack`, so `break` and `continue` in user code don't interfere with TCE. They refer to user-written loops, while TCE's loop is compiler-generated and invisible.

## Open Questions

1. **Should TCE apply to closure functions that call themselves?**
   - What we know: Closures have generated names (`__closure_3`) that are not visible in user code. A closure cannot call itself by name without explicit self-reference.
   - What's unclear: Whether any Snow pattern allows a closure to reference itself.
   - Recommendation: Don't worry about this case. Closures won't match because the generated name doesn't appear in user call expressions. If a user writes a named function that is used as a closure and calls itself by name, that function is a regular function (not a closure) and TCE applies normally.

2. **Should we warn when a self-recursive call is NOT in tail position?**
   - What we know: Some languages (Scala's `@tailrec`) provide compile-time warnings/errors for non-tail recursive functions marked for TCE.
   - What's unclear: Whether this is desired for Snow v1.9.
   - Recommendation: Defer annotations to future work (already listed in deferred requirements as `@tailrec compile-time annotation`). For v1.9, TCE is best-effort and silent.

## Sources

### Primary (HIGH confidence)
- Snow compiler source code: `crates/snow-codegen/src/mir/mod.rs` -- MIR expression tree structure, 586 lines
- Snow compiler source code: `crates/snow-codegen/src/mir/lower.rs` -- MIR lowering, ~8200 lines
- Snow compiler source code: `crates/snow-codegen/src/codegen/mod.rs` -- function compilation, ~1500 lines
- Snow compiler source code: `crates/snow-codegen/src/codegen/expr.rs` -- expression codegen, ~3600 lines
- Snow project requirements: `.planning/REQUIREMENTS.md` -- TCE-01, TCE-02 requirements
- Snow project roadmap: `.planning/ROADMAP.md` -- Phase 48 description and success criteria

### Secondary (MEDIUM confidence)
- Prior decision recorded in project: "TCE uses MIR loop transformation (not LLVM musttail) for reliability"
- Standard compiler design: self-recursive TCE via loop transformation is well-established in Erlang/BEAM, Kotlin, and Scala compilers

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- changes are entirely within the existing snow-codegen crate, no new dependencies
- Architecture: HIGH -- MIR tree structure makes tail position detection structurally simple; existing codegen patterns (alloca, basic blocks, branches) directly support loop wrapping
- Pitfalls: HIGH -- pitfalls are well-understood from compiler literature; argument evaluation order and reduction check placement are the main concerns

**Research date:** 2026-02-10
**Valid until:** 2026-03-12 (stable compiler internals, unlikely to change significantly)
