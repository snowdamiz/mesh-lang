# Snow

## What This Is

Snow is a programming language that combines Elixir/Ruby-style expressive syntax with static Hindley-Milner type inference and BEAM-style concurrency (actors, supervision trees, fault tolerance), compiled via LLVM to native single-binary executables. The compiler is written in Rust. v1.0 shipped a complete compiler pipeline, actor runtime, standard library, and developer tooling. v1.1 polished the language by resolving all five documented v1.0 limitations. v1.2 added Fun() type annotations and a mark-sweep garbage collector for per-actor heaps. v1.3 completed the trait/protocol system with user-defined interfaces, impl blocks, static dispatch via monomorphization, and six stdlib protocols (Display, Debug, Eq, Ord, Hash, Default) with auto-derive support. v1.4 fixed all five compiler correctness issues: pattern matching codegen, Ordering type, nested collection Display, generic type deriving, and type system soundness for constrained function aliases.

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

### Active

(No active milestone -- planning next)

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

## Context

Shipped v1.4 with 64,548 lines of Rust (+1,359 from v1.3).
Tech stack: Rust compiler, LLVM 21 (Inkwell 0.8), corosensei coroutines, rowan CST, ariadne diagnostics.
Crates: snow-lexer, snow-parser, snow-typeck, snow-mir, snow-codegen, snow-rt, snow-fmt, snow-repl, snow-pkg, snow-lsp, snowc.

1,206 tests passing across all crates. Zero known critical bugs.

Known limitations:
- List type is monomorphic (Int only) -- nested collection Display infrastructure exists but List<List<Int>> cannot be created
- Ord deriving without Eq causes runtime error instead of compile-time error (workaround: derive both)
- Higher-order function argument constraint propagation not supported (e.g., apply(show, value)) -- requires qualified types

## Constraints

- **Compiler language**: Rust -- chosen for safety, LLVM ecosystem (inkwell), and compiler development ergonomics
- **Compilation target**: LLVM IR -- enables native binaries across platforms without writing multiple backends
- **No OOP**: Functional paradigm only -- structs, traits/protocols, pattern matching. No class hierarchies.
- **Runtime**: Actor runtime bundled into the binary. Lightweight enough to not bloat small CLI tools.

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Rust for compiler | Strong LLVM bindings, memory safe, good for complex software | ✓ Good -- 63K LOC Rust, stable compiler |
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

---
*Last updated: 2026-02-08 after v1.4 milestone*
