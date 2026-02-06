# Requirements: Snow

**Defined:** 2026-02-05
**Core Value:** Expressive, readable concurrency — writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.

## v1 Requirements

Requirements for initial release. Each maps to roadmap phases.

### Core Language

- [ ] **LANG-01**: Variables with `let` bindings, immutable by default
- [ ] **LANG-02**: Functions with parameters and return values (first-class)
- [ ] **LANG-03**: `do/end` block syntax (Elixir/Ruby-style)
- [ ] **LANG-04**: Control flow (`if/else`, `case/match`) as expressions
- [ ] **LANG-05**: Pattern matching with exhaustiveness checking
- [ ] **LANG-06**: Algebraic data types (sum types + product types/structs)
- [ ] **LANG-07**: String interpolation (`"Hello, #{name}"`)
- [ ] **LANG-08**: Closures / anonymous functions with scope capture
- [ ] **LANG-09**: Comments (`# line comment`)
- [ ] **LANG-10**: Pipe operator (`|>`) with type inference
- [ ] **LANG-11**: Guards in pattern matching

### Type System

- [ ] **TYPE-01**: Static type checking with Hindley-Milner inference
- [ ] **TYPE-02**: Parametric polymorphism (generics) — inferred, not annotated
- [ ] **TYPE-03**: `Option[T]` type (no null/nil)
- [ ] **TYPE-04**: `Result[T, E]` type with `?` propagation operator
- [ ] **TYPE-05**: Struct types (named product types)
- [ ] **TYPE-06**: Type aliases
- [ ] **TYPE-07**: Typed actor PIDs (`Pid[MessageType]`) — compile-time protocol checking
- [ ] **TYPE-08**: Traits/protocols for polymorphism

### Concurrency

- [ ] **CONC-01**: Lightweight actor processes (millions, not OS threads)
- [ ] **CONC-02**: Typed message passing via `send`
- [ ] **CONC-03**: `receive` blocks with pattern matching
- [ ] **CONC-04**: Process linking and monitoring
- [ ] **CONC-05**: Supervision trees (one_for_one, one_for_all, rest_for_one)
- [ ] **CONC-06**: Let-it-crash with automatic restarts (permanent/transient/temporary)
- [ ] **CONC-07**: Typed supervision (compile-time child spec validation)

### Code Organization

- [ ] **ORG-01**: Module system with namespacing
- [ ] **ORG-02**: Import system
- [ ] **ORG-03**: Visibility control (pub/private)

### Standard Library

- [ ] **STD-01**: Standard I/O (print, read)
- [ ] **STD-02**: String operations
- [ ] **STD-03**: List type with operations (map, filter, reduce)
- [ ] **STD-04**: Map type (hash map)
- [ ] **STD-05**: File I/O
- [ ] **STD-06**: HTTP client and server
- [ ] **STD-07**: GenServer behavior
- [ ] **STD-08**: Task abstraction
- [ ] **STD-09**: JSON encoding/decoding

### Tooling

- [ ] **TOOL-01**: Compiler CLI (`snowc build`) producing single native binary
- [ ] **TOOL-02**: Clear, human-readable error messages
- [ ] **TOOL-03**: Code formatter
- [ ] **TOOL-04**: REPL
- [ ] **TOOL-05**: Package manager
- [ ] **TOOL-06**: LSP server

### Compilation

- [ ] **COMP-01**: LLVM backend producing native code
- [ ] **COMP-02**: Single-binary output with bundled runtime
- [ ] **COMP-03**: Cross-platform support (macOS, Linux)

## v2 Requirements

Deferred to future release. Tracked but not in current roadmap.

### Networking

- **NET-01**: Distributed actors across nodes
- **NET-02**: Clustering / node discovery

### Advanced

- **ADV-01**: Hot code reloading (may not be feasible with native compilation)
- **ADV-02**: Macros / metaprogramming
- **ADV-03**: Self-hosting compiler

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Classes/OOP | Functional-first design, conflicts with actor model philosophy |
| Null/nil values | Option[T] is the only way to represent absence |
| Exceptions (try/catch/throw) | Result[T,E] + let-it-crash philosophy replaces them |
| Shared mutable state between actors | Defeats actor model, causes data races |
| Systems programming (drivers, embedded) | Not targeting bare-metal performance |
| GUI framework | Web and CLI are the primary targets |
| Async/await colored functions | Runtime handles concurrency transparently |
| Operator overloading | Creates hidden semantics, hurts readability |
| Inheritance | Functional paradigm uses composition + traits instead |
| Manual memory management | Per-actor GC handles this; target use cases don't need it |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| (populated by roadmapper) | | |

**Coverage:**
- v1 requirements: 38 total
- Mapped to phases: 0
- Unmapped: 38

---
*Requirements defined: 2026-02-05*
*Last updated: 2026-02-05 after initial definition*
