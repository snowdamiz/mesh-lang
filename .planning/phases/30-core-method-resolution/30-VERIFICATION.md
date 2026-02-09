---
phase: 30-core-method-resolution
verified: 2026-02-08T00:00:00Z
status: passed
score: 9/9 must-haves verified
re_verification: false
---

# Phase 30: Core Method Resolution Verification Report

**Phase Goal:** Users can call trait impl methods on struct values using dot syntax, with the receiver automatically passed as the first argument
**Verified:** 2026-02-08
**Status:** passed
**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | Method resolution searches all trait impl blocks for the receiver's concrete type | ✓ VERIFIED | `trait_registry.resolve_trait_method()` called in infer.rs:4077, 4113; `find_method_traits()` checks multiple traits |
| 2 | infer_field_access returns the method's full function type when a trait method matches | ✓ VERIFIED | `build_method_fn_type()` constructs `Ty::Fun([self_type, ...params], ret_type)` at infer.rs:4078, 4114 |
| 3 | Calling a nonexistent method produces a NoSuchMethod error, not NoSuchField | ✓ VERIFIED | `TypeError::NoSuchMethod` emitted at infer.rs:4082, 4117; NoSuchField only for non-method-call context (4091) |
| 4 | Ambiguous methods (multiple traits) produce AmbiguousMethod error | ✓ VERIFIED | `matching_traits.len() > 1` check at infer.rs:4104 emits `TypeError::AmbiguousMethod` at 4105-4111 |
| 5 | Struct field access, module-qualified access, service methods, and variant construction all continue to work unchanged | ✓ VERIFIED | Guard chain in lower.rs:3448-3463 preserves existing paths; regression tests pass (e2e_method_dot_syntax_field_access_preserved, e2e_method_dot_syntax_module_qualified_preserved) |
| 6 | User can write `point.to_string()` and it compiles and runs, producing the same result as `to_string(point)` | ✓ VERIFIED | e2e_method_dot_syntax_basic test passes (snowc/tests/e2e.rs:794); e2e_method_dot_syntax_equivalence proves identical behavior |
| 7 | The receiver is automatically passed as the first argument to the resolved impl method | ✓ VERIFIED | MIR lowering prepends receiver: `args = vec![receiver]` at lower.rs:3472 |
| 8 | `point.to_string()` and `to_string(point)` produce identical MIR (same mangled function name, same args) | ✓ VERIFIED | Shared `resolve_trait_callee()` helper used by both paths (lower.rs:3377, 3487, 3713) |
| 9 | Struct field access (`point.x`), module-qualified calls (`String.length(s)`), and variant construction (`Shape.Circle`) are unaffected | ✓ VERIFIED | Regression tests pass; guard chain prevents interception of STDLIB_MODULES, service_modules, sum types, struct types |

**Score:** 9/9 truths verified

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/snow-typeck/src/error.rs` | NoSuchMethod error variant | ✓ VERIFIED | `NoSuchMethod { ty, method_name, span }` at line 121; Display impl at 343 |
| `crates/snow-typeck/src/diagnostics.rs` | NoSuchMethod diagnostic rendering with E0030 code | ✓ VERIFIED | E0030 code mapping at line 101; span extraction at 467; full diagnostic rendering at 879-901 with "no method \`x\` on type \`Y\`" message and help text |
| `crates/snow-typeck/src/infer.rs` | Method resolution fallback in infer_field_access after struct field lookup fails | ✓ VERIFIED | Retry-based detection in infer_call; `is_method_call` parameter in infer_field_access; method resolution at lines 4077-4089, 4113-4123 |
| `crates/snow-typeck/src/traits.rs` | find_method_sig accessor | ✓ VERIFIED | Public method `find_method_sig()` added; used by `build_method_fn_type()` |
| `crates/snow-codegen/src/mir/lower.rs` | Method call interception in lower_call_expr and shared trait dispatch helper | ✓ VERIFIED | Method call interception at 3436-3540; `resolve_trait_callee()` helper at 3377-3419; guard chain at 3448-3463 |

All artifacts exist, are substantive (not stubs), and are wired into the codebase.

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|----|--------|---------|
| crates/snow-typeck/src/infer.rs | crates/snow-typeck/src/traits.rs | trait_registry.resolve_trait_method in infer_field_access | ✓ WIRED | Lines 4077, 4113 call `trait_registry.resolve_trait_method()` |
| crates/snow-typeck/src/infer.rs | crates/snow-typeck/src/error.rs | NoSuchMethod error emission | ✓ WIRED | `TypeError::NoSuchMethod` constructed at 4082, 4117 |
| crates/snow-codegen/src/mir/lower.rs | crates/snow-typeck/src/traits.rs | trait_registry.find_method_traits for method dispatch | ✓ WIRED | Lines 3385, 4733, 4940, 4988 call `find_method_traits()` |
| crates/snow-codegen/src/mir/lower.rs | crates/snow-codegen/src/mir/lower.rs | shared dispatch helper used by both bare-name and dot-syntax calls | ✓ WIRED | `resolve_trait_callee()` at 3377 called from method path (3487) and bare-name path (3713) |

All key links verified and functioning.

### Requirements Coverage

| Requirement | Status | Blocking Issue |
|-------------|--------|----------------|
| METH-01: User can call trait impl methods via dot syntax | ✓ SATISFIED | None — e2e tests prove feature works |
| METH-02: Receiver is automatically passed as the first argument | ✓ SATISFIED | None — MIR lowering prepends receiver; equivalence test proves identical behavior |
| METH-03: Method resolution searches all trait impl blocks for the receiver's type | ✓ SATISFIED | None — `find_method_traits()` and `resolve_trait_method()` search trait registry |
| DIAG-01: Calling a nonexistent method produces "no method \`x\` on type \`Y\`" error | ✓ SATISFIED | None — NoSuchMethod variant with E0030 code and diagnostic rendering |

All requirements satisfied.

### Anti-Patterns Found

None. No TODO/FIXME/PLACEHOLDER comments, no stub implementations, no empty handlers in the modified method resolution code.

### Test Coverage

**Plan 01 (Type Checker):**
- All 390 existing tests pass (233 typeck + 157 codegen) with 0 regressions
- Commits: 754d23a (NoSuchMethod variant), fbdda27 (method resolution)

**Plan 02 (MIR Lowering):**
- 5 MIR-level tests in `crates/snow-codegen/src/mir/lower.rs`:
  - `e2e_method_dot_syntax_basic` (line 9697)
  - `e2e_method_dot_syntax_equivalence` (line 9735)
  - `e2e_method_dot_syntax_with_args` (line 9803)
  - `e2e_method_dot_syntax_field_access_preserved` (line 9840)
  - `e2e_method_dot_syntax_module_qualified_preserved` (line 9886)
- 5 compile-and-run e2e tests in `crates/snowc/tests/e2e.rs`:
  - `e2e_method_dot_syntax_basic` (line 794) — `p.to_string()` works
  - `e2e_method_dot_syntax_equivalence` (line 813) — proves `p.to_string()` ≡ `"${p}"`
  - `e2e_method_dot_syntax_field_access_preserved` (line 834) — `p.x` still works
  - `e2e_method_dot_syntax_module_qualified_preserved` (line 853) — `String.length(s)` still works
  - `e2e_method_dot_syntax_multiple_traits` (line 866) — deriving(Display, Eq) both work
- All 1,242 tests pass (10 new, 0 regressions)
- Commits: 196084b (method call interception), be4f3bd (e2e tests)

**Verification run:**
```bash
cargo test e2e_method_dot_syntax
# Result: 10 passed (5 in snow-codegen, 5 in snowc)
```

### Integration Points

**Preserved behaviors (regression tests):**
- Struct field access: `point.x` still produces `MirExpr::FieldAccess` (not intercepted as method call)
- Module-qualified calls: `String.length(s)` routes through existing lower_field_access path
- Service methods: `Counter.start()` preserved
- Variant construction: `Shape.Circle(5.0)` preserved

**Guard chain in MIR lowering (lower.rs:3448-3463):**
1. STDLIB_MODULES (String, IO, List, etc.)
2. service_modules (user-defined services)
3. Sum type names (Shape, Option, etc.)
4. Struct type names (Point, etc.)
5. Everything else → method call interception

**Resolution priority in type checker (infer_field_access):**
1. Module-qualified access
2. Service method access
3. Variant construction
4. Struct field access
5. **Method resolution** (new)
6. Error (NoSuchField for non-method-call, NoSuchMethod for method-call)

## Summary

**All 9 observable truths verified. All 5 artifacts verified substantive and wired. All 4 key links verified functional. All 4 requirements satisfied.**

Phase 30 goal achieved: Users can call trait impl methods on struct values using dot syntax (`point.to_string()`), with the receiver automatically passed as the first argument. Method calls and bare-name calls produce identical results. All existing functionality (field access, module-qualified calls, services, variants) preserved with 0 regressions.

**End-to-end proof:**
```snow
struct Point do
  x :: Int
  y :: Int
end deriving(Display)

fn main() do
  let p = Point { x: 10, y: 20 }
  println(p.to_string())  # Prints: Point(10, 20)
end
```

**Test result:** ✓ PASSED (e2e_method_dot_syntax_basic)

---

_Verified: 2026-02-08_
_Verifier: Claude (gsd-verifier)_
