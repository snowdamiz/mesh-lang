# Project Research Summary

**Project:** Snow Compiler - Loops & Iteration (v1.7)
**Domain:** Compiler feature addition for statically-typed, functional-first language with LLVM codegen
**Researched:** 2026-02-08
**Confidence:** HIGH

## Executive Summary

Snow's loops and iteration feature (v1.7) requires NO new external dependencies. The entire feature is implemented through changes across all six compiler layers (lexer, parser, typeck, MIR, codegen, runtime) plus two small runtime additions. Research confirms that Snow should implement for-in loops as expression-returning list comprehensions (matching Elixir/Scala semantics), while loops as Unit-returning constructs, and break/continue as non-local control flow via LLVM basic block branches.

The critical architectural decision is using the alloca+mem2reg pattern for loop state management (matching Snow's existing if-expression pattern) rather than explicit phi nodes. For-in loops desugar to index-based iteration at the MIR level, avoiding the complexity of iterator protocols entirely. Range iteration compiles to pure integer arithmetic with zero heap allocation, making `for i in 0..n` as fast as a C for-loop. The actor scheduler's cooperative preemption requires reduction checks at loop back-edges to prevent starvation.

Key risks are well-understood and preventable: (1) alloca placement inside loop bodies breaks mem2reg optimization and causes stack overflow, (2) for-in collection via immutable append chains creates O(N²) performance, requiring a list builder API, (3) continue in for-loops must target a separate latch block (not the header) to avoid infinite loops. All three have clear mitigation strategies grounded in LLVM best practices and existing Snow patterns.

## Key Findings

### Recommended Stack

**No new dependencies.** Loops are implemented entirely within the existing compiler architecture using Inkwell 0.8.0 (llvm21-1), Rowan 0.16, and Snow's HM type inference. Three new keywords (`while`, `break`, `continue`) join the existing `for` and `in` keywords.

**Core technologies:**
- **Inkwell LLVM bindings** — Loop codegen uses `build_alloca`, `append_basic_block`, `build_conditional_branch` (all available in current version)
- **Snow's alloca+mem2reg pattern** — Established in `codegen_if` (expr.rs:856-913), reused for loop result values and state
- **Conservative GC stack scanning** — Works correctly with loop allocas; reduction checks force register spills at GC points
- **MIR desugaring** — For-in becomes explicit indexed iteration; codegen never sees high-level loop syntax

**Version impact:** None. Internal crate changes only, no dependency updates.

### Expected Features

**Must have (table stakes):**
- `for x in collection do body end` — Universal iteration pattern expected by users from every language
- `for i in range do body end` — Counted iteration via Range type (1..10)
- `while condition do body end` — Conditional repetition for event loops and retry logic
- `break` / `continue` — Early exit and skip-to-next-iteration, standard in all loop implementations
- `for {k, v} in map do ... end` — Destructuring iteration with pattern matching
- Correct scoping (loop variable fresh per iteration) and zero-iteration semantics (empty collection)

**Should have (differentiators):**
- **for-in as expression returning List<T>** — `let doubled = for x in list do x * 2 end` collects body results (killer feature for functional-first language)
- **Range iteration without allocation** — `for i in 0..n` compiles to pure integer arithmetic, zero heap allocation
- **for-in with filter clause (when)** — `for x in list when x > 0 do ... end` reuses existing guard syntax
- **break without value** — Exits early from for-in, returns partial collected list

**Defer (v2+):**
- `break(value)` from while loops — Type unification problem (loop has two return types), punted by Rust RFC 1767
- Iterator protocol / Iterable trait — Requires state machines, lazy evaluation, trait dispatch (full milestone)
- Labeled breaks — Requires label scoping, rare use case
- String character iteration — Requires Unicode handling (grapheme clusters, UTF-8 decoding)

### Architecture Approach

Loops desugar at the MIR level to avoid adding general mutation to Snow's functional core. For-in becomes MirExpr::ForIn with an IterKind enum (List/Map/Set/Range) that codegen translates to indexed iteration. While becomes MirExpr::While with straightforward header/body/exit block structure. Break and continue are MirExpr::Break / MirExpr::Continue with type Never (diverging), matching return and panic semantics.

**Major components:**
1. **Lexer/Parser** — Add 3 keywords, 4 SyntaxKind variants, 4 AST nodes; mechanical changes to token.rs and expressions.rs
2. **Type Checker** — Element type extraction from collections (List<T> -> T, Range -> Int), loop context tracking for break/continue validation, for-in result type as List<body_type>
3. **MIR Lowering** — For-in desugars to indexed iteration with collection-specific access patterns; while lowers directly; loop context not needed (break/continue are primitives)
4. **LLVM Codegen** — Three-block structure (header/body/latch) for for-in, two-block (header/body) for while; loop context stack for break/continue targeting; entry-block allocas for all loop state
5. **Runtime** — List builder API (snow_list_builder_new/push/finish) for O(N) collection, snow_set_to_list for Set iteration

### Critical Pitfalls

1. **LLVM alloca inside loop body breaks mem2reg** — Allocas must be placed in function entry block before loop header, or mem2reg won't promote to registers. Loop-body allocas cause stack overflow (1M iterations = 1M stack slots) and 10-100x performance regression. Prevention: Use `emit_entry_block_alloca` helper that temporarily repositions builder.

2. **O(N²) list collection via immutable append** — `snow_list_append` copies all N elements per append, causing O(N²) work and O(N²) garbage. For N=100K, this is 5 billion copies and bytes. Prevention: Implement list builder with doubling growth (like Rust Vec), giving O(N) amortized. This is not an optimization, it's required for correctness.

3. **Actor starvation from tight loops without reduction checks** — A loop with no function calls never yields to the scheduler. `for i in 0..1_000_000 do i+1 end` monopolizes the worker thread, starving other actors. Prevention: Insert `snow_reduction_check()` at loop back-edge (before branching to header). This matches BEAM VM and Go 1.14+ preemption.

4. **Continue in for-loop skips index increment causing infinite loop** — If continue branches to loop header instead of latch block, the index never advances. Prevention: Use three-block structure (body -> latch -> header) where continue targets latch (increment), not header.

5. **Expression-returning loops with break(value) breaks HM unification** — For-in returns List<T>, but break(value) returns value_type. These can't unify (List<Int> vs String). Rust punted on this for 8+ years. Prevention: Defer break(value) to future version; for-in break without value returns partial list; while always returns Unit.

## Implications for Roadmap

Based on research, loops require strictly sequential implementation through the compiler pipeline. Each layer depends on the previous one completing.

### Phase 1: Foundation - Keywords + While Loop
**Rationale:** While loops are simpler than for-in (no collection iteration, no destructuring, no result accumulation). Establishing the basic loop infrastructure (keywords, parser, MIR nodes, codegen block structure) on the simpler construct reduces risk.

**Delivers:** Working while loops, break, and continue. Demonstrates LLVM loop codegen pattern and reduction check integration.

**Addresses:**
- STACK.md: New keywords (`while`, `break`, `continue`), WHILE_EXPR/BREAK_EXPR/CONTINUE_EXPR SyntaxKind, MirExpr::While/Break/Continue
- FEATURES.md: while loop, break, continue (table stakes)
- PITFALLS.md: Establishes alloca placement pattern (P2), reduction check integration (P3), loop context stack for nested loops (P9)

**Avoids:**
- P2 (alloca in loop body) by implementing `emit_entry_block_alloca` helper upfront
- P3 (actor starvation) by adding reduction check to initial while implementation
- P5 (terminated block writes) by modeling break/continue as Never-typed from the start

**Estimated effort:** 5-7 days

### Phase 2: For-In Over Range (Zero-Allocation Fast Path)
**Rationale:** Range iteration is the simplest for-in case — no runtime function calls in the loop body, just direct arithmetic. This validates the for-in desugaring pattern before adding collection-specific complexities.

**Delivers:** `for i in 1..10 do ... end` working with Range type, demonstrating optimized integer counter loop.

**Addresses:**
- STACK.md: FOR_EXPR parser production, MirExpr::ForIn with IterKind::Range, for-in as expression semantics
- FEATURES.md: Range iteration (table stakes), range iteration without allocation (differentiator)
- ARCHITECTURE.md: For-in MIR lowering pattern, three-block loop structure (header/body/latch)
- PITFALLS.md: Continue targeting latch block (P8), for-in result type as List<T> (P1 partial)

**Uses:** While loop infrastructure from Phase 1 (loop context stack, reduction checks)

**Estimated effort:** 3-4 days

### Phase 3: List Builder Runtime + For-In Collection Semantics
**Rationale:** Before adding List/Map/Set iteration, implement the list builder to avoid O(N²) collection. This is not an optimization — it's required for correctness with realistic data sizes.

**Delivers:** List builder API (snow_list_builder_new/push/finish), for-in returning List<T> with collected body results, GC-safe builder allocation.

**Addresses:**
- STACK.md: snow_list_builder runtime functions, GC-safe builder on actor heap
- PITFALLS.md: O(N²) list collection (P4), GC roots in loops (P7)
- FEATURES.md: for-in as expression returning List<T> (differentiator)

**Uses:** MirExpr::ForIn from Phase 2, codegen loop structure

**Estimated effort:** 2-3 days

### Phase 4: For-In Over List/Map/Set + Destructuring
**Rationale:** With list builder and Range working, add collection iteration. Map/Set require minimal runtime additions (snow_set_to_list, Map already has snow_map_keys). Pattern destructuring reuses existing pattern infrastructure limited to irrefutable patterns.

**Delivers:** Full for-in support across all collection types, tuple destructuring for Map entries.

**Addresses:**
- STACK.md: IterKind::List/Map/Set, collection-specific runtime functions (snow_set_to_list)
- FEATURES.md: for {k,v} in map (table stakes), destructuring (table stakes)
- ARCHITECTURE.md: Collection iteration strategy, pattern binding in for-loops
- PITFALLS.md: Pattern conflicts (P6) by restricting to irrefutable patterns, Map/Set runtime functions (P11)

**Uses:** List builder from Phase 3, pattern matching infrastructure (restricted subset)

**Estimated effort:** 4-5 days

### Phase 5: Filter Clause (when) + Integration Testing
**Rationale:** With core loop functionality complete, add filter clause as ergonomic enhancement. Integration testing covers closures in loops, nested loops, pipe interaction, tooling (formatter, LSP).

**Delivers:** `for x in list when x > 0 do ... end` syntax, comprehensive e2e tests, formatter/LSP updates.

**Addresses:**
- FEATURES.md: for-in with filter clause (differentiator), break in for-in returns partial list
- PITFALLS.md: Closure capture in loops (P10), nested loops (P9), tooling integration
- ARCHITECTURE.md: Break/continue in nested contexts (if/case/loops)

**Uses:** All prior phases

**Estimated effort:** 3-4 days

### Phase Ordering Rationale

- **Sequential through compiler layers** — Each phase builds on the previous one. Cannot implement for-in without while's loop infrastructure. Cannot implement collection without list builder. This is dictated by architecture, not preference.
- **Simple before complex** — While before for-in, Range before List/Map/Set, basic functionality before filter clauses. Validates patterns on simpler cases.
- **Performance-critical upfront** — List builder in Phase 3 (not deferred) because O(N²) collection breaks realistic workloads. Alloca placement and reduction checks in Phase 1 because fixing later requires full codegen refactor.
- **Pitfall prevention integrated** — Each phase explicitly addresses specific pitfalls from research. Not a separate "fix bugs" phase.

### Research Flags

**Phases with standard patterns (no additional research needed):**
- **Phase 1-5:** All loop patterns are well-documented in LLVM codegen literature, Rust RFC discussions, and existing Snow patterns (if/match codegen). The research files provide comprehensive implementation guidance.

**Validation during implementation:**
- LLVM IR verification (`opt -verify`) must pass for every test case
- GC stress tests (DEFAULT_REDUCTIONS=1) to verify root scanning
- Benchmark for-in collection to confirm O(N) not O(N²)

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Based on direct codebase analysis (67,546 lines across 11 crates), Inkwell API verification, LLVM IR patterns established in existing codegen |
| Features | HIGH | Loops are the most-studied area of language design with extensive prior art (Rust, Elixir, Scala, Kotlin, Swift, Zig) |
| Architecture | HIGH | MIR desugaring pattern established by pipe operator, alloca+mem2reg pattern established by if-expression, reduction checks established by actor scheduler |
| Pitfalls | HIGH | LLVM alloca issues documented in LLVM Frontend Performance Tips and MLIR bug reports; Rust RFC 1624/1767 for break-value type unification; O(N²) collection is algorithmic fact |

**Overall confidence:** HIGH

### Gaps to Address

- **String iteration deferred** — Research confirms this requires Unicode handling (grapheme clusters, UTF-8). Not a gap, but an explicit defer-to-v2 decision.
- **Break-with-value semantics unresolved** — Rust hasn't solved this after 8+ years (RFC 1767 unmerged). Not attempting for v1.7. For-in break returns partial list; while break returns Unit.
- **Iterator protocol design** — Research confirms this is a full milestone (state machines, lazy evaluation, trait dispatch). Not in scope for v1.7. Index-based iteration sufficient.

No unresolved technical gaps. All implementation details grounded in verified patterns.

## Sources

### Primary (HIGH confidence)
- Snow codebase direct analysis: 67,546 lines across 11 crates, 1,255 tests
- snow-codegen/src/codegen/expr.rs lines 856-913 (codegen_if alloca+mem2reg pattern)
- snow-codegen/src/mir/mod.rs (MirExpr enum, current 30+ variants)
- snow-common/src/token.rs (45 existing keywords, `for`/`in` present, `while`/`break`/`continue` absent)
- snow-rt/src/collections/list.rs (snow_list_append O(N) copy at line 76)
- snow-rt/src/actor/mod.rs lines 160-191 (snow_reduction_check with GC trigger, "loop back-edges" comment at line 155)
- Inkwell GitHub Repository — API for build_alloca, build_conditional_branch, append_basic_block (verified against 0.8.0)
- LLVM Language Reference: Loop Terminology — LLVM loop detection via back-edges
- LLVM Frontend Performance Tips — "alloca in entry block" requirement for mem2reg

### Secondary (MEDIUM-HIGH confidence)
- Rust RFC 1624: Loop Break Value — Design discussion on break-with-value, type unification challenges
- Rust RFC Issue 1767: Allow for/while to return value — Unresolved after 8+ years, documents the type unification problem
- Elixir Comprehensions documentation — for/do returns collected list by default
- Scala for-comprehension documentation — for/yield desugars to map, returns collection
- Zig Loops as Expressions documentation — while-else pattern for break-with-value
- LLVM Kaleidoscope Tutorial Ch. 5 — Loop codegen with basic blocks, "codegen recursively could change current block" warning
- MLIR alloca-in-loop stack overflow (tensorflow/mlir#210) — Real-world example of loop-body alloca causing stack overflow

### Tertiary (MEDIUM confidence)
- Go Goroutine Preemption (1.14+) — Compiler-inserted preemption checks at loop back-edges for cooperative scheduling
- BEAM VM reduction counting — Per-opcode reduction counting for fair scheduling
- OCaml Imperative Programming — OCaml for/while loops always return unit, similar to proposed while semantics

---
*Research completed: 2026-02-08*
*Ready for roadmap: yes*
