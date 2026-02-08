# Project Milestones: Snow

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
