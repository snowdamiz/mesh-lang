# Project Milestones: Snow

## v1.6 Method Dot-Syntax (Shipped: 2026-02-09)

**Delivered:** Method dot-syntax (`value.method(args)`) with automatic self-parameter desugaring, working across struct, primitive, generic, and collection types, with true chaining, mixed field/method access, deterministic ambiguity diagnostics, and zero regressions across all existing syntax forms.

**Phases completed:** 30-32 (6 plans total)

**Key accomplishments:**
- Method dot-syntax (`value.method(args)`) with automatic self-parameter desugaring via retry-based resolution in type checker and shared resolve_trait_callee in MIR
- Primitive, generic, and collection types all callable via dot syntax (`42.to_string()`, `my_list.to_string()`)
- True method chaining (`p.to_string().length()`) and mixed field/method access (`p.name.length()`)
- Deterministic alphabetical ambiguity diagnostics with per-trait qualified syntax suggestions
- Stdlib module method fallback enabling dot-syntax for String, List, Map, Set, Range module functions
- Zero regressions across all 5 existing syntax forms (struct fields, module-qualified, pipes, sum types, actors)

**Stats:**
- 28 files modified
- 67,546 lines of Rust (+1,025 net from v1.5)
- 3 phases, 6 plans, 12 tasks
- 1 day (2026-02-08 → 2026-02-09)
- 24 commits
- 1,255 tests passing (+23 new in v1.6)

**Git range:** `feat(30-01)` → `feat(32-01)`

**What's next:** TBD -- method dot-syntax complete. Potential directions include inherent methods (impl without trait), method references, associated types, Iterator/From protocols, distributed actors, or hot code reloading.

---

## v1.5 Compiler Correctness (Shipped: 2026-02-09)

**Delivered:** Resolved all three remaining known limitations -- polymorphic List<T>, Ord-requires-Eq compile-time enforcement, and higher-order constraint propagation -- making the Snow type and trait systems fully correct with zero known compiler correctness issues.

**Phases completed:** 26-29 (6 plans total)

**Key accomplishments:**
- Polymorphic List<T> with any element type (String, Bool, structs, nested lists) via ListLit MIR + snow_list_from_array codegen
- List trait integration: callback-based Display/Debug/Eq/Ord dispatch for List<T>
- Cons pattern destructuring (head :: tail) for all list element types with ListDecons decision tree
- Compile-time trait deriving safety: E0029 error when deriving Ord without Eq, with suggestion
- Qualified types: trait constraints propagate through higher-order function arguments (apply(show, 42))

**Stats:**
- 54 files modified
- 66,521 lines of Rust (+1,973 net from v1.4)
- 4 phases, 6 plans
- 1 day (2026-02-08 → 2026-02-09)
- 29 commits
- 1,232 tests passing (+26 new in v1.5)

**Git range:** `feat(26-01)` → `test(29-01)`

**What's next:** TBD -- all compiler correctness issues resolved. Zero known limitations. Potential directions include Iterator/From protocols (associated types), method dot-syntax, blanket impls, distributed actors, or hot code reloading.

---

## v1.4 Compiler Polish (Shipped: 2026-02-08)

**Delivered:** Fixed all five known limitations from v1.3 -- pattern matching codegen, Ordering type, nested collection Display, generic type deriving, and higher-order constraint soundness -- making the compiler fully correct across its type and trait systems.

**Phases completed:** 23-25 (5 plans total)

**Key accomplishments:**
- Sum type pattern matching codegen fixed: constructor field extraction via sum_type_defs threading through compile_match pipeline
- Ordering (Less|Equal|Greater) registered as built-in sum type with compare() dispatching via Ord trait
- Recursive nested collection Display with synthetic MIR wrapper functions for callback bridging
- Generic type auto-derive with parametric trait impl registration and lazy monomorphization at struct literal sites
- Type system soundness: where-clause constraints propagate through let-binding aliases, preventing unsound calls

**Stats:**
- 28 files modified
- 64,548 lines of Rust (+1,359 net from v1.3)
- 3 phases, 5 plans, ~10 tasks
- 1 day (2026-02-08)
- 13 commits
- 1,206 tests passing (+19 new in v1.4)

**Git range:** `feat(23-01)` → `test(25-01)`

**What's next:** TBD -- all v1.x compiler correctness issues resolved. Potential directions include Iterator/From protocols (requires associated types), method dot-syntax, blanket impls, distributed actors, or hot code reloading.

---

## v1.3 Traits & Protocols (Shipped: 2026-02-08)

**Delivered:** Complete trait/protocol system with user-defined interfaces, impl blocks, static dispatch via monomorphization, and six stdlib protocols (Display, Debug, Eq, Ord, Hash, Default) plus auto-derive support.

**Phases completed:** 18-22 (18 plans total)

**Key accomplishments:**
- Trait infrastructure: structural type matching via temporary unification, replacing string-based type_to_key
- Trait method codegen: ImplDef lowering to MIR with Trait__Method__Type mangled names and static dispatch
- Essential stdlib protocols: Display, Debug, Eq, Ord with string interpolation integration and auto-derived for all non-generic types
- Extended protocols: Hash (FNV-1a), Default (static methods), default method implementations, collection Display/Debug
- Auto-derive system: `deriving(Eq, Ord, Display, Debug, Hash)` with conditional gating and backward compatibility

**Stats:**
- 77 files modified
- 63,189 lines of Rust (+5,532 net from v1.2)
- 5 phases, 18 plans
- 1 day (2026-02-07 → 2026-02-08)
- 65 commits
- 1,187 tests passing (+130 new in v1.3)

**Git range:** `feat(18-01)` → `feat(22-02)`

**What's next:** TBD -- trait system complete. Potential directions include Iterator/From protocols, method dot-syntax, blanket impls, distributed actors, or hot code reloading.

---

## v1.2 Runtime & Type Fixes (Shipped: 2026-02-08)

**Delivered:** Fun() type annotation parsing and mark-sweep garbage collector for per-actor heaps, fixing the two remaining known issues from v1.1.

**Phases completed:** 16-17 (6 plans total)

**Key accomplishments:**
- Fun() type annotations fully integrated: parser (FUN_TYPE CST node) through type checker (Ty::Fun) to codegen (MirType::Closure)
- Mark-sweep GC with 16-byte GcHeader, conservative stack scanning, and worklist-based tricolor marking
- Per-actor cooperative GC at yield points -- no stop-the-world pauses across actors
- All runtime allocations migrated to GC-managed per-actor heaps (snow_gc_alloc_actor)
- Bounded memory validated: long-running actors reclaim memory across 50 message cycles

**Stats:**
- 44 files modified (26 Rust source files)
- 57,657 lines of Rust (+1,118 net from v1.1)
- 2 phases, 6 plans
- 1 day (2026-02-07 → 2026-02-08)
- 22 commits

**Git range:** `feat(16-01)` → `feat(17-04)`

**What's next:** TBD -- all known issues resolved. Potential directions include distributed actors, hot code reloading, macros, generational GC, and precise stack scanning.

---

## v1.1 Language Polish (Shipped: 2026-02-08)

**Delivered:** Fixed all five documented v1.0 limitations -- multi-clause functions, string pattern matching, pipe operator with closures, actor-per-connection HTTP, and generic map types -- making the language feel complete and polished.

**Phases completed:** 11-15 (10 plans total)

**Key accomplishments:**
- Multi-clause function definitions with guard clauses, exhaustiveness warnings, and cross-clause type unification
- Full closure syntax rewrite with bare params, do/end body, multi-clause closures, and pipe-aware type checking
- Compile-time string pattern matching in case expressions via snow_string_eq
- Generic Map<K, V> types with string-key support, runtime key_type dispatch, and map literal syntax %{k => v}
- Actor-per-connection HTTP server with catch_unwind crash isolation replacing thread-per-connection model

**Stats:**
- 88 files modified
- 56,539 lines of Rust (+3,928 from v1.0)
- 5 phases, 10 plans
- 2 days (2026-02-07 → 2026-02-08)
- 45 commits

**Git range:** `feat(11-01)` → `feat(15-01)`

**What's next:** TBD -- all v1.0 limitations resolved. Potential directions include distributed actors, hot code reloading, macros, and mark-sweep GC.

---

## v1.0 MVP (Shipped: 2026-02-07)

**Delivered:** A statically typed, LLVM-compiled programming language with Elixir-style syntax, BEAM-style actor concurrency, supervision trees, a standard library for web backends, and full developer tooling.

**Phases completed:** 1-10 (55 plans total)

**Key accomplishments:**
- Full compiler pipeline (lexer, parser, HM type inference, MIR, LLVM codegen) producing native single-binary executables
- M:N work-stealing actor runtime with typed Pid<M>, 100K actor benchmark, process linking and exit signals
- OTP-style supervision trees with one_for_one/one_for_all/rest_for_one strategies and compile-time child spec validation
- Algebraic data types with Maranget's exhaustiveness/redundancy checking and ariadne diagnostics
- Standard library: I/O, strings, collections, file I/O, HTTP client/server, JSON encoding/decoding
- High-level concurrency: Service (GenServer) and Job (Task) abstractions with full type inference
- Developer tooling: code formatter, REPL with JIT, package manager, LSP server, VS Code extension

**Stats:**
- 107 Rust source files
- 52,611 lines of Rust
- 10 phases, 55 plans
- 2 days from start to ship (2026-02-05 → 2026-02-07)
- 213 commits

**Git range:** `feat(01-01)` → `feat(10-10)`

**What's next:** TBD -- language is feature-complete for v1. Potential v2 directions include distributed actors, hot code reloading, and macros.

---

## v1.7 Loops & Iteration (Shipped: 2026-02-09)

**Delivered:** Complete loop and iteration system with while loops, for-in over ranges and collections (List, Map, Set), break/continue control flow, comprehension semantics returning collected lists, filter clause (`when`), and actor-safe reduction checks at loop back-edges.

**Phases completed:** 33-36 (8 plans total)

**Key accomplishments:**
- While loops (`while condition do body end`) with break/continue, loop-depth tracking, closure boundary enforcement (E0032/E0033)
- For-in over integer ranges (`for i in 0..10 do body end`) with zero-allocation counter and half-open range semantics
- For-in over collections (List, Map with `{k,v}` destructuring, Set) with indexed iteration and O(N) list builder
- Comprehension semantics: all for-in loops return `List<T>` of collected body results; break returns partial list
- Filter clause (`for x in list when condition do body end`) with five-block codegen pattern across all collection types
- Reduction checks at loop back-edges for actor scheduler fairness; runtime list builder for O(N) allocation

**Stats:**
- 53 files modified
- 70,501 lines of Rust (+2,955 net from v1.6)
- 4 phases, 8 plans
- 2 days (2026-02-08 → 2026-02-09)
- 34 commits

**Git range:** `feat(33-01)` → `docs(phase-36)`

**What's next:** TBD -- loop system complete. Potential directions include iterator protocol/Iterable trait, `break value`, labeled breaks, string character iteration, distributed actors, or hot code reloading.

---


## v1.8 Module System (Shipped: 2026-02-09)

**Delivered:** Complete module system enabling multi-file projects with file-based modules, pub visibility, qualified and selective imports, dependency graph resolution with cycle detection, cross-module type checking for functions/structs/sum types/traits, module-qualified name mangling, and module-aware diagnostics -- compiled into a single LLVM binary via MIR merge.

**Phases completed:** 37-42 (12 plans total)

**Key accomplishments:**
- File-based module graph with recursive `.snow` discovery, path-to-name convention (`math/vector.snow` -> `Math.Vector`), Kahn's toposort, and cycle detection
- Multi-file build pipeline (`snowc build <dir>`) with ProjectData, per-module parsing, and zero single-file regressions across 84 pre-existing E2E tests
- Cross-module type checking with qualified imports (`import M` -> `M.fn()`), selective imports (`from M import { fn }`), accumulator-pattern inference, and MIR merge codegen
- Private-by-default visibility with `pub` modifier, PrivateItem E0035 error with "add pub" suggestion
- Module-qualified name mangling (`ModuleName__fn`) preventing private name collisions, cross-module generic monomorphization
- Module-aware diagnostics: file paths in errors via ariadne named-sources, module-qualified type names in type errors (`Geometry.Point`)

**Stats:**
- 70 files modified
- 73,384 lines of Rust (+2,883 net from v1.7)
- 6 phases, 12 plans
- 4 days (2026-02-05 -> 2026-02-09)
- 52 commits
- 27/27 requirements satisfied, 111 E2E tests passing (+31 new in v1.8)

**Git range:** `feat(37-01)` -> `docs(phase-42)`

**What's next:** TBD -- module system complete. Potential directions include import aliasing, unused import warnings, re-exports, incremental compilation, iterator protocol, distributed actors, or hot code reloading.

---


## v1.9 Stdlib & Ergonomics (Shipped: 2026-02-10)

**Delivered:** Made Snow practical for real programs by adding math stdlib via LLVM intrinsics, ? operator for Result/Option error propagation, receive timeouts and timer primitives, 20 collection operations across List/Map/Set/String, and self-recursive tail-call elimination -- all with zero new Rust crate dependencies and zero regressions.

**Phases completed:** 43-48 (13 plans total)

**Key accomplishments:**
- Math stdlib: 10 numeric operations (abs, min, max, pow, sqrt, floor, ceil, round, pi) via LLVM intrinsics + bidirectional Int/Float type conversion
- ? operator: Result<T,E> and Option<T> error propagation desugared to Match+Return in MIR, with E0036/E0037 diagnostics for misuse
- Receive timeouts: `receive { ... } after ms -> body` with null-check branching codegen completing the actor timeout feature
- Timer primitives: Timer.sleep (cooperative yield loop) and Timer.send_after (background OS thread with deep-copied message)
- Collection operations: 20 functions across List (sort, find, any, all, contains, zip, flat_map, flatten, enumerate, take, drop), String (split, join, to_int, to_float), Map (merge, to_list, from_list), Set (difference, to_list, from_list)
- Tail-call elimination: Self-recursive tail calls transformed to loops via MIR rewrite pass, supporting 7 tail position contexts (if/else, case, receive, blocks, let-chains, multi-clause, actor), 1M+ iterations without stack overflow

**Stats:**
- 89 files modified
- 76,100 lines of Rust (+2,716 net from v1.8)
- 6 phases, 13 plans
- 2 days (2026-02-09 -> 2026-02-10)
- 56 commits
- 28/28 requirements satisfied, 1,419 tests passing (+37 new v1.9-specific e2e tests)

**Git range:** `feat(43-01)` -> `docs(48-02)`

**What's next:** TBD -- stdlib and ergonomics complete. Potential directions include mutual TCE via musttail, From trait for ? operator error conversion, iterator protocol/Iterable trait, distributed actors, hot code reloading, or incremental compilation.

---


## v2.0 Database & Serialization (Shipped: 2026-02-12)

**Delivered:** Made Snow viable for real backend applications with JSON struct/sum-type serde via `deriving(Json)`, SQLite and PostgreSQL database drivers with parameterized queries, HTTP path parameters with method-specific routing, and composable middleware pipelines.

**Phases completed:** 49-54 (13 plans total)

**Key accomplishments:**
- JSON struct serde with `deriving(Json)`, typed extractors, and callback-based collection encode/decode for nested structs, Option, List, Map fields
- Sum type and generic struct JSON serialization via tagged union encoding (`{"tag":"V","fields":[...]}`) and monomorphization
- HTTP path parameter matching (`/users/:id`) with method-specific routing (GET/POST/PUT/DELETE) and three-pass priority (exact > parameterized > wildcard)
- Composable middleware pipeline with trampoline-based chain execution, short-circuit support, and automatic 404 wrapping
- SQLite driver with bundled C FFI (zero system deps), opaque GC-safe u64 handles, and `?` parameterized queries
- PostgreSQL pure wire protocol client (~550 lines Rust) with SCRAM-SHA-256/MD5 auth and Extended Query protocol ($1, $2 params)

**Stats:**
- 76 files modified
- 81,006 lines of Rust (+4,906 net from v1.9)
- 6 phases, 13 plans
- 2 days (2026-02-11 -> 2026-02-12)
- 52 commits
- 32/32 requirements satisfied, 287+ tests passing (+16 new v2.0 E2E tests)

**Git range:** `feat(49-01)` -> `docs(54-02)`

**What's next:** TBD -- database and serialization complete. Potential directions include connection pooling, TLS for PostgreSQL, transaction support, struct-to-row mapping, JSON pretty-print, iterator protocol, distributed actors, or hot code reloading.

---


## v3.0 Production Backend (Shipped: 2026-02-12)

**Delivered:** Made Snow viable for production backend deployments with TLS encryption for PostgreSQL and HTTPS, connection pooling with health checks, panic-safe database transactions, and automatic struct-to-row mapping via `deriving(Row)`.

**Phases completed:** 55-58 (8 plans total)

**Key accomplishments:**
- PostgreSQL TLS via SSLRequest protocol with sslmode=disable/prefer/require, PgStream enum for zero-cost Plain/Tls dispatch, rustls 0.23 with webpki-roots
- HTTPS server via Http.serve_tls with hand-rolled HTTP/1.1 parser replacing tiny_http, HttpStream enum mirroring PgStream pattern
- Connection pooling with Mutex+Condvar synchronization, configurable min/max/timeout, health check (SELECT 1) on checkout, automatic ROLLBACK on dirty checkin
- Database transactions: Pg.begin/commit/rollback/transaction with catch_unwind panic-safe rollback, Sqlite.begin/commit/rollback manual control
- Struct-to-row mapping via deriving(Row) with from_row function generation, Pg.query_as/Pool.query_as one-step query+hydration, E0039 compile error for non-mappable fields

**Stats:**
- 48 files modified
- 83,451 lines of Rust (+2,445 net from v2.0)
- 4 phases, 8 plans
- 1 day (2026-02-12)
- 33 commits
- 24/24 requirements satisfied, 290+ tests passing (+18 new v3.0 tests)

**Git range:** `feat(55-01)` -> `docs(v3.0)`

**What's next:** TBD -- production backend complete. Potential directions include WebSocket support, HTTP/2, async file I/O, distributed actors, hot code reloading, iterator protocol, or incremental compilation.

---

