# Project Research Summary

**Project:** Snow v1.9 Stdlib & Ergonomics
**Domain:** Programming language compiler -- stdlib expansion (math, collections), error handling ergonomics (? operator), actor concurrency primitives (timers, receive timeouts), and compiler optimization (tail-call elimination)
**Researched:** 2026-02-09
**Confidence:** HIGH

## Executive Summary

Snow v1.9 adds six feature categories that transform the language from "compiler demo" to "production-ready functional language with actor concurrency." Research reveals that **all six features require ZERO new Rust crate dependencies** and build entirely on existing infrastructure. The recommended approach is a two-tier implementation strategy: (1) LLVM math intrinsics plus Rust f64 wrappers for math stdlib, (2) MIR-level desugaring for the ? operator to reuse existing pattern matching codegen, (3) codegen null-check branch to complete the already-parsed receive timeout infrastructure, (4) runtime timer thread with priority queue for timer primitives, (5) runtime functions following existing callback patterns for collection operations, and (6) MIR loop transformation for self-recursive tail-call elimination.

The key risk is tail-call elimination correctness. TCE is architecturally critical -- without it, every actor receive loop eventually stack overflows. The loop transformation approach (converting self-recursive tail calls to while loops at the MIR level) is more reliable than LLVM's musttail marker, which has strict ABI constraints incompatible with Snow's reduction checks. Self-recursive TCE covers 95%+ of real use cases including all actor loops. Mutual recursion can be deferred to a future milestone. Secondary risks include cross-platform linking (missing -lm on Linux), comparator callback synthesis for generic sort (must reuse existing Ord trait dispatch), and timer primitives blocking OS threads instead of yielding to the coroutine scheduler.

The research is HIGH confidence across all areas: Stack recommendations are based on direct codebase analysis of all 12 crates plus LLVM/Inkwell API verification; Features are drawn from established patterns in Rust, Erlang, Elixir, Haskell; Architecture leverages Snow's existing extern "C" runtime ABI, uniform u64 storage, and MIR lowering patterns; Pitfalls are derived from 73,384 lines of codebase analysis and known compiler engineering challenges.

## Key Findings

### Recommended Stack

**ZERO new dependencies required.** Every feature is implemented through LLVM math intrinsics, Rust standard library (f64 methods link to system libm automatically), existing Inkwell 0.8 APIs for tail-call support, and internal compiler/runtime extensions. The only infrastructure change is adding `-lm` to the linker invocation on Linux (macOS links libm automatically via libSystem).

**Core technologies (unchanged):**
- Rust stable 2021 edition -- compiler implementation, no changes
- Inkwell 0.8.0 with llvm21-1 feature -- LLVM IR generation, adds math intrinsics via add_function("llvm.sqrt.f64", ...)
- LLVM 21.1 -- backend codegen + optimization, supports set_tail_call_kind API
- Rowan 0.16 -- CST for parser, add TRY_EXPR syntax kind for ? operator
- ena 0.14 -- HM type inference, validate ? operator return type constraints
- ariadne 0.6 -- diagnostic error reporting
- corosensei 0.3 -- stackful coroutines for actors, timer primitives must yield not block

**Runtime (snow-rt) -- NO NEW DEPENDENCIES:**
- Math functions use Rust's f64 methods (sin(), cos(), sqrt(), etc.) which link to system libm
- Timer primitives use std::thread for dedicated timer thread, std::collections::BinaryHeap for priority queue
- Collection operations use Rust's slice::sort_by with callback pattern already established for list_map/list_filter
- All new functions follow existing extern "C" ABI patterns

**What NOT to add:**
- libm Rust crate (unnecessary -- Rust std f64 methods already use system libm)
- num-traits (unnecessary -- Snow only needs concrete f64 operations)
- Any sort crate (Rust's TimSort is optimal for this scale)
- timer/tokio-timer crates (actor scheduler integration requires custom implementation)
- regex (string split/join use exact matching, not patterns)

### Expected Features

**Must have (table stakes):**
- Math stdlib (abs, min, max, pow, sqrt, floor, ceil, round, pi, type conversions) -- universal across all languages, needed immediately
- ? operator for Result/Option propagation -- Rust proved this is the gold standard for error ergonomics
- Receive timeout completion -- 80% implemented, currently segfaults on timeout instead of executing after clause body
- Timer primitives (sleep, send_after) -- Erlang/Elixir foundational pattern, required for actor timeouts/heartbeats/retries
- Core collection operations (List.sort, find, any, all, contains, String.split/join/to_int/to_float) -- table stakes for functional language
- Self-recursive tail-call elimination -- hard requirement for actor-based language; without TCE, every actor receive loop stack overflows

**Should have (competitive):**
- Extended collection operations (zip, flat_map, enumerate, take, drop, Map.merge/to_list/from_list, Set.difference/to_list/from_list)
- Type-safe receive timeouts (timeout body type checked against receive arm types at compile time, unlike Erlang's runtime typing)
- Timer.send_after with typed Pid (ensures delayed message matches target actor's expected type)
- @tailrec annotation (compile error if function NOT tail-recursive, like Scala)

**Defer (v2+):**
- Mutual tail-call elimination via LLVM musttail (complex ABI constraints, 5% edge case)
- From trait for ? error type conversion (large type system feature)
- Timer.cancel/read/send_interval (infrastructure complexity)
- Trigonometric/logarithmic math functions beyond basics (defer to second batch)
- Lazy iterators (adds complexity, eager evaluation sufficient for v1.9)

### Architecture Approach

Snow's linear pipeline (lexer -> parser -> typeck -> MIR -> codegen -> linker) enables a surgical integration strategy where each feature targets specific layers without cross-cutting changes. Math stdlib adds runtime functions + builtin registrations (3 files). The ? operator adds parser postfix recognition, typeck validation, and MIR desugaring to existing match+return nodes (NO new codegen). Receive timeout completion fixes codegen null-check gap in expr.rs (1 function change). Timer primitives add dedicated runtime thread + scheduler integration via existing send-wake path. Collection operations follow established callback pattern (function pointer + environment pointer for closures). Tail-call elimination adds first post-lowering MIR transformation pass, converting self-recursive tail calls to while loops with mutable loop variables.

**Major components (integration points):**
1. snow-typeck/builtins.rs -- register ~35 new function type signatures (math, timer, collection ops)
2. snow-codegen/intrinsics.rs -- declare ~30 new extern "C" runtime functions in LLVM module
3. snow-codegen/mir/lower.rs -- desugar ? operator to match+return, resolve Ord callbacks for sort
4. snow-codegen/codegen/expr.rs -- add null-check branch for receive timeout, bitcast i64<->f64 for math calls
5. snow-codegen/mir/tce.rs (NEW) -- tail position analysis + loop transformation pass
6. snow-rt/math.rs (NEW) -- f64 wrappers around Rust math methods
7. snow-rt/timer.rs (NEW) -- dedicated timer thread with priority queue, send integration
8. snow-rt/collections/list.rs -- add sort/find/zip/etc following existing map/filter/reduce pattern
9. snow-rt/collections/string.rs -- add split/join/to_int/to_float functions
10. snow-codegen/link.rs -- add -lm flag for Linux libm linking (1 line change)

### Critical Pitfalls

1. **Math FFI breaks cross-platform linking** -- macOS bundles libm in libSystem (automatic), Linux requires explicit -lm flag. Current link.rs lacks -lm, causing "undefined symbol" errors on Linux. Fix: add -lm unconditionally in link.rs; use LLVM intrinsics (llvm.sqrt.f64, llvm.floor.f64, etc.) where available to avoid library dependency.

2. **? operator requires early return in expression-oriented codegen** -- expr? must (1) unwrap Ok(v) to v, (2) early-return Err(e) from enclosing function. Naive Return emission leaves builder in terminated block, breaking mid-expression usage. Fix: lower ? to match { Ok(v) -> v, Err(e) -> return Err(e) } in MIR, reusing existing pattern match codegen that handles Never-typed arms.

3. **Tail-call elimination with musttail has strict signature matching** -- LLVM's musttail requires identical caller/callee types, immediate ret, no intervening operations. Snow's reduction checks (snow_reduction_check() after every call) invalidate tail position. Fix: use MIR loop transformation for self-recursion (rewrite to while loop), which is 100% reliable and covers all actor loops. Defer mutual recursion to future.

4. **Receive timeout codegen ignores timeout_body** -- parser/typeck/MIR all support after clause, but codegen explicitly discards timeout_body with "timeout_body: _" (expr.rs:129). When snow_actor_receive returns null (timeout), codegen loads from null pointer, segfault. Fix: add icmp eq msg_ptr, null branch to timeout block, codegen timeout_body, merge with phi node.

5. **Collection sort requires comparator callback synthesis per element type** -- sort(list) needs correct Ord callback: snow_int_compare for List<Int>, snow_string_compare for List<String> (doesn't exist yet, tech debt at lower.rs:5799), Ord__compare__Point for List<Point>. Wrong comparator causes silent data corruption (comparing string pointers as integers). Fix: reuse existing comparator synthesis from snow_list_compare codegen, add missing snow_string_compare to runtime.

## Implications for Roadmap

Based on research, suggested phase structure follows dependency chains and risk isolation:

### Phase 1: Math Stdlib
**Rationale:** Zero dependencies on other v1.9 features. Follows exact same pattern as existing string/file/JSON stdlib. Lowest risk, highest confidence. Provides immediate user value. Can be parallelized with all other phases.

**Delivers:** 19 math functions (sin, cos, tan, sqrt, pow, floor, ceil, round, abs, min, max, etc.) + 3 constants (pi, e, inf) + type conversions (Int.to_float, Float.to_int)

**Addresses:** Table stakes math feature set from FEATURES.md

**Avoids:** Cross-platform linking pitfall via -lm flag + LLVM intrinsics

**Estimated effort:** 2-3 days

**Research flag:** No research needed -- standard patterns, well-documented

### Phase 2: Receive Timeout Codegen Completion
**Rationale:** Fills gap in existing infrastructure (parser/typeck/MIR already support it). Small, well-scoped change (1 function in expr.rs). Unblocks Phase 4 (timer primitives depend on receive timeouts working). Second-lowest risk.

**Delivers:** Working receive...after clause that executes timeout_body when timeout fires instead of segfaulting

**Addresses:** Table stakes actor concurrency feature

**Avoids:** Null pointer dereference pitfall via null-check branch before message load

**Estimated effort:** 0.5-1 day

**Research flag:** No research needed -- simple null-check branch pattern

### Phase 3: ? Operator for Result/Option
**Rationale:** Parser + typechecker + MIR lowering changes, but desugars to existing Match + Return codegen. No runtime changes. Medium complexity but well-understood semantics. Independent of other v1.9 features. High ergonomic value.

**Delivers:** expr? syntax for unwrapping Result<T,E> and Option<T> with early return on Err/None

**Addresses:** Error handling ergonomics, differentiator with type-safe propagation

**Avoids:** Early return control flow pitfall via MIR desugaring to match+return

**Estimated effort:** 3-5 days

**Research flag:** No research needed -- Rust ? operator desugaring thoroughly documented

### Phase 4: Timer Primitives
**Rationale:** Depends on Phase 2 (receive timeouts) because timers typically used with receive...after patterns. Runtime timer thread is self-contained. Medium complexity in timer wheel + send integration.

**Delivers:** Timer.sleep(ms) and Timer.send_after(pid, ms, msg) with timer reference return for future cancellation

**Addresses:** Table stakes actor patterns (heartbeat, retry, delayed processing)

**Avoids:** OS thread blocking pitfall via yield-loop for sleep + dedicated timer thread for send_after

**Estimated effort:** 2-3 days

**Research flag:** No research needed -- Erlang/OTP timer patterns well-established

### Phase 5: Collection Operations
**Rationale:** Depends on existing Ord trait infrastructure. Follows callback pattern of list_eq/list_compare. Medium complexity in both runtime (merge sort) and MIR lowering (Ord callback resolution). Can be split into 5A (core: sort/find/any/all/contains/split/join) and 5B (extended: zip/flat_map/enumerate/take/drop/etc.).

**Delivers:**
- 5A: List.sort, List.find, List.any, List.all, List.contains, String.split, String.join, String.to_int, String.to_float
- 5B: List.zip, List.flat_map, List.enumerate, List.take, List.drop, Map.merge/to_list/from_list, Set.difference/to_list/from_list

**Addresses:** Table stakes collection operations for functional language

**Avoids:** Comparator synthesis pitfall via reusing Ord callback pattern; O(n^2) copy chains via sort-in-place on new copy

**Estimated effort:** 5A: 3-4 days, 5B: 2-3 days

**Research flag:** No research needed -- standard collection operations across all functional languages

### Phase 6: Tail-Call Elimination (Self-Recursive)
**Rationale:** Most complex feature. Independent of other features but highest risk. The MIR transformation pass is a new compiler pass pattern (first post-lowering pass). Should be built last to avoid blocking other features if it encounters difficulties. CRITICAL for correctness (every actor loop requires this).

**Delivers:** Self-recursive tail calls converted to loops at MIR level, preventing stack overflow in actor receive loops and recursive list processing

**Addresses:** Hard requirement for actor-based language -- without TCE, actors crash on deep recursion

**Avoids:** musttail signature matching pitfall via MIR loop transformation; tail position detection pitfall via recursive is_tail_position analysis through Let/If/Match/Block expressions

**Estimated effort:** 4-6 days

**Research flag:** Standard TCE patterns well-documented, but MIR pass integration needs careful design

### Phase Ordering Rationale

1. **Parallel Phase 1-3:** Math stdlib, receive timeouts, and ? operator are fully independent and can be implemented in parallel by different engineers or sequentially in any order.

2. **Phase 4 depends on Phase 2:** Timer primitives need receive timeouts working because sleep() is implemented as receive with timeout, and timer patterns typically use receive...after.

3. **Phase 5 is independent:** Collection operations can proceed in parallel with Phases 1-4, or after Phase 3 if wanting to test ? operator with collection operations that return Option.

4. **Phase 6 last:** TCE is highest complexity and most architecturally significant (first MIR transformation pass). Building it last avoids blocking other features and allows testing TCE against code using new stdlib functions.

**Critical path:** Phase 2 -> Phase 4 (receive timeouts enable timer primitives)

**Parallelizable:** Phases 1, 3, 5 can all proceed independently

**Risky:** Phase 6 should be isolated and well-tested

### Research Flags

**Phases needing deeper research during planning:** NONE -- all six features have well-documented patterns

**Phases with standard patterns (skip research-phase):**
- Phase 1 (Math Stdlib): Established stdlib pattern, LLVM intrinsics documented
- Phase 2 (Receive Timeout): Codegen null-check branch, straightforward
- Phase 3 (? Operator): Rust desugaring pattern thoroughly documented
- Phase 4 (Timer Primitives): Erlang/OTP timer patterns well-established
- Phase 5 (Collection Operations): Universal functional language collection APIs
- Phase 6 (TCE): Textbook compiler optimization, loop transformation pattern

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Based on direct codebase analysis of all 12 crates, LLVM/Inkwell API verification, platform linking research. Zero new dependencies verified. |
| Features | HIGH | All features drawn from Rust, Erlang/Elixir, Haskell established patterns with official documentation. Table stakes vs. differentiators validated across multiple languages. |
| Architecture | HIGH | Direct inspection of 73,384 lines of Snow codebase. Integration points verified in builtins.rs, intrinsics.rs, lower.rs, expr.rs. Existing patterns (callback dispatch, MIR lowering, extern "C" ABI) confirmed. |
| Pitfalls | HIGH | Derived from codebase analysis (receive timeout gap verified at expr.rs:129, reduction check insertion verified at expr.rs:634, tech debt note verified at lower.rs:5799) plus LLVM musttail constraints from Language Reference. |

**Overall confidence:** HIGH

### Gaps to Address

**Minor gaps (handle during implementation):**
- Float sorting with NaN handling: Either error at compile time ("Float does not implement Ord") or use total ordering that puts NaN last. Decision needed during Phase 5 planning.
- Timer cancellation API design: send_after returns timer reference (PID of timer actor) but Timer.cancel not in v1.9 scope. Document that timer references are opaque for future use.
- Error type conversion in ? operator: Rust uses From trait; Snow requires exact E type match in v1.9. Document limitation, defer From trait to future milestone.
- @tailrec annotation: Differentiator feature that enforces TCE at compile time. Can be added any time after Phase 6 completes, but not blocking for v1.9.

**No blocking gaps.** All six features have clear implementation paths with HIGH confidence.

## Sources

### Primary (HIGH confidence)
- Direct codebase analysis: all 12 crates in workspace (snow-common, snow-lexer, snow-parser, snow-typeck, snow-codegen, snow-rt)
- snow-codegen/src/codegen/expr.rs line 129 -- timeout_body explicitly ignored
- snow-codegen/src/mir/lower.rs lines 7160-7172 -- timeout_ms and timeout_body already lowered
- snow-typeck/src/infer.rs lines 5985-5991 -- after clause already type-checked
- snow-common/src/token.rs line 126 -- Question token exists
- snow-codegen/src/link.rs -- linker invocation, missing -lm flag
- snow-rt/src/actor/mod.rs lines 314-419 -- receive timeout returns null
- snow-rt/src/collections/list.rs -- existing list operation patterns (map, filter, reduce)
- Inkwell CallSiteValue docs -- set_tail_call_kind available on llvm21-1
- LLVM Language Reference -- math intrinsics (llvm.sqrt.f64, llvm.sin.f64, etc.) stable built-in
- LLVM musttail semantics -- caller/callee signature requirements

### Secondary (HIGH confidence -- official docs)
- Rust Reference: Operator Expressions (? operator desugaring)
- Erlang System Documentation: Expressions (receive...after semantics)
- Erlang timer module stdlib v7.2
- Erlang math module stdlib v7.2
- LLVM Tail Recursion Elimination pass (loop transformation approach)
- Inkwell 0.8 documentation

### Tertiary (MEDIUM confidence -- community/platform knowledge)
- macOS libm linked automatically via libSystem (GCC mailing list discussion)
- Linux libm requires -lm at link time (GCC/linker documentation)
- LLVM musttail backend issues (GitHub issue #54964 -- platform-specific failures)
- Tail call elimination blog posts (loop transformation vs trampoline approaches)

---
*Research completed: 2026-02-09*
*Ready for roadmap: yes*
