---
phase: 27-list-trait-pattern-integration
plan: 02
subsystem: compiler
tags: [pattern-matching, cons-pattern, list, destructuring, codegen, MIR, decision-tree]

# Dependency graph
requires:
  - phase: 26-polymorphic-list-foundation
    provides: "List<T> type, snow_list_head/tail/length runtime functions, list literal codegen"
  - phase: 27-01
    provides: "List Eq/Ord traits, extract_list_elem_type helper"
provides:
  - "head :: tail cons pattern destructuring in case expressions"
  - "CONS_PAT parser + ConsPat AST node"
  - "MirPattern::ListCons + DecisionTree::ListDecons"
  - "Full pipeline: parser -> typeck -> MIR -> pattern compiler -> LLVM codegen"
  - "Recursive list processing via cons patterns"
affects: [28-comprehensive-testing, 29-documentation]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "ListDecons decision tree node for runtime list length check + head/tail extraction"
    - "AccessPath::ListHead/ListTail for navigating list sub-values in pattern bindings"
    - "Local variable precedence over builtin name mapping in lower_name_ref"

key-files:
  created:
    - tests/e2e/list_cons_int.snow
    - tests/e2e/list_cons_string.snow
  modified:
    - crates/snow-parser/src/syntax_kind.rs
    - crates/snow-parser/src/parser/patterns.rs
    - crates/snow-parser/src/ast/pat.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/mir/mod.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/pattern/mod.rs
    - crates/snow-codegen/src/pattern/compile.rs
    - crates/snow-codegen/src/codegen/pattern.rs
    - crates/snowc/tests/e2e_stdlib.rs

key-decisions:
  - "27-02-D1: ListDecons decision tree node rather than reusing Switch or Test for cons patterns"
  - "27-02-D2: AccessPath::ListHead/ListTail enum variants for navigating list sub-values"
  - "27-02-D3: Local variable bindings take precedence over builtin function name mappings"
  - "27-02-D4: Conservative exhaustiveness -- cons patterns treated as wildcards (lists are infinite)"

patterns-established:
  - "ListDecons: runtime length check + head/tail extraction as a decision tree node"
  - "u64-to-type conversion via convert_list_elem_from_u64 for pattern-bound head values"

# Metrics
duration: 25min
completed: 2026-02-09
---

# Phase 27 Plan 02: Cons Pattern Destructuring Summary

**Right-associative `head :: tail` cons pattern for List<T> with full parser-to-LLVM pipeline, recursive list processing, and u64 element type conversion**

## Performance

- **Duration:** ~25 min
- **Started:** 2026-02-09T00:35:00Z
- **Completed:** 2026-02-09T01:00:31Z
- **Tasks:** 2
- **Files modified:** 11

## Accomplishments

- Full `head :: tail` cons pattern pipeline from parser through LLVM codegen
- Recursive list processing works end-to-end (sum_list via cons pattern produces correct results)
- Element type conversion from u64 (uniform storage) to actual type (Int/Bool/Float/String/Ptr)
- Fixed critical name shadowing bug where pattern binding `head` was incorrectly mapped to `snow_list_head` function

## Task Commits

Each task was committed atomically:

1. **Task 1: Parser + AST + Typeck + MIR + pattern compiler for cons patterns** - `b1323f0` (feat)
2. **Task 2: E2e tests + fix variable shadowing of builtin names** - `08a4848` (feat)

## Files Created/Modified

- `crates/snow-parser/src/syntax_kind.rs` - Added CONS_PAT variant
- `crates/snow-parser/src/parser/patterns.rs` - parse_cons_pattern function (right-associative ::)
- `crates/snow-parser/src/ast/pat.rs` - ConsPat AST node with head()/tail() accessors
- `crates/snow-typeck/src/infer.rs` - infer_cons_pattern (head=T, tail=List<T>), exhaustiveness handling
- `crates/snow-codegen/src/mir/mod.rs` - MirPattern::ListCons variant
- `crates/snow-codegen/src/mir/lower.rs` - Cons pattern lowering + fix local var precedence
- `crates/snow-codegen/src/pattern/mod.rs` - DecisionTree::ListDecons, AccessPath::ListHead/ListTail
- `crates/snow-codegen/src/pattern/compile.rs` - HeadCtor::ListCons, compile_list_cons, specialize_for_list_cons
- `crates/snow-codegen/src/codegen/pattern.rs` - codegen_list_decons, ListHead/ListTail navigation, convert_list_elem_from_u64
- `tests/e2e/list_cons_int.snow` - Recursive sum via cons pattern
- `tests/e2e/list_cons_string.snow` - String first-or-default via cons pattern
- `crates/snowc/tests/e2e_stdlib.rs` - Registered e2e_list_cons_int and e2e_list_cons_string

## Decisions Made

- **27-02-D1:** Used a dedicated `DecisionTree::ListDecons` node rather than reusing Switch or Test. Cons patterns need runtime length check + extraction, which doesn't fit the existing constructor tag switch or literal comparison patterns.
- **27-02-D2:** Added `AccessPath::ListHead` and `AccessPath::ListTail` enum variants. These enable the pattern compiler to express "navigate to the head/tail of a list" in the same way VariantField navigates constructor fields.
- **27-02-D3:** Fixed `lower_name_ref` to check local variable scope BEFORE applying `map_builtin_name`. Pattern binding `head` was being mapped to `snow_list_head` function, producing incorrect codegen. Local bindings must always shadow builtin names.
- **27-02-D4:** Cons patterns are treated as wildcards for exhaustiveness checking. Lists are infinite types, so `head :: tail` alone can never make a match exhaustive -- a wildcard/empty-list arm is always needed.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] MIR + pattern compiler added in Task 1 instead of Task 2**
- **Found during:** Task 1
- **Issue:** Adding Pattern::Cons to the AST required handling it in lower.rs and compile.rs for the code to compile (non-exhaustive pattern match errors)
- **Fix:** Added MirPattern::ListCons, DecisionTree::ListDecons, AccessPath::ListHead/ListTail, and pattern compilation support in Task 1
- **Files modified:** All codegen files
- **Verification:** `cargo build` succeeds
- **Committed in:** b1323f0 (Task 1 commit)

**2. [Rule 1 - Bug] Fixed variable name shadowing by builtin function mappings**
- **Found during:** Task 2
- **Issue:** `map_builtin_name("head")` returns `"snow_list_head"`, which was applied BEFORE checking local variable scope. Pattern binding `head` was resolved as the runtime function pointer instead of the local variable.
- **Fix:** Moved `self.lookup_var(&name)` check BEFORE `map_builtin_name` call in `lower_name_ref`
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Verification:** `head :: tail` correctly binds head as local variable; sum_list outputs 15
- **Committed in:** 08a4848 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes necessary for correct operation. The blocking issue was purely organizational (code in Task 1 vs Task 2). The bug fix was a real correctness issue that would have affected any pattern variable named `head` or `tail`.

## Issues Encountered

- **Snow uses `::` for type annotations AND cons patterns:** No actual ambiguity because type annotations are parsed in parameter position (after IDENT), while cons patterns are parsed in pattern position (case arms). The grammar hierarchy correctly handles this.
- **Snow uses `println("${expr}")` not `println(to_string(expr))`:** Adapted test fixtures to use string interpolation per existing codebase conventions.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Phase 27 (List Trait Pattern Integration) is complete
- LIST-08 cons pattern destructuring requirement satisfied
- All 1,212+ tests pass with 0 regressions
- Ready for Phase 28 (comprehensive testing) or Phase 29 (documentation)

## Self-Check: PASSED

---
*Phase: 27-list-trait-pattern-integration*
*Completed: 2026-02-09*
