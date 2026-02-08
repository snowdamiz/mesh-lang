---
phase: 23-pattern-matching-codegen
verified: 2026-02-08T18:23:30Z
status: human_needed
score: 8/8 must-haves verified (structural)
human_verification:
  - test: "Compile and run: case Some(42) do Some(x) -> x | None -> 0 end"
    expected: "Returns 42 (extracts inner value at runtime)"
    why_human: "Tests verify MIR structure but don't execute LLVM-compiled code"
  - test: "Compile and run: case compare(3, 5) do Less -> 1 | Equal -> 2 | Greater -> 3 end"
    expected: "Returns 1 (Less variant matches)"
    why_human: "Tests verify dispatch but don't execute to confirm runtime behavior"
  - test: "Compile and run: let ord = compare(5, 3); case ord do Greater -> \"yes\" | _ -> \"no\" end"
    expected: "Returns \"yes\" (Ordering as first-class value works)"
    why_human: "Tests don't verify Ordering can be bound to variables and pattern matched later"
  - test: "Compile and run: case Some(Some(42)) do Some(Some(x)) -> x | _ -> 0 end"
    expected: "Returns 42 (nested pattern extracts doubly-wrapped value)"
    why_human: "Tests don't verify nested constructor patterns at runtime"
---

# Phase 23: Pattern Matching Codegen Verification Report

**Phase Goal:** Sum type pattern matching fully works in LLVM codegen -- users can destructure non-nullary variant fields and use the Ordering type directly in Snow programs

**Verified:** 2026-02-08T18:23:30Z

**Status:** human_needed

**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | Pattern matching dispatches to correct arm regardless of ordering | ✓ VERIFIED | Tag lookup uses sum_type_defs (compile.rs:367-387); test_tag_from_sum_type_def passes |
| 2 | Non-nullary variant fields extracted with correct types | ✓ VERIFIED | Field type resolution from sum_type_defs (compile.rs:555-571); test_field_type_from_sum_type_def passes |
| 3 | Nested constructor patterns extract inner values | ✓ VERIFIED | Field expansion recursive (compile.rs:494-496); test passes structurally |
| 4 | Ordering registered as built-in sum type | ✓ VERIFIED | Registration in infer.rs:723-754; ordering_sum_type_registered_in_mir test passes |
| 5 | compare() method in Ord trait | ✓ VERIFIED | Trait registration builtins.rs:677-682; Ord impls 701-707 |
| 6 | compare() dispatches to Ord__compare__Type | ✓ VERIFIED | MIR lowerer generates functions; compare_call_dispatches_to_mangled test passes |
| 7 | Ordering variants (Less/Equal/Greater) available | ✓ VERIFIED | Variant constructors registered (infer.rs:754); MIR tests verify |
| 8 | Pattern matching on Ordering compiles | ✓ VERIFIED | pattern_match_ordering_variants test passes |

**Score:** 8/8 truths verified structurally

**Note:** All truths verified at the MIR/structural level. Runtime execution verification required (see Human Verification section).

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/snow-codegen/src/pattern/compile.rs` | Pattern compiler with sum_type_defs parameter | ✓ VERIFIED | Line 72: compile_match accepts sum_type_defs; 143 lines changed |
| `crates/snow-codegen/src/pattern/compile.rs` | Tag lookup from sum type definition | ✓ VERIFIED | Lines 367-387: collect_head_constructors uses sum_type_defs.get() |
| `crates/snow-codegen/src/pattern/compile.rs` | Field type resolution from sum type definition | ✓ VERIFIED | Lines 555-571: specialize_for_constructor resolves field_types |
| `crates/snow-codegen/src/codegen/expr.rs` | compile_match call passes sum_type_defs | ✓ VERIFIED | Line 965: &self.sum_type_defs passed |
| `crates/snow-typeck/src/infer.rs` | Ordering sum type registration | ✓ VERIFIED | Lines 723-754: Less/Equal/Greater registered with tags 0/1/2 |
| `crates/snow-typeck/src/builtins.rs` | compare() in Ord trait | ✓ VERIFIED | Lines 677-682: compare method with Ordering return type |
| `crates/snow-typeck/src/builtins.rs` | compare() polymorphic built-in | ✓ VERIFIED | Lines 79-93: compare(T,T)->Ordering registered |
| `crates/snow-codegen/src/mir/lower.rs` | Ord__compare__Type generation | ✓ VERIFIED | Lines 2068-2253: generate_compare_struct/sum/primitive methods |

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|----|--------|---------|
| codegen/expr.rs | pattern/compile.rs | compile_match call | ✓ WIRED | Line 965 passes sum_type_defs |
| pattern/compile.rs | MirSumTypeDef | Tag lookup | ✓ WIRED | Lines 370-379 use sum_type_defs.get() |
| pattern/compile.rs | MirSumTypeDef | Field type resolution | ✓ WIRED | Lines 558-567 use sum_type_defs.get() |
| builtins.rs | infer.rs | Ord trait references Ordering | ✓ WIRED | TyCon::new("Ordering") at line 680 |
| mir/lower.rs | MirExpr::ConstructVariant | compare constructs Ordering | ✓ WIRED | Lines 2092-2110 construct Less/Equal/Greater |
| mir/lower.rs call dispatch | Ord__compare__Type | compare(a,b) rewrite | ✓ WIRED | Test compare_call_dispatches_to_mangled verifies |

### Requirements Coverage

| Requirement | Status | Evidence |
|-------------|--------|----------|
| PATM-01: Non-nullary field extraction | ✓ SATISFIED | Tag assignment fixed (lines 367-387); field types resolved (lines 555-571); 2 new tests pass |
| PATM-02: Ordering type usable | ✓ SATISFIED | Ordering registered (infer.rs:723-754); compare() in Ord trait (builtins.rs:677-682); 5 new tests pass |

### Anti-Patterns Found

None. No TODO/FIXME blockers found in modified code.

### Human Verification Required

**Critical gap:** Tests verify STRUCTURAL correctness (MIR contains correct patterns, functions exist, types resolved) but do NOT verify RUNTIME execution. The phase goal explicitly states "fully works in LLVM codegen" and all 4 success criteria describe RUNTIME behavior ("binds x to the inner value at runtime", "compiles and runs correctly").

#### 1. Option Pattern Match Field Extraction

**Test:** Create a Snow program:
```snow
fn main() -> Int do
  let opt = Some(42)
  case opt do
    Some(x) -> x
    None -> 0
  end
end
```
Compile with `snowc build` and execute the binary.

**Expected:** Program returns exit code 42 (or prints 42 if modified to use println)

**Why human:** The test `pattern_match_some_extracts_field` verifies that MirExpr::Match contains Constructor patterns with the right structure, but doesn't compile to LLVM or execute. Runtime binding of x to 42 is not tested.

#### 2. Ordering Pattern Match with compare()

**Test:** Create a Snow program:
```snow
fn main() -> Int do
  case compare(3, 5) do
    Less -> 1
    Equal -> 2
    Greater -> 3
  end
end
```
Compile and execute.

**Expected:** Program returns 1 (Less arm matches because 3 < 5)

**Why human:** The test `pattern_match_ordering_variants` verifies compare() dispatches to Ord__compare__Int in MIR, but doesn't verify the function returns the correct variant or that pattern matching dispatches to the right arm at runtime.

#### 3. Ordering as First-Class Value

**Test:** Create a Snow program:
```snow
fn main() -> String do
  let ord = compare(5, 3)
  case ord do
    Greater -> "correct"
    Equal -> "wrong"
    Less -> "wrong"
  end
end
```
Compile and execute.

**Expected:** Program prints "correct" (Greater variant binds to ord, pattern match works on variable)

**Why human:** Tests don't verify Ordering can be stored in variables and used later. This tests that Ordering is truly a first-class sum type, not just pattern-matchable inline.

#### 4. Nested Constructor Patterns

**Test:** Create a Snow program:
```snow
fn main() -> Int do
  let nested = Some(Some(42))
  case nested do
    Some(Some(x)) -> x
    Some(None) -> 0
    None -> 0
  end
end
```
Compile and execute.

**Expected:** Program returns 42 (doubly-nested value extracted correctly)

**Why human:** While field type resolution is tested structurally, runtime extraction through multiple nesting levels (variant field is itself a variant containing a field) is not verified. The recursive field expansion (compile.rs:494-496) is structurally correct but not runtime-tested.

### Gaps Summary

**No structural gaps.** All code changes are implemented correctly:
- Pattern compiler receives sum_type_defs and uses it for tag assignment and field type resolution
- Ordering type registered with 3 variants (Less=0, Equal=1, Greater=2)
- compare() added to Ord trait and dispatched correctly
- Ord__compare__Type functions generated for primitives, structs, and sum types
- All 1,196 tests pass including 9 new tests (2 pattern compiler + 7 MIR lowering)

**Runtime verification gap:** The test suite validates that:
- MIR structures are correct (Constructor patterns exist, types resolved)
- Functions exist (Ord__compare__Int generated)
- Dispatch happens (compare() rewrites to Ord__compare__Int)

But does NOT validate that:
- Extracted values are actually bound at runtime (x = 42)
- compare() returns the correct Ordering variant at runtime
- Pattern matching on Ordering dispatches to the correct arm
- Nested extraction works through LLVM codegen and execution

The e2e test infrastructure exists (`compile_and_run` in snowc/tests/e2e_stdlib.rs) but no tests exercise the 4 success criteria. This is a test coverage gap, not an implementation gap.

**Recommendation:** Add 4 e2e tests to crates/snowc/tests/e2e.rs that compile and execute Snow programs covering each success criterion. If all 4 tests pass, phase goal is achieved. If any fail, LLVM codegen has a gap that structural tests missed.

---

_Verified: 2026-02-08T18:23:30Z_
_Verifier: Claude (gsd-verifier)_
