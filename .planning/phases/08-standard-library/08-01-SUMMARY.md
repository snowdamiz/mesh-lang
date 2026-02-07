---
phase: 08-standard-library
plan: 01
subsystem: stdlib-core
tags: [string-ops, io, env, module-resolution, builtins, intrinsics]
requires: [07-supervision-fault-tolerance]
provides: [stdlib-module-infrastructure, string-operations, io-module, env-module]
affects: [08-02-PLAN, 08-03-PLAN, 08-04-PLAN, 08-05-PLAN]
tech-stack:
  added: []
  patterns: [stdlib-module-namespace, from-import-resolution, module-qualified-access, i8-i1-bool-coercion]
key-files:
  created:
    - crates/snow-rt/src/io.rs
    - crates/snow-rt/src/env.rs
    - crates/snowc/tests/e2e_stdlib.rs
    - tests/e2e/stdlib_string_length.snow
    - tests/e2e/stdlib_string_contains.snow
    - tests/e2e/stdlib_string_trim.snow
    - tests/e2e/stdlib_string_case.snow
    - tests/e2e/stdlib_string_replace.snow
    - tests/e2e/stdlib_module_qualified.snow
    - tests/e2e/stdlib_from_import.snow
    - tests/e2e/stdlib_io_eprintln.snow
  modified:
    - crates/snow-rt/src/string.rs
    - crates/snow-rt/src/lib.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/expr.rs
key-decisions:
  - id: stdlib-module-resolution
    decision: "Module-qualified access (String.length) resolved by checking FieldAccess base against known module names, short-circuiting before struct field lookup"
    rationale: "Cleanest integration with existing FieldAccess inference; modules checked first, then sum types, then struct fields"
  - id: from-import-dual-registration
    decision: "from/import inserts both bare name and prefixed name (e.g., 'length' and 'string_length') into type env"
    rationale: "Bare name for user access, prefixed name for lowerer's map_builtin_name to resolve correctly"
  - id: bool-i8-coercion
    decision: "Added automatic i1<->i8 coercion at runtime intrinsic call boundaries"
    rationale: "Snow Bool is i1, but C FFI uses i8; zext arguments before call, trunc return values after call"
  - id: skip-string-split
    decision: "Deferred string_split to Plan 02 when List<T> exists"
    rationale: "Plan called for temporary packed array representation but List is the proper return type"
patterns-established:
  - "stdlib_modules() HashMap<String, HashMap<String, Scheme>> for module namespace registry"
  - "is_stdlib_module() check in FieldAccess inference and MIR lowering"
  - "STDLIB_MODULES const array shared between type checker and lowerer"
  - "Runtime i8/i1 bool coercion pattern for future stdlib boolean returns"
duration: 9 minutes
completed: 2026-02-07
---

# Phase 8 Plan 01: Core Stdlib Infrastructure Summary

String operations, IO, Env modules with module-qualified access and from/import resolution through full compiler pipeline.

## Performance

- Execution time: 9 minutes
- All 3 tasks completed in single pass
- 1 deviation (Rule 1 bug fix) for i8/i1 Bool coercion
- Full workspace: 400+ tests passing

## Accomplishments

1. **Runtime functions (snow-rt)**: 10 new string operations (length, slice, contains, starts_with, ends_with, trim, to_upper, to_lower, replace, eq), IO module (read_line returning SnowResult, eprintln), Env module (get returning SnowOption, args). All use SnowString pattern with GC allocation.

2. **Type checker (snow-typeck)**: All stdlib functions registered as builtins. Module namespace resolution via `stdlib_modules()` HashMap. `from/import` resolves names into local scope via FromImportDecl handling. FieldAccess on module names (String.length) resolves to function type.

3. **Codegen pipeline (snow-codegen)**: 14 new LLVM intrinsic declarations. MIR lowerer maps all names via extended `map_builtin_name`. Known functions registered for direct call dispatch. Module-qualified FieldAccess lowering converts `String.length` to `snow_string_length`.

4. **E2E tests**: 8 integration tests verify the full compile-run cycle for string ops, module-qualified access, from/import syntax, and IO operations.

## Task Commits

| Task | Name | Commit | Key Files |
|------|------|--------|-----------|
| 1 | Runtime functions -- String ops, IO, Env | 900004f | string.rs, io.rs, env.rs, lib.rs |
| 2 | Compiler pipeline -- types, intrinsics, lowering, module resolution | 7a74946 | builtins.rs, infer.rs, intrinsics.rs, lower.rs |
| 3 | E2E integration tests and i1/i8 bool fix | 24e489a | e2e_stdlib.rs, expr.rs, 8 .snow fixtures |

## Files Created

- `crates/snow-rt/src/io.rs` -- IO module: read_line (SnowResult), eprintln
- `crates/snow-rt/src/env.rs` -- Env module: get (SnowOption), args (packed array)
- `crates/snowc/tests/e2e_stdlib.rs` -- 8 E2E tests for stdlib
- `tests/e2e/stdlib_*.snow` -- 8 Snow fixture programs

## Files Modified

- `crates/snow-rt/src/string.rs` -- 10 new string operations + 14 unit tests
- `crates/snow-rt/src/lib.rs` -- Wired io, env modules; re-exported all new functions
- `crates/snow-typeck/src/builtins.rs` -- Registered 12 stdlib functions
- `crates/snow-typeck/src/infer.rs` -- Module namespace resolution, from/import handling, module FieldAccess
- `crates/snow-codegen/src/codegen/intrinsics.rs` -- 14 new LLVM function declarations
- `crates/snow-codegen/src/mir/lower.rs` -- Extended map_builtin_name (30+ mappings), known_functions, module-qualified lowering
- `crates/snow-codegen/src/codegen/expr.rs` -- Bool i1/i8 coercion at runtime call boundaries

## Decisions Made

1. **Module-qualified resolution**: FieldAccess checks `is_stdlib_module` before sum type variants and struct fields. This means `String.length` is intercepted before any struct lookup.

2. **Dual name registration for from/import**: Both `length` and `string_length` inserted into env, so the lowerer's `map_builtin_name` can resolve either path to `snow_string_length`.

3. **Bool i1/i8 coercion**: Added automatic `zext` for i1->i8 arguments and `trunc` for i8->i1 return values at runtime intrinsic boundaries. This is a general fix that benefits all future boolean-returning runtime functions.

4. **Deferred string_split**: Skipped `snow_string_split` since it returns `List<String>` which doesn't exist until Plan 02.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Bool i1/i8 type mismatch at runtime call boundaries**

- **Found during:** Task 3 (E2E testing)
- **Issue:** Snow Bool is i1 in LLVM IR, but runtime C FFI functions use i8 for booleans. When `string_contains` returns i8 and the result is passed to `snow_bool_to_string(i8)`, the codegen passed i1 causing LLVM verification failure.
- **Fix:** Added argument coercion (i1->i8 zext) and return coercion (i8->i1 trunc) at the runtime intrinsic call dispatch in `expr.rs`.
- **Files modified:** `crates/snow-codegen/src/codegen/expr.rs`
- **Commit:** 24e489a

## Issues

None. All success criteria met.

## Next Phase Readiness

**For Plan 02 (Collections - List, Map, Set):**
- Module resolution infrastructure is ready: add "List", "Map", "Set" to `stdlib_modules()`
- `string_split` should be implemented in Plan 02 to return proper `List<String>`
- Known function registration pattern established for quick addition
- i8/i1 coercion pattern available for any new boolean-returning functions

**For Plan 03 (File I/O):**
- IO module pattern established in `io.rs`
- SnowResult struct ready for Result-returning file operations
- "File" already in STDLIB_MODULES/STDLIB_MODULE_NAMES

## Self-Check: PASSED
