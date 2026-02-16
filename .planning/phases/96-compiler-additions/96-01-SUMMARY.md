---
phase: 96-compiler-additions
plan: 01
subsystem: compiler
tags: [atom, lexer, parser, typeck, mir, codegen, llvm]

# Dependency graph
requires: []
provides:
  - "Atom literal syntax (:name, :email, :asc) across full compiler pipeline"
  - "Distinct Atom type in type checker (separate from String)"
  - "ATOM_LITERAL token kind and ATOM_EXPR AST node"
affects: [96-02, 96-03, 97-schema-metadata, 98-query-builder]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Atom literals compile to string constants at LLVM level (MirType::String)"
    - "Type distinction exists only in type checker (Atom vs String)"

key-files:
  created: []
  modified:
    - "crates/mesh-common/src/token.rs"
    - "crates/mesh-lexer/src/lib.rs"
    - "crates/mesh-parser/src/syntax_kind.rs"
    - "crates/mesh-parser/src/parser/expressions.rs"
    - "crates/mesh-parser/src/ast/expr.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/meshc/tests/e2e.rs"

key-decisions:
  - "Atoms lower to MirExpr::StringLit (string constants at LLVM level) -- no MirType::Atom needed"
  - "Atom type check uses Ty::Con(TyCon::new('Atom')) for distinct type identity"
  - "ATOM_EXPR composite node wraps ATOM_LITERAL token (consistent with LITERAL wrapping pattern)"
  - "Atom lexing requires lowercase letter or underscore after colon (no uppercase to avoid :: ambiguity)"

patterns-established:
  - "New literal type pattern: TokenKind variant -> SyntaxKind leaf + composite -> AST node -> type check -> MIR lower"

# Metrics
duration: 10min
completed: 2026-02-16
---

# Phase 96 Plan 01: Atom Literals Summary

**Atom literal syntax (:name, :email, :asc) spanning lexer through LLVM codegen with distinct Atom type in type checker**

## Performance

- **Duration:** 10 min
- **Started:** 2026-02-16T08:26:14Z
- **Completed:** 2026-02-16T08:37:07Z
- **Tasks:** 2
- **Files modified:** 8

## Accomplishments
- Full atom literal syntax from lexer to LLVM: `:name` lexes as single Atom token, parses to ATOM_EXPR, type-checks as Atom, compiles to string constant
- Atom type is distinct from String in the type checker (assigning atom to String-annotated variable produces type error)
- ColonColon (::) type annotations unaffected -- no ambiguity with atom syntax
- All 153 e2e tests pass including 2 new atom-specific tests

## Task Commits

Each task was committed atomically:

1. **Task 1: Add Atom token to lexer and parser infrastructure** - `e9889528` (feat)
2. **Task 2: Add Atom type checking, MIR lowering, LLVM codegen, and e2e test** - `03cfe577` (feat)

## Files Created/Modified
- `crates/mesh-common/src/token.rs` - Added TokenKind::Atom variant (literals count 7->8, total 96->97)
- `crates/mesh-lexer/src/lib.rs` - Extended lex_colon to detect atom literals (:lowercase/underscore)
- `crates/mesh-parser/src/syntax_kind.rs` - Added ATOM_LITERAL token kind and ATOM_EXPR composite node
- `crates/mesh-parser/src/parser/expressions.rs` - Added atom literal case in lhs() expression parser
- `crates/mesh-parser/src/ast/expr.rs` - Added AtomLiteral AST node with atom_text() accessor
- `crates/mesh-typeck/src/infer.rs` - Added Atom type inference and guard expression support
- `crates/mesh-codegen/src/mir/lower.rs` - Added atom-to-StringLit MIR lowering
- `crates/meshc/tests/e2e.rs` - Added e2e_atom_literals and e2e_atom_type_distinct tests

## Decisions Made
- Atoms lower to MirExpr::StringLit at runtime (no new MirType::Atom needed) -- the Atom vs String distinction is purely a type checker concern for compile-time validation
- Used ATOM_EXPR composite node to wrap ATOM_LITERAL leaf token, following the same pattern as LITERAL composite wrapping INT_LITERAL/FLOAT_LITERAL tokens
- Atom lexing requires first character after `:` to be lowercase ASCII or underscore -- uppercase is rejected to prevent ambiguity with `::Module` (ColonColon) patterns
- Atoms allowed in guard expressions alongside literals and name references

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed e2e test using IO.puts instead of println**
- **Found during:** Task 2 (e2e test creation)
- **Issue:** Plan suggested `IO.puts()` but the Mesh stdlib uses `println()` for console output
- **Fix:** Changed all `IO.puts()` calls to `println()` in the e2e test
- **Files modified:** crates/meshc/tests/e2e.rs
- **Verification:** e2e_atom_literals test passes
- **Committed in:** 03cfe577 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Trivial API name fix. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Atom literals fully operational across the compiler pipeline
- Ready for Phase 96 Plan 02 (next compiler addition)
- Atom type available for ORM query builder in Phase 98 (e.g., `Query.where(:name, "Alice")`)

---
*Phase: 96-compiler-additions*
*Completed: 2026-02-16*
