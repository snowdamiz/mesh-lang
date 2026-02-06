# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-05)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** Phase 2 complete. Parser & AST fully implemented with typed wrappers, 191 workspace tests. Ready for Phase 3 (Type System).

## Current Position

Phase: 2 of 10 (Parser & AST)
Plan: 5 of 5 in current phase
Status: Phase complete
Last activity: 2026-02-06 -- Completed 02-05-PLAN.md

Progress: [████████░░] 20% (8 plans of ~41 estimated total)

## Performance Metrics

**Velocity:**
- Total plans completed: 8
- Average duration: 4min
- Total execution time: 35min

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-project-foundation-lexer | 3/3 | 12min | 4min |
| 02-parser-ast | 5/5 | 23min | 5min |

**Recent Trend:**
- Last 5 plans: 02-01 (7min), 02-02 (3min), 02-03 (4min), 02-04 (4min), 02-05 (5min)
- Trend: stable at 4-5min average

*Updated after each plan completion*

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- [Roadmap]: Compiler pipeline phases (1-5) must complete before actor runtime (Phase 6) -- sequential code first, actors later
- [Roadmap]: Actor runtime (libsnowrt) developed as standalone Rust library tested independently before compiler integration
- [Roadmap]: Type system and pattern matching are separate phases due to individual complexity and risk
- [01-01]: 39 keywords (not 37 as plan header stated) -- when, where, with bring the actual count to 39
- [01-01]: SelfKw variant for self keyword (Rust keyword conflict avoidance)
- [01-01]: Match-based keyword dispatch over HashMap (compiler optimizes string matching)
- [01-02]: StringMode enum state machine (None/Single/Triple) with pending_token queue for string tokenization
- [01-02]: Comments skip optional leading space after delimiter for cleaner content spans
- [01-03]: State stack (Vec<LexerState>) replaces StringMode for nested interpolation contexts
- [01-03]: InString stays on stack when InInterpolation pushed; pop returns to string scanning
- [01-03]: Pending token queue (Vec<Token>) for multi-token emissions
- [01-03]: All newlines emit as Newline tokens; parser decides significance
- [01-03]: Bare pipe | produces Error token; struct update syntax needs parser-level handling
- [02-01]: SyntaxKind uses SCREAMING_SNAKE_CASE with #[allow(non_camel_case_types)] (rowan convention)
- [02-01]: Comments always trivia in parser (skipped by lookahead, preserved in CST)
- [02-01]: Forward parent technique for open_before() wrapping (matklad pattern)
- [02-01]: Parser internals pub(crate); public API deferred to 02-05
- [02-01]: Lexer does not emit whitespace tokens; WHITESPACE SyntaxKind exists for future use
- [02-02]: Grouped expressions and single-element tuples both use TUPLE_EXPR (parser does not distinguish)
- [02-02]: PIPE_EXPR separate from BINARY_EXPR for pipe operator identification
- [02-02]: parse_expr() and debug_tree() added as public API for testing
- [02-03]: Trailing closures only attach after explicit arg list () -- bare `do` on identifier does not create CALL_EXPR
- [02-03]: Closures always use `fn (params) -> body end` with explicit end keyword
- [02-04]: fn/def followed by IDENT = named fn def; fn followed by L_PAREN = closure expression
- [02-04]: "from" is contextual identifier, not keyword -- recognized via text check
- [02-04]: Glob imports (from M import *) rejected at parse time
- [02-04]: Patterns replace expressions in match arms (LITERAL_PAT, IDENT_PAT, TUPLE_PAT, WILDCARD_PAT)
- [02-04]: Let bindings support tuple destructuring via pattern parsing
- [02-04]: Shared parse_type() for all type annotation positions
- [02-05]: pub(crate) syntax field in ast_node! macro for cross-module construction
- [02-05]: Lossless round-trip tests strip spaces (lexer omits whitespace by design)
- [02-05]: Expr/Item/Pattern enums provide polymorphic cast()-based access to typed AST nodes

### Pending Todos

None.

### Blockers/Concerns

- Phase 3 (Type System) is highest intellectual risk -- HM inference has subtle implementation pitfalls
- Phase 6 (Actor Runtime) is highest engineering risk -- preemptive scheduling, per-actor GC, work-stealing
- Typed actor messaging (TYPE-07) is a research-level problem -- design on paper during early phases

## Session Continuity

Last session: 2026-02-06
Stopped at: Completed 02-05-PLAN.md (Phase 2 complete)
Resume file: None
