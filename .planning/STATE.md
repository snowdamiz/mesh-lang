# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-10)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v2.0 Database & Serialization -- Phase 49 (JSON Serde -- Structs)

## Current Position

Phase: 49 of 54 (JSON Serde -- Structs)
Plan: 1 of 3 in current phase
Status: Executing
Last activity: 2026-02-10 -- Completed 49-01 (JSON runtime foundation)

Progress: [###░░░░░░░] 33%

## Performance Metrics

**All-time Totals:**
- Plans completed: 142
- Phases completed: 48
- Milestones shipped: 10 (v1.0-v1.9)
- Lines of Rust: 76,100
- Timeline: 6 days (2026-02-05 -> 2026-02-10)

## Accumulated Context

### Decisions

- 49-01: Separate JSON_INT (tag 2) and JSON_FLOAT (tag 6) for round-trip fidelity instead of single JSON_NUMBER
- 49-01: as_int coerces Float to Int (truncation), as_float promotes Int to Float -- matching Snow's numeric widening
- 49-01: Collection helpers use extern C function pointer callbacks for per-element encode/decode

### Research Notes

- JSON serde follows existing deriving(Eq/Hash/Debug) MIR lowering pattern
- SnowJson NUMBER tag needs Int/Float split before struct-aware decode
- SQLite via libsqlite3-sys bundled (C FFI), opaque u64 handles for GC safety
- PostgreSQL pure wire protocol (Parse/Bind/Execute/Sync), SCRAM-SHA-256 + MD5 auth
- HTTP path params: extend router.rs segment matching for :param
- HTTP middleware: function pipeline using existing closure calling convention
- See .planning/research/SUMMARY.md for full analysis

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-10
Stopped at: Completed 49-01-PLAN.md
Resume file: None
Next action: Execute 49-02-PLAN.md (MIR lowering for to_json/from_json)
