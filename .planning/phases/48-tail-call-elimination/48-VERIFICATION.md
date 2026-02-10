---
phase: 48-tail-call-elimination
verified: 2026-02-10T17:47:12Z
status: passed
score: 9/9 must-haves verified
---

# Phase 48: Tail-Call Elimination Verification Report

**Phase Goal:** Self-recursive functions execute in constant stack space, making actor receive loops safe from stack overflow

**Verified:** 2026-02-10T17:47:12Z

**Status:** passed

**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | A self-recursive function in tail position runs for 1,000,000+ iterations without stack overflow | ✓ VERIFIED | `tce_countdown.snow` test passes with countdown(1000000) completing successfully in 2.37s |
| 2 | Tail position is correctly detected through if/else branches, case arms, receive arms, blocks, and let-chains | ✓ VERIFIED | `rewrite_tail_calls` function handles 7 tail contexts (Block, Let, If, Match, ActorReceive, Return, Call); `tce_case_arms.snow` test proves case arm detection |
| 3 | Actor receive loops using self-recursive tail calls run indefinitely without growing the stack | ✓ VERIFIED | `tce_actor_loop.snow` test runs count_loop(0, 1000000) inside actor context, completes without stack overflow |

**Score:** 3/3 truths verified

### Plan 01 Must-Haves (MIR Infrastructure)

#### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | MirExpr::TailCall variant exists and can represent a self-recursive tail call with args and ty | ✓ VERIFIED | Variant defined at `mir/mod.rs:311`, contains `args: Vec<MirExpr>` and `ty: MirType` |
| 2 | MirFunction has a has_tail_calls bool flag that is true when the body contains TailCall nodes | ✓ VERIFIED | Field defined at `mir/mod.rs:57`, set by rewrite pass in 6 lowering paths |
| 3 | rewrite_tail_calls correctly rewrites self-recursive Call nodes in tail position to TailCall nodes | ✓ VERIFIED | Function at `lower.rs:8075`, converts `MirExpr::Call` to `MirExpr::TailCall` when func name matches current_fn_name |
| 4 | Tail position is correctly propagated through Block (last), Let (body), If (both branches), Match (all arms), ActorReceive (all arms + timeout), and Return (inner) | ✓ VERIFIED | All 7 contexts handled in `rewrite_tail_calls` match arms (lines 8089-8136) |
| 5 | Non-tail positions (call args, conditions, scrutinees, let values, earlier block exprs) are NOT rewritten | ✓ VERIFIED | Default `_ => false` arm at line 8140 prevents recursion into non-tail contexts |
| 6 | The rewrite pass runs after lowering every function body (lower_fn_def, lower_impl_method, lower_actor_def) | ✓ VERIFIED | Integrated into 6 paths: fn_def (871), impl_method (1002), default_method (1118), multi_clause_fn single (1225), multi_clause_fn multi (1259), actor_def (6551) |

#### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/snow-codegen/src/mir/mod.rs` | MirExpr::TailCall variant and has_tail_calls field on MirFunction | ✓ VERIFIED | TailCall variant at line 311 with args+ty, has_tail_calls field at line 57, ty() returns Never at line 426 |
| `crates/snow-codegen/src/mir/lower.rs` | rewrite_tail_calls function and integration into all function lowering paths | ✓ VERIFIED | Function at line 8075, 6 integration call sites verified, all 40 MirFunction constructors include has_tail_calls field |

#### Key Link Verification

| From | To | Via | Status | Details |
|------|----|----|--------|---------|
| `lower.rs` | `mod.rs` | rewrite_tail_calls creates MirExpr::TailCall nodes and sets MirFunction.has_tail_calls | ✓ WIRED | Pattern found: `*expr = MirExpr::TailCall` at line 8083, `has_tail_calls` returned and assigned in all 6 lowering paths |

### Plan 02 Must-Haves (Codegen)

#### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | A self-recursive function counting down from 1,000,000 completes without stack overflow | ✓ VERIFIED | `tce_countdown` e2e test passes (2.37s), produces "done" output |
| 2 | A self-recursive function that swaps parameters produces correct results (args evaluated before storing) | ✓ VERIFIED | `tce_param_swap` e2e test passes (1.95s), produces "2\n1" (correct after 100,001 swaps) |
| 3 | Self-recursive tail calls through case/match arms are correctly eliminated | ✓ VERIFIED | `tce_case_arms` e2e test passes (1.87s), produces "30" (process chain 2->1->0) |
| 4 | Actor receive loops using self-recursive tail calls run without growing the stack | ✓ VERIFIED | `tce_actor_loop` e2e test passes (2.03s), 1M iterations inside actor completes successfully |
| 5 | Non-tail-recursive functions and normal calls are completely unaffected by TCE | ✓ VERIFIED | All 118 pre-existing e2e tests pass unchanged (total 122 pass) |
| 6 | Reduction checks are emitted before loop-back branches for proper actor scheduling | ✓ VERIFIED | `emit_reduction_check()` called at line 194 in TailCall codegen, before branch to loop header |

#### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/snow-codegen/src/codegen/mod.rs` | Loop wrapping in compile_function when has_tail_calls is true | ✓ VERIFIED | Lines 451-457: creates tce_loop block, sets tce_loop_header/tce_param_names, cleared at line 493 |
| `crates/snow-codegen/src/codegen/expr.rs` | TailCall codegen: evaluate args, store to param allocas, branch to loop header | ✓ VERIFIED | Lines 171-204: two-phase arg eval (178-181), store to allocas (184-190), reduction check (194), branch (197-199) |
| `tests/e2e/tce_countdown.snow` | E2e test for 1M iteration countdown | ✓ VERIFIED | File exists, 16 lines, countdown(1000000) test |
| `tests/e2e/tce_param_swap.snow` | E2e test for parameter swap correctness | ✓ VERIFIED | File exists, 18 lines, swap_count(1,2,100001) test |
| `tests/e2e/tce_case_arms.snow` | E2e test for tail calls in case arms | ✓ VERIFIED | File exists, 17 lines, process(2,0) chain test |
| `tests/e2e/tce_actor_loop.snow` | E2e test for actor receive loop with TCE | ✓ VERIFIED | File exists, 23 lines, count_loop inside actor test |
| `crates/snowc/tests/e2e.rs` | Rust test functions that run the Snow e2e test programs | ✓ VERIFIED | 4 test functions registered at lines 2489, 2498, 2507, 2516 |

#### Key Link Verification

| From | To | Via | Status | Details |
|------|----|----|--------|---------|
| `codegen/mod.rs` | `mir/mod.rs` | compile_function reads func.has_tail_calls to decide loop wrapping | ✓ WIRED | `if func.has_tail_calls` at line 451, creates tce_loop block and sets fields |
| `codegen/expr.rs` | `codegen/mod.rs` | TailCall codegen reads tce_loop_header and tce_param_names from CodeGen | ✓ WIRED | `self.tce_loop_header` at line 172, `self.tce_param_names` at line 184 |
| `codegen/expr.rs` | `codegen/mod.rs` | TailCall stores new values into self.locals param allocas and branches to tce_loop_header | ✓ WIRED | `build_store(alloca, new_vals[i])` at line 187, `build_unconditional_branch(tce_loop_bb)` at line 198 |

### Anti-Patterns Found

No critical anti-patterns detected. Only one unrelated TODO in `lower.rs:5947` for string comparison callback (pre-existing, not part of this phase).

### Test Results

**Unit Tests (snow-codegen):**
- 175 passed, 0 failed

**E2E Tests (full suite):**
- 122 passed (118 pre-existing + 4 TCE tests), 0 failed
- TCE-specific tests:
  - `tce_countdown`: 2.37s (1M iterations)
  - `tce_param_swap`: 1.95s (100K+ swaps)
  - `tce_case_arms`: 1.87s (case arm tail calls)
  - `tce_actor_loop`: 2.03s (1M iterations in actor)

### Commits Verified

All 4 commits from summaries verified in git log:
1. `8c7cec5` - feat(48-01): add MirExpr::TailCall variant and MirFunction.has_tail_calls field
2. `be6854b` - feat(48-01): implement rewrite_tail_calls pass and integrate into function lowering
3. `2c9082b` - feat(48-02): implement TCE loop wrapping and TailCall codegen
4. `3656cef` - feat(48-02): add TCE e2e tests and fix alloca hoisting in TCE loops

### Summary

Phase 48 goal fully achieved. All observable truths verified through code inspection and passing tests. The implementation is complete, correct, and well-integrated:

**MIR Infrastructure (Plan 01):**
- TailCall variant correctly defined with Never type semantics
- Rewrite pass correctly identifies tail position through 7 expression contexts
- Integration into all 6 function lowering paths ensures complete coverage
- 40 MirFunction constructors updated with has_tail_calls field

**Codegen (Plan 02):**
- Loop wrapping creates tce_loop block when has_tail_calls is true
- TailCall codegen uses correct two-phase argument evaluation
- Reduction checks prevent actor starvation in tight tail loops
- Entry-block alloca hoisting prevents stack growth in TCE loops
- All 4 e2e tests pass, proving 1M+ iteration capability

**Key Strengths:**
1. Correct tail position detection (all 7 contexts covered)
2. Two-phase arg evaluation prevents parameter corruption
3. Reduction checks maintain actor scheduling fairness
4. Entry-block alloca hoisting crucial for deep recursion
5. Zero regressions (all 118 pre-existing tests pass)
6. Comprehensive e2e coverage (countdown, swap, case arms, actor loop)

---

_Verified: 2026-02-10T17:47:12Z_
_Verifier: Claude (gsd-verifier)_
