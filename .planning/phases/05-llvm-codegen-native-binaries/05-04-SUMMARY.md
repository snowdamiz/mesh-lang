---
phase: 05-llvm-codegen-native-binaries
plan: 04
subsystem: codegen
tags: [llvm, inkwell, codegen, ir-generation, pattern-matching, closures]
depends_on:
  requires: ["05-01", "05-02", "05-03"]
  provides: ["llvm-codegen", "object-emission", "ir-emission", "target-configuration"]
  affects: ["05-05"]
tech-stack:
  added: []
  patterns: ["alloca-mem2reg", "tagged-union-layout", "closure-env-gc-alloc", "decision-tree-to-switch"]
key-files:
  created:
    - crates/snow-codegen/src/codegen/types.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/codegen/pattern.rs
  modified:
    - crates/snow-codegen/src/codegen/mod.rs
    - crates/snow-codegen/src/lib.rs
decisions:
  - id: "05-04-01"
    description: "ValueKind::basic() replaces Either::left() in Inkwell 0.8.0 API"
  - id: "05-04-02"
    description: "build_switch takes all cases upfront as &[(IntValue, BasicBlock)]"
  - id: "05-04-03"
    description: "Struct field index lookup via mir_struct_defs stored during compile"
  - id: "05-04-04"
    description: "Alloca+mem2reg pattern for if/else and match result merging"
  - id: "05-04-05"
    description: "String comparison placeholder (pointer identity) for Phase 5"
metrics:
  duration: "13 minutes"
  completed: "2026-02-06"
  tests_added: 30
  total_tests: 67
---

# Phase 5 Plan 4: LLVM IR Code Generation Summary

Complete LLVM IR codegen from MIR using Inkwell 0.8.0 with configurable target triple support, all expression types, decision tree pattern matching, and object file emission.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | LLVM type mapping, runtime intrinsics, CodeGen scaffolding | bd58700 | CodeGen struct, type mapping, intrinsic declarations, target config |
| 2 | Expression codegen, decision tree codegen, function compilation | dff01d8 | All MIR expr types, pattern matching, closures, public API |

## What Was Built

### CodeGen Infrastructure (Task 1)

**CodeGen struct** (`codegen/mod.rs`):
- Holds LLVM context, module, builder, target machine
- Type caches for structs and sum type layouts
- Function lookup table, local variable allocas
- Configurable target triple (None = host default)
- Object file emission, LLVM IR emission, optimization passes

**Type Mapping** (`codegen/types.rs`):
- Int -> i64, Float -> f64, Bool -> i1, String -> ptr
- Unit -> {}, Tuple -> anonymous struct, Struct -> named struct
- SumType -> tagged union: { i8 tag, [max_payload x i8] }
- Closure -> { ptr, ptr } (fn_ptr + env_ptr)
- FnPtr -> ptr, Ptr -> ptr, Never -> i8
- Variant overlay structs for sum type field access

**Runtime Intrinsics** (`codegen/intrinsics.rs`):
- All 10 snow-rt extern "C" functions declared in LLVM module
- snow_panic marked with noreturn attribute

### Expression Codegen (Task 2)

**All MIR expression types** (`codegen/expr.rs`):
- Literals: IntLit (const_int), FloatLit (const_float), BoolLit (const_int 0/1), StringLit (snow_string_new)
- Variables: load from alloca, function reference as pointer
- Binary ops: int (add/sub/mul/sdiv/srem/icmp), float (fadd/fsub/fmul/fdiv/frem/fcmp), bool (icmp eq/ne), string (snow_string_concat)
- Short-circuit And/Or with separate basic blocks and phi nodes
- Unary ops: int/float negation, boolean not (xor)
- Function calls: direct (build_call), indirect (build_indirect_call)
- Closure calls: extract fn_ptr+env_ptr from { ptr, ptr }, indirect call with env as first arg
- If/else: alloca result, conditional branch, store in each arm, merge block loads result
- Let binding: alloca + store + codegen body with scope restoration
- Block: codegen sequence, return last expression value
- Match: compile patterns to decision tree, delegate to pattern.rs
- Struct literal: alloca + GEP field stores
- Field access: struct field index lookup from mir_struct_defs
- Construct variant: alloca sum type, store tag, store fields via variant overlay GEP
- Make closure: GC-alloc env, store captures, pack { fn_ptr, env_ptr }
- Return: codegen inner + build_return
- Panic: snow_panic call + unreachable

**Decision Tree Codegen** (`codegen/pattern.rs`):
- Leaf: bind variables via access path navigation, codegen arm body, store result, branch to merge
- Switch: load i8 tag from sum type, LLVM switch instruction with cases
- Test: load value at path, compare with literal, conditional branch
- Guard: codegen guard expression, conditional branch for success/failure
- Fail: snow_panic + unreachable
- Access path navigation: Root (scrutinee ptr), TupleField (GEP), VariantField (overlay GEP), StructField (GEP)
- Type resolution along access paths for correct LLVM types

**Public API** (`lib.rs`):
- `compile_to_object(parse, typeck, output, opt_level, target_triple)` -- full pipeline to .o
- `compile_to_llvm_ir(parse, typeck, output, target_triple)` -- full pipeline to .ll
- `compile(parse, typeck)` -- verify-only pipeline for testing

### Main Wrapper

Generated `main(argc: i32, argv: ptr) -> i32`:
1. Calls `snow_rt_init()`
2. Calls Snow entry function
3. Returns 0

## Decisions Made

1. **Inkwell 0.8.0 ValueKind API**: `try_as_basic_value()` returns `ValueKind<'ctx>` enum with `.basic()` method (not `Either::left()` from older versions)
2. **Switch instruction API**: `build_switch` takes all cases upfront as `&[(IntValue, BasicBlock)]` -- no `add_case` method
3. **Struct field index tracking**: Added `mir_struct_defs` field to CodeGen, populated during `compile()`, used by field access and pattern matching
4. **Alloca+mem2reg for control flow**: If/else and match results stored via alloca, loaded at merge point -- LLVM's mem2reg pass promotes these to phi nodes
5. **String comparison placeholder**: String equality uses constant false/true for Phase 5 (runtime string_eq to be added later)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] PassBuilderOptions::create() instead of Default::default()**
- **Found during:** Task 1
- **Issue:** Inkwell 0.8.0 `PassBuilderOptions` does not implement `Default`
- **Fix:** Used `PassBuilderOptions::create()` static method
- **Files modified:** codegen/mod.rs

**2. [Rule 3 - Blocking] ValueKind API change in Inkwell 0.8.0**
- **Found during:** Task 2
- **Issue:** `try_as_basic_value()` returns `ValueKind<'ctx>` not `Either<BasicValueEnum, InstructionValue>`
- **Fix:** Changed all `.left()` calls to `.basic()` across expr.rs
- **Files modified:** codegen/expr.rs

**3. [Rule 3 - Blocking] build_switch API takes cases upfront**
- **Found during:** Task 2
- **Issue:** `build_switch` does not have `add_case` method; takes all cases in initial call
- **Fix:** Collected all (IntValue, BasicBlock) pairs before calling build_switch
- **Files modified:** codegen/pattern.rs

**4. [Rule 2 - Missing Critical] Struct field name to index mapping**
- **Found during:** Task 2
- **Issue:** No way to map field names to indices for FieldAccess and StructField patterns
- **Fix:** Added `mir_struct_defs` field to CodeGen, populated from MIR structs during compile
- **Files modified:** codegen/mod.rs, codegen/expr.rs

## Verification Results

1. `cargo build -p snow-codegen` compiles cleanly
2. All 67 tests pass (37 pre-existing + 30 new)
3. `cargo test` across full workspace passes with no regressions
4. Hello world LLVM IR contains main wrapper, snow_rt_init, snow_string_new, snow_println
5. LLVM module verification passes for all test cases
6. CodeGen accepts optional target_triple (None = host, Some("...") = custom)
7. Object file and LLVM IR emission both work
8. Decision tree switch/branch/test/guard/fail all generate correct IR

## Next Phase Readiness

Plan 05-05 (driver/linker integration) can proceed. The codegen module provides:
- `compile_to_object()` for generating .o files
- `compile_to_llvm_ir()` for debugging IR
- Configurable target triples for cross-platform compilation
- Verified LLVM module output

## Self-Check: PASSED
