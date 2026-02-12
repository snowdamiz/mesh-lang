---
phase: 58-struct-to-row-mapping
plan: 01
subsystem: database
tags: [postgresql, row-parsing, struct-mapping, llvm, runtime]

# Dependency graph
requires:
  - phase: 57-connection-pooling-transactions
    provides: "PgConn, Pool checkout/checkin, snow_pg_query returning List<Map<String,String>>"
  - phase: 54-postgresql
    provides: "PostgreSQL wire protocol, snow_pg_query, SnowResult pattern"
provides:
  - "Row parsing functions: snow_row_from_row_get, snow_row_parse_int/float/bool"
  - "query_as functions: snow_pg_query_as, snow_pool_query_as with from_row callback"
  - "Three-point LLVM registration for all 6 new runtime functions"
  - "map_builtin_name entries for Pg.query_as and Pool.query_as"
affects: [58-02-struct-codegen, query-as-usage, from-row-generation]

# Tech tracking
tech-stack:
  added: []
  patterns: ["from_row callback pattern via function pointer transmute", "PostgreSQL text-format normalization (Infinity->inf, t/f->true/false)"]

key-files:
  created: ["crates/snow-rt/src/db/row.rs"]
  modified: ["crates/snow-rt/src/db/mod.rs", "crates/snow-rt/src/db/pg.rs", "crates/snow-rt/src/db/pool.rs", "crates/snow-codegen/src/codegen/intrinsics.rs", "crates/snow-codegen/src/mir/lower.rs"]

key-decisions:
  - "FromRowFn type alias as unsafe extern C fn(*mut u8) -> *mut u8 for callback transmute"
  - "snow_pg_query_as returns List<SnowResult> (per-row results, not unwrapped values)"
  - "PostgreSQL Infinity/-Infinity pre-normalized to inf/-inf before f64::parse"
  - "Bool parsing accepts 8 variants (true/t/1/yes and false/f/0/no) case-insensitive"

patterns-established:
  - "Row field extraction: snow_row_from_row_get with string-keyed map lookup + descriptive error"
  - "Type parsing with SnowResult: parse success returns raw value, failure returns error string"
  - "query_as pattern: query + iterate rows + apply from_row callback + collect results"

# Metrics
duration: 5min
completed: 2026-02-12
---

# Phase 58 Plan 01: Runtime Row Parsing & Query-As Summary

**Row parsing functions (get/int/float/bool) with PostgreSQL text-format handling, plus query_as callback pattern in pg.rs and pool.rs, registered through full LLVM pipeline**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-12T20:31:14Z
- **Completed:** 2026-02-12T20:36:55Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Created row.rs with 4 extern "C" parsing functions handling all PostgreSQL text format edge cases
- Added snow_pg_query_as and snow_pool_query_as combining query + from_row function pointer callback
- Registered all 6 new functions through three-point pipeline (runtime, LLVM declarations, known_functions)
- 13 unit tests covering all parsing paths including infinity, boolean variants, and error cases

## Task Commits

Each task was committed atomically:

1. **Task 1: Create row.rs runtime functions and add query_as to pg.rs and pool.rs** - `2131ed6` (feat)
2. **Task 2: Three-point LLVM and known_functions registration for all 7 new functions** - `cf175de` (feat)

## Files Created/Modified
- `crates/snow-rt/src/db/row.rs` - Row parsing: snow_row_from_row_get, snow_row_parse_int/float/bool with 13 tests
- `crates/snow-rt/src/db/mod.rs` - Added `pub mod row` export
- `crates/snow-rt/src/db/pg.rs` - Added snow_pg_query_as with from_row callback iteration
- `crates/snow-rt/src/db/pool.rs` - Added snow_pool_query_as with checkout/query_as/checkin pattern
- `crates/snow-codegen/src/codegen/intrinsics.rs` - LLVM declarations for 6 new functions + test assertions
- `crates/snow-codegen/src/mir/lower.rs` - known_functions entries for 6 functions + map_builtin_name for query_as

## Decisions Made
- Used `FromRowFn = unsafe extern "C" fn(*mut u8) -> *mut u8` type alias for callback transmute (clean, reusable)
- snow_pg_query_as returns `List<SnowResult>` not `List<T>` -- each row mapping can independently succeed or fail
- Pre-normalize PostgreSQL "Infinity"/"-Infinity" to Rust "inf"/"-inf" before f64::parse (PG text format quirk)
- Bool parsing accepts 8 case-insensitive variants (true/t/1/yes + false/f/0/no) for PostgreSQL compatibility
- snow_string_length already had three-point registration -- no additions needed (confirmed in intrinsics.rs line 142 and lower.rs line 449)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All runtime row parsing functions exist and are callable from generated LLVM IR
- Plan 02 can now generate MIR `from_row` functions that call snow_row_from_row_get + snow_row_parse_int/float/bool
- query_as functions ready for Plan 02 to wire into Pg.query_as / Pool.query_as method dispatch

## Self-Check: PASSED

- FOUND: crates/snow-rt/src/db/row.rs
- FOUND: 58-01-SUMMARY.md
- FOUND: 2131ed6 (Task 1 commit)
- FOUND: cf175de (Task 2 commit)

---
*Phase: 58-struct-to-row-mapping*
*Completed: 2026-02-12*
