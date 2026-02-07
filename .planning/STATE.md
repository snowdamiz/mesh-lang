# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-05)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** Phase 10 IN PROGRESS -- Developer Tooling. Plans 01, 02, 03, 04, 05, 06, 07, 08 complete.

## Current Position

Phase: 10 of 10 (Developer Tooling)
Plan: 08 of 10 in current phase (plans 01, 02, 03, 04, 05, 06, 07, 08 complete)
Status: In progress
Last activity: 2026-02-07 -- Completed 10-03-PLAN.md (Formatter CLI Integration)

Progress: [███████████████████████████████████████████████████] 95% (52 plans of 55 total)

## Performance Metrics

**Velocity:**
- Total plans completed: 52
- Average duration: 9min
- Total execution time: 485min

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-project-foundation-lexer | 3/3 | 12min | 4min |
| 02-parser-ast | 5/5 | 23min | 5min |
| 03-type-system | 5/5 | 83min | 17min |
| 04-pattern-matching-adts | 5/5 | 42min | 8min |
| 05-llvm-codegen-native-binaries | 5/5 | 50min | 10min |
| 06-actor-runtime | 7/7 | 70min | 10min |
| 07-supervision-fault-tolerance | 3/3 | 27min | 9min |
| 08-standard-library | 7/7 | 67min | 10min |
| 09-concurrency-standard-library | 5/5 | 51min | 10min |
| 10-developer-tooling | 8/10 | 60min | 8min |

**Recent Trend:**
- Last 5 plans: 10-05 (7min), 10-07 (5min), 10-08 (8min), 10-04 (10min), 10-02 (12min)

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
- [06-04]: Pid<M> as Ty::App(Ty::Con("Pid"), [M]) -- uses existing generic type machinery, not a new Ty variant
- [06-04]: Actor message type tracked via ACTOR_MSG_TYPE_KEY env binding (not a separate context struct)
- [06-04]: Let binding with annotation uses annotation type for scheme (enables Pid escape hatch)
- [06-04]: Untyped Pid unifies with typed Pid<M> bidirectionally without constraining M
- [06-04]: Error codes E0014 (SendTypeMismatch), E0015 (SelfOutsideActor), E0016 (SpawnNonFunction), E0017 (ReceiveOutsideActor)
- [06-05]: Pid maps to i64 at LLVM level (was previously ptr; runtime uses u64 process IDs)
- [06-05]: Reduction checks inserted after user function calls only, not after runtime intrinsic calls
- [06-05]: Terminate callbacks lower to separate __terminate_<name> MIR functions with (Ptr, Ptr) -> Unit signature
- [06-05]: snow_actor_link and snow_actor_set_terminate runtime stubs added for link-time resolution
- [06-06]: EXIT_SIGNAL_TAG = u64::MAX reserved as exit signal sentinel in messages
- [06-06]: Process.links changed from Vec to HashSet for O(1) bidirectional link operations
- [06-06]: ProcessRegistry maintains PID-to-names reverse index for efficient cleanup_process on exit
- [06-06]: Normal exit delivers message to linked processes but does NOT crash them (Erlang semantics)
- [06-06]: Terminate callback invoked BEFORE exit signal propagation to linked processes
- [06-07]: Scheduler run loop uses interior mutability to avoid deadlock during coroutine resume
- [06-07]: Compiler emits snow_main (not main) to avoid symbol collision with runtime
- [06-07]: Actor spawn functions return Unit at MIR/codegen level (runtime allocates Pid externally)
- [06-07]: Reduction check only yields when inside coroutine context (guards against bare main thread)
- [07-01]: OnceLock + Mutex<FxHashMap> for global supervisor state registry (no lazy_static dependency)
- [07-01]: Supervisor state stored in global registry keyed by PID (coroutine entry only receives *const u8)
- [07-01]: Shutdown treated as non-crashing for exit propagation (same as Normal) -- Transient children do NOT restart on Shutdown
- [07-01]: Custom(String) treated as crashing for exit propagation (same as Error)
- [07-01]: Binary config format for snow_supervisor_start: strategy(u8) + max_restarts(u32) + max_seconds(u64) + child specs
- [07-02]: Supervisors registered as zero-arg functions returning Pid<Unit> -- they don't receive user messages
- [07-02]: Function pointers in supervisor config patched at codegen time via stack-copied buffer with GEP stores
- [07-02]: Child spec start closures resolved by walking CST tokens to find SPAWN_KW and extracting actor name
- [07-03]: Child start fn validation uses SPAWN_KW token detection (not full type inference of closure body)
- [07-03]: Error codes E0018 (InvalidChildStart), E0019 (InvalidStrategy), E0020 (InvalidRestartType), E0021 (InvalidShutdownValue)
- [07-03]: Negative E2E test pattern: compile_only helper returns raw Output, test asserts failure + error code
- [08-01]: Module-qualified access (String.length) resolved by checking FieldAccess base against known module names
- [08-01]: from/import dual registration: both bare and prefixed names into type env
- [08-01]: Bool i1/i8 coercion at runtime intrinsic call boundaries (zext args, trunc returns)
- [08-01]: string_split deferred to Plan 02 when List<T> type exists
- [08-02]: Uniform u64 representation for all collection elements (type-erased at runtime, typed at compile time)
- [08-02]: Linear-scan maps/sets backed by Vec-of-pairs (simple, efficient for Phase 8 typical sizes)
- [08-02]: Two-list queue without reversal (append-based back list already in chronological order)
- [08-02]: Prelude names (map, filter, reduce, head, tail) auto-imported, resolve to List operations
- [08-02]: All collection types resolve to MirType::Ptr (opaque pointers at LLVM level)
- [08-03]: Aligned sum type layout { i8, ptr } for runtime-returned Result/Option (matches #[repr(C)])
- [08-03]: Monomorphized generic type lookup fallback (Result_String_String -> Result base name)
- [08-03]: Generic type params (T, E) replaced with MirType::Ptr in builtin sum type variant fields
- [08-03]: Runtime-returned ptr-to-sum-type dereferenced at let binding site via LLVM load
- [08-04]: Json type registered as opaque Ptr (not full sum type) -- pattern matching on Json variants deferred
- [08-04]: SnowJson uses 16-byte layout {tag: u8, _pad: [u8;7], value: u64} for 8-byte alignment
- [08-04]: JSON numbers stored as i64 (not f64) since Snow primarily uses integer types
- [08-04]: serde_json bridge converts between serde_json::Value and GC-allocated SnowJson recursively
- [08-05]: Thread-per-connection instead of actor-per-connection for HTTP server (tiny_http + std::thread::spawn)
- [08-05]: 3-arg snow_http_route (router, pattern, handler_fn) with null env -- bare function handlers only
- [08-05]: No bare name mappings for HTTP/Request functions to avoid collision with common variable names
- [08-05]: Router/Request/Response resolve to MirType::Ptr (opaque pointers at LLVM level)
- [08-05]: Handler calling convention: fn_ptr(request) -> response for bare named functions
- [08-06]: Closure struct splitting: codegen extracts {fn_ptr, env_ptr} from closure struct for HOF runtime intrinsics
- [08-06]: Non-null dummy env for zero-capture closures ensures runtime HOFs use closure calling convention
- [08-06]: Direct function calls for closure HOF chains (pipe operator has parser limitation with inline closures)
- [08-07]: Fixed port 18080 for HTTP server runtime E2E test (avoids port-0 coordination complexity)
- [08-07]: Raw TcpStream for HTTP test requests (no additional dependency needed)
- [08-07]: Snow string literals preserve backslash characters literally (no escape interpretation)
- [09-01]: Call handler return type uses :: syntax (ColonColon + Type), distinct from fn's -> syntax
- [09-01]: fn init inside service body dispatched through parse_item_or_stmt as regular FN_DEF
- [09-01]: State parameter uses simple |ident| bar-delimited parsing (single identifier, not full closure params)
- [09-01]: Service body tuples use () not {} (Snow tuple syntax is parenthesized)
- [09-01]: Keyword count increased from 42 to 45 (service, call, cast)
- [09-02]: Service helper functions registered in TypeEnv as "ServiceName.method_name" entries (avoids threading user_modules)
- [09-02]: Service Pid type is Pid<Unit> for callers -- internal dispatching uses type_tags at runtime
- [09-02]: Job module uses synthetic TyVars (u32::MAX - 10..12) for polymorphic Schemes
- [09-02]: User-defined service modules resolved in infer_field_access before sum type variant lookup
- [09-03]: Service loop function has MirExpr::Unit body; codegen generates dispatch loop by detecting __service_*_loop naming
- [09-03]: Service dispatch uses switch on integer type_tag (call handlers: 0,1,2...; cast handlers: N,N+1,...)
- [09-03]: Call handler returns tuple (new_state, reply); codegen extracts via snow_tuple_first/second
- [09-03]: Cast message format [tag][0 caller][args] for uniform dispatch in service loop
- [09-03]: Service call/cast helpers intercepted in codegen_call by function name pattern (snow_service_call/snow_actor_send)
- [09-04]: JOB_RESULT_TAG = u64::MAX - 1 distinguishes job results from EXIT_SIGNAL_TAG = u64::MAX
- [09-04]: Job.await returns SnowResult (tag 0 = Ok, tag 1 = Err) matching existing Result layout
- [09-04]: Job entry function packs fn_ptr/env_ptr/caller_pid into GC-heap args buffer
- [09-04]: Job.map spawns one actor per list element, awaits all in order
- [09-05]: Main thread gets PID and mailbox via create_main_process for service call support from non-coroutine context
- [09-05]: Scheduler workers start eagerly during snow_rt_init_actor so actors execute concurrently during snow_main
- [09-05]: snow_actor_receive detects coroutine context via CURRENT_YIELDER; uses spin-wait for main thread
- [09-05]: Reduction check uses CURRENT_YIELDER (not CURRENT_PID) to detect coroutine context
- [09-05]: Graceful service actor shutdown via wake-and-null pattern (avoids corosensei panic on force-drop through extern C)
- [10-06]: Serde untagged enum for Dependency (Git variant with git/rev/branch/tag fields, Path variant with path field)
- [10-06]: BTreeMap for dependencies and lockfile packages ensures deterministic ordering
- [10-06]: DFS visiting set for cycle detection; source key comparison for conflict detection
- [10-06]: Git deps cloned to project_dir/.snow/deps/<name>/ (project-local, not global cache)
- [10-06]: Path dep source keys use canonicalize() for consistent diamond deduplication
- [10-06]: Lockfile version field (always 1) for future format evolution
- [10-08]: Mutex<HashMap> document store for LSP (simple, correct for single-client model)
- [10-08]: UTF-16 position conversion per LSP spec (char.len_utf16() for non-ASCII)
- [10-08]: Smallest-range lookup for hover type (finds innermost expression at cursor)
- [10-08]: tokio::runtime::Runtime::new() in snowc main for LSP (avoids async main)
- [10-04]: LLVM Context created per evaluation (persistent context requires complex lifetime management)
- [10-04]: Keyword-prefix heuristic for definition vs expression classification in REPL
- [10-04]: Token-based do/end and delimiter balancing for multi-line input detection
- [10-04]: value :: Type display format for REPL results (Haskell-inspired)
- [10-04]: into_module() on CodeGen for JIT execution engine creation
- [10-02]: Wadler-Lindig FormatIR with 8 variants (Text, Space, Hardline, Indent, Group, IfBreak, Concat, Empty)
- [10-02]: sp() literal space helper vs ir::space() break-sensitive -- root context always break mode
- [10-02]: TYPE_ANNOTATION gets space before it in inline token walker (Snow :: is type annotation, not path separator)
- [10-02]: Stack-based printer with (indent, mode, ir_node) triples and measure_flat() for Group decisions
- [10-01]: DiagnosticOptions struct with color/json fields replaces hardcoded colorless config
- [10-01]: JsonDiagnostic struct with Serialize derive for machine-readable output
- [10-01]: FnArg multi-span uses call_site + param_idx (no param_span in ConstraintOrigin)
- [10-01]: Levenshtein distance for "did you mean X?" suggestions (max distance 2)
- [10-01]: --json and --no-color CLI flags on snowc build subcommand
- [10-07]: Lockfile freshness uses filesystem mtime comparison (manifest vs lockfile) -- simple, no hashing
- [10-05]: LLVMAddSymbol via extern C block for JIT symbol registration (inkwell 0.8 lacks add_symbol)
- [10-05]: snow-rt linked as Rust lib dependency for REPL runtime symbol availability
- [10-05]: Runtime init (GC + actor scheduler) once at REPL startup via std::sync::Once
- [10-05]: History persisted to $HOME/.snow_repl_history
- [10-03]: File not rewritten when already formatted (preserves mtime for build systems and CI)
- [10-03]: Pipe operator and interface method body formatting are known idempotency limitations (pre-existing parser/walker issues)

### Pending Todos

None.

### Blockers/Concerns

- Phase 9 COMPLETE -- all 5 plans done, all E2E tests passing
- Phase 10 IN PROGRESS -- plans 01, 02, 03, 04, 05, 06, 07, 08 complete
- string_split now possible with List type available (can be added in future plan)
- String-keyed maps use pointer identity (not content comparison) -- documented limitation
- Multiline pipe operator (`|>` at start of continuation line) fails to parse -- pre-existing parser limitation
- Pipe operator also fails when used with inline closures (fn(x) -> expr end) on same line -- parser merges with previous expression
- Closure handlers for HTTP.route not supported in Phase 8 (bare function handlers only)
- Map.put typed as (Map, Int, Int) -- string-keyed maps need type system refinement for proper E2E testing

## Session Continuity

Last session: 2026-02-07
Stopped at: Completed 10-03-PLAN.md (Formatter CLI Integration)
Resume file: None
