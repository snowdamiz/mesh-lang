# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-05)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** Phase 4 in progress -- Pattern Matching & Algebraic Data Types. Plan 01 complete (lexer/parser/AST for sum types and extended patterns).

## Current Position

Phase: 4 of 10 (Pattern Matching & ADTs)
Plan: 1 of 5 in current phase
Status: In progress
Last activity: 2026-02-06 -- Completed 04-01-PLAN.md

Progress: [██████████████░░░░░░] 34% (14 plans of ~41 estimated total)

## Performance Metrics

**Velocity:**
- Total plans completed: 14
- Average duration: 9min
- Total execution time: 126min

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-project-foundation-lexer | 3/3 | 12min | 4min |
| 02-parser-ast | 5/5 | 23min | 5min |
| 03-type-system | 5/5 | 83min | 17min |
| 04-pattern-matching-adts | 1/5 | 8min | 8min |

**Recent Trend:**
- Last 5 plans: 03-03 (45min), 03-04 (15min), 03-05 (6min), 04-01 (8min)
- Trend: Phase 4 starting with parser work. Moderate complexity.

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
- [01-03]: Bare pipe | produces Bar token (upgraded from Error in 04-01)
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
- [03-01]: Angle brackets <T> for generics (migrated from square brackets [T])
- [03-01]: Option/Result sugar emits raw tokens (QUESTION/BANG) inside TYPE_ANNOTATION
- [03-01]: self keyword accepted as parameter name (needed for interface methods)
- [03-01]: ena unify_var_var/unify_var_value API for fallible unification
- [03-01]: Type variables use Option<Ty> as UnifyKey::Value with level-based generalization side-table
- [03-01]: Builtin operators hardcoded as monomorphic -- trait dispatch deferred to 03-04
- [03-01]: GENERIC_PARAM_LIST (definition) vs GENERIC_ARG_LIST (usage) distinction in CST
- [03-02]: result_type field on TypeckResult for tracking last expression's inferred type
- [03-02]: Single-element TupleExpr treated as grouping parens (returns element type, not Tuple)
- [03-02]: Block tail_expr deduplication via range comparison to avoid double-inference
- [03-02]: Named functions pre-bind name to fresh var for recursion support
- [03-03]: Token-based type annotation parsing (collect_annotation_tokens + parse_type_tokens) for sugar syntax
- [03-03]: enter_level/leave_level for proper polymorphic constructor generalization (Option/Result)
- [03-03]: Union-find resolve() normalizes unbound vars to root key for correct identity
- [03-03]: Struct literals parsed as postfix expressions (NAME_REF followed by L_BRACE)
- [03-03]: Type aliases stored as resolved Ty values parsed from CST tokens after =
- [03-04]: Trait methods callable as regular functions with self-type dispatch (to_string(42) not 42.to_string())
- [03-04]: Where-clause constraints stored per-function in FnConstraints map, checked at call site
- [03-04]: Compiler-known traits registered in builtins alongside existing operator schemes for backward compat
- [03-04]: Snow uses :: for param type annotations, : for trait bounds in where clauses
- [03-04]: type_to_key() string-based impl lookup for exact type matching
- [03-05]: ariadne 0.6 with colorless Config for deterministic snapshot test output
- [03-05]: render_diagnostic() returns String for test capture (not printed directly)
- [03-05]: Error codes E0001-E0009 for each TypeError variant
- [03-05]: param_type_param_names maps fn params to type param names for call-site where-clause resolution
- [04-01]: Bare `|` lexes as TokenKind::Bar / SyntaxKind::BAR (upgraded from Error)
- [04-01]: Constructor patterns use heuristic: uppercase IDENT + L_PAREN = constructor, else IDENT_PAT; nullary constructors resolve in type checker
- [04-01]: `as` is contextual keyword in patterns (IDENT text check, not reserved keyword)
- [04-01]: Sum type vs type alias dispatch: scan past name/generics to find DO_KW vs EQ
- [04-01]: Layered pattern precedence: as > or > primary (composable postfix pattern layers)

### Pending Todos

None.

### Blockers/Concerns

- Phase 6 (Actor Runtime) is highest engineering risk -- preemptive scheduling, per-actor GC, work-stealing
- Typed actor messaging (TYPE-07) is a research-level problem -- design on paper during early phases

## Session Continuity

Last session: 2026-02-06
Stopped at: Completed 04-01-PLAN.md
Resume file: None
