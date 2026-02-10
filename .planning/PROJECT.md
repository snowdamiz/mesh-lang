# Snow

## What This Is

Snow is a programming language that combines Elixir/Ruby-style expressive syntax with static Hindley-Milner type inference and BEAM-style concurrency (actors, supervision trees, fault tolerance), compiled via LLVM to native single-binary executables. The compiler is written in Rust. v1.0 shipped a complete compiler pipeline, actor runtime, standard library, and developer tooling. v1.1 polished the language by resolving all five documented v1.0 limitations. v1.2 added Fun() type annotations and a mark-sweep garbage collector for per-actor heaps. v1.3 completed the trait/protocol system with user-defined interfaces, impl blocks, static dispatch via monomorphization, and six stdlib protocols (Display, Debug, Eq, Ord, Hash, Default) with auto-derive support. v1.4 fixed all five compiler correctness issues: pattern matching codegen, Ordering type, nested collection Display, generic type deriving, and type system soundness for constrained function aliases. v1.5 resolved the final three known limitations: polymorphic List<T>, compile-time Ord-requires-Eq enforcement, and qualified types for higher-order constraint propagation. v1.6 added method dot-syntax (`value.method(args)`) with automatic self-parameter desugaring, working across struct, primitive, generic, and collection types, with true chaining, mixed field/method access, and deterministic ambiguity diagnostics. v1.7 added complete loop and iteration support: while loops with break/continue, for-in over ranges and collections (List, Map, Set) with comprehension semantics returning collected lists, filter clause (`when`), and actor-safe reduction checks. v1.8 added a complete module system: file-based modules with path-to-name convention, `pub` visibility (private by default), qualified and selective imports, dependency graph with toposort and cycle detection, cross-module type checking for functions/structs/sum types/traits, MIR merge codegen with module-qualified name mangling, and module-aware diagnostics. Zero known compiler correctness issues remain.

## Core Value

Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.

## Requirements

### Validated

- ✓ Elixir/Ruby-style syntax (do/end blocks, pattern matching, keyword-based, minimal punctuation) -- v1.0
- ✓ Static type system with Hindley-Milner inference (rarely write type annotations) -- v1.0
- ✓ BEAM-style concurrency: lightweight actor processes with typed message passing -- v1.0
- ✓ Supervision trees with let-it-crash fault tolerance -- v1.0
- ✓ LLVM backend producing native single-binary executables (runtime bundled) -- v1.0
- ✓ Functional-first paradigm (no classes, no OOP hierarchies) -- v1.0
- ✓ General purpose -- suitable for web backends/APIs and CLI tools -- v1.0
- ✓ Pattern matching as a core language feature with exhaustiveness checking -- v1.0
- ✓ Standard library sufficient for HTTP servers and file I/O -- v1.0
- ✓ Developer tooling: formatter, REPL, package manager, LSP server -- v1.0
- ✓ Multi-clause function definitions with guard clauses and exhaustiveness warnings -- v1.1
- ✓ String comparison in pattern matching (compile-time string matching via snow_string_eq) -- v1.1
- ✓ Pipe operator with inline closures (full closure rewrite + pipe-aware type inference) -- v1.1
- ✓ Actor-per-connection HTTP server with catch_unwind crash isolation -- v1.1
- ✓ Generic map types Map<K, V> with string keys and map literal syntax -- v1.1
- ✓ Fun() type annotation parsed as function type instead of type constructor -- v1.2
- ✓ Mark-sweep garbage collector for per-actor heaps (replacing arena/bump allocation) -- v1.2
- ✓ User-defined interface definitions with method signatures and default implementations -- v1.3
- ✓ impl blocks to implement interfaces for concrete types with static dispatch via monomorphization -- v1.3
- ✓ Where clauses working with user-defined traits (TraitNotSatisfied enforcement) -- v1.3
- ✓ Trait-based operator overloading for user types (all 6 comparison operators via Eq/Ord) -- v1.3
- ✓ Stdlib protocols: Display, Debug, Eq, Ord, Hash, Default -- v1.3
- ✓ Auto-derive: deriving(Eq, Ord, Display, Debug, Hash) from struct/sum-type metadata -- v1.3
- ✓ Collection Display/Debug for List, Map, Set -- v1.3
- ✓ Sum type constructor pattern matching extracts field values in LLVM codegen -- v1.4
- ✓ Ordering sum type (Less | Equal | Greater) user-visible with compare() via Ord trait -- v1.4
- ✓ Nested collection Display renders recursively with synthetic MIR wrapper callbacks -- v1.4
- ✓ Generic types support auto-derive with monomorphization-aware trait impl registration -- v1.4
- ✓ Higher-order constrained functions preserve trait constraints when captured as values -- v1.4
- ✓ Polymorphic List<T> -- lists work with any element type (String, Bool, structs, nested lists) -- v1.5
- ✓ List trait integration -- Display/Debug/Eq/Ord work for List<T> via callback dispatch -- v1.5
- ✓ Cons pattern destructuring -- head :: tail pattern matching for all list element types -- v1.5
- ✓ Compile-time trait deriving safety -- Ord without Eq emits E0029 error with suggestion -- v1.5
- ✓ Qualified types -- trait constraints propagate through higher-order function arguments -- v1.5
- ✓ Method dot-syntax: `expr.method(args)` resolves impl block methods for receiver type -- v1.6
- ✓ Self-parameter desugaring: receiver passed as first argument automatically -- v1.6
- ✓ Chained method calls: `expr.method1().method2()` -- v1.6
- ✓ Trait method dot-syntax: trait methods callable via dot on implementing types -- v1.6
- ✓ Generic method resolution: dot syntax works with monomorphized generic types -- v1.6
- ✓ While loops (`while condition do body end`) with break/continue and loop-depth tracking -- v1.7
- ✓ For-in over ranges (`for i in 0..10 do body end`) with zero-allocation integer arithmetic -- v1.7
- ✓ For-in over collections (List, Map with destructuring, Set) with indexed iteration -- v1.7
- ✓ Comprehension semantics: for-in returns `List<T>` of collected body results -- v1.7
- ✓ Filter clause (`for x in list when cond do body end`) across all collection types -- v1.7
- ✓ Break/continue: early exit returns partial list, closure boundary enforcement (E0032/E0033) -- v1.7
- ✓ Reduction checks at loop back-edges for actor scheduler fairness -- v1.7
- ✓ File-based modules with recursive discovery and path-to-name convention (math/vector.snow -> Math.Vector) -- v1.8
- ✓ Module dependency graph with Kahn's toposort and circular import detection -- v1.8
- ✓ Multi-file build pipeline (`snowc build <dir>`) with per-module parsing and zero regressions -- v1.8
- ✓ Qualified imports (`import M` -> `M.fn()`) and selective imports (`from M import { fn }`) -- v1.8
- ✓ Cross-module type checking for functions, structs, sum types, and traits -- v1.8
- ✓ Private-by-default visibility with `pub` modifier and PrivateItem error with suggestion -- v1.8
- ✓ Global trait impl visibility across all modules without explicit import -- v1.8
- ✓ Cross-module generic monomorphization and module-qualified name mangling -- v1.8
- ✓ Module-aware diagnostics: file paths in errors and module-qualified type names -- v1.8
- ✓ Full backward compatibility: single-file programs compile identically -- v1.8

### Active

## Current Milestone: v1.9 Stdlib & Ergonomics

**Goal:** Make Snow practical for real programs by adding math stdlib, error propagation sugar, receive timeouts, timer primitives, collection operations, and tail-call optimization.

**Target features:**
- Math stdlib (abs, min, max, pow, sqrt, floor, ceil) via libm FFI
- `?` operator for Result<T,E> propagation
- Receive timeout `after` clause (finish codegen for existing parser/runtime support)
- Timer primitives (sleep, send_after)
- Collection operations (sort, split, join, find, zip for List/Map/Set/String)
- Tail-call elimination (full TCE including mutual recursion)

### Out of Scope

- Classes and OOP -- functional-first design, use structs/traits/protocols instead
- Systems programming (drivers, embedded, OS-level) -- not targeting bare-metal performance
- GUI framework -- web and CLI are the primary targets
- Self-hosting compiler -- Rust is the compiler language, bootstrapping is not a v1 goal
- Ad-hoc operator overloading -- trait-based overloading (impl Add for T) is supported; arbitrary symbol overloading is not
- Shared mutable state between actors -- defeats actor model, causes data races
- Null/nil values -- Option<T> is the only way to represent absence
- Exceptions (try/catch/throw) -- Result<T,E> + let-it-crash philosophy replaces them
- Async/await colored functions -- runtime handles concurrency transparently
- Inheritance -- functional paradigm uses composition + traits instead
- Manual memory management -- per-actor GC handles this
- Generational GC -- mark-sweep sufficient for now; generational optimization is future work
- Concurrent/incremental GC -- per-actor isolation means pauses only affect one actor
- Compacting GC -- mark-sweep with free-list is sufficient
- Dynamic dispatch / vtables / trait objects -- use sum types instead; static dispatch via monomorphization
- Higher-kinded types (Functor/Monad) -- out of language philosophy
- Specialization (overlapping impls) -- unsound without careful design; not planned
- UFCS (any function callable via dot) -- pipe operator covers this use case; UFCS blurs method/function distinction
- Auto-ref/auto-deref on receiver -- Snow has no references; all values are value-typed
- Method overloading by parameter count -- Snow does not support function overloading
- Extension methods without traits -- breaks coherence; use pipe + module functions instead

## Context

Shipped v1.8 with 73,384 lines of Rust (+2,883 from v1.7).
Tech stack: Rust compiler, LLVM 21 (Inkwell 0.8), corosensei coroutines, rowan CST, ariadne diagnostics.
Crates: snow-lexer, snow-parser, snow-typeck, snow-mir, snow-codegen, snow-rt, snow-fmt, snow-repl, snow-pkg, snow-lsp, snowc.

111 E2E tests passing, 235+ unit tests across workspace. Zero known critical bugs. Zero known compiler correctness issues.

Known limitations: None.

Tech debt (minor):
- Pre-existing TODO in lower.rs:5799 ("Add proper snow_string_compare") -- unrelated to module system
- build_module_graph wrapper in discovery.rs used only in Phase 37 tests -- consider deprecation
- report_diagnostics function in main.rs appears to be dead code

## Constraints

- **Compiler language**: Rust -- chosen for safety, LLVM ecosystem (inkwell), and compiler development ergonomics
- **Compilation target**: LLVM IR -- enables native binaries across platforms without writing multiple backends
- **No OOP**: Functional paradigm only -- structs, traits/protocols, pattern matching. No class hierarchies.
- **Runtime**: Actor runtime bundled into the binary. Lightweight enough to not bloat small CLI tools.

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Rust for compiler | Strong LLVM bindings, memory safe, good for complex software | ✓ Good -- 73K LOC Rust, stable compiler |
| LLVM as backend | Proven codegen, multi-platform, avoids writing own backend | ✓ Good -- native binaries on macOS/Linux |
| Elixir/Ruby syntax style | Expressive, readable, pattern matching native | ✓ Good -- clean do/end blocks, pipe operator |
| Static types with HM inference | Safety without verbosity | ✓ Good -- rarely need annotations |
| No OOP | Functional-first aligns with actor model | ✓ Good -- simpler language, structs+traits sufficient |
| Bundled runtime | Single binary deployment like Go | ✓ Good -- self-contained executables |
| Angle brackets <T> for generics | Disambiguates from list syntax | ✓ Good -- migrated from [T] in Phase 3 |
| corosensei for coroutines | M:N scheduling without OS threads per actor | ✓ Good -- 100K actors in ~2.78s |
| Rowan for CST | Lossless syntax tree, editor tooling support | ✓ Good -- powers formatter and LSP |
| Actor-per-connection HTTP | Crash isolation, lightweight, uses existing actor runtime | ✓ Good -- v1.1, replaced threads with actors |
| Mark-sweep GC for actor heaps | Arena/bump allocation caused unbounded growth in long-running actors | ✓ Good -- v1.2, bounded memory validated |
| Lazy key_type tagging for Maps | HM let-generalization prevents type resolution at Map.new() | ✓ Good -- runtime dispatch at put/get sites |
| Pipe-aware type inference | infer_pipe handles CallExpr RHS, prepends lhs_ty before arity check | ✓ Good -- enables pipe+closure chains |
| panic!() instead of abort() | catch_unwind requires catchable panics for crash isolation | ✓ Good -- actors survive peer crashes |
| Fun() as text-comparison, not keyword | Type-position disambiguation only; avoids breaking existing code | ✓ Good -- v1.2, clean integration with HM |
| Conservative stack scanning | No type maps yet; every 8-byte word treated as potential pointer | ✓ Good -- safe, may retain some garbage |
| GC at yield points only | Cooperative; never interrupts other actors | ✓ Good -- per-actor isolation preserved |
| Static dispatch for traits | Monomorphization fits LLVM codegen naturally, no runtime vtable overhead | ✓ Good -- v1.3, zero-overhead trait calls |
| MIR lowering as trait integration point | Type checker resolves concrete types; MIR mangles names and emits direct calls | ✓ Good -- v1.3, clean separation of concerns |
| Trait__Method__Type name mangling | Double-underscore separators extend existing mangle_type_name infrastructure | ✓ Good -- v1.3, consistent naming scheme |
| FNV-1a for Hash protocol | Deterministic, platform-independent, ~35 lines in snow-rt | ✓ Good -- v1.3, zero new Rust dependencies |
| Trust typeck for where-clause enforcement | Type checker already comprehensively checks; MIR adds warning-only defense-in-depth | ✓ Good -- v1.3, no duplicate checking logic |
| deriving as contextual keyword | IDENT text check avoids adding to TokenKind; backward compatible | ✓ Good -- v1.3, no breaking changes |
| Thread sum_type_defs as parameter | PatMatrix cloned frequently; reference avoids data duplication | ✓ Good -- v1.4, correct tag resolution |
| Ordering as non-generic built-in | Simpler than Option/Result; no type parameters needed | ✓ Good -- v1.4, clean Ord integration |
| Synthetic wrapper functions | Runtime expects fn(u64)->ptr; wrappers bridge two-arg calls to one-arg callback | ✓ Good -- v1.4, enables nested Display |
| Lazy monomorphization at struct literal sites | Generate trait functions on demand when generic type instantiated | ✓ Good -- v1.4, correct field type substitution |
| Clone-locally fn_constraints | Avoids &mut cascade to 10+ callers; cloning small map is cheap | ✓ Good -- v1.4, contained mutability |
| ListLit MIR + snow_list_from_array | Single allocation O(n) vs O(n^2) append chain for list literals | ✓ Good -- v1.5, efficient list creation |
| Uniform u64 storage with codegen conversion | No runtime type tags; all conversion at compile time | ✓ Good -- v1.5, zero-overhead polymorphism |
| Callback-based list Eq/Ord | Matches snow_list_to_string pattern; runtime receives fn ptr | ✓ Good -- v1.5, consistent callback architecture |
| ListDecons decision tree node | Cons patterns need runtime length check + extraction; doesn't fit Switch/Test | ✓ Good -- v1.5, clean pattern compilation |
| Local var precedence over builtin names | Pattern binding `head` was incorrectly mapped to snow_list_head | ✓ Good -- v1.5, correct name resolution |
| E0029 error + early-return for Ord without Eq | User opted into selective deriving; respect with clear error and suggestion | ✓ Good -- v1.5, user-friendly diagnostics |
| Soft error collection for argument constraints | Callee check returns Err; argument check uses extend to avoid aborting inference early | ✓ Good -- v1.5, non-disruptive constraint checking |
| NameRef-only argument constraint checking | Covers direct names and let aliases; complex expressions out of scope | ✓ Good -- v1.5, practical coverage |
| Retry-based method resolution | Normal inference first, method-call fallback on NoSuchField; preserves backward compat | ✓ Good -- v1.6, zero regressions |
| Method as last in resolution priority | module > service > variant > struct field > method; method is fallback | ✓ Good -- v1.6, no existing syntax affected |
| Shared resolve_trait_callee helper | Eliminates duplication between bare-name and dot-syntax dispatch | ✓ Good -- v1.6, single maintenance point |
| Stdlib module method fallback | Maps receiver type to module name (String, List, Map, Set, Range) | ✓ Good -- v1.6, dot-syntax for stdlib functions |
| Defense-in-depth sort in MIR | Sort matching_traits before selection, independent of typeck ambiguity check | ✓ Good -- v1.6, deterministic regardless of HashMap order |
| AmbiguousMethod with TextRange span | Consistent with other span-bearing error variants | ✓ Good -- v1.6, precise error locations |
| InferCtx.loop_depth for break/continue | Threading through 55+ signatures too invasive; field on context is clean | ✓ Good -- v1.7, simple loop validation |
| Reset loop_depth in closures | BRKC-05 requires boundary enforcement; reset to 0 in closure bodies | ✓ Good -- v1.7, correct closure semantics |
| alloca counter for loop state | mem2reg promotes to register; matches existing if-expression pattern | ✓ Good -- v1.7, zero-overhead loops |
| Indexed iteration for collections | Counter 0..len avoids Rust iterator complexity; works for List/Map/Set | ✓ Good -- v1.7, uniform codegen |
| List builder for comprehensions | Pre-allocated O(N) vs O(N^2) append chains for for-in results | ✓ Good -- v1.7, efficient collection |
| Half-open range [start, end) | Consistent with Rust/Python; SLT comparison for termination | ✓ Good -- v1.7, familiar semantics |
| Five-block codegen for filter | Filter false skips to latch directly; clean separation from body | ✓ Good -- v1.7, minimal overhead |
| ForInRange returns List<T> not Unit | Comprehension semantics apply uniformly to all for-in variants | ✓ Good -- v1.7, consistent behavior |
| Hand-written Kahn's algorithm for toposort | Avoids petgraph dependency for simple DAG | ✓ Good -- v1.8, zero new dependencies |
| Sequential u32 ModuleId | Simple, zero-allocation, direct Vec indexing | ✓ Good -- v1.8, efficient module lookup |
| Two-phase graph construction | Register all modules first, then parse and build edges | ✓ Good -- v1.8, correct forward references |
| Single LLVM module via MIR merge | Avoids cross-module linking complexity | ✓ Good -- v1.8, single binary output |
| Accumulator-pattern type checking | Each module's exports feed into next module's ImportContext | ✓ Good -- v1.8, correct dependency ordering |
| Module-qualified name mangling (ModuleName__fn) | Double-underscore separators prevent private name collisions | ✓ Good -- v1.8, safe multi-module codegen |
| TyCon::display_prefix for module-qualified types | Excluded from PartialEq/Hash to preserve type identity | ✓ Good -- v1.8, display-only qualification |
| ariadne named-source spans | (String, Range) spans replace anonymous Source::from() for file-aware diagnostics | ✓ Good -- v1.8, file paths in errors |
| Trait impls unconditionally exported | XMOD-05: global visibility without explicit import | ✓ Good -- v1.8, coherent trait dispatch |
| PrivateItem error with pub suggestion | Clear diagnostic when accessing non-pub items across modules | ✓ Good -- v1.8, user-friendly errors |

---
*Last updated: 2026-02-09 after v1.9 milestone started*
