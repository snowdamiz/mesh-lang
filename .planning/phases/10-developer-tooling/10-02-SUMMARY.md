---
phase: 10-developer-tooling
plan: 02
subsystem: tooling
tags: [formatter, wadler-lindig, cst, code-formatting, rowan]

# Dependency graph
requires:
  - phase: 02-parser-ast
    provides: "CST with rowan SyntaxNode/SyntaxKind for walking"
provides:
  - "snow-fmt crate with FormatIR, Wadler-Lindig printer, CST walker"
  - "format_source() public API for formatting Snow source code"
  - "FormatConfig with indent_size and max_width settings"
affects: [10-03-PLAN, 10-05-PLAN, 10-09-PLAN]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Wadler-Lindig document IR for code formatting", "CST-preserving formatter (comments/trivia survive formatting)"]

key-files:
  created:
    - "crates/snow-fmt/Cargo.toml"
    - "crates/snow-fmt/src/lib.rs"
    - "crates/snow-fmt/src/ir.rs"
    - "crates/snow-fmt/src/printer.rs"
    - "crates/snow-fmt/src/walker.rs"
  modified:
    - "Cargo.toml"
    - "Cargo.lock"

key-decisions:
  - "Wadler-Lindig document IR with 8 variants (Text, Space, Hardline, Indent, Group, IfBreak, Concat, Empty)"
  - "sp() literal space helper vs ir::space() break-sensitive space -- root context is always break mode"
  - "TYPE_ANNOTATION nodes get explicit space before them (Snow uses :: for type annotations, not path separator)"
  - "Stack-based printer algorithm with (indent, mode, ir_node) triples and measure_flat() for Group decisions"

patterns-established:
  - "FormatIR enum as intermediate representation between CST walk and string output"
  - "walk_node() dispatch on SyntaxKind for formatting decisions"
  - "walk_tokens_inline() generic handler with needs_space_before/needs_space_before_node for smart spacing"
  - "Idempotency testing pattern: format(format(src)) == format(src)"

# Metrics
duration: 12min
completed: 2026-02-07
---

# Phase 10 Plan 02: snow-fmt Code Formatter Summary

**Wadler-Lindig document IR formatter with CST walker handling 40+ Snow syntax constructs, idempotent output, and comment preservation**

## Performance

- **Duration:** 12 min
- **Started:** 2026-02-07
- **Completed:** 2026-02-07
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Wadler-Lindig FormatIR with 8 variants and stack-based printer respecting line width
- Comprehensive CST walker dispatching on 40+ SyntaxKind variants
- Idempotent formatting verified across 6 different Snow constructs
- Comments preserved in formatted output
- 32 total tests (11 printer unit tests + 20 walker integration tests + 1 doctest)

## Task Commits

Each task was committed atomically:

1. **Task 1: Create snow-fmt crate with FormatIR and printer** - `1bdcb18` (feat)
2. **Task 2: CST walker that converts Snow syntax tree to FormatIR** - `de50807` (feat)

## Files Created/Modified
- `crates/snow-fmt/Cargo.toml` - Crate manifest with snow-parser, snow-common, rowan deps
- `crates/snow-fmt/src/lib.rs` - Public API: format_source() and FormatConfig re-export
- `crates/snow-fmt/src/ir.rs` - FormatIR enum with 8 variants and helper constructors
- `crates/snow-fmt/src/printer.rs` - Wadler-Lindig stack-based printer with FormatConfig
- `crates/snow-fmt/src/walker.rs` - CST-to-FormatIR walker with 40+ SyntaxKind handlers
- `Cargo.toml` - Added snow-fmt to workspace members
- `Cargo.lock` - Updated with new crate

## Decisions Made
- **Wadler-Lindig IR:** 8-variant enum captures all formatting intent without committing to layout until print time
- **sp() vs ir::space():** Root printer context is always break mode, so `ir::space()` outside a Group becomes a newline. Introduced `sp()` (literal `ir::text(" ")`) for unconditional spaces, reserving `ir::space()` for inside Group nodes where break behavior is desired
- **TYPE_ANNOTATION spacing:** Removed TYPE_ANNOTATION from `needs_space_before_node()` exclusion list so `x :: Int` gets proper spacing (Snow uses `::` for type annotations, not path separators like Rust)
- **Stack-based printer:** Uses `(indent, mode, ir_node)` triples with `measure_flat()` to determine if a Group fits on the remaining line width

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed Space vs literal space confusion in walker**
- **Found during:** Task 2 (CST walker implementation)
- **Issue:** Using `ir::space()` throughout the walker produced newlines instead of spaces because root context is always break mode
- **Fix:** Introduced `fn sp() -> FormatIR { ir::text(" ") }` for unconditional spaces; reserved `ir::space()` for inside Group nodes only
- **Files modified:** crates/snow-fmt/src/walker.rs
- **Verification:** All 20 walker tests pass with correct spacing
- **Committed in:** de50807 (Task 2 commit)

**2. [Rule 1 - Bug] Fixed TYPE_ANNOTATION spacing in inline token walker**
- **Found during:** Task 2 (CST walker implementation)
- **Issue:** `needs_space_before_node()` excluded TYPE_ANNOTATION, causing `x:: Int` instead of `x :: Int` in PARAM and STRUCT_FIELD contexts
- **Fix:** Removed TYPE_ANNOTATION from exclusion list; added explicit space before TYPE_ANNOTATION in walk_let_binding
- **Files modified:** crates/snow-fmt/src/walker.rs
- **Verification:** let_with_type_annotation, struct_definition, and typed_fn_def tests all pass
- **Committed in:** de50807 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 bugs)
**Impact on plan:** Both fixes essential for correct formatting output. No scope creep.

## Issues Encountered
- The Wadler-Lindig `Space` IR node has dual behavior (space in flat mode, newline in break mode) which is non-obvious. Since the root context is always break mode, any `Space` outside a `Group` becomes a newline. This fundamental insight drove the `sp()` helper pattern.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- snow-fmt crate ready for CLI integration (`snowc fmt` command in plan 10-03 or 10-05)
- format_source() API available for LSP formatting requests
- FormatConfig supports customization of indent_size and max_width

## Self-Check: PASSED

---
*Phase: 10-developer-tooling*
*Completed: 2026-02-07*
