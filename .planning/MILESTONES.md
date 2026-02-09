# Project Milestones: Snow

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
