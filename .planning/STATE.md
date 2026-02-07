# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-05)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** Phase 6 -- Actor Runtime. Plan 03 complete (per-actor heaps and FIFO message passing). Each actor has its own heap; messages deep-copied on send; FIFO mailbox with blocking receive.

## Current Position

Phase: 6 of 10 (Actor Runtime)
Plan: 3 of 7 in current phase
Status: In progress
Last activity: 2026-02-07 -- Completed 06-03-PLAN.md (per-actor heaps and message passing)

Progress: [██████████████████████████░░] 63% (26 plans of ~41 estimated total)

## Performance Metrics

**Velocity:**
- Total plans completed: 26
- Average duration: 9min
- Total execution time: 237min

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-project-foundation-lexer | 3/3 | 12min | 4min |
| 02-parser-ast | 5/5 | 23min | 5min |
| 03-type-system | 5/5 | 83min | 17min |
| 04-pattern-matching-adts | 5/5 | 42min | 8min |
| 05-llvm-codegen-native-binaries | 5/5 | 50min | 10min |
| 06-actor-runtime | 3/7 | 27min | 9min |

**Recent Trend:**
- Last 5 plans: 05-05 (15min), 06-01 (10min), 06-02 (10min), 06-03 (7min)

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
- [04-02]: Bare uppercase ident patterns (Red, None, Point) resolve to constructors when found in env
- [04-02]: Or-pattern binding validation uses semantic-aware env lookup to skip constructor names
- [04-02]: Variant constructors registered under both qualified (Shape.Circle) and unqualified (Circle) names
- [04-02]: Error codes E0010 (UnknownVariant), E0011 (OrPatternBindingMismatch)
- [04-03]: TypeRegistry parameter on check_exhaustiveness/check_redundancy for complete nested type resolution
- [04-03]: Bool literals treated as named constructors true/false (unifies with sum type handling)
- [04-03]: Pattern-based registry building in is_useful() for standalone use; caller-provided registry for check_exhaustiveness/check_redundancy
- [04-04]: Guards restricted to comparisons, boolean ops, literals, name refs, and named function calls
- [04-04]: Guarded arms excluded from exhaustiveness matrix (treated as potentially non-matching)
- [04-04]: Multi-clause function definitions deferred to future plan (requires parser + type checker changes)
- [04-04]: Redundant arms stored in ctx.warnings (warning-level, not error)
- [04-05]: Error codes E0012 (NonExhaustiveMatch), W0001 (RedundantArm), E0013 (InvalidGuardExpression)
- [04-05]: RedundantArm uses Warning report kind (W0001), distinct from Error-level diagnostics
- [04-05]: Option/Result registered as SumTypeDefInfo in TypeRegistry (proper sum types, not just constructors)
- [04-05]: Shared register_variant_constructors function unifies user-defined and builtin sum type registration
- [04-05]: Qualified access (Option.Some, Result.Ok) works for builtin sum types
- [05-01]: Arena/bump allocator for Phase 5 GC (no collection); true mark-sweep deferred to Phase 6
- [05-01]: SnowString repr(C) with inline length prefix: { u64 len, [u8; len] data }
- [05-01]: Mutex-protected global arena for thread safety (single-threaded in Phase 5)
- [05-01]: TypeRegistry and all type definition info structs made fully pub with pub fields
- [05-01]: LLVM_SYS_211_PREFIX configured for LLVM 21.1.8 at /opt/homebrew/opt/llvm
- [05-02]: Ty::Var falls back to MirType::Unit for graceful degradation on type errors
- [05-02]: Pipe operator desugared as pure syntactic transform at MIR level
- [05-02]: Closures lifted with __env first param; captures detected by free variable analysis
- [05-02]: String interpolation compiled to snow_string_concat/snow_*_to_string runtime call chains
- [05-02]: Monomorphization is reachability-based for Phase 5 (type checker resolves concrete types)
- [05-03]: Constructor sub-pattern bindings collected via recursive column processing (not Constructor bindings field)
- [05-03]: Or-patterns expanded before matrix construction by duplicating (arm_index, pattern, guard) tuples
- [05-03]: Boolean exhaustive match produces Test chain with terminal Fail (LLVM optimizes away unreachable Fail)
- [05-03]: Tuple patterns handled by column expansion, not Switch/Test
- [05-03]: Column selection heuristic: column with most distinct head constructors wins
- [05-03]: Guard failure chains to next row or Fail node
- [05-04]: ValueKind::basic() replaces Either::left() in Inkwell 0.8.0 API
- [05-04]: build_switch takes all cases upfront as &[(IntValue, BasicBlock)]
- [05-04]: Struct field index lookup via mir_struct_defs stored during compile
- [05-04]: Alloca+mem2reg pattern for if/else and match result merging
- [05-04]: String comparison placeholder (pointer identity) for Phase 5
- [05-05]: System cc as linker driver (handles macOS clang / Linux gcc transparently)
- [05-05]: snowc build auto-builds snow-rt via cargo before linking
- [05-05]: Closure parameter type annotations resolved in infer_closure (was missing, caused string interpolation to fail in closures)
- [06-01]: Coroutines are !Send -- work-stealing operates on SpawnRequest (Send) not Coroutine
- [06-01]: Thread-local shadow reduction counter avoids locking on every reduction check
- [06-01]: Yielder re-installation after suspend to handle interleaved coroutines on same thread
- [06-02]: Actor blocks parsed as top-level items (same level as fn/struct/module)
- [06-02]: Actor expressions (spawn/send/receive/self/link) dispatch from expression lhs()
- [06-02]: Receive arms reuse existing pattern matching infrastructure from case/match
- [06-02]: Terminate clause enforces single-occurrence-per-actor at parse time
- [06-02]: MirType::Pid uses Option<Box<MirType>> for optional message type parameterization
- [06-02]: Placeholder stubs in type checker, MIR lowering, and LLVM codegen for actor constructs
- [06-03]: Per-actor heap reuses Arena bump allocation algorithm from gc.rs
- [06-03]: MessageBuffer stores raw bytes + u64 type_tag; tags derived from first 8 bytes of data
- [06-03]: Mailbox uses Mutex<VecDeque<Message>> for thread-safe FIFO
- [06-03]: Blocking receive yields to scheduler (Waiting state); woken by state transition to Ready
- [06-03]: Heap message layout: [u64 type_tag, u64 data_len, u8... data] -- 16-byte fixed header
- [06-03]: snow_gc_alloc_actor falls back to global arena when no actor context

### Pending Todos

None.

### Blockers/Concerns

- Phase 6 (Actor Runtime) is highest engineering risk -- preemptive scheduling, per-actor GC, work-stealing
- Typed actor messaging (TYPE-07) is a research-level problem -- design on paper during early phases
- Plan 06-01 test isolation bugs fixed: all 27 snow-rt tests now pass (test counters isolated per-test)

## Session Continuity

Last session: 2026-02-07
Stopped at: Completed 06-03-PLAN.md
Resume file: None
