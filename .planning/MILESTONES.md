# Project Milestones: Snow

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
