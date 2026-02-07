---
phase: 08-standard-library
plan: 03
subsystem: file-io
tags: [file, io, result, pattern-matching, codegen, runtime]
depends_on:
  requires: ["08-01"]
  provides: ["File module with read/write/append/exists/delete"]
  affects: ["08-04", "08-05"]
tech-stack:
  added: ["tempfile (dev-dependency)"]
  patterns: ["Aligned sum type layout for runtime interop", "Monomorphized generic type fallback in codegen"]
key-files:
  created:
    - crates/snow-rt/src/file.rs
    - tests/e2e/stdlib_file_write_read.snow
    - tests/e2e/stdlib_file_exists.snow
    - tests/e2e/stdlib_file_process.snow
    - tests/e2e/stdlib_file_error.snow
  modified:
    - crates/snow-rt/src/lib.rs
    - crates/snow-rt/Cargo.toml
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/codegen/mod.rs
    - crates/snow-codegen/src/codegen/pattern.rs
    - crates/snow-codegen/src/codegen/types.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snowc/tests/e2e_stdlib.rs
decisions:
  - id: D-08-03-01
    title: "Aligned sum type layout for runtime interop"
    choice: "Use { i8, ptr } layout for sum types with single pointer-sized variant fields"
    rationale: "Runtime's #[repr(C)] SnowResult has 7 bytes padding after tag before pointer field. Old { i8, [8 x i8] } layout put data at offset 1 instead of offset 8, causing field access corruption."
  - id: D-08-03-02
    title: "Monomorphized generic type lookup fallback"
    choice: "When looking up Result_String_String, fall back to base name Result"
    rationale: "Generic sum types are registered under their base name but used via monomorphized names in MIR types."
  - id: D-08-03-03
    title: "Ptr-to-sum-type dereference at let binding"
    choice: "When binding a runtime-returned ptr to a SumType variable, insert LLVM load to dereference"
    rationale: "Runtime functions return heap pointers but codegen treats sum types as by-value structs."
  - id: D-08-03-04
    title: "Generic type parameters resolve to MirType::Ptr"
    choice: "Replace generic type params (T, E) with Ptr in builtin sum type variant fields"
    rationale: "Type params resolve to MirType::Struct('T') which creates opaque unsized LLVM types. All variant payloads are pointers at runtime."
metrics:
  duration: ~15min
  completed: 2026-02-07
---

# Phase 8 Plan 3: File I/O Summary

File I/O module with read/write/append/exists/delete returning Result types, plus critical codegen fixes enabling runtime sum type pattern matching.

## Performance

- All 4 E2E file I/O tests pass
- Full workspace: 0 failures across all test suites
- 7 new unit tests for runtime file functions
- First working Result pattern matching on runtime-returned values

## Accomplishments

### Task 1: File I/O Runtime Functions
- Implemented 5 extern "C" functions in `crates/snow-rt/src/file.rs`:
  - `snow_file_read(path) -> *mut SnowResult` -- reads file as UTF-8
  - `snow_file_write(path, content) -> *mut SnowResult` -- creates/overwrites
  - `snow_file_append(path, content) -> *mut SnowResult` -- appends, creates if missing
  - `snow_file_exists(path) -> i8` -- returns 1/0
  - `snow_file_delete(path) -> *mut SnowResult` -- removes file
- All use SnowResult (tag 0 = Ok, tag 1 = Err) for error handling
- 7 unit tests covering full CRUD cycle with tempfile isolation

### Task 2: Compiler Pipeline Integration and E2E Tests
- Registered file functions in type checker (builtins.rs) and File module (infer.rs)
- Added LLVM intrinsic declarations for all 5 functions
- Added MIR lowering name mappings (file_read -> snow_file_read, etc.)
- Fixed 4 critical codegen bugs for runtime sum type interop (see Deviations)
- 4 E2E tests proving file operations work end-to-end

## Task Commits

| Task | Commit | Description |
|------|--------|-------------|
| 1 | `7154056` | File I/O runtime functions with 7 unit tests |
| 2 | `2fb473e` | Compiler pipeline integration, codegen fixes, 4 E2E tests |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Monomorphized sum type layout lookup failure**
- **Found during:** Task 2 E2E testing
- **Issue:** Pattern matching on `Result<String, String>` failed with "Unknown sum type layout 'Result_String_String'" because codegen only stored layouts under base names (e.g., "Result")
- **Fix:** Added `lookup_sum_type_layout` and `lookup_sum_type_def` methods to CodeGen that strip type suffixes to find base layout
- **Files modified:** codegen/mod.rs, codegen/pattern.rs, codegen/expr.rs, codegen/types.rs

**2. [Rule 1 - Bug] Generic type parameters produced unsized LLVM types**
- **Found during:** Task 2 E2E testing
- **Issue:** Builtin sum type variant fields with generic params (T, E) resolved to `MirType::Struct("T")` which created opaque unsized LLVM types, causing "GEP into unsized type" errors
- **Fix:** In MIR lowering, detect generic type params in variant fields and replace with `MirType::Ptr`
- **Files modified:** mir/lower.rs

**3. [Rule 1 - Bug] Sum type layout alignment mismatch with runtime**
- **Found during:** Task 2 E2E testing
- **Issue:** Codegen's `{ i8, [8 x i8] }` layout placed payload at offset 1, but runtime's `#[repr(C)] SnowResult { u8, *mut u8 }` has pointer at offset 8 (with 7 bytes padding). Pattern matching read garbage bytes as the tag value.
- **Fix:** Changed `create_sum_type_layout` to use `{ i8, ptr }` for sum types with single pointer-sized fields, which LLVM naturally aligns to match `#[repr(C)]`
- **Files modified:** codegen/types.rs

**4. [Rule 1 - Bug] Runtime-returned pointers stored as raw bytes in sum type allocas**
- **Found during:** Task 2 E2E testing
- **Issue:** `let result = File.read(path)` stored the raw pointer value (8 bytes) into a sum type alloca (16 bytes), interpreting pointer address bytes as struct fields
- **Fix:** In `codegen_let`, detect when a pointer value is being bound to a SumType variable and insert an LLVM load to dereference the heap pointer first
- **Files modified:** codegen/expr.rs

**5. [Rule 3 - Blocking] Missing TyCon import in builtins.rs**
- **Found during:** Task 2 compilation
- **Issue:** Plan 02 (Collections) added `TyCon` references to builtins.rs without importing it
- **Fix:** Added `TyCon` to the import statement
- **Files modified:** builtins.rs

## Decisions Made

1. **Aligned sum type layout**: Sum types with single pointer-sized variant payloads use `{ i8, ptr }` instead of `{ i8, [N x i8] }` to match runtime `#[repr(C)]` struct alignment.

2. **Monomorphized name fallback**: Generic sum types are looked up by stripping monomorphized suffixes (Result_String_String -> Result) rather than creating separate layouts for each monomorphization.

3. **Generic params as Ptr**: Unresolved generic type parameters (T, E) in builtin sum type variant fields are replaced with `MirType::Ptr` in MIR lowering, since all variant payloads are heap-allocated pointers.

4. **Deref at binding site**: Runtime-returned sum type pointers are dereferenced when bound to a local variable via `let`, ensuring the local holds the actual struct value rather than a raw pointer.

## Issues & Risks

- **Parallel execution race**: Plan 02 running in parallel repeatedly overwrote uncommitted changes to shared files (mod.rs, pattern.rs, expr.rs, types.rs). Required multiple re-applications of edits. This is an inherent risk of parallel plan execution on shared codegen files.
- **Sum type layout assumption**: The `{ i8, ptr }` layout assumes all runtime sum type payloads are single pointer-sized fields. If future sum types have multi-field variants or non-pointer payloads, the byte array fallback handles them.

## Next Phase Readiness

- File I/O is fully operational for Strings and Results
- Pattern matching on runtime-returned Result types now works correctly
- Ready for Plan 04 (Math module) and Plan 05 (Advanced features)
- The aligned sum type layout fix also benefits IO.read_line (which returns Result)

## Self-Check: PASSED
