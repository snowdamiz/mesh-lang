# Stack Research: Loops & Iteration

**Domain:** Compiler feature addition -- for..in loops, while loops, break/continue, Range iteration
**Researched:** 2026-02-08
**Confidence:** HIGH (based on direct codebase analysis + LLVM IR patterns + Inkwell API verification)

## Executive Summary

Loops and iteration require NO new external dependencies. The feature is implemented entirely through changes across all compiler layers (lexer, parser, typeck, MIR, codegen) plus two small runtime additions. The critical design decisions are: (1) use the alloca+branch pattern for loop result values (matching the existing `codegen_if` pattern, NOT phi nodes), (2) implement `for..in` as a compiler-generated index loop over collections (NOT an iterator protocol), and (3) implement `break`/`continue` via alloca+conditional-branch to loop exit/header blocks (NOT exception-style unwinding or setjmp/longjmp).

The existing codebase already has `for`, `in`, and `do`/`end` as keywords. `while`, `break`, and `continue` must be added as new keywords. The Range runtime already has `snow_range_new` and `snow_range_length`; Set needs a new `snow_set_to_list` runtime function for iteration. All other collections (List, Map, Range) already expose the necessary runtime primitives for index-based iteration (`snow_list_length`/`snow_list_get`, `snow_map_keys`/`snow_map_values`, `snow_range_length`/direct start+end access).

## What Exists Today (DO NOT CHANGE)

These are existing capabilities that loops build on, not replaces.

### Keywords Already Reserved

| Keyword | Status | Relevance |
|---------|--------|-----------|
| `for` | Already lexed as `TokenKind::For`, mapped to `SyntaxKind::FOR_KW` | Used for `for..in` syntax |
| `in` | Already lexed as `TokenKind::In`, mapped to `SyntaxKind::IN_KW` | Used for `for x in collection` |
| `do` | Already lexed as `TokenKind::Do` | Block delimiter: `for x in coll do ... end` |
| `end` | Already lexed as `TokenKind::End` | Block terminator |

### Runtime Primitives Already Available

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_list_length(list) -> i64` | `(ptr) -> i64` | Get list element count |
| `snow_list_get(list, index) -> u64` | `(ptr, i64) -> i64` | Get element at index |
| `snow_map_keys(map) -> ptr` | `(ptr) -> ptr` | Get List of keys |
| `snow_map_values(map) -> ptr` | `(ptr) -> ptr` | Get List of values |
| `snow_map_size(map) -> i64` | `(ptr) -> i64` | Get entry count |
| `snow_set_size(set) -> i64` | `(ptr) -> i64` | Get element count |
| `snow_range_new(start, end) -> ptr` | `(i64, i64) -> ptr` | Create Range [start, end) |
| `snow_range_length(range) -> i64` | `(ptr) -> i64` | Get Range element count |

### LLVM Codegen Patterns Already Established

| Pattern | Where Used | Relevance |
|---------|-----------|-----------|
| Alloca + store + load for control flow merges | `codegen_if` (expr.rs:856-913) | Loop result values use same pattern |
| `append_basic_block` + `build_conditional_branch` | `codegen_if`, `codegen_match` | Loop header/body/exit blocks |
| `build_unconditional_branch` with terminator check | `codegen_if` then/else branches | Loop back-edge and break/continue |
| `build_phi` for short-circuit booleans | `codegen_binop` AND/OR (expr.rs:334-410) | Available but NOT recommended for loops (alloca pattern is simpler) |

### Type System

| Type | Representation | Iteration Behavior |
|------|---------------|-------------------|
| `List<T>` | `Ty::App(Con("List"), [T])` | Yields `T` elements by index |
| `Map<K, V>` | `Ty::App(Con("Map"), [K, V])` | Yields `(K, V)` tuple pairs via keys+values lists |
| `Set<T>` | `Ty::App(Con("Set"), [T])` | Yields `T` elements (needs `snow_set_to_list`) |
| `Range` | `Ty::Con(TyCon::new("Range"))` | Yields `Int` values from start to end-1 |
| `String` | `Ty::Con(TyCon::new("String"))` | Defer string iteration to future milestone |

## Recommended Stack Changes

### Change 1: New Keywords -- `while`, `break`, `continue` (Lexer Layer)

**What:** Add three new keywords to `TokenKind` and `SyntaxKind`.

**Why:** `for` and `in` are already keywords. `while`, `break`, and `continue` are not. These are needed for while-loop syntax and non-local loop control flow.

| File | Change | Details |
|------|--------|---------|
| `snow-common/src/token.rs` | Add `While`, `Break`, `Continue` to `TokenKind` | 3 new keyword variants |
| `snow-common/src/token.rs` | Add to `keyword_from_str()` match | `"while" => Some(TokenKind::While)`, etc. |
| `snow-parser/src/syntax_kind.rs` | Add `WHILE_KW`, `BREAK_KW`, `CONTINUE_KW` | 3 new SyntaxKind variants |
| `snow-parser/src/syntax_kind.rs` | Add `From<TokenKind>` conversions | `TokenKind::While => SyntaxKind::WHILE_KW`, etc. |

**Keyword count:** Goes from 45 to 48. Tests that assert `keywords.len() == 45` must be updated.

**Why not reuse existing keywords:** `while` has no existing keyword equivalent. `break` and `continue` could theoretically be functions, but they need special control-flow semantics (jump to loop header/exit) that cannot be expressed as function calls.

### Change 2: New CST Nodes -- FOR_EXPR, WHILE_EXPR, BREAK_EXPR, CONTINUE_EXPR (Parser Layer)

**What:** Add four new `SyntaxKind` variants and corresponding AST wrapper types.

**Why:** The parser must produce distinct CST nodes for each loop construct so that later passes (typeck, MIR) can handle them with the correct semantics.

| SyntaxKind | Syntax | AST Type | Children |
|------------|--------|----------|----------|
| `FOR_EXPR` | `for x in coll do body end` | `ForExpr` | binding pattern, iterable expr, body block |
| `WHILE_EXPR` | `while cond do body end` | `WhileExpr` | condition expr, body block |
| `BREAK_EXPR` | `break` or `break value` | `BreakExpr` | optional value expr |
| `CONTINUE_EXPR` | `continue` | `ContinueExpr` | (none) |

**Parser grammar (Elixir-style):**

```
for_expr   := FOR pattern IN expr DO block END
while_expr := WHILE expr DO block END
break_expr := BREAK [expr]
continue_expr := CONTINUE
```

**Why `for..in..do..end` (not `for..in..{}`)**:  Snow uses `do..end` blocks everywhere (if, case, fn, actor, service, supervisor). Consistency with existing syntax is non-negotiable.

**Why `break value` (not just `break`):** Both `for` and `while` are expressions that return values. `break value` provides the return value when breaking early, matching Rust's `break 'label value` semantics. Without a value, `break` returns `Unit`.

### Change 3: New MIR Nodes -- WhileLoop, ForLoop, Break, Continue (MIR Layer)

**What:** Add four new `MirExpr` variants.

**Why:** MIR must represent loops explicitly so that codegen can emit the correct basic block structure. Desugaring loops entirely at the typeck/AST level would be possible but fragile -- the MIR is the right level for explicit control flow.

| MirExpr Variant | Fields | Type |
|-----------------|--------|------|
| `WhileLoop { cond, body, ty }` | condition: `Box<MirExpr>`, body: `Box<MirExpr>`, ty: `MirType` | Result type of the loop (from `break value` or `Unit`) |
| `ForLoop { var, iterable, body, ty, collection_kind }` | var: `String`, iterable: `Box<MirExpr>`, body: `Box<MirExpr>`, ty: `MirType`, collection_kind: `CollectionKind` | Loop result type |
| `Break { value, ty }` | value: `Option<Box<MirExpr>>`, ty: `MirType::Never` | Never type (break transfers control) |
| `Continue` | (none) | `MirType::Never` |

**`CollectionKind` enum:**

```rust
enum CollectionKind {
    List,        // index loop: snow_list_length + snow_list_get
    Map,         // key-value pairs: snow_map_keys + snow_map_values + indexed access
    Set,         // convert to list: snow_set_to_list + indexed access
    Range,       // integer range: direct start/end arithmetic, no runtime calls in loop body
}
```

**Why `CollectionKind` in MIR (not generic iteration):** Snow does not have an Iterator trait or protocol. For v1.7, loop iteration is a compiler-known desugaring for each collection type. This avoids the complexity of iterator state machines, lazy evaluation, and trait resolution. Adding a generic `Iterable` trait is a future milestone.

**Why NOT desugar `for` into `while` in MIR:** While it is possible to desugar `for x in list do ... end` into a `while` loop with explicit index management, keeping `ForLoop` as a distinct MIR node has advantages: (1) codegen can emit optimized IR per collection type (Range uses pure integer arithmetic without runtime calls in the loop body), (2) future iterator protocol can be grafted onto the `ForLoop` node without changing the while-loop implementation, (3) debug info and error messages can reference the original `for` construct.

### Change 4: Type Checking Rules (Typeck Layer)

**What:** Add inference rules for all four new expression types.

**Why:** The type checker must determine: (1) what type the loop variable binds to (element type from collection type), (2) what type the loop expression evaluates to (from `break value` or body type for `for`, `Unit` for `while` by default), (3) that `break`/`continue` only appear inside loops.

**Key type rules:**

| Expression | Inferred Type | Rule |
|------------|--------------|------|
| `for x in list do body end` | `List<T>` where `T` = body's return type | Collects body results into a new list (map-like semantics) |
| `for x in range do body end` | `List<T>` where `T` = body's return type | Same map-like semantics |
| `while cond do body end` | `Unit` (or type of `break value` if used) | Loops are Unit by default; `break value` overrides |
| `break` | `Never` | Transfers control; does not produce a value at the break site |
| `break value` | `Never` | The value's type must unify with the loop's result type |
| `continue` | `Never` | Transfers control |

**For-loop element type extraction:**

| Collection Type | Element Type | How Extracted |
|----------------|--------------|---------------|
| `List<T>` | `T` | `extract_list_elem_type` (already exists in lower.rs) |
| `Map<K, V>` | `(K, V)` tuple | New extraction from `Ty::App(Con("Map"), [K, V])` |
| `Set<T>` | `T` | New extraction from `Ty::App(Con("Set"), [T])` |
| `Range` | `Int` | Hardcoded (Range always yields Int) |

**For-loop return semantics (critical design decision):**

Two plausible designs exist:

- **Option A: for-loop as map (returns `List<T>`)** -- `for x in [1,2,3] do x * 2 end` returns `[2,4,6]`
- **Option B: for-loop as statement (returns `Unit`)** -- `for x in [1,2,3] do x * 2 end` returns `()`

**Recommendation: Option A (map semantics).** This follows Elixir's `for` comprehension and makes `for..in` immediately useful as an expression. The `while` loop serves the "statement-like, repeat until done" use case. This differentiation makes both constructs feel purposeful rather than redundant.

**Context validation for break/continue:**

The type checker must track whether it is inside a loop. A boolean `in_loop` flag or a loop-context stack is sufficient. `break`/`continue` outside a loop produces a diagnostic error. Break's value type must unify with the for-loop's element type or the while-loop's result type.

### Change 5: LLVM Codegen -- Loop Basic Block Structure (Codegen Layer)

**What:** Implement `codegen_while_loop`, `codegen_for_loop`, `codegen_break`, `codegen_continue`.

**Why:** This is where loops become actual machine code. The LLVM IR structure follows well-established patterns used by every LLVM-based compiler.

#### While-loop IR pattern (alloca+branch, NOT phi):

```
entry:
  %result = alloca T                  ; result slot (for break value)
  store unit, %result                 ; default: Unit
  br label %loop_header

loop_header:
  %cond = <codegen condition>
  br i1 %cond, label %loop_body, label %loop_exit

loop_body:
  <codegen body>
  br label %loop_header               ; back-edge

loop_exit:
  %val = load T, %result              ; load result (unit or break value)
  ; continues to next instruction
```

**Why alloca+branch, NOT phi nodes:** The existing Snow codegen uses alloca+store+load for `if` expressions (expr.rs:866-912). The alloca pattern is simpler to implement, easier to reason about for `break` from nested contexts, and LLVM's `mem2reg` pass promotes these allocas to SSA registers anyway. Phi nodes require tracking which basic block each incoming value came from, which gets complex with nested loops, break, and continue. The alloca pattern sidesteps this entirely.

#### For-loop over List IR pattern:

```
entry:
  %result_list = call @snow_list_new()    ; accumulator for map semantics
  %len = call @snow_list_length(%iterable)
  %idx = alloca i64
  store i64 0, %idx
  br label %for_header

for_header:
  %i = load i64, %idx
  %done = icmp sge i64 %i, %len
  br i1 %done, label %for_exit, label %for_body

for_body:
  %elem = call @snow_list_get(%iterable, %i)
  ; bind elem to loop variable
  <codegen body>
  %result_list = call @snow_list_append(%result_list, %body_val)
  %next = add i64 %i, 1
  store i64 %next, %idx
  br label %for_header

for_exit:
  ; %result_list is the loop's value
```

#### For-loop over Range IR pattern (optimized -- no runtime calls in body):

```
entry:
  %start_ptr = bitcast %range to i64*
  %start = load i64, %start_ptr           ; direct memory access, no fn call
  %end_ptr = getelementptr i64, %start_ptr, 1
  %end = load i64, %end_ptr
  %result_list = call @snow_list_new()
  %idx = alloca i64
  store i64 %start, %idx
  br label %range_header

range_header:
  %i = load i64, %idx
  %done = icmp sge i64 %i, %end
  br i1 %done, label %range_exit, label %range_body

range_body:
  ; %i IS the element (no snow_list_get needed)
  <codegen body with %i as loop variable>
  %result_list = call @snow_list_append(%result_list, %body_val)
  %next = add i64 %i, 1
  store i64 %next, %idx
  br label %range_header

range_exit:
  ; %result_list is the loop's value
```

**Why Range is optimized:** Range has a known layout `{i64 start, i64 end}`. The loop counter IS the element value. No runtime function calls inside the loop body (unlike List which needs `snow_list_get` per iteration). The start and end values are loaded once before the loop. This makes `for i in 0..1000000 do ... end` as fast as a C for-loop.

#### Break/Continue codegen:

```
; break (no value):
  store unit, %loop_result_alloca
  br label %loop_exit

; break value:
  %v = <codegen value>
  store %v, %loop_result_alloca
  br label %loop_exit

; continue:
  br label %loop_header
```

**Implementation mechanism:** The `CodeGen` struct needs a **loop context stack** to track the current loop's header block, exit block, and result alloca:

```rust
struct LoopContext<'ctx> {
    header_bb: BasicBlock<'ctx>,
    exit_bb: BasicBlock<'ctx>,
    result_alloca: Option<PointerValue<'ctx>>,  // None for for-loops (result is the list)
}

// Added to CodeGen struct:
loop_stack: Vec<LoopContext<'ctx>>,
```

`break` and `continue` pop from this stack to find their target blocks. Nested loops work naturally -- inner break/continue target the innermost loop's blocks.

### Change 6: New Runtime Functions (Runtime Layer)

**What:** Add `snow_set_to_list` and `snow_range_start`/`snow_range_end` helper functions.

| Function | Signature | Purpose | Why Needed |
|----------|-----------|---------|------------|
| `snow_set_to_list(set) -> ptr` | `(ptr) -> ptr` | Convert Set elements to a List for indexed iteration | Set has no `get(index)` -- must convert to List first |
| `snow_range_start(range) -> i64` | `(ptr) -> i64` | Get start value of Range | Codegen can GEP directly, but this is cleaner for the initial impl |
| `snow_range_end(range) -> i64` | `(ptr) -> i64` | Get end value of Range | Same rationale |

**Note on Range codegen:** For the optimized Range path, codegen could directly emit GEP instructions to read the `{i64, i64}` struct fields (since Range layout is `{start: i64, end: i64}` at offset 0 and 8). The `snow_range_start`/`snow_range_end` functions are an optional convenience. The recommendation is to use direct GEP for Range (matching how struct field access already works in codegen) and add the runtime functions only if needed for other purposes.

**Set iteration strategy:** Convert Set to List upfront (`snow_set_to_list`), then iterate the List. This is simpler than adding `snow_set_get(set, index)` (which would require the Set to maintain a stable index, conflicting with its unordered semantics). The conversion cost is O(n) and happens once before the loop, which is acceptable for the typical set sizes in Snow programs.

**Map iteration strategy:** Use existing `snow_map_keys()` to get a List of keys, then iterate that List. For each key, call `snow_map_get(map, key)` to get the value. Alternatively, iterate both `snow_map_keys()` and `snow_map_values()` in parallel by index (both return Lists of the same length). The parallel-list approach avoids redundant hash lookups.

### Change 7: Intrinsic Declarations (Codegen Layer)

**What:** Declare new runtime functions in `intrinsics.rs`.

| Declaration | Purpose |
|-------------|---------|
| `snow_set_to_list(ptr) -> ptr` | Set-to-List conversion for for..in iteration |

**Why minimal:** Most iteration is done via existing intrinsics (`snow_list_length`, `snow_list_get`, `snow_range_length`). Only Set lacks the necessary primitives.

### Change 8: Formatter Support (Tooling Layer)

**What:** Handle new SyntaxKind variants in `snow-fmt`.

| File | Change |
|------|--------|
| `snow-fmt/src/walker.rs` | Walk `FOR_EXPR`, `WHILE_EXPR`, `BREAK_EXPR`, `CONTINUE_EXPR` |
| `snow-fmt/src/printer.rs` | Format with correct indentation and newlines |
| `snow-fmt/src/ir.rs` | IR nodes for new constructs |

### Change 9: LSP Support (Tooling Layer)

**What:** Handle new expression types in `snow-lsp` for syntax highlighting and analysis.

| File | Change |
|------|--------|
| `snow-lsp/src/analysis.rs` | Recognize loop constructs |
| `snow-lsp/src/server.rs` | Semantic tokens for new keywords |

## Alternatives Considered

| Decision | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Loop result pattern | Alloca + branch | Phi nodes | Alloca is simpler, matches existing `if` pattern, LLVM `mem2reg` optimizes to SSA anyway |
| For-loop iteration | Compiler-generated index loop | Iterator trait/protocol | Too complex for v1.7; iterator state machines, lazy evaluation, and trait resolution are a full milestone |
| Break/continue mechanism | Branch to loop header/exit blocks | Exception-style unwinding (longjmp) | Unwinding is expensive, fragile with GC, and unnecessary for structured loops |
| For-loop return type | `List<T>` (map semantics) | `Unit` (statement semantics) | Map semantics makes `for` immediately useful as an expression; `while` handles the statement case |
| Set iteration | Convert to List upfront | Add `snow_set_get(index)` | Set is unordered; index-based access is semantically misleading |
| Range iteration | Direct GEP on range struct | Call `snow_range_to_list` then iterate list | Direct GEP avoids O(n) allocation; Range loop is as fast as C for-loop |
| MIR representation | Explicit `WhileLoop`/`ForLoop` nodes | Desugar to basic blocks in MIR | Explicit nodes preserve source-level semantics for diagnostics and future optimization |

## What NOT to Add

| Feature | Why Not |
|---------|---------|
| Iterator trait/protocol | Too complex for v1.7. Requires trait method dispatch for `next()`, state machine generation, lazy evaluation semantics. Separate milestone. |
| `loop` (infinite loop keyword) | `while true do ... end` is clear enough. Can add later. |
| Labeled breaks (`break :outer`) | Nested loop break-to-label requires label scoping, block naming. Defer to future. |
| `for..in` over String (character iteration) | Requires exposing String as a sequence of characters. String is currently opaque `{len, data}`. Defer. |
| Parallel/async iteration | Actor-based parallelism exists via `Job.map`. Loop-level parallelism is a separate concern. |
| Generator/yield | Coroutine-based generators require stack management. The actor system uses corosensei but loops should not. |
| Comprehension guards (`for x in list, x > 0`) | Elixir supports this, but it adds parser complexity. Use `for x in list.filter(fn x -> x > 0 end)` instead. |
| `else` clause on loops | Python's `for...else` is widely considered confusing. Do not add. |

## Technology Versions

| Technology | Current Version | Required Changes | Version Impact |
|------------|----------------|-----------------|----------------|
| Inkwell | 0.8.0 (llvm21-1) | None -- `build_alloca`, `build_conditional_branch`, `append_basic_block`, `build_phi` all available | No version change |
| Rowan | 0.16 | None -- add SyntaxKind variants (no API change) | No version change |
| Ariadne | 0.6 | None -- new diagnostic messages use existing API | No version change |
| snow-common | - | Add 3 keywords to `TokenKind` | Internal change only |
| snow-parser | - | Add 4 SyntaxKind variants, 4 AST types, parser rules | Internal change only |
| snow-typeck | - | Add loop/break/continue inference rules, loop context tracking | Internal change only |
| snow-codegen (MIR) | - | Add 4 MirExpr variants, `CollectionKind` enum, lowering logic | Internal change only |
| snow-codegen (LLVM) | - | Add loop codegen functions, `LoopContext` struct, break/continue codegen | Internal change only |
| snow-rt | - | Add `snow_set_to_list` function | Internal change only |

**No dependency additions. No version bumps. No new crates.**

## Crate-by-Crate Change Summary

| Crate | Changes | Estimated Lines |
|-------|---------|----------------|
| `snow-common` | 3 keywords in `TokenKind`, update `keyword_from_str` | ~15 |
| `snow-lexer` | Update tests for keyword count (45 -> 48) | ~10 |
| `snow-parser` | 4 SyntaxKind variants, 4 AST types, parser rules for for/while/break/continue | ~200 |
| `snow-typeck` | Inference rules for 4 new expressions, loop context tracking, element type extraction | ~150 |
| `snow-codegen` (MIR) | 4 MirExpr variants, CollectionKind enum, lowering for for/while/break/continue | ~250 |
| `snow-codegen` (codegen) | `codegen_while_loop`, `codegen_for_loop`, `codegen_break`, `codegen_continue`, `LoopContext` | ~300 |
| `snow-codegen` (intrinsics) | Declare `snow_set_to_list` | ~5 |
| `snow-rt` | `snow_set_to_list` implementation | ~20 |
| `snow-fmt` | Handle 4 new SyntaxKind variants | ~40 |
| `snow-lsp` | Recognize new constructs | ~20 |
| Tests | Parser, typeck, MIR, codegen, e2e tests | ~400 |

**Total estimated:** ~1,400 lines of new/modified code.

## Critical LLVM IR Patterns Reference

### 1. The Alloca+Branch Pattern (established in Snow codebase)

From `codegen_if` (expr.rs:856-913), the pattern Snow uses for control-flow merges:

```
%result = alloca T
; ... branches that store to %result ...
%val = load T, %result
```

LLVM's `mem2reg` pass (part of the standard optimization pipeline) converts these allocas to phi nodes automatically. This means the generated IR is semantically equivalent to hand-written phi nodes but much easier to produce programmatically.

### 2. Loop Back-Edge Detection

LLVM identifies loops via back-edges in the CFG. The `loop_body -> loop_header` branch is a back-edge because `loop_header` dominates `loop_body`. LLVM's loop optimization passes (LICM, loop unrolling, induction variable simplification) automatically detect this structure. No special annotations needed.

### 3. Terminator Checking

From `codegen_if` (expr.rs:887, 899): before emitting a branch, check if the current block already has a terminator. This is critical for `break`/`continue` -- after emitting a branch to the loop exit/header, the current block IS terminated, so subsequent code in the loop body must not emit another terminator. The pattern:

```rust
if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
    self.builder.build_unconditional_branch(target_bb)?;
}
```

This is essential for correctness when `break`/`continue` appear in the middle of a block (before other expressions).

## Sources

### Primary (HIGH confidence)
- Direct codebase analysis: 67,546 lines of Snow compiler across 11 crates
- `snow-codegen/src/codegen/expr.rs` lines 856-913: existing alloca+branch pattern for `if`
- `snow-codegen/src/codegen/expr.rs` lines 334-410: existing phi node pattern for short-circuit AND/OR
- `snow-codegen/src/mir/mod.rs`: full MirExpr enum (current 30+ variants)
- `snow-common/src/token.rs`: keyword list (45 keywords, `for`/`in` present, `while`/`break`/`continue` absent)
- `snow-rt/src/collections/range.rs`: Range layout `{i64 start, i64 end}`, existing API
- `snow-rt/src/collections/list.rs`: List layout `{u64 len, u64 cap, data[]}`, `snow_list_get`/`snow_list_length`
- `snow-rt/src/collections/set.rs`: Set API (NO `to_list` or index-get function)
- `snow-rt/src/collections/map.rs`: `snow_map_keys`/`snow_map_values` return Lists
- `snow-codegen/src/codegen/intrinsics.rs`: all declared runtime functions

### Secondary (MEDIUM-HIGH confidence)
- [Inkwell GitHub Repository](https://github.com/TheDan64/inkwell) -- API for `build_phi`, `build_alloca`, `build_conditional_branch`
- [Inkwell BasicBlock Documentation](https://thedan64.github.io/inkwell/inkwell/basic_block/struct.BasicBlock.html) -- BasicBlock API
- [LLVM Language Reference: Loop Terminology](https://llvm.org/docs/LoopTerminology.html) -- LLVM's loop detection via back-edges
- [LLVM mem2reg pass](https://llvm.org/docs/Passes.html#mem2reg-promote-memory-to-register) -- alloca promotion to SSA

### Tertiary (MEDIUM confidence)
- [Create Your Own Programming Language with Rust](https://createlang.rs/01_calculator/basic_llvm.html) -- Inkwell tutorial with basic block patterns
- Elixir for-comprehension semantics -- `for x <- list, do: expr` returns collected results

---
*Stack research for: Snow compiler loops & iteration features (v1.7)*
*Researched: 2026-02-08*
