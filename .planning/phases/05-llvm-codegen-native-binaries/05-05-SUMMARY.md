---
phase: 05-llvm-codegen-native-binaries
plan: 05
subsystem: codegen
tags: [cli, linker, e2e-tests, native-binary, snowc, cross-platform, target-triple]
depends_on:
  requires: ["05-01", "05-02", "05-03", "05-04"]
  provides: ["snowc-build-cli", "native-binary-compilation", "e2e-test-suite", "linker-integration"]
  affects: ["06-actor-runtime"]
tech-stack:
  added: [assert_cmd, predicates]
  patterns: ["cli-subcommand-dispatch", "system-cc-linker-driver", "e2e-compile-and-run"]
key-files:
  created:
    - crates/snow-codegen/src/link.rs
    - crates/snowc/tests/e2e.rs
    - tests/e2e/hello.snow
    - tests/e2e/functions.snow
    - tests/e2e/pattern_match.snow
    - tests/e2e/closures.snow
    - tests/e2e/pipe.snow
    - tests/e2e/string_interp.snow
    - tests/e2e/adts.snow
    - tests/e2e/comprehensive.snow
  modified:
    - crates/snowc/src/main.rs
    - crates/snowc/Cargo.toml
    - crates/snow-codegen/src/lib.rs
    - crates/snow-codegen/src/codegen/mod.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/builtins.rs
decisions:
  - id: "05-05-01"
    description: "System cc as linker driver (handles macOS clang / Linux gcc transparently)"
  - id: "05-05-02"
    description: "snowc build auto-builds snow-rt via cargo before linking"
  - id: "05-05-03"
    description: "Closure parameter type annotations resolved in infer_closure (was missing, caused string interpolation to fail in closures)"
patterns-established:
  - "E2E test pattern: write .snow to temp dir, invoke snowc build, run binary, assert stdout"
  - "Linker integration: emit .o then invoke cc with -lsnow_rt"
  - "CLI pipeline: lex -> parse -> typecheck -> MIR lower -> LLVM codegen -> link"
metrics:
  duration: "~15 minutes (across checkpoint pause)"
  completed: "2026-02-06"
  tests_added: 13
  total_tests: 481
---

# Phase 5 Plan 5: snowc build CLI & E2E Integration Summary

**snowc build CLI producing native binaries with --target/--emit-llvm/--opt-level flags, system cc linker integration with statically linked snow-rt, and 13 end-to-end integration tests verifying all phase success criteria**

## Performance

- **Duration:** ~15 min (including checkpoint pause for human verification)
- **Tasks:** 3 (2 auto + 1 checkpoint, plus 1 deviation bug fix)
- **Files modified:** 18
- **Lines changed:** +1357 / -81

## Accomplishments

- `snowc build <dir>` compiles Snow source to native binary through full pipeline (lex -> parse -> typecheck -> MIR lower -> LLVM codegen -> object -> link)
- `--target <triple>` flag for cross-platform object file generation, `--emit-llvm` for IR dump, `--opt-level` for optimization control
- System cc linker driver integrates snow-rt static library for self-contained binaries
- 13 end-to-end tests verify: hello world, functions, pattern matching, ADTs, closures, pipe operator, string interpolation, comprehensive multi-feature program, emit-llvm, optimization levels, self-contained binary, compilation performance, and --target flag
- All 481 tests pass across the full workspace (13 new e2e + 468 existing)

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | snowc build CLI with --target, linker, pipeline | 893e54d | main.rs CLI, link.rs linker, lib.rs compile_to_binary |
| 2 | End-to-end integration tests | ce220b8 | e2e.rs harness, 8 .snow test programs, MIR/typeck fixes |
| - | Bug fix: closure param type annotations | e1d6df4 | infer.rs infer_closure fix for typed closure params |

## Files Created/Modified

### Created
- `crates/snow-codegen/src/link.rs` -- Object file linking via system cc with snow-rt static library
- `crates/snowc/tests/e2e.rs` -- 13 end-to-end integration tests
- `tests/e2e/hello.snow` -- Hello World test program
- `tests/e2e/functions.snow` -- Functions and arithmetic test
- `tests/e2e/pattern_match.snow` -- Pattern matching with wildcards
- `tests/e2e/closures.snow` -- Closures with captured variables
- `tests/e2e/pipe.snow` -- Pipe operator chaining
- `tests/e2e/string_interp.snow` -- String interpolation with variables
- `tests/e2e/adts.snow` -- Algebraic data types with variant matching
- `tests/e2e/comprehensive.snow` -- 100+ line multi-feature integration program

### Modified
- `crates/snowc/src/main.rs` -- CLI with build subcommand, --target/--emit-llvm/--opt-level/--output flags
- `crates/snowc/Cargo.toml` -- Added all pipeline crate dependencies + assert_cmd/predicates for tests
- `crates/snow-codegen/src/lib.rs` -- compile_to_binary public API, link module
- `crates/snow-codegen/src/codegen/mod.rs` -- Optimization pass integration
- `crates/snow-codegen/src/mir/lower.rs` -- MIR lowering fixes for closures and string interpolation
- `crates/snow-typeck/src/infer.rs` -- Closure parameter type annotation inference fix
- `crates/snow-typeck/src/builtins.rs` -- Additional builtin registrations

## Decisions Made

1. **System cc as linker driver**: Uses `cc` command which resolves to clang on macOS and gcc/clang on Linux, handling platform differences transparently
2. **Auto-build snow-rt**: `snowc build` locates the snow-rt static library from the workspace target directory, ensuring it is available for linking
3. **Closure param type annotations**: Fixed `infer_closure` to properly handle typed closure parameters (e.g., `fn(y :: Int) -> ...`), which was needed for string interpolation inside closures to work correctly

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Closure parameter type annotations not applied in infer_closure**
- **Found during:** Post-Task 2 (during checkpoint verification)
- **Issue:** When a closure had typed parameters like `fn(y :: Int) -> x + y end`, the type annotations were not being applied in the type inference pass. This caused string interpolation inside closures to fail because the type checker could not determine the types of closure parameters for implicit conversions.
- **Fix:** Added annotation resolution in `infer_closure` to unify closure parameter type variables with their declared annotation types before inferring the closure body
- **Files modified:** `crates/snow-typeck/src/infer.rs`
- **Verification:** All 13 e2e tests pass, including closures and string interpolation
- **Committed in:** e1d6df4

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Bug fix was necessary for correct closure type inference. No scope creep.

## Issues Encountered

None beyond the closure type annotation bug documented above.

## User Setup Required

None -- no external service configuration required.

## Next Phase Readiness

Phase 5 (LLVM Codegen & Native Binaries) is now **complete**. All 5 plans delivered:

1. **05-01**: Runtime library (snow-rt) with string operations and GC arena
2. **05-02**: MIR lowering from typed AST
3. **05-03**: Pattern match compilation to decision trees
4. **05-04**: LLVM IR code generation from MIR
5. **05-05**: snowc build CLI, linker integration, and end-to-end verification

**Phase 5 success criteria verified:**
- SC1: `snowc build hello_project/` produces a binary that prints "Hello, World!"
- SC2: Functions, pattern matching, ADTs, closures, pipe, string interp all work in a single program
- SC3: Binary is self-contained (snow-rt statically linked, verified by otool)
- SC4: Tests pass on macOS; --target flag supports configurable triples
- SC5: 100-line program compiles in < 5 seconds at -O0

**Ready for Phase 6 (Actor Runtime):**
- The compiler produces working native binaries
- Runtime library foundation (snow-rt) is in place for actor runtime extensions
- The snowc CLI is ready for additional subcommands (run, test, etc.)

---
*Phase: 05-llvm-codegen-native-binaries*
*Plan: 05*
*Completed: 2026-02-06*

## Self-Check: PASSED
