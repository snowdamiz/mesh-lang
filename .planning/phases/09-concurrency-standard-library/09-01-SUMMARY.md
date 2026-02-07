---
phase: 09-concurrency-standard-library
plan: 01
subsystem: parser
tags: [service, genserver, call, cast, parser, ast, lexer, keywords]

# Dependency graph
requires:
  - phase: 06-actor-runtime
    provides: Actor parsing infrastructure (ACTOR_DEF, parse_actor_def pattern)
  - phase: 07-supervision-fault-tolerance
    provides: Supervisor parsing infrastructure (SUPERVISOR_DEF, parse_supervisor_def pattern)
provides:
  - TokenKind::Service, TokenKind::Call, TokenKind::Cast keywords
  - SyntaxKind::SERVICE_KW, CALL_KW, CAST_KW, SERVICE_DEF, CALL_HANDLER, CAST_HANDLER
  - parse_service_def() with call/cast handler body parsing
  - ServiceDef, CallHandler, CastHandler AST wrappers
affects: [09-02 type checking, 09-03 MIR lowering, 09-04 codegen]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Service body parser: dispatches fn/call/cast keywords in loop"
    - "Call handler return type uses :: (not ->) to distinguish from fn return syntax"
    - "State parameter parsed as |ident| bar-delimited pattern"

key-files:
  created: []
  modified:
    - crates/snow-common/src/token.rs
    - crates/snow-parser/src/syntax_kind.rs
    - crates/snow-parser/src/parser/items.rs
    - crates/snow-parser/src/parser/mod.rs
    - crates/snow-parser/src/ast/item.rs
    - crates/snow-parser/tests/parser_tests.rs

key-decisions:
  - "Call handler return type uses :: syntax (not ->) to distinguish from fn return type"
  - "fn init inside service parsed as regular FN_DEF via parse_item_or_stmt dispatch"
  - "State parameter uses |ident| bar-delimited parsing (simpler than full closure params)"
  - "Service body returns tuple with () not {} since Snow tuples use parentheses"
  - "Keyword count increased from 42 to 45 (service, call, cast)"

patterns-established:
  - "Service handler pattern: keyword + name + params + optional return type + do + |state| + body + end"
  - "Contextual keyword reuse: call/cast registered globally but only meaningful inside service blocks"

# Metrics
duration: 7min
completed: 2026-02-07
---

# Phase 9 Plan 1: Service Definition Parsing Summary

**Service (GenServer) parsing with service/call/cast keywords, call and cast handler parsing, and typed AST wrappers for downstream type checking**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-07T07:50:38Z
- **Completed:** 2026-02-07T07:57:49Z
- **Tasks:** 2
- **Files modified:** 5 source + 1 test + 4 snapshots

## Accomplishments
- Three new keywords (service, call, cast) recognized by the Snow lexer
- Service definition syntax fully parseable: `service Name do fn init(...) end call Name() :: Type do |state| body end cast Name() do |state| body end end`
- AST wrappers (ServiceDef, CallHandler, CastHandler) provide typed access to all components
- 166 parser tests pass (160 existing + 6 new service tests, zero regressions)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add Service/Call/Cast tokens and SyntaxKinds** - `ca9d60c` (feat)
2. **Task 2: Add Service parser and AST wrappers** - `09e836a` (feat)

## Files Created/Modified
- `crates/snow-common/src/token.rs` - Added Service, Call, Cast keyword variants and keyword_from_str mappings
- `crates/snow-parser/src/syntax_kind.rs` - Added SERVICE_KW, CALL_KW, CAST_KW, SERVICE_DEF, CALL_HANDLER, CAST_HANDLER
- `crates/snow-parser/src/parser/items.rs` - Added parse_service_def, parse_service_body, parse_call_handler, parse_cast_handler
- `crates/snow-parser/src/parser/mod.rs` - Added SERVICE_KW dispatch in parse_item_or_stmt
- `crates/snow-parser/src/ast/item.rs` - Added ServiceDef, CallHandler, CastHandler AST wrappers with accessors
- `crates/snow-parser/tests/parser_tests.rs` - 6 new tests: simple, call, cast, full, AST accessors, items enum

## Decisions Made
- [09-01]: Call handler return type uses :: syntax (ColonColon + Type), distinct from fn's -> syntax
- [09-01]: fn init inside service body dispatched through parse_item_or_stmt as regular FN_DEF
- [09-01]: State parameter uses simple |ident| bar-delimited parsing (single identifier, not full closure params)
- [09-01]: Service body tuples use () not {} (Snow tuple syntax is parenthesized)
- [09-01]: Keyword count increased from 42 to 45 (service, call, cast); TokenKind variants from 90 to 93

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed fn return type syntax in test examples**
- **Found during:** Task 2 (parser testing)
- **Issue:** Plan examples used `fn init(start_val :: Int) :: Int do` but Snow fn definitions use `->` for return types, not `::`
- **Fix:** Changed test source to use `-> Int` for fn return types while keeping `::` for call handler return types
- **Files modified:** crates/snow-parser/tests/parser_tests.rs
- **Verification:** All tests pass with correct syntax
- **Committed in:** 09e836a (Task 2 commit)

**2. [Rule 1 - Bug] Fixed tuple syntax in test examples**
- **Found during:** Task 2 (parser testing)
- **Issue:** Plan examples used `{state, state}` but Snow tuples use `()` not `{}`; `{` triggers struct literal parsing
- **Fix:** Changed test source to use `(state, state)` and `(state + amount, state + amount)`
- **Files modified:** crates/snow-parser/tests/parser_tests.rs
- **Verification:** Tuples parse correctly as TUPLE_EXPR nodes
- **Committed in:** 09e836a (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 bugs in plan examples)
**Impact on plan:** Both fixes correct plan example syntax to match existing Snow language rules. No scope creep.

## Issues Encountered
None beyond the plan example syntax corrections documented above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Service parsing infrastructure complete, ready for type checking (09-02)
- ServiceDef/CallHandler/CastHandler AST wrappers ready for downstream consumption
- No blockers for Phase 9 Plan 2

## Self-Check: PASSED

---
*Phase: 09-concurrency-standard-library*
*Completed: 2026-02-07*
