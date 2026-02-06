# Snow

## What This Is

Snow is a programming language that combines Elixir/Ruby-style expressive syntax with static type inference and BEAM-style concurrency (actors, supervision trees, fault tolerance), compiled via LLVM to native single-binary executables. The compiler is written in Rust.

## Core Value

Expressive, readable concurrency — writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] Elixir/Ruby-style syntax (do/end blocks, pattern matching, keyword-based, minimal punctuation)
- [ ] Static type system with inference (rarely write type annotations, compiler figures it out)
- [ ] BEAM-style concurrency: lightweight actor processes with message passing
- [ ] Supervision trees with let-it-crash fault tolerance
- [ ] LLVM backend producing native single-binary executables (runtime bundled)
- [ ] Functional-first paradigm (no classes, no OOP hierarchies)
- [ ] General purpose — suitable for web backends/APIs and CLI tools
- [ ] Pattern matching as a core language feature
- [ ] Standard library sufficient for HTTP servers and file I/O

### Out of Scope

- Classes and OOP — functional-first design, use structs/traits/protocols instead
- Systems programming (drivers, embedded, OS-level) — not targeting bare-metal performance
- GUI framework — web and CLI are the primary targets
- Self-hosting compiler — Rust is the compiler language, bootstrapping is not a v1 goal

## Context

- Fills a gap between Elixir (dynamic types, BEAM-only deployment) and Go (static types, native binaries, but primitive concurrency model)
- The BEAM VM's actor model and supervision philosophy is proven at scale (Erlang/Elixir in telecom, WhatsApp, Discord) but has never been paired with static types and native compilation
- Compiler written in Rust using LLVM bindings (inkwell/llvm-sys)
- Learning project with real-world ambitions — deep dive into compiler construction and language design while building something genuinely usable
- v1 target is a language usable for real web backends with concurrency, not just a toy

## Constraints

- **Compiler language**: Rust — chosen for safety, LLVM ecosystem (inkwell), and compiler development ergonomics
- **Compilation target**: LLVM IR — enables native binaries across platforms without writing multiple backends
- **No OOP**: Functional paradigm only — structs, traits/protocols, pattern matching. No class hierarchies.
- **Runtime**: Actor runtime must be bundled into the binary. Lightweight enough to not bloat small CLI tools.

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Rust for compiler | Strong LLVM bindings, memory safe, good for complex software | — Pending |
| LLVM as backend | Proven codegen, multi-platform, avoids writing own backend | — Pending |
| Elixir/Ruby syntax style | Expressive, readable, pattern matching native. Matches language philosophy | — Pending |
| Static types with inference | Safety of static types without verbosity. Hindley-Milner or similar | — Pending |
| No OOP | Functional-first aligns with actor model. Simpler language design | — Pending |
| Bundled runtime | Single binary deployment like Go, but with actor scheduler inside | — Pending |

---
*Last updated: 2026-02-05 after initialization*
