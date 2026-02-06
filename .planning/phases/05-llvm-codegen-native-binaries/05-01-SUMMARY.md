---
phase: 05-llvm-codegen-native-binaries
plan: 01
subsystem: codegen
tags: [llvm, inkwell, runtime, gc, staticlib, extern-c, bump-allocator]

# Dependency graph
requires:
  - phase: 04-pattern-matching-adts
    provides: TypeRegistry with StructDefInfo, SumTypeDefInfo, VariantInfo
provides:
  - snow-rt crate (runtime library with GC, strings, panic)
  - snow-codegen crate (scaffolding with inkwell dependency)
  - TypeckResult.type_registry public field for codegen consumption
  - LLVM 21 build environment (.cargo/config.toml)
affects: [05-02, 05-03, 05-04, 05-05, 06-actor-runtime]

# Tech tracking
tech-stack:
  added: [inkwell 0.8.0, llvm-sys 211, clap 4.5]
  patterns: [extern-C runtime ABI, arena/bump GC, length-prefixed strings]

key-files:
  created:
    - crates/snow-rt/Cargo.toml
    - crates/snow-rt/src/lib.rs
    - crates/snow-rt/src/gc.rs
    - crates/snow-rt/src/string.rs
    - crates/snow-rt/src/panic.rs
    - crates/snow-codegen/Cargo.toml
    - crates/snow-codegen/src/lib.rs
    - crates/snow-codegen/src/mir/mod.rs
    - crates/snow-codegen/src/codegen/mod.rs
  modified:
    - Cargo.toml
    - .cargo/config.toml
    - crates/snow-typeck/src/lib.rs
    - crates/snow-typeck/src/infer.rs

key-decisions:
  - "Arena/bump allocator for Phase 5 GC (no collection); true mark-sweep deferred to Phase 6"
  - "SnowString repr(C) with inline length prefix: { u64 len, [u8; len] data }"
  - "Mutex-protected global arena for thread safety (single-threaded in Phase 5)"
  - "TypeRegistry and all type definition info structs made fully pub with pub fields"
  - "LLVM_SYS_211_PREFIX configured for LLVM 21.1.8 at /opt/homebrew/opt/llvm"

patterns-established:
  - "extern C ABI: all runtime functions use #[no_mangle] pub extern C for LLVM IR callability"
  - "GC allocation: all heap values go through snow_gc_alloc(size, align) -> *mut u8"
  - "String layout: SnowString header followed by inline data bytes, allocated via GC"
  - "Codegen crate structure: mir/ and codegen/ submodules for MIR lowering and LLVM emission"

# Metrics
duration: 5min
completed: 2026-02-06
---

# Phase 5 Plan 1: Runtime & Codegen Foundation Summary

**snow-rt runtime crate with arena GC, GC-managed strings, and panic handler; snow-codegen scaffolding with inkwell/LLVM 21; TypeckResult type registry exposed for codegen consumption**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-06T22:55:23Z
- **Completed:** 2026-02-06T23:00:14Z
- **Tasks:** 2
- **Files modified:** 16

## Accomplishments
- snow-rt runtime library compiles as both staticlib (.a) and Rust lib, exporting 10 extern "C" functions
- snow-codegen crate builds with inkwell 0.8.0 linked against LLVM 21.1.8
- TypeckResult.type_registry provides codegen access to StructDefInfo, SumTypeDefInfo, VariantInfo, TypeAliasInfo
- 11 new unit tests for GC allocation, string operations, and type conversions
- Zero regressions across all 400 workspace tests

## Task Commits

Each task was committed atomically:

1. **Task 1: Create snow-rt runtime crate with extern C functions** - `545badb` (feat)
2. **Task 2: Create snow-codegen crate scaffolding and expose TypeckResult internals** - `8c5eb49` (feat)

## Files Created/Modified
- `crates/snow-rt/Cargo.toml` - Runtime crate manifest (staticlib + lib)
- `crates/snow-rt/src/lib.rs` - Runtime entry point, re-exports
- `crates/snow-rt/src/gc.rs` - Arena/bump allocator with snow_gc_alloc and snow_rt_init
- `crates/snow-rt/src/string.rs` - SnowString type, new/concat/format/print functions
- `crates/snow-rt/src/panic.rs` - snow_panic with source location reporting
- `crates/snow-codegen/Cargo.toml` - Codegen crate manifest with inkwell dependency
- `crates/snow-codegen/src/lib.rs` - compile() entry point (todo! placeholder)
- `crates/snow-codegen/src/mir/mod.rs` - MIR module stub
- `crates/snow-codegen/src/codegen/mod.rs` - Codegen module stub
- `Cargo.toml` - Added snow-rt and snow-codegen to workspace, inkwell and clap deps
- `.cargo/config.toml` - Updated from LLVM 18 to LLVM 21 (LLVM_SYS_211_PREFIX)
- `crates/snow-typeck/src/lib.rs` - Added type_registry field to TypeckResult, re-exports
- `crates/snow-typeck/src/infer.rs` - Made StructDefInfo, SumTypeDefInfo, VariantInfo, VariantFieldInfo, TypeAliasInfo, TypeRegistry all pub with pub fields

## Decisions Made
- Arena/bump allocator chosen over mark-sweep for Phase 5 GC: simplest correct implementation, no collection needed yet. Phase 6 will introduce per-actor GC with actual collection.
- SnowString uses inline length prefix layout `{ u64 len, data... }` rather than a separate allocation for data. More cache-friendly for small strings.
- Global arena uses `std::sync::Mutex<Option<Arena>>` rather than `static mut` with unsafe: marginally slower but eliminates UB risk.
- TypeRegistry fields made fully `pub` (not just pub methods) to give codegen maximum flexibility for memory layout computation.
- Updated LLVM_SYS_211_PREFIX from the Phase 1 placeholder (LLVM 18) to the actual installed version (LLVM 21.1.8).

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required. LLVM 21 was already installed at /opt/homebrew/opt/llvm.

## Next Phase Readiness
- snow-rt provides all runtime functions needed by compiled Snow programs (GC alloc, strings, I/O, panic)
- snow-codegen is ready for MIR type definitions (05-02) and AST-to-MIR lowering (05-03)
- TypeckResult.type_registry gives codegen full access to struct/sum type layouts
- inkwell builds and links correctly against LLVM 21
- No blockers for subsequent plans

## Self-Check: PASSED

---
*Phase: 05-llvm-codegen-native-binaries*
*Completed: 2026-02-06*
