---
phase: 96-compiler-additions
plan: 02
subsystem: compiler
tags: [parser, keyword-args, multi-line-pipe, pratt-parser, desugaring]

# Dependency graph
requires:
  - "96-01: Atom literals (ATOM_LITERAL token kind, ATOM_EXPR node)"
provides:
  - "Keyword argument syntax (name: value) desugaring to Map literals in function calls"
  - "Multi-line pipe chain support (|> at line start continues previous expression)"
affects: [96-03, 96-04, 97-schema-metadata, 98-query-builder]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Keyword args desugar to MAP_LITERAL at parser level (no new AST node kinds)"
    - "is_keyword_entry() on MapEntry distinguishes kwarg entries from regular map entries by COLON vs FAT_ARROW"
    - "Multi-line pipe uses peek_past_newlines() + skip_newlines_for_continuation() in Pratt loop"

key-files:
  created: []
  modified:
    - "crates/mesh-parser/src/parser/expressions.rs"
    - "crates/mesh-parser/src/parser/mod.rs"
    - "crates/mesh-parser/src/ast/expr.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/meshc/tests/e2e.rs"
    - "crates/mesh-parser/tests/snapshots/parser_tests__full_chained_pipes_and_field_access.snap"

key-decisions:
  - "Keyword args reuse existing MAP_LITERAL/MAP_ENTRY nodes (no new SyntaxKind variants needed)"
  - "Keyword entry keys are NAME_REF nodes; typeck returns String type, MIR lowerer converts to StringLit"
  - "is_keyword_entry() detects COLON child (not FAT_ARROW) to distinguish keyword from regular map entries"
  - "Multi-line pipe continuation uses peek-ahead in Pratt loop (not lexer-level newline suppression)"
  - "Pipe binding power check (3 >= min_bp) ensures continuation only at appropriate precedence levels"

patterns-established:
  - "Parser-level desugaring: syntactic sugar -> existing CST nodes with AST-level detection helpers"
  - "Newline continuation pattern: peek past newlines for specific operator, skip if found"

# Metrics
duration: 11min
completed: 2026-02-16
---

# Phase 96 Plan 02: Keyword Arguments and Multi-line Pipes Summary

**Keyword argument syntax (name: value) desugaring to Map literals plus multi-line pipe chain continuation via parser-level newline lookahead**

## Performance

- **Duration:** 11 min
- **Started:** 2026-02-16T08:39:56Z
- **Completed:** 2026-02-16T08:51:31Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Keyword arguments in function calls: `f(name: "Alice", age: 30)` desugars to `f(%{"name" => "Alice", "age" => 30})` at the parser level
- Mixed positional + keyword args: `f(x, name: "Alice")` correctly places kwargs as the final Map argument
- Multi-line pipe chains: `expr\n  |> func` parses as a single pipe expression instead of two separate statements
- Both features work together: multi-line pipe chains can include keyword argument calls
- All 158 e2e tests pass (5 new + 153 existing), all 231 parser tests pass

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement keyword argument desugaring in parser** - `461964ed` (feat)
2. **Task 2: Implement multi-line pipe chain continuation** - `fc2032b8` (feat)

## Files Created/Modified
- `crates/mesh-parser/src/parser/expressions.rs` - Added keyword arg detection in parse_arg_list, parse_keyword_args_as_map/parse_keyword_entry helpers, multi-line pipe continuation check in Pratt loop
- `crates/mesh-parser/src/parser/mod.rs` - Added peek_past_newlines() and skip_newlines_for_continuation() methods to Parser
- `crates/mesh-parser/src/ast/expr.rs` - Added is_keyword_entry() and keyword_key_text() methods to MapEntry
- `crates/mesh-typeck/src/infer.rs` - Handle keyword entries in infer_map_literal (return String type for key)
- `crates/mesh-codegen/src/mir/lower.rs` - Handle keyword entries in lower_map_literal (convert key to StringLit)
- `crates/meshc/tests/e2e.rs` - Added 5 new tests: e2e_keyword_arguments, e2e_keyword_args_mixed, e2e_multiline_pipe, e2e_multiline_pipe_complex, e2e_multiline_pipe_with_keyword_args
- `crates/mesh-parser/tests/snapshots/parser_tests__full_chained_pipes_and_field_access.snap` - Updated: now parses correctly as nested PIPE_EXPR (was error before)

## Decisions Made
- Keyword arguments reuse existing MAP_LITERAL and MAP_ENTRY CST nodes rather than introducing new SyntaxKind variants. This keeps the grammar simple and leverages the existing map literal infrastructure in typeck and codegen.
- Keyword entry detection uses COLON vs FAT_ARROW child token presence (is_keyword_entry method). This avoids ambiguity since regular map entries always have FAT_ARROW (=>) and keyword entries always have COLON (:).
- Multi-line pipe continuation implemented at the Pratt parser level (not the lexer). This is more localized than lexer-level newline suppression and only activates when |> specifically follows newlines.
- The peek_past_newlines helper is read-only (does not consume tokens). Only skip_newlines_for_continuation actually advances the parser position, and only after confirming PIPE follows.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Updated parser snapshot for multi-line pipe test**
- **Found during:** Task 2 (multi-line pipe implementation)
- **Issue:** Existing parser snapshot `full_chained_pipes_and_field_access` expected parse errors for multi-line pipes (the old behavior). With the fix, the test now correctly parses the multi-line pipe chain.
- **Fix:** Accepted the updated snapshot which shows correct nested PIPE_EXPR structure
- **Files modified:** crates/mesh-parser/tests/snapshots/parser_tests__full_chained_pipes_and_field_access.snap
- **Verification:** All 231 parser tests pass
- **Committed in:** fc2032b8 (Task 2 commit)

**2. [Rule 1 - Bug] Used println instead of IO.puts in e2e tests**
- **Found during:** Task 1 (e2e test creation)
- **Issue:** Plan examples used IO.puts() but Mesh stdlib uses println() for console output
- **Fix:** Used println() in all e2e tests
- **Files modified:** crates/meshc/tests/e2e.rs
- **Verification:** All e2e tests pass
- **Committed in:** 461964ed (Task 1 commit)

**3. [Rule 2 - Missing Critical] Added typeck handling for keyword entries**
- **Found during:** Task 1 (keyword argument implementation)
- **Issue:** Plan only mentioned parser changes, but the type checker would try to look up keyword entry keys as variables (NAME_REF), causing variable-not-found errors
- **Fix:** Added keyword entry detection in infer_map_literal that returns Ty::string() for keyword keys instead of inferring via variable lookup
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Verification:** Keyword arg e2e tests compile and run correctly
- **Committed in:** 461964ed (Task 1 commit)

**4. [Rule 2 - Missing Critical] Added MIR lowering for keyword entries**
- **Found during:** Task 1 (keyword argument implementation)
- **Issue:** Plan only mentioned parser changes, but the MIR lowerer would try to lower keyword keys as variable references, producing incorrect code
- **Fix:** Added keyword entry detection in lower_map_literal that converts key NAME_REF text to MirExpr::StringLit
- **Files modified:** crates/mesh-codegen/src/mir/lower.rs
- **Verification:** Keyword arg e2e tests produce correct output
- **Committed in:** 461964ed (Task 1 commit)

---

**Total deviations:** 4 auto-fixed (1 bug, 1 bug, 2 missing critical)
**Impact on plan:** Deviations 3 and 4 were essential for end-to-end correctness. The plan focused on parser changes but the typeck and codegen layers also needed modifications to handle the new MAP_ENTRY structure. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Keyword arguments fully operational: `f(name: "Alice")` compiles and runs correctly
- Multi-line pipe chains fully operational: `|>` at line start continues previous expression
- Both features work together in combination
- Ready for Phase 96 Plan 03 (next compiler addition)
- ORM DSL syntax foundation in place: `where(name: "Alice")` and multi-line query chains

---
*Phase: 96-compiler-additions*
*Completed: 2026-02-16*
