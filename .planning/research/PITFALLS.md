# Pitfalls Research: Adding Loops & Iteration to Snow

**Domain:** Compiler feature addition -- for..in loops, while loops, break/continue for a statically-typed, functional-first, LLVM-compiled language with actor concurrency and per-actor GC
**Researched:** 2026-02-08
**Confidence:** HIGH (based on direct Snow codebase analysis, LLVM documentation, Rust RFC precedent, and established compiler engineering knowledge)

**Scope:** This document covers pitfalls specific to adding loop constructs (`for..in`, `while`, `break`, `continue`) to the Snow compiler (v1.7). Snow is functional-first with HM type inference, alloca+mem2reg codegen pattern, cooperative scheduling via reduction counting, and mark-sweep GC per actor heap. The existing compiler has 67,546 lines of Rust across 11 crates, 1,255 tests, and zero known correctness issues.

---

## Critical Pitfalls

Mistakes that cause rewrites, soundness holes, or silent codegen bugs.

---

### Pitfall 1: Expression-Returning Loops Break HM Unification When `break(value)` Types Disagree

**What goes wrong:**
Snow is expression-based: every construct returns a value. A `for..in` loop that collects results returns `List<T>`. A `while` loop returns the last expression value or Unit. But `break(value)` introduces an ADDITIONAL return path -- the loop's type is now the UNIFICATION of the natural-termination type AND every `break(value)` type. If these disagree, the type checker must either reject the program or pick a common type.

Concrete example of the failure:
```
# What type does this loop have?
let result = for x in [1, 2, 3] do
  if x == 2 do
    break("found")    # break type: String
  end
  x * 10              # body type: Int (collected into List<Int>)
end
```

The natural result is `List<Int>` (collected body values), but `break("found")` produces `String`. These are incompatible. Without careful design, the type checker will either: (a) unify `List<Int>` with `String` and produce an inscrutable error like `"cannot unify List<?0> with String"`, or (b) silently pick one and produce incorrect codegen.

**Why it happens:**
In classic HM inference, the type of an expression is determined by a single unification. But loops with `break(value)` have two distinct return paths -- the "completed" path and the "early exit" path. Rust solved this for `loop {}` by requiring all `break` expressions to agree, and the loop type IS the break type (natural termination is impossible in `loop {}`). But for `for` and `while`, natural termination is the common case, making the design much harder.

Rust explicitly punted on this: RFC 1624 addresses only `loop {}` break-with-value, and the discussion at rust-lang/rfcs#1767 ("Allow for and while loop to return value") remains unresolved after 8+ years precisely because of this type-unification problem.

**How to avoid:**
Define clear semantics BEFORE implementing. The recommended design for Snow:

1. **`for..in` returns `List<T>`** where `T` is the body expression type. This is the "collect" semantic, matching Scala's `for/yield` and Elixir's `for/do` comprehension.
2. **`break` without a value** exits the loop early. The collected list so far is the result.
3. **`break(value)` is NOT supported in `for..in`** -- it would require the loop to have EITHER `List<T>` or `value_type` as its type, which breaks simple HM unification. Instead, use `Enum.reduce_while` or pattern-match results.
4. **`while` returns `Unit`** -- it is inherently side-effecting. `break` without value exits early.
5. **`while` with `break(value)` is NOT supported in v1.7** -- defer to a future version if needed. OCaml's for/while loops also always return unit.

This avoids the unification problem entirely. The loop type is always deterministic from the loop form alone.

**Warning signs:**
- Type checker tests where a loop with `break(value)` produces a type variable instead of a concrete type.
- Error messages mentioning `"cannot unify List<T> with U"` where U is the break value type.
- Any test requiring the user to annotate the loop's result type.

**Phase to address:**
Syntax/AST design and type inference phase. Must be decided during AST design and enforced in `infer_for_expr` / `infer_while_expr`.

---

### Pitfall 2: LLVM `alloca` Placed Inside Loop Body Instead of Entry Block

**What goes wrong:**
Snow's codegen uses the alloca+mem2reg pattern (visible in `codegen_if` at expr.rs:867-869 and `codegen_let` at expr.rs:925). LLVM's `mem2reg` pass ONLY promotes allocas in the function's entry block. If loop codegen creates allocas inside the loop body (e.g., for the loop variable, the accumulator, or temporary results), these allocas will:

1. **Not be promoted to SSA registers** -- every iteration reads/writes through memory instead of registers, severely degrading performance.
2. **Accumulate stack space** -- each iteration allocates a new stack frame slot that is NOT freed until the function returns. A loop iterating 1 million times will allocate 1 million stack slots, causing stack overflow.

This is a well-documented LLVM pitfall. The LLVM Frontend Performance Tips explicitly state: "placing alloca instructions at the beginning of the entry block should be preferred." The MLIR project encountered this exact bug (tensorflow/mlir#210) when lowering memref descriptors inside loops.

**Why it happens:**
The natural codegen pattern for Snow's `codegen_expr` dispatches recursively. When generating the loop body, the builder is positioned inside the loop's basic block. Any `build_alloca` call within that context places the alloca in the loop body, not the entry block. The existing `codegen_let` and `codegen_if` work correctly because they are not inside loops -- each alloca executes exactly once per function call.

**How to avoid:**
All allocas for loop-related storage must be emitted in the function's entry block BEFORE the loop header. Implement a helper:

```rust
/// Emit an alloca in the function's entry block (for mem2reg eligibility).
fn emit_entry_block_alloca(&self, ty: BasicTypeEnum<'ctx>, name: &str) -> PointerValue<'ctx> {
    let entry_bb = self.current_function().get_first_basic_block().unwrap();
    let saved_bb = self.builder.get_insert_block();
    // Position at start of entry block
    match entry_bb.get_first_instruction() {
        Some(instr) => self.builder.position_before(&instr),
        None => self.builder.position_at_end(entry_bb),
    }
    let alloca = self.builder.build_alloca(ty, name).unwrap();
    // Restore builder position
    if let Some(bb) = saved_bb {
        self.builder.position_at_end(bb);
    }
    alloca
}
```

The target IR structure:
```
entry_bb:
  %loop_var = alloca i64          ; loop variable
  %result_list = alloca ptr       ; accumulated result
  %loop_idx = alloca i64          ; iteration index
  br label %loop_header

loop_header:
  ; condition check
  br i1 %cond, label %loop_body, label %loop_exit

loop_body:
  ; use %loop_var, %result_list via load/store ONLY
  br label %loop_header

loop_exit:
  %result = load ptr, ptr %result_list
```

**Warning signs:**
- LLVM IR output shows `alloca` instructions between `br` and `br` inside a loop body block.
- Programs with large loops crash with stack overflow.
- Performance regression: loops run 10-100x slower than expected.
- `opt -mem2reg` does not eliminate loop allocas (verify with `opt -S -mem2reg` on output IR).

**Phase to address:**
LLVM codegen phase. Must be correct from the first implementation of `codegen_for` and `codegen_while`.

---

### Pitfall 3: Actor Starvation from Tight Loops Without Reduction Checks

**What goes wrong:**
Snow's actor scheduler uses cooperative preemption via `snow_reduction_check()`, which decrements a thread-local counter and yields when it reaches zero (actor/mod.rs:160-191). Currently, reduction checks are inserted AFTER function calls and closure calls (codegen/expr.rs:613, 759, 843). This works because recursive algorithms naturally contain function calls.

Loops change this calculus. A tight loop like:
```
while x < 1_000_000 do
  x = x + 1
end
```
contains NO function calls -- only integer comparison and addition. Without a reduction check inside the loop, this actor will monopolize its OS worker thread for the entire loop duration. Other actors on the same thread will be starved. If the loop is infinite (`while true do ... end`), the actor never yields, and the scheduler deadlocks.

The BEAM VM (Erlang/Elixir's runtime) solves this by counting "reductions" per opcode. Go solved it in 1.14 by adding compiler-inserted preemption checks in loop back-edges. Snow's `snow_reduction_check()` comment at actor/mod.rs:155 already states it should be inserted "at loop back-edges and function call sites", but back-edge insertion is not yet implemented because loops did not exist until v1.7.

**Why it happens:**
The current `emit_reduction_check()` is called explicitly after specific MIR expression forms (Call, ClosureCall). Loop codegen is new code -- if the developer forgets to insert a reduction check on the loop's back-edge (the branch from loop body back to loop header), the loop will never yield.

**How to avoid:**
Insert `self.emit_reduction_check()` in the loop codegen at the back-edge -- the point where the loop body branches back to the loop header:

```
loop_body:
  ; ... body codegen ...
  call void @snow_reduction_check()   ; <-- HERE, before back-edge
  br label %loop_header
```

This matches the BEAM's approach: every loop iteration costs one reduction. A loop iterating 4000 times will cause one yield, which is the correct granularity (DEFAULT_REDUCTIONS = 4000).

The `snow_reduction_check` function also triggers GC via `try_trigger_gc()` (actor/mod.rs:182-186), which means putting it at the back-edge also handles GC pressure from loop allocations.

**Warning signs:**
- An actor running a loop blocks all other actors on the same worker thread.
- `test_reduction_yield_does_not_starve` (scheduler.rs:797) fails when a loop-based actor is added.
- A `while true` loop causes the entire runtime to hang.

**Phase to address:**
LLVM codegen phase. Must be part of `codegen_for` and `codegen_while` from day one -- not a follow-up.

---

### Pitfall 4: O(N^2) List Collection via Immutable Append Chains

**What goes wrong:**
Snow lists are immutable -- `snow_list_append` (list.rs:76-87) allocates a NEW list and copies ALL existing elements plus the new one. For a `for..in` loop that collects N results, the total work is: copy 0 + copy 1 + copy 2 + ... + copy (N-1) = O(N^2) copies and O(N^2) total bytes allocated.

For N=100,000, this means ~5 billion element copies and ~5 billion bytes of intermediate garbage. The intermediate lists are dead immediately after the next append, but GC only triggers every ~4000 iterations (DEFAULT_REDUCTIONS), allowing garbage to pile up.

**Why it happens:**
Two compounding issues:
1. Immutable list append is O(N) per operation (full copy), making accumulated append O(N^2).
2. GC only runs at yield points (every ~4000 iterations), allowing garbage to accumulate.

This is NOT a problem for the existing `snow_list_map` (list.rs:174) because that function pre-allocates a new list of the correct size and fills it in-place. But for-loop collection cannot pre-allocate because the final size is unknown (continue/break may skip elements).

**How to avoid:**
Implement a `snow_list_builder` API for `for..in` loops that accumulates into a MUTABLE buffer and produces an immutable list at the end. This is the standard pattern (Rust's `Vec` -> immutable slice, Scala's `ListBuffer` -> `List`).

```rust
// New runtime functions
snow_list_builder_new(estimated_cap: u64) -> *mut u8     // O(1)
snow_list_builder_push(builder: *mut u8, elem: u64)       // O(1) amortized
snow_list_builder_finish(builder: *mut u8) -> *mut u8     // O(1), returns SnowList
```

The builder uses doubling growth (like Vec), giving O(N) total allocation and O(N) total copies. This changes the for-loop from O(N^2) to O(N).

**Critical GC consideration:** The builder's internal buffer must be GC-visible. Since Snow uses conservative stack scanning, the builder pointer on the stack will keep the buffer alive. But the builder must be allocated on the actor heap (via `snow_gc_alloc_actor`) so the GC can scan it.

**Warning signs:**
- `for x in (1..10000) do x end` takes noticeably longer than `List.map(range, fn(x) -> x end)`.
- Actor heap size grows quadratically with loop iteration count.
- Benchmarks show O(N^2) time for `for..in` loops.

**Phase to address:**
Runtime phase (snow-rt). Must be implemented alongside loop codegen -- `for..in` codegen should emit `snow_list_builder_*` calls, not `snow_list_append` chains. This is a prerequisite for correct loop implementation, not a future optimization.

---

### Pitfall 5: `break`/`continue` Codegen Creates Orphaned Basic Blocks and Terminated-Block Writes

**What goes wrong:**
`break` compiles to `br label %loop_exit` and `continue` compiles to `br label %loop_increment`. These are unconditional branches that terminate the current basic block. Any code AFTER the break/continue in the loop body is unreachable. Two failure modes:

1. **Terminated block writes:** The parent expression's codegen (e.g., `codegen_if`, `codegen_block`) tries to emit instructions after the break/continue, producing "Terminator found in the middle of a basic block" LLVM verification errors.

2. **Missing phi inputs:** If break/continue is inside one branch of an if-expression, the merge block's alloca-load pattern (used by `codegen_if` at expr.rs:906-910) expects both branches to store a value. The break branch never stores to the result alloca, so the load reads an uninitialized value.

The LLVM Kaleidoscope tutorial explicitly warns: "calling `codegen()` recursively could arbitrarily change the notion of the current block." The existing Snow codegen handles this for `Return` and `Panic` (both `MirType::Never`) by checking `get_terminator().is_none()` before emitting branches (expr.rs:887, 899).

**Why it happens:**
Break and continue are non-local control flow -- they jump out of the current expression evaluation context to the loop header or exit. But Snow's `codegen_expr` returns a `BasicValueEnum`, meaning every expression is expected to produce a value. A `break` does not produce a value in the current context. This is analogous to `MirExpr::Return` and `MirExpr::Panic`, which return `MirType::Never`.

**How to avoid:**
Model `break` and `continue` as diverging expressions (type `Never`):

1. **MIR representation:** Add `MirExpr::Break` and `MirExpr::Continue` with type `MirType::Never`.

2. **Codegen:** Emit the branch and then create a new "dead" basic block, positioning the builder there. Return a dummy `undef` value. The parent codegen's terminator check will prevent any branch from the dead block.

```
then_bb:
  br label %loop_exit          ; break
  ; Fall through to dead_bb (LLVM builder positioned here)
dead_bb:
  ; Any subsequent codegen goes here; block has no predecessors
  ; DCE will remove it
```

3. **Critical:** Ensure `codegen_if` and `codegen_block` check for terminated blocks before storing to the result alloca. The existing pattern at expr.rs:887 already does this for branches but not for stores. After generating the then-body, check if the block is terminated before `build_store(result_alloca, then_val)`.

4. **Loop context stack:** The codegen must maintain a stack of loop contexts so break/continue knows where to branch:
```rust
struct LoopContext<'ctx> {
    header_bb: BasicBlock<'ctx>,
    exit_bb: BasicBlock<'ctx>,
    increment_bb: BasicBlock<'ctx>,  // for continue in for-loops
    result_alloca: Option<PointerValue<'ctx>>,
}
// In CodeGen struct:
loop_stack: Vec<LoopContext<'ctx>>,
```

**Warning signs:**
- LLVM verification errors mentioning "terminator" or "successor".
- Compiler crashes after break/continue because `codegen_expr` tries to use the `undef` return value.
- Nested `if/break` patterns causing "basic block does not have terminator" on the merge block.
- Uninitialized alloca reads producing nondeterministic values.

**Phase to address:**
LLVM codegen phase. Must be designed into the codegen from the start -- break/continue require a loop context stack and careful basic block management.

---

### Pitfall 6: `for..in` Pattern Destructuring Conflicts with Existing Pattern System

**What goes wrong:**
`for {key, value} in my_map do ... end` requires pattern matching on the iteration variable. Snow already has a sophisticated pattern matching system (v1.4-v1.5) with decision tree compilation, exhaustiveness checking, and cons destructuring. But for-loop patterns are subtly DIFFERENT from case/match patterns:

1. **No exhaustiveness checking needed:** The loop pattern applies to EVERY element. If the pattern does not match, is it a runtime error or a silent skip? This must be decided explicitly.

2. **Binding scope:** In `case`, each arm has its own scope. In `for`, the pattern binding exists for the entire loop body and is RE-BOUND on each iteration. The binding must be stored to the same alloca (entry-block, per Pitfall 2) on each iteration.

3. **Tuple destructuring for maps:** `Map<K, V>` iteration produces `{K, V}` tuples. The pattern `{key, value}` must destructure this. Using the full pattern decision tree compiler for what is always a single irrefutable pattern is overkill and may produce incorrect codegen if the decision tree assumes multiple branches.

4. **Variable binding name resolution:** The v1.5 pitfall where pattern bindings like `head` were incorrectly mapped to the builtin function `snow_list_head` (PROJECT.md line 149) could recur. If someone writes `for list in lists do ... end`, the binding `list` must shadow any existing `list` variable.

**Why it happens:**
For-loop patterns have different requirements than match patterns. Reusing the match pattern infrastructure without adaptation leads to semantic mismatches.

**How to avoid:**
Implement for-loop patterns as a SIMPLIFIED subset of match patterns:

1. **Allowed patterns:** Variable binding (`x`), tuple destructuring (`{a, b}`), wildcard (`_`). No literal patterns, no constructor patterns, no nested matching.
2. **Codegen:** Do NOT use the decision tree compiler for for-loop patterns. Instead, emit direct load/GEP instructions.
3. **Name resolution:** Process loop variable bindings through the same scope push/pop mechanism used by `codegen_let` (expr.rs:952-968).
4. **Irrefutable only:** For v1.7, require irrefutable patterns. `for Some(x) in list do ... end` should be a compile error: "for-loop patterns must be irrefutable. Use `for item in list do case item do Some(x) -> ... end end` instead."

**Warning signs:**
- For-loop patterns producing decision trees with "unreachable" or "match failure" branches.
- Loop variable names resolving to existing functions/modules instead of fresh bindings.
- Tuple destructuring in for-loops producing incorrect field ordering.

**Phase to address:**
Parser and type inference phase. Pattern restrictions must be enforced in the parser or type checker before MIR lowering.

---

### Pitfall 7: GC Roots Lost During Loop -- Pointers Lifted to Registers Become Invisible

**What goes wrong:**
Collections (List, Map, Set) are GC-managed pointers. During a for-loop, the iterable collection pointer AND the accumulator (list builder) pointer must remain GC-reachable. Snow's mark-sweep GC per actor scans the stack conservatively (every 8-byte word treated as potential pointer, per PROJECT.md line 132). But LLVM's `mem2reg` pass promotes allocas to SSA registers. If the collection pointer is promoted to a register, it may not be on the stack when GC runs, and the collection could be freed mid-loop.

This is especially dangerous with `snow_list_builder_push` (or `snow_list_append`), which allocates and may trigger GC. The GC runs at yield points within `snow_reduction_check()` (actor/mod.rs:182-186). If the iterable's pointer is in a register when GC fires, the iterable may be collected.

**Why it happens:**
The alloca+mem2reg pattern is designed to produce efficient register-based code. But conservative GC requires root pointers to be on the stack. These goals conflict: mem2reg removes the stack slot, making the pointer invisible to the stack scanner.

**How to avoid:**
This is actually handled correctly by Snow's existing design, but requires careful attention:

1. **Allocas for GC pointers should NOT be promoted** by mem2reg. However, mem2reg promotes ALL eligible entry-block allocas. The solution: ensure that GC-managed pointers stored in allocas are loaded and stored frequently enough that LLVM keeps them on the stack, OR accept that conservative scanning of registers may miss them.

2. **Practical mitigation:** The `snow_reduction_check()` call at the loop back-edge creates a function call, which forces LLVM to spill live values to the stack (callee-saved registers). This means at the point where GC actually runs, the values ARE on the stack. This is the same reason why GC-at-yield-points works for the current actor code.

3. **Critical invariant:** The reduction check (and thus GC) only fires at the back-edge call. Between iterations, there is no GC. Within an iteration, allocations may grow the heap but will not trigger collection (collection only happens at `snow_reduction_check`). So the only point where roots matter is the back-edge, and at that point, the function call forces a spill.

4. **Still verify:** Add stress tests that force GC on every iteration (set DEFAULT_REDUCTIONS to 1 in test mode) and verify that loop collections are not collected prematurely.

**Warning signs:**
- Sporadic crashes or corrupted data in loops under memory pressure.
- Valgrind/ASAN reporting use-after-free in loop iterations.
- Test failures that only reproduce with specific GC timing.

**Phase to address:**
Codegen and runtime testing phase. Verify with GC stress tests, but the existing architecture should handle this correctly as long as `snow_reduction_check()` is called at the back-edge (Pitfall 3).

---

### Pitfall 8: Continue in For-Loop Skips Index Increment -- Infinite Loop

**What goes wrong:**
In a for-loop, `continue` must skip the rest of the body but ADVANCE to the next element. If `continue` branches directly to the loop header (condition check), and the loop header re-checks the SAME index (because the increment was in the body, which was skipped), the loop repeats the same element forever.

```
for_header:
  %idx = load %idx_alloca
  %cond = icmp slt %idx, %len
  br i1 %cond, %for_body, %for_exit

for_body:
  ; ... user code ...
  continue  ->  br label %for_header   ; BUG: idx not incremented!
  ; ... index increment below was skipped ...
  %next = add %idx, 1
  store %next, %idx_alloca
  br label %for_header
```

**Why it happens:**
The natural codegen structure puts the body expression and the index increment in the same block. `continue` jumps past the increment. This is the most common loop codegen bug in any compiler.

**How to avoid:**
Use a THREE-block structure separating the body from the latch (increment):

```
for_body:
  %elem = get_element(iterable, %idx)
  ; bind to loop var
  <body codegen>               ; continue branches to for_latch
  ; append body result to accumulator
  br label %for_latch

for_latch:                      ; continue targets HERE, not for_header
  %next = add %idx, 1
  store %next, %idx_alloca
  call void @snow_reduction_check()
  br label %for_header
```

`continue` branches to `for_latch`, ensuring the index always advances. The body result append is bypassed (continue means skip this element), but the index increment is not.

For `while` loops, `continue` branches directly to the header (condition re-evaluation), which is correct because while loops have no index.

**Warning signs:**
- `for x in list do if cond do continue end; x end` hangs (infinite loop).
- Condition true on first element + continue = repeat first element forever.

**Phase to address:**
LLVM codegen phase. The for-loop basic block structure must be designed with this three-block pattern from the start.

---

## Moderate Pitfalls

Mistakes that cause technical debt, confusing errors, or delayed regressions.

---

### Pitfall 9: Nested Loops Break/Continue Target Wrong Loop

**What goes wrong:**
Without labeled breaks, `break` and `continue` always target the innermost loop. If the codegen stores only one loop context (overwriting the outer loop's context when entering the inner loop), `break` in the inner loop will incorrectly jump to the outer loop's exit.

**How to avoid:**
Maintain a `Vec<LoopContext>` stack in the `CodeGen` struct. On loop entry: push a new context. On `break`: branch to `loop_stack.last().exit_bb`. On `continue`: branch to `loop_stack.last().latch_bb`. On loop exit: pop. Document that labeled breaks are a future feature.

**Warning signs:**
- Nested loops where `break` exits the wrong loop.
- LLVM verification errors about branches to blocks in the wrong scope.

**Phase to address:** LLVM codegen phase -- must be designed from the start.

---

### Pitfall 10: Closure Capture in Loop Body Captures Wrong Iteration's Value

**What goes wrong:**
If a closure is created inside a for-loop body, it captures the loop variable. Snow captures by value (no references -- PROJECT.md line 92). But if the closure captures the alloca pointer (implementation detail) rather than the current value, all closures see the final iteration's value.

**How to avoid:**
Snow's `MirExpr::MakeClosure` captures values by copying them into a GC-allocated environment struct at closure creation time. The MIR lowerer emits the capture list with the current iteration's variable reference, and codegen reads from the alloca at that point. This SHOULD work correctly because each closure creation reads the current alloca value.

Verify with an explicit test:
```
let fns = for x in [1, 2, 3] do fn() -> x end
# Assert: fns[0]() == 1, fns[1]() == 2, fns[2]() == 3
```

**Phase to address:** MIR lowering + codegen testing phase. Should work correctly but must be tested explicitly.

---

### Pitfall 11: Map/Set Iteration Requires New Runtime Functions

**What goes wrong:**
The runtime has `snow_map_keys()` and `snow_map_values()` (map.rs:229, 243) that return lists, but no function to iterate entries as `{key, value}` tuples. `for {k, v} in map do ... end` needs a way to get the i-th entry as a tuple. The current Map uses a vector of `(u64, u64)` pairs internally, but there is no `snow_map_get_entry(map, index)` function.

**How to avoid:**
Add runtime functions:
- `snow_map_entry_count(map) -> i64` (alias for `snow_map_size`)
- `snow_map_entry_key(map, index) -> u64`
- `snow_map_entry_value(map, index) -> u64`

Or, for the simpler approach: convert to a list of tuples before iteration. This doubles memory but is simple. For v1.7, the simple approach is acceptable; optimize later if maps are commonly iterated.

Similarly for Sets: add `snow_set_get(set, index) -> u64` for index-based iteration.

**Phase to address:** Runtime phase (snow-rt). Must be available before for-loop codegen can target maps/sets.

---

### Pitfall 12: MirExpr Match Exhaustiveness -- Adding New Variants Without Updating All Match Sites

**What goes wrong:**
The `MirExpr::ty()` method (mod.rs:308-340), `codegen_expr` (expr.rs:25-148), and `collect_function_refs` (mono.rs:86) all match on every MirExpr variant. Adding `ForLoop`, `WhileLoop`, `Break`, `Continue` requires updating ALL these matches. Rust's exhaustiveness checker will catch missing arms, but developers may be tempted to add a `_ => unreachable!()` wildcard that silently passes.

**How to avoid:**
Do NOT add wildcard catch-all arms. Let Rust's exhaustiveness checker force updates. Grep for `match.*MirExpr` across the codebase before claiming the new variants are integrated. Key match sites to update:
- `MirExpr::ty()` in mir/mod.rs
- `codegen_expr` in codegen/expr.rs
- `collect_function_refs` in mir/mono.rs
- Any Display/Debug impls for MirExpr

**Phase to address:** MIR definition phase. Audit all match sites when adding new variants.

---

### Pitfall 13: Range Operator Precedence with `in` Keyword

**What goes wrong:**
`for i in 0..10 do` must parse as `for i in (0..10) do`, not `for (i in 0) .. (10 do)`. The `..` operator already exists as `DOT_DOT` in the lexer. But its precedence relative to the new `in` keyword must be correct.

**How to avoid:**
The `for_expr` parser rule should parse the iterable as a regular expression (which handles `..` as a binary operator with its existing precedence). The parser structure is:
```
for_expr := "for" pattern "in" expr "do" block "end"
```
Where `expr` is parsed with normal expression rules, naturally handling `0..10` as `BinOp(0, .., 10)`.

**Phase to address:** Parser phase. Verify with parser tests for `for i in 0..10`, `for i in list`, `for {k,v} in map`.

---

## Technical Debt Patterns

Shortcuts that seem reasonable but create long-term problems.

| Shortcut | Immediate Benefit | Long-term Cost | When Acceptable |
|----------|-------------------|----------------|-----------------|
| `for..in` collects via `snow_list_append` chain | Reuses existing API, no new runtime code | O(N^2) time and memory; creates N garbage lists per loop | NEVER -- use list builder from day one |
| `while` loop returns `Unit` always | Simpler type inference; avoids break-with-value complexity | Cannot use `while` as expression to compute values | Acceptable for v1.7 -- even Rust punted on this |
| Single loop context instead of stack | Simpler codegen, fewer fields in CodeGen struct | Nested loops silently break | NEVER -- nested loops are table-stakes |
| Reuse full pattern compiler for for-loop patterns | Less new code; pattern infrastructure already robust | Overkill decision trees; confusing errors for non-exhaustive patterns | Only if restricted to irrefutable patterns with clear validation |
| Skip reduction check in loops | Slightly faster tight loops | Actor starvation, scheduler deadlock | NEVER -- breaks core actor model guarantee |
| `break(value)` support in for/while | More expressive loops | Complex type unification, two return types per loop | Defer to future version -- implement without break-with-value first |
| Convert map/set to list before iterating | No new runtime functions needed | Doubles memory usage; O(N) conversion overhead | Acceptable for v1.7 if maps/sets are small; optimize later |

## Integration Gotchas

Common mistakes when connecting loop constructs to existing Snow systems.

| Integration | Common Mistake | Correct Approach |
|-------------|----------------|------------------|
| Type inference + for..in | Inferring body type from first element; failing on empty collections | Infer body type from the body expression using element type as input; empty collection returns `List<T>` where T comes from the collection's type parameter |
| MIR lowering + break/continue | Lowering break/continue as expressions that return values | Lower as `MirExpr::Break` / `MirExpr::Continue` with type `Never`; MirExpr::ty() returns `MirType::Never` |
| Codegen + existing if/match | Placing loop result alloca inside loop body because codegen_if pattern "works" | Entry-block alloca for ALL loop-related storage; use `emit_entry_block_alloca` helper |
| GC + for..in collection | List builder's buffer not GC-visible because it is stack-only | Allocate builder on actor heap via `snow_gc_alloc_actor`; conservative scanning finds stack pointer to it |
| Monomorphization + for..in | New MirExpr variants not handled by `collect_function_refs` | Add explicit match arms; or lower loops to Call expressions that existing infrastructure handles |
| Pipe operator + for..in | Users try `list \|> for x do ... end` | `for..in` is a statement-level construct, not a function; clear parse error, not confusing type error |
| Formatter + new keywords | Formatter does not recognize `for`/`while`/`break`/`continue` | Add keyword handling to snow-fmt walker; test multi-line loop formatting |
| LSP + loop variables | LSP hover/completion does not show loop variable bindings | Add loop variable bindings to scope map in TypeckResult |
| Existing recursion patterns | Loops discourage idiomatic recursion; tail-call optimization may be neglected | Document that loops are syntactic sugar, not replacement for recursion; both patterns valid |

## Performance Traps

Patterns that work at small scale but fail as usage grows.

| Trap | Symptoms | Prevention | When It Breaks |
|------|----------|------------|----------------|
| O(N^2) list collection via append | Small loops fine; N=10K takes seconds | Use list builder with amortized O(1) push | N > ~1000 elements |
| No GC between iterations | Small loops fit in heap; large loops cause unbounded growth | Reduction check triggers GC; builder eliminates intermediate garbage | Loop allocation > 64KB (actor page size) before yield |
| Map/Set to-list conversion | Small maps fine; large maps double memory | Index-based iteration without intermediate list | Maps/Sets > ~1000 entries |
| String concatenation in loops | `result = result <> str` creates N intermediates | Accumulate in list and join, or provide String.Builder | N > ~100 string concatenations |
| Nested loops with collection | `for x in xs do for y in ys do {x,y} end end` creates N outer lists | Use `List.flatten` or flat_map pattern | N*M > ~10K |

## "Looks Done But Isn't" Checklist

Things that appear complete but are missing critical pieces.

- [ ] **for..in over empty List:** Returns empty list `[]`, not crash or Unit
- [ ] **for..in over empty Range:** `for x in 5..5 do x end` returns `[]`; inverted range `5..3` also returns `[]`
- [ ] **for..in over Map:** Iteration order is deterministic (insertion order); test with multiple entries
- [ ] **break in nested if:** `for x in list do if cond do break end; x end` -- break does not prevent collection of prior iterations
- [ ] **continue in for-loop:** Continued elements excluded from result; index still advances (no infinite loop)
- [ ] **Loop variable shadowing:** `let x = 10; for x in [1,2,3] do x end` -- loop x shadows outer; outer restored after
- [ ] **Closure capture in loop:** Closures in loop capture per-iteration value, not final value
- [ ] **Nested loops:** `for x in xs do for y in ys do {x, y} end end` produces correct `List<List<{Int, Int}>>`
- [ ] **Reduction check present:** `while true do 1 end` in an actor eventually yields (does not starve scheduler)
- [ ] **GC under pressure:** for..in over 100K elements does not OOM; GC collects between iterations
- [ ] **LLVM IR verified:** `opt -verify` passes for every loop test case
- [ ] **Pattern destructuring:** `for {k, v} in map do k end` correctly extracts tuple fields
- [ ] **Formatter:** `snow fmt` preserves loop syntax with correct indentation
- [ ] **Error messages:** break/continue outside loop gives specific error, not generic parse failure
- [ ] **Non-iterable error:** `for x in 5 do x end` gives "Int is not iterable", not generic type error
- [ ] **No alloca in loop body:** IR dump shows all loop-related allocas in entry block only
- [ ] **while returns Unit:** `let x: Int = while true do break end` is a type error (Unit != Int)

## Recovery Strategies

When pitfalls occur despite prevention, how to recover.

| Pitfall | Recovery Cost | Recovery Steps |
|---------|---------------|----------------|
| O(N^2) list collection (no builder) | MEDIUM | Add list builder runtime API; change codegen to use it; no source-level changes |
| Alloca inside loop body | LOW | Move alloca to entry block; add helper function; single codegen refactor |
| Missing reduction check | LOW | Add `emit_reduction_check()` call at back-edge; one-line codegen change |
| Break/continue orphaned blocks | MEDIUM | Add dead-block creation; may require refactoring codegen_expr Never handling |
| Wrong loop context in nested loops | LOW-MEDIUM | Replace single context with stack; straightforward if LoopContext struct is well-defined |
| Pattern system conflict | HIGH | Separating for-loop patterns from match patterns requires decision tree compiler refactor; prevent by keeping them separate from start |
| break(value) type unification | HIGH | Redesigning loop type system after implementation; prevent by deferring break(value) |
| Continue skips increment | LOW | Fix continue target to latch block; straightforward once identified |

## Pitfall-to-Phase Mapping

How roadmap phases should address these pitfalls.

| Pitfall | Prevention Phase | Verification |
|---------|------------------|--------------|
| P1: break(value) type unification | Syntax/AST design | Decision documented: break-with-value deferred; type rules specified |
| P2: alloca inside loop body | LLVM codegen | `opt -mem2reg` eliminates ALL loop allocas; verify with IR dump |
| P3: actor starvation | LLVM codegen | `snow_reduction_check` in IR at every back-edge; scheduler starvation test passes |
| P4: O(N^2) collection | Runtime (snow-rt) | List builder API; for..in codegen uses builder; benchmark shows O(N) |
| P5: orphaned basic blocks | LLVM codegen | `opt -verify` passes for all break/continue cases |
| P6: for pattern conflicts | Parser / typeck | Only irrefutable patterns accepted; refutable patterns produce clear error |
| P7: GC roots in loops | Codegen + runtime testing | GC stress tests pass with DEFAULT_REDUCTIONS=1 |
| P8: continue skips increment | LLVM codegen | Three-block structure: body / latch / header; continue targets latch |
| P9: nested loop context | LLVM codegen | Nested loop tests pass; break/continue target correct level |
| P10: closure capture | MIR + codegen testing | Explicit test: closures in loop capture per-iteration values |
| P11: map/set iteration | Runtime (snow-rt) | Entry-access functions or list conversion available |
| P12: MirExpr match exhaustiveness | MIR definition | No wildcard arms; all match sites updated |
| P13: range precedence | Parser | `for i in 0..10` parses correctly |

## Sources

### Snow Codebase Analysis (HIGH confidence -- direct code reading)
- `crates/snow-codegen/src/codegen/expr.rs` -- `codegen_if` alloca+mem2reg pattern (line 856-913), `codegen_let` variable scoping (line 917-971), `emit_reduction_check` (line 1653-1666), terminator checks (line 887, 899)
- `crates/snow-codegen/src/codegen/mod.rs` -- CodeGen struct, reduction check test (line 1254)
- `crates/snow-codegen/src/mir/mod.rs` -- MirExpr enum (no loop variants yet), MirType::Never (line 85), MirExpr::ty() exhaustive match (line 308-340)
- `crates/snow-codegen/src/mir/mono.rs` -- `collect_function_refs` reachability (line 86)
- `crates/snow-codegen/src/mir/lower.rs` -- MIR lowering, no loop lowering yet
- `crates/snow-codegen/src/pattern/compile.rs` -- Decision tree compiler for match patterns
- `crates/snow-rt/src/actor/mod.rs` -- `snow_reduction_check` with GC trigger (line 160-191), "loop back-edges" documented (line 155)
- `crates/snow-rt/src/actor/process.rs` -- DEFAULT_REDUCTIONS = 4000 (line 157)
- `crates/snow-rt/src/actor/heap.rs` -- Per-actor GC heap, conservative stack scanning, ACTOR_PAGE_SIZE = 64KB
- `crates/snow-rt/src/collections/list.rs` -- `snow_list_append` O(N) copy (line 76), `snow_list_map` pre-allocated (line 174)
- `crates/snow-rt/src/collections/range.rs` -- `snow_range_map` iteration pattern (line 51), empty/inverted range handling
- `crates/snow-rt/src/collections/map.rs` -- `snow_map_keys` / `snow_map_values` (line 229, 243), no entry-based iteration
- `crates/snow-rt/src/gc.rs` -- Global arena, `snow_gc_alloc_actor` per-actor allocation
- `crates/snow-typeck/src/infer.rs` -- Algorithm J inference, `infer_expr` dispatch
- `crates/snow-typeck/src/unify.rs` -- InferCtx, ena-based union-find unification

### LLVM Documentation (HIGH confidence)
- [LLVM Kaleidoscope Tutorial Ch. 5: Control Flow](https://llvm.org/docs/tutorial/MyFirstLanguageFrontend/LangImpl05.html) -- Loop codegen with phi nodes; "codegen() recursively could change the current block" warning; variable scope save/restore
- [LLVM Loop Terminology](https://llvm.org/docs/LoopTerminology.html) -- LCSSA form, loop-closing phi nodes for values live across loop boundaries
- [LLVM Frontend Performance Tips](https://llvm.org/docs/Frontend/PerformanceTips.html) -- "alloca in entry block" requirement; mem2reg only promotes entry-block allocas; SSA as canonical form
- [MLIR alloca-in-loop stack overflow](https://github.com/tensorflow/mlir/issues/210) -- Real-world example of stack overflow from loop-body allocas
- [LLVM Discourse: alloca in a loop](https://discourse.llvm.org/t/how-to-do-a-short-lived-alloca-in-a-loop/63248) -- Confirming cumulative stack growth from loop allocas

### Rust Language Design (HIGH confidence)
- [RFC 1624: Loop Break Value](https://rust-lang.github.io/rfcs/1624-loop-break-value.html) -- Type rules for break-with-value in `loop {}`; all breaks must agree; Never coerces to any type; natural termination unresolved for for/while
- [RFC Issue #1767: Allow for/while to return value](https://github.com/rust-lang/rfcs/issues/1767) -- Unresolved after 8+ years; five proposed approaches for natural-termination semantics; no consensus
- [Rust Loop Expressions Reference](https://doc.rust-lang.org/reference/expressions/loop-expr.html) -- Break-with-value only in `loop {}`; for/while always return `()`

### Runtime Scheduling & GC (MEDIUM confidence -- cross-language patterns)
- [Go Goroutine Preemption (1.14+)](https://dzone.com/articles/go-runtime-goroutine-preemption) -- Compiler-inserted preemption checks in loop back-edges; tight loops without calls starve peers
- [Nature Language Cooperative Scheduling](https://nature-lang.org/news/20260115) -- Safepoint instruction preemption as cooperative scheduling mechanism
- [OCaml Imperative Programming](https://dev.realworldocaml.org/imperative-programming.html) -- OCaml for/while loops always return unit; explicit loops complement recursion

---
*Pitfalls research for: Snow v1.7 Loops & Iteration milestone*
*Researched: 2026-02-08*
