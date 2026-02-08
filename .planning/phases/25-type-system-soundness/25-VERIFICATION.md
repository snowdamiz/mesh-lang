---
phase: 25-type-system-soundness
verified: 2026-02-08T20:32:23Z
status: passed
score: 5/5 must-haves verified
---

# Phase 25: Type System Soundness Verification Report

**Phase Goal:** Higher-order constrained functions preserve their trait constraints when captured as values -- the type system prevents unsound calls at compile time

**Verified:** 2026-02-08T20:32:23Z
**Status:** PASSED
**Re-verification:** No -- initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | `let f = show; f(non_display_value)` produces a compile-time TraitNotSatisfied error | ✓ VERIFIED | Test `e2e_where_clause_alias_propagation` passes, explicitly tests this scenario |
| 2 | `let f = show; let g = f; g(non_display_value)` produces a compile-time TraitNotSatisfied error (chain aliases) | ✓ VERIFIED | Test `e2e_where_clause_chain_alias` passes, tests chain propagation |
| 3 | Constraint preservation works for user-defined traits, not just stdlib Display | ✓ VERIFIED | Test `e2e_where_clause_alias_user_trait` passes, uses `Greetable` trait |
| 4 | Existing direct-call constraint checking (`show(42)`) still works (no regressions) | ✓ VERIFIED | Test `e2e_where_clause_enforcement` still passes (pre-existing test) |
| 5 | `let f = show; f(display_value)` still compiles successfully (no false positives) | ✓ VERIFIED | Test `e2e_where_clause_alias_user_trait` Part B verifies conforming types succeed |

**Score:** 5/5 truths verified

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/snow-typeck/src/infer.rs` | Constraint propagation through let bindings | ✓ VERIFIED | Lines 2168-2173: NameRef detection + fn_constraints.insert |
| `crates/snow-codegen/src/mir/lower.rs` | End-to-end tests for constraint alias propagation | ✓ VERIFIED | Lines 7585-7711: 3 e2e tests added |

#### Artifact Detail: infer.rs

**Level 1: Existence** - ✓ EXISTS (5424 lines)

**Level 2: Substantive** - ✓ SUBSTANTIVE
- Length: 5424 lines (well above minimum)
- No stub patterns: Zero TODO/FIXME related to constraints
- Has exports: Module exports infer functions
- Real implementation:
  - Line 2128: `infer_let_binding` signature changed to `&mut FxHashMap<String, FnConstraints>`
  - Lines 2168-2173: Constraint propagation via NameRef detection and cloning
  - Line 3075: `infer_block` creates `local_fn_constraints` clone
  - Line 3149: Passes `&mut local_fn_constraints` to `infer_let_binding`

**Level 3: Wired** - ✓ WIRED
- Imported: Used throughout snow-typeck crate
- Called by: `infer_block` (line 3142), which is called by all expression contexts
- Integration verified: Tests pass, proving end-to-end wiring

#### Artifact Detail: lower.rs tests

**Level 1: Existence** - ✓ EXISTS (7700+ lines, test section lines 7585-7711)

**Level 2: Substantive** - ✓ SUBSTANTIVE
- 3 new test functions: 126 lines of test code
- No stub patterns: Zero TODO/FIXME in test code
- Real implementations:
  - `e2e_where_clause_alias_propagation`: Tests direct alias (42 lines)
  - `e2e_where_clause_chain_alias`: Tests chain alias (43 lines)
  - `e2e_where_clause_alias_user_trait`: Tests user trait + no false positives (63 lines)

**Level 3: Wired** - ✓ WIRED
- All 4 tests run successfully via `cargo test -p snow-codegen -- e2e_where_clause`
- Tests exercise full pipeline: parse -> typeck -> error checking
- Integration verified: Tests pass with expected TraitNotSatisfied errors

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|----|--------|---------|
| `infer_let_binding` | fn_constraints map | NameRef detection + constraint cloning | ✓ WIRED | Lines 2168-2173: Detects `Expr::NameRef`, gets source constraints, inserts into map |
| `infer_block` | `infer_let_binding` | Passing real fn_constraints instead of empty default | ✓ WIRED | Line 3075: Creates `local_fn_constraints = fn_constraints.clone()`, Line 3149: passes `&mut local_fn_constraints` |
| `infer_call` | fn_constraints map | Existing name-based lookup now finds propagated entries | ✓ WIRED | Line 2677: `fn_constraints.get(&fn_name)` - existing code now sees propagated constraints |

#### Link Detail: Constraint Propagation Flow

1. **Function definition** (line 1137): `fn_constraints.insert(fn_name, FnConstraints { where_constraints, ... })`
   - Stores constraints for `show<T> where T: Display`

2. **Let binding alias** (lines 2168-2173): 
   - Detects `let f = show` (NameRef to constrained function)
   - Clones constraints: `fn_constraints.insert("f", constraints_from_show)`

3. **Call site** (line 2677):
   - On `f(42)`, checks `fn_constraints.get("f")`
   - Finds propagated constraints, checks trait requirements
   - Returns `TraitNotSatisfied` error for Int (no Display impl)

### Requirements Coverage

No REQUIREMENTS.md entries mapped to Phase 25 (TSND-01 tracked in ROADMAP only).

### Anti-Patterns Found

None. Zero blocker or warning patterns detected:
- No TODO/FIXME comments in constraint code
- No placeholder implementations
- No empty return statements
- No console.log-only stubs

### Human Verification Required

None needed. All verification performed programmatically:
- Compile-time errors are tested via typechecker error checking
- Test suite verifies behavior mechanically
- No runtime behavior, visual elements, or external services involved

---

## Verification Details

### Test Execution Results

```
$ cargo test -p snow-codegen -- e2e_where_clause

running 4 tests
test mir::lower::tests::e2e_where_clause_chain_alias ... ok
test mir::lower::tests::e2e_where_clause_alias_propagation ... ok
test mir::lower::tests::e2e_where_clause_alias_user_trait ... ok
test mir::lower::tests::e2e_where_clause_enforcement ... ok

test result: ok. 4 passed; 0 failed
```

### Full Test Suite Results

```
$ cargo test --workspace

Total: 1206 passed, 0 failed, 1 ignored
```

All tests pass with no regressions. 3 new tests added (1203 -> 1206).

### Code Evidence

**Constraint propagation (infer.rs:2168-2173):**
```rust
// Propagate where-clause constraints if RHS is a NameRef
// to a constrained function (fixes TSND-01 soundness bug).
if let Expr::NameRef(ref name_ref) = init_expr {
    if let Some(source_name) = name_ref.text() {
        if let Some(source_constraints) = fn_constraints.get(&source_name).cloned() {
            fn_constraints.insert(name_text.clone(), source_constraints);
        }
    }
}
```

**Clone-locally strategy (infer.rs:3075):**
```rust
let mut local_fn_constraints = fn_constraints.clone();
```

**Test coverage:**
1. Direct alias: `let f = show; f(42)` → TraitNotSatisfied ✓
2. Chain alias: `let f = show; let g = f; g(42)` → TraitNotSatisfied ✓
3. User trait: `let f = say_hello; f(42)` (Greetable) → TraitNotSatisfied ✓
4. No false positives: `let f = say_hello; f(person)` → compiles ✓
5. Direct call: `show(42)` → TraitNotSatisfied (regression check) ✓

---

## Success Criteria Checklist

- [x] `let f = show; f(42)` produces compile-time TraitNotSatisfied error
- [x] `let f = show; let g = f; g(42)` produces compile-time error (chain aliases)
- [x] User-defined trait constraints preserved through aliases
- [x] No false positives (conforming types still compile)
- [x] All existing tests pass (no regressions)
- [x] fn_constraints.insert present in infer_let_binding
- [x] e2e_where_clause_alias tests present and passing
- [x] Key links verified (NameRef detection, constraint cloning, call-site lookup)

---

_Verified: 2026-02-08T20:32:23Z_
_Verifier: Claude (gsd-verifier)_
