---
phase: 67-remote-spawn-llvm-integration
plan: 03
subsystem: codegen
tags: [llvm, codegen, mir, typechecker, parser, node, process, distribution, remote-spawn]

# Dependency graph
requires:
  - phase: 67-remote-spawn-llvm-integration
    plan: 01
    provides: "LLVM intrinsic declarations for snow_node_* and snow_process_* functions"
provides:
  - "MIR lowering of Node.* and Process.* qualified access to snow_node_*/snow_process_* runtime calls"
  - "Codegen helpers for SnowString unpacking to (data_ptr, len) pairs"
  - "Codegen for Node.spawn/spawn_link with function name as string constant and arg packing"
  - "Type checker support for Node and Process stdlib modules"
  - "Parser support for keywords (self, monitor, spawn, link) as field names after dot"
affects: [67-04, end-to-end-node-tests, future-stdlib-modules]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "codegen_unpack_string pattern for extracting raw bytes from SnowString pointers"
    - "Parser keyword-as-field-name pattern for stdlib modules using reserved words"
    - "Fresh type variable return for variadic stdlib functions (Node.spawn)"

key-files:
  created: []
  modified:
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-parser/src/parser/expressions.rs
    - crates/snow-parser/src/ast/expr.rs

key-decisions:
  - "Handle Node.spawn variadic typing by returning fresh type variable instead of fixed function type"
  - "Unpack SnowString to (data_ptr, len) using GEP arithmetic rather than runtime helper"
  - "Use GC heap allocation for remote spawn argument packing (same pattern as local spawn)"
  - "Extend parser to accept keywords as field names rather than renaming Node/Process API methods"

patterns-established:
  - "codegen_unpack_string: reusable pattern for any future runtime function needing raw (ptr, len) from SnowString"
  - "Keyword-as-field parser extension: future stdlib modules with keyword-named methods can use same pattern"

# Metrics
duration: 15min
completed: 2025-02-12
---

# Phase 67 Plan 03: Node/Process LLVM Codegen Pipeline Summary

**MIR lowering, LLVM codegen, type checker, and parser support for all Node.* and Process.* distribution APIs**

## Performance

- **Duration:** ~15 min
- **Started:** 2026-02-12T22:09:00Z
- **Completed:** 2026-02-12T22:33:00Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- All 9 Node/Process APIs compile from Snow source to correct LLVM IR: Node.start, Node.connect, Node.self, Node.list, Node.monitor, Node.spawn, Node.spawn_link, Process.monitor, Process.demonitor
- Node.spawn converts function reference to string constant for wire protocol transmission
- SnowString unpacking helper extracts raw (data_ptr, len) pairs for runtime functions expecting C-style strings
- Parser extended to accept keywords (self, monitor, spawn, link) in field access position

## Task Commits

Each task was committed atomically:

1. **Task 1: Add Node and Process to STDLIB_MODULES and map_builtin_name** - `204951a` (feat)
2. **Task 2: Add Node.spawn codegen with function name string conversion** - `b543de6` (feat)

## Files Created/Modified
- `crates/snow-codegen/src/mir/lower.rs` - Added "Node" and "Process" to STDLIB_MODULES, all map_builtin_name entries for node_*/process_* functions
- `crates/snow-codegen/src/codegen/expr.rs` - Added codegen_unpack_string, codegen_node_start, codegen_node_string_call, codegen_node_spawn helpers for special Node.* codegen
- `crates/snow-typeck/src/infer.rs` - Added Node and Process module type definitions to stdlib_modules(), added to STDLIB_MODULE_NAMES, special variadic handling for Node.spawn/spawn_link
- `crates/snow-parser/src/parser/expressions.rs` - Extended field access parsing to accept SELF_KW, MONITOR_KW, SPAWN_KW, LINK_KW as field names
- `crates/snow-parser/src/ast/expr.rs` - Extended FieldAccess.field() to return keyword tokens in addition to IDENT

## Decisions Made
- Used fresh type variable (ctx.fresh_var()) for Node.spawn/spawn_link return type to bypass arity checking for variadic calls, rather than adding a new variadic type mechanism
- Unpack SnowString layout (u64 len at offset 0, data bytes at offset 8) using GEP arithmetic rather than calling a runtime helper
- Use GC heap (snow_gc_alloc_actor) for remote spawn argument packing to match local spawn convention
- Extended parser to accept keywords as field names (self, monitor, spawn, link) rather than renaming the API methods to avoid keyword conflicts

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added Node/Process type definitions to type checker**
- **Found during:** Task 2 (codegen implementation)
- **Issue:** Snow programs using Node.* APIs failed at type checking with "undefined variable: Node" because the type checker had no knowledge of the Node or Process modules
- **Fix:** Added Node and Process modules to stdlib_modules() in snow-typeck/src/infer.rs with full type signatures, added to STDLIB_MODULE_NAMES constant, added special Node.spawn variadic type handling
- **Files modified:** crates/snow-typeck/src/infer.rs
- **Verification:** Snow programs with all 9 Node/Process APIs type-check and compile
- **Committed in:** b543de6 (Task 2 commit)

**2. [Rule 3 - Blocking] Extended parser to accept keywords as field names**
- **Found during:** Task 2 (codegen implementation)
- **Issue:** Node.self, Node.monitor, Node.spawn failed parsing because `self`, `monitor`, and `spawn` are keywords in the lexer and the field access parser only accepted IDENT tokens
- **Fix:** Modified field access parsing in expressions.rs to accept SELF_KW, MONITOR_KW, SPAWN_KW, and LINK_KW after dot, and updated FieldAccess.field() in ast/expr.rs to also match these keyword tokens
- **Files modified:** crates/snow-parser/src/parser/expressions.rs, crates/snow-parser/src/ast/expr.rs
- **Verification:** Node.self(), Node.monitor(), Node.spawn(), and Node.spawn_link() all parse and compile correctly
- **Committed in:** b543de6 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 blocking issues)
**Impact on plan:** Both auto-fixes were essential for the plan to work end-to-end. The plan focused on codegen but the type checker and parser also needed updates to support the new module APIs. No scope creep.

## Issues Encountered
- SnowString layout (len at offset 0, data at offset 8) required careful GEP arithmetic rather than struct field access, since SnowString is a variable-length type (header + inline data bytes)
- Node.spawn_link identity: since `spawn_link` is a single identifier (not `spawn` + `_link`), the lexer treats it as IDENT, not as two keywords. Only `spawn` needed the keyword-as-field parser extension.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All Node.* and Process.* APIs compile from Snow source to correct LLVM IR
- Ready for Plan 04 (if applicable) or end-to-end integration testing
- Wire protocol and runtime (Plan 02) need to be completed for runtime execution
- Full end-to-end testing requires two Snow nodes running

## Self-Check: PASSED

All 5 modified files verified present on disk. Both task commits (204951a, b543de6) verified in git log.

---
*Phase: 67-remote-spawn-llvm-integration, Plan: 03*
*Completed: 2026-02-12*
