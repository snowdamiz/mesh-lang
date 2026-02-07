---
phase: 08-standard-library
plan: 06
subsystem: testing
tags: [e2e, closures, map, filter, reduce, io, stdin, hof, pipe-chain]

# Dependency graph
requires:
  - phase: 08-standard-library/02
    provides: "List collection type with map/filter/reduce runtime HOFs"
  - phase: 08-standard-library/01
    provides: "IO module with read_line, module-qualified access"
provides:
  - "E2E test proving map/filter/reduce with closures work through full compiler pipeline"
  - "E2E test proving IO.read_line works through full compiler pipeline with piped stdin"
  - "compile_and_run_with_stdin helper for testing interactive I/O"
  - "Closure-to-HOF struct splitting fix in codegen (fn_ptr/env_ptr extraction)"
  - "Non-null env_ptr for zero-capture closures (runtime HOF calling convention fix)"
affects: [phase-9, future-closure-hof-integration]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Closure struct splitting: codegen extracts {fn_ptr, env_ptr} into separate args for runtime intrinsics"
    - "Non-null env sentinel: zero-capture closures get dummy env allocation so HOFs use closure calling convention"

key-files:
  created:
    - "tests/e2e/stdlib_list_pipe_chain.snow"
    - "tests/e2e/stdlib_io_read_line.snow"
  modified:
    - "crates/snowc/tests/e2e_stdlib.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"

key-decisions:
  - "Direct function calls (not pipe operator) for closure HOF chains due to parser limitation with |> near closures"
  - "Non-null dummy env for zero-capture closures ensures correct HOF calling convention"
  - "Option A (compile_and_run_with_stdin) chosen for IO.read_line test, not compile-only"

patterns-established:
  - "compile_and_run_with_stdin helper: pipe stdin to compiled Snow binaries for interactive I/O testing"
  - "Closure args to runtime intrinsics auto-split into fn_ptr + env_ptr at codegen"

# Metrics
duration: 10min
completed: 2026-02-07
---

# Phase 8 Plan 6: Pipe Chain and IO.read_line E2E Gap Closure Summary

**Closure HOF chain (map/filter/reduce) and IO.read_line verified end-to-end, with codegen fix for closure struct splitting**

## Performance

- **Duration:** 10 min
- **Started:** 2026-02-07T06:51:01Z
- **Completed:** 2026-02-07T07:01:50Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Map/filter/reduce chain with closures produces correct result (80) through full compiler pipeline
- IO.read_line compiles and runs with piped stdin, pattern-matching Ok/Err correctly
- Fixed fundamental codegen bug: closure structs now correctly split into fn_ptr + env_ptr for runtime HOFs
- Fixed zero-capture closure env_ptr: non-null sentinel ensures runtime uses closure calling convention

## Task Commits

Each task was committed atomically:

1. **Task 1: Pipe chain E2E test with closures** - `8189b4d` (feat)
2. **Task 2: IO.read_line E2E compilation test** - `91e9dd1` (feat)

**Plan metadata:** (pending) (docs: complete plan)

## Files Created/Modified
- `tests/e2e/stdlib_list_pipe_chain.snow` - Snow fixture: list [1..10] -> map(x*2) -> filter(x>10) -> reduce(sum) = 80
- `tests/e2e/stdlib_io_read_line.snow` - Snow fixture: IO.read_line() with Ok/Err pattern match
- `crates/snowc/tests/e2e_stdlib.rs` - Two new E2E tests + compile_and_run_with_stdin helper
- `crates/snow-codegen/src/codegen/expr.rs` - Closure struct splitting for HOF intrinsics + non-null env fix

## Decisions Made
- Used direct function calls (map/filter/reduce) instead of pipe operator (|>) for the closure HOF chain test, because the parser has a known limitation where |> near inline closures causes cross-line expression merging
- Chose Option A (compile_and_run_with_stdin with piped stdin) for IO.read_line test rather than compile-only, since the binary runs correctly with piped input
- Zero-capture closures now allocate a minimal 8-byte GC heap object for env_ptr instead of null, ensuring runtime HOFs always use the closure calling convention

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Closure struct not split for HOF runtime intrinsics**
- **Found during:** Task 1 (Pipe chain E2E test)
- **Issue:** When calling map(list, fn(x) -> x * 2 end), the closure is compiled as a { fn_ptr, env_ptr } struct but runtime snow_list_map expects 3 separate pointer arguments (list, fn_ptr, env_ptr). LLVM verification failed with "Incorrect number of arguments passed to called function"
- **Fix:** Added closure struct splitting in codegen_call: when a closure-typed argument is passed to a runtime intrinsic, extract fn_ptr and env_ptr via alloca + GEP + load and pass as two separate args
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** LLVM module verification passes, map/filter/reduce produce correct results
- **Committed in:** 8189b4d (Task 1 commit)

**2. [Rule 1 - Bug] Zero-capture closures used null env_ptr causing wrong calling convention**
- **Found during:** Task 1 (Pipe chain E2E test)
- **Issue:** Zero-capture closures set env_ptr to null. Runtime HOFs check env_ptr: if null, call fn(element); if non-null, call fn(env, element). But Snow always lifts closures with __env as first parameter, so the runtime called fn(element) but the function expected fn(__env, element), causing the element value to land in the wrong parameter
- **Fix:** Changed codegen_make_closure to allocate a minimal 8-byte GC heap object for env_ptr when captures are empty, ensuring env_ptr is non-null and runtime uses the closure calling convention
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** map(fn(x) -> x * 2 end) on [1] correctly returns 2 (was returning 0 with null env)
- **Committed in:** 8189b4d (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes were essential for closures to work with HOF runtime functions. Without these fixes, map/filter/reduce with closures would fail at compile time (LLVM verification) or produce wrong results at runtime. No scope creep.

## Issues Encountered
- Pipe operator (|>) cannot be used with inline closures (fn(x) -> expr end) because the parser merges the expression with the previous line's let binding. This is a pre-existing parser limitation documented in STATE.md. Workaround: use direct function calls (map(list, closure)) instead of pipe syntax (list |> map(closure)).

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All Phase 8 gap closure plans complete
- 28 E2E stdlib tests passing (was 26 before this plan)
- Closure-to-HOF integration verified end-to-end
- IO.read_line verified end-to-end with piped stdin
- Ready for Phase 9

---
*Phase: 08-standard-library*
*Completed: 2026-02-07*

## Self-Check: PASSED
