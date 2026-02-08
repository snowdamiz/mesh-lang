---
phase: 21
plan: 01
subsystem: runtime-typeck-codegen
tags: [hash, fnv1a, map-keys, trait-dispatch, auto-derive, mir-generation]
depends_on:
  requires: [18-01, 18-03, 19-01, 19-02, 20-01, 20-03]
  provides: [hash-trait-registration, hash-runtime-functions, hash-mir-generation, map-struct-key-support]
  affects: [21-02, 21-03, 21-04]
tech_stack:
  added: []
  patterns: [fnv1a-hash-chaining, hash-as-integer-key-for-maps, call-site-key-hashing]
key_files:
  created:
    - crates/snow-rt/src/hash.rs
  modified:
    - crates/snow-rt/src/lib.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
decisions:
  - id: 21-01-01
    decision: "Hash auto-derived for non-generic structs only (not sum types)"
    rationale: "Sum types need variant tag hashing which adds complexity; structs cover the primary use case for Map keys"
  - id: 21-01-02
    decision: "Hash-as-integer-key approach for Map struct keys"
    rationale: "Hashing at the call site and using FNV-1a hash as an integer key avoids modifying the Map runtime; collisions are extremely rare for typical values"
  - id: 21-01-03
    decision: "FNV-1a 64-bit as the hash algorithm"
    rationale: "Simple (~35 lines), well-studied, good distribution, zero dependencies; matches research recommendation"
metrics:
  duration: 10min
  completed: 2026-02-08
---

# Phase 21 Plan 01: Hash Protocol End-to-End Summary

**One-liner:** FNV-1a runtime hash functions + Hash trait with primitive/struct auto-derive + Hash__hash MIR generation + Map key hashing at call site for struct keys.

## What Was Done

### Task 1: FNV-1a runtime + Hash trait registration + auto-derive for structs

**snow-rt/src/hash.rs (NEW):**
- Created FNV-1a 64-bit hash implementation with `FNV_OFFSET_BASIS` and `FNV_PRIME` constants
- Internal `fnv1a_bytes(bytes: &[u8]) -> u64` helper
- `snow_hash_int(i64) -> i64` -- hash integer via little-endian bytes
- `snow_hash_float(f64) -> i64` -- hash float via bits-to-bytes
- `snow_hash_bool(i8) -> i64` -- hash boolean as single byte
- `snow_hash_string(*const SnowString) -> i64` -- hash string bytes
- `snow_hash_combine(i64, i64) -> i64` -- combine two hashes via XOR+multiply chain
- 3 unit tests (deterministic, distinguishing, order-sensitive combine)

**snow-rt/src/lib.rs:**
- Added `pub mod hash;` module declaration
- Re-exported all 5 hash functions

**snow-typeck/src/builtins.rs:**
- Registered Hash trait with `hash(self) -> Int` method signature
- Registered Hash impls for Int, Float, String, Bool
- Added `hash_trait_registered_for_primitives` test

**snow-typeck/src/infer.rs:**
- Added Hash auto-registration in `register_struct_def()` for non-generic structs
- Follows exact same pattern as Debug/Eq/Ord auto-derive

### Task 2: Generate Hash__hash MIR for structs + Map key hashing at call site

**snow-codegen/src/mir/lower.rs:**
- Added `generate_hash_struct(name, fields)` -- creates `Hash__hash__StructName` MIR functions
  - Single-field structs: direct `snow_hash_<type>(self.field)` call
  - Multi-field structs: chain via `snow_hash_combine(prev_hash, field_hash)`
  - Empty structs: return FNV offset basis constant
- Added `emit_hash_for_type(expr, ty)` helper -- dispatches to correct `snow_hash_*` function per MIR type, with recursive `Hash__hash__InnerStruct` for nested structs
- Called `generate_hash_struct` from `lower_struct_def` (after Eq/Ord generation)
- Added primitive Hash dispatch redirects in trait method call rewriting:
  - `Hash__hash__Int` -> `snow_hash_int`
  - `Hash__hash__Float` -> `snow_hash_float`
  - `Hash__hash__Bool` -> `snow_hash_bool`
  - `Hash__hash__String` -> `snow_hash_string`
- Extended `map_put`/`map_get`/`map_has_key`/`map_delete` interception to handle struct keys:
  - When key type is `MirType::Struct(_)` with Hash impl, emit `Hash__hash__TypeName(key)` and pass hash as integer key
  - Existing String key tagging via `snow_map_tag_string` preserved
- Added 4 tests: `hash_struct_generates_mir_function`, `hash_struct_field_chaining`, `hash_empty_struct_returns_constant`, `map_put_with_struct_key_hashes`

**snow-codegen/src/codegen/intrinsics.rs:**
- Declared 5 hash runtime functions: `snow_hash_int`, `snow_hash_float`, `snow_hash_bool`, `snow_hash_string`, `snow_hash_combine`
- Added intrinsics test assertions for all 5

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | FNV-1a runtime + Hash trait registration + auto-derive | f652c32 | hash.rs: 71 lines (NEW); builtins.rs: +51 lines; infer.rs: +17 lines; lib.rs: +2 lines |
| 2 | Generate Hash__hash MIR + Map key hashing | 31b71b1 | lower.rs: +218 lines (generation + tests); intrinsics.rs: +21 lines |

## Verification Results

- `cargo test --workspace`: all tests pass, 0 failures
- `cargo build --workspace`: clean compilation
- snow-rt: 230 tests (was 227, +3 hash tests)
- snow-codegen: 129 tests (was 125, +4 hash MIR tests)
- snow-typeck: 75 tests (was 74, +1 hash trait test)
- Hash trait registered for Int, Float, String, Bool
- Non-generic structs auto-derive Hash in typeck
- `Hash__hash__Point` MIR function generated with correct params and combine chaining
- Map.put with struct key emits `Hash__hash__Point` call before `snow_map_put`
- FNV-1a runtime functions compiled and declared in codegen intrinsics

## Deviations from Plan

None -- plan executed exactly as written.

## Decisions Made

| ID | Decision | Rationale |
|----|----------|-----------|
| 21-01-01 | Hash auto-derived for non-generic structs only (not sum types) | Sum types need variant tag hashing; structs cover the primary Map key use case |
| 21-01-02 | Hash-as-integer-key approach for Map struct keys | Avoids modifying Map runtime; FNV-1a collision rate is negligible for typical values |
| 21-01-03 | FNV-1a 64-bit as hash algorithm | Simple, well-studied, good distribution, zero dependencies |

## Known Limitations

- **Hash collisions:** Two different struct values with the same FNV-1a hash will be treated as the same Map key. This is extremely rare for typical field values but is a known v1.3 limitation.
- **Sum type Hash:** Not implemented -- sum types cannot be used as Map keys in v1.3. Would require variant tag hashing.
- **Generic struct Hash:** Not implemented -- generic structs need monomorphized Hash impls. Non-generic structs cover the common case.

## Next Phase Readiness

**Unblocked:** Hash protocol fully operational end-to-end. User-defined struct types can be used as Map keys.

**Ready for:**
- Default protocol (21-02) -- same auto-derive + MIR generation pattern
- Default method implementations (21-03)
- Collection Display/Debug (21-04)

**Pattern established:** The Hash protocol reuses the proven auto-register + MIR generation pattern from Debug/Eq/Ord. The new call-site key hashing pattern for Maps is a novel addition that could be extended to other collection types.

## Self-Check: PASSED
