# Roadmap: Snow

## Overview

Snow is a statically typed, LLVM-compiled programming language with Elixir-style syntax and BEAM-style actor concurrency. The roadmap follows the critical dependency chain: Lexer -> Parser -> Type System -> Pattern Matching -> Codegen -> Actor Runtime -> Supervision -> Standard Library -> Tooling. Each phase delivers a verifiable capability that the next phase builds on. The compiler pipeline (Phases 1-5) must produce working native binaries for sequential code before actor concurrency is introduced (Phases 6-7), because debugging two hard problems simultaneously (type system + runtime) causes compounding failures.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Project Foundation & Lexer** - Reproducible build, test infrastructure, and tokenization of Snow source code
- [ ] **Phase 2: Parser & AST** - Recursive descent parser producing a complete abstract syntax tree for all Snow syntax
- [ ] **Phase 3: Type System** - Hindley-Milner type inference with generics, structs, traits, Option/Result types
- [ ] **Phase 4: Pattern Matching & Algebraic Data Types** - Exhaustive pattern matching, sum types, and guards
- [ ] **Phase 5: LLVM Codegen & Native Binaries** - Full compilation pipeline producing native single-binary executables for sequential code
- [ ] **Phase 6: Actor Runtime** - Lightweight actor processes with typed message passing, standalone runtime library integrated with compiler
- [ ] **Phase 7: Supervision & Fault Tolerance** - Supervision trees with restart strategies and let-it-crash semantics
- [ ] **Phase 8: Standard Library** - Core library for I/O, strings, collections, file operations, HTTP, and JSON
- [ ] **Phase 9: Concurrency Standard Library** - GenServer, Task, and other OTP-style behavior abstractions
- [ ] **Phase 10: Developer Tooling** - Error message polish, formatter, REPL, package manager, and LSP server

## Phase Details

### Phase 1: Project Foundation & Lexer
**Goal**: A reproducible Rust workspace with pinned LLVM 18, snapshot test infrastructure, and a lexer that tokenizes all Snow syntax with accurate span tracking
**Depends on**: Nothing (first phase)
**Requirements**: LANG-09
**Success Criteria** (what must be TRUE):
  1. Running `cargo build` on a fresh clone produces a successful build with LLVM 18 linked (no manual LLVM setup beyond documented steps)
  2. The lexer tokenizes a Snow source file containing all token types (keywords, operators, literals, identifiers, comments, string interpolation markers) and produces a correct token stream verified by snapshot tests
  3. Every token carries accurate source span information (line, column, byte offset) enabling error messages to point at the right location
  4. The test suite runs via `cargo test` with snapshot tests (insta) covering at least the full token vocabulary and error recovery cases
**Plans**: 3 plans

Plans:
- [x] 01-01-PLAN.md -- Rust workspace, shared types (TokenKind, Span, LexError), and test infrastructure
- [x] 01-02-PLAN.md -- Core lexer with Cursor, keywords, operators, numbers, identifiers, simple strings
- [x] 01-03-PLAN.md -- String interpolation, block comments, newlines, error recovery, full test suite

### Phase 2: Parser & AST
**Goal**: A recursive descent parser that transforms token streams into a lossless CST and typed AST representing all Snow language constructs, with human-readable parse error messages
**Depends on**: Phase 1
**Requirements**: LANG-01, LANG-02, LANG-03, LANG-04, LANG-07, LANG-08, LANG-10, ORG-01, ORG-02, ORG-03
**Success Criteria** (what must be TRUE):
  1. Snow source code with `let` bindings, function definitions (with `do/end` blocks), `if/else`, `case/match`, closures, pipe operator, string interpolation, and module/import declarations parses into a correct AST verified by snapshot tests
  2. Parse errors produce messages identifying the location and nature of the problem (e.g., "expected `end` to close `do` block started at line 5") rather than generic "unexpected token" errors
  3. The AST preserves enough structure for downstream type checking and codegen (function signatures, expression nesting, module boundaries, visibility modifiers)
  4. Pattern syntax (match arms, function parameter patterns) parses correctly even though exhaustiveness checking comes later
**Plans**: 5 plans

Plans:
- [ ] 02-01-PLAN.md -- Parser crate scaffolding, SyntaxKind enum, rowan CST types, event-based Parser struct
- [ ] 02-02-PLAN.md -- Pratt expression parser with operator precedence, literals, calls, field access, pipe
- [ ] 02-03-PLAN.md -- Compound expressions (if/else, case/match, closures, blocks) and let/return statements
- [ ] 02-04-PLAN.md -- Declarations (fn, module, import, struct), patterns, type annotations, visibility
- [ ] 02-05-PLAN.md -- Typed AST wrappers, public parse() API, comprehensive snapshot tests

### Phase 3: Type System
**Goal**: A Hindley-Milner type inference engine that type-checks Snow programs without requiring type annotations, supporting generics, structs, traits, and Option/Result types
**Depends on**: Phase 2
**Requirements**: TYPE-01, TYPE-02, TYPE-03, TYPE-04, TYPE-05, TYPE-06, TYPE-08
**Success Criteria** (what must be TRUE):
  1. `let id = fn x -> x end` followed by `(id(1), id("hello"))` type-checks successfully, proving let-polymorphism works (the identity function is used at both Int and String)
  2. `fn x -> x(x) end` is rejected with a type error referencing the occurs check, proving infinite type prevention works
  3. Struct types, Option[T], and Result[T, E] can be defined and used with full type inference (no annotations needed for straightforward usage)
  4. Trait definitions and implementations type-check correctly, enabling polymorphic function dispatch based on trait constraints
  5. Type errors include the source location of the conflict and the inferred types involved (e.g., "expected Int, found String at line 12, column 5")
**Plans**: TBD

Plans:
- [ ] 03-01: Type representation, unification, and constraint generation infrastructure (ena-based)
- [ ] 03-02: Algorithm W implementation with let-polymorphism and occurs check
- [ ] 03-03: Structs, Option[T], Result[T, E], type aliases
- [ ] 03-04: Traits/protocols with constraint-based polymorphism
- [ ] 03-05: Type error reporting with source locations

### Phase 4: Pattern Matching & Algebraic Data Types
**Goal**: Exhaustive pattern matching compilation with algebraic data types (sum types), guards, and compile-time warnings for missing or redundant patterns
**Depends on**: Phase 3
**Requirements**: LANG-05, LANG-06, LANG-11
**Success Criteria** (what must be TRUE):
  1. Sum types (enums with data variants) can be defined, constructed, and destructured via pattern matching with full type inference
  2. The compiler warns when a match expression does not cover all variants of a sum type (exhaustiveness checking)
  3. The compiler warns when a pattern arm is unreachable (redundancy checking)
  4. Guards (`when` clauses) work in match arms and function heads, with the type checker understanding guard implications
**Plans**: TBD

Plans:
- [ ] 04-01: Algebraic data type representation and type checking integration
- [ ] 04-02: Pattern matching exhaustiveness and redundancy checking (Maranget's algorithm)
- [ ] 04-03: Guards in pattern matching and function heads

### Phase 5: LLVM Codegen & Native Binaries
**Goal**: The complete compilation pipeline from Snow source to native single-binary executables, producing correct and runnable programs for all sequential language features
**Depends on**: Phase 4
**Requirements**: COMP-01, COMP-02, COMP-03, TOOL-01
**Success Criteria** (what must be TRUE):
  1. `snowc build hello.snow` produces a native executable that, when run, prints "Hello, World!" to stdout
  2. A Snow program using functions, pattern matching, algebraic data types, closures, pipe operator, and string interpolation compiles and runs correctly with the expected output
  3. The output is a single binary with no external runtime dependencies (runtime statically linked)
  4. The compiler produces binaries on both macOS and Linux from the same Snow source code
  5. Compilation of a 100-line Snow program completes in under 5 seconds (including LLVM codegen at -O0)
**Plans**: TBD

Plans:
- [ ] 05-01: Mid-level IR design and lowering from typed AST
- [ ] 05-02: Pattern matching compilation to decision trees
- [ ] 05-03: LLVM codegen via Inkwell (functions, control flow, data types, closures)
- [ ] 05-04: Binary linking, runtime stub, and `snowc build` CLI
- [ ] 05-05: Cross-platform support (macOS + Linux) and end-to-end integration tests

### Phase 6: Actor Runtime
**Goal**: Lightweight actor processes with typed message passing, a work-stealing scheduler, and per-actor isolation, integrated into compiled Snow programs
**Depends on**: Phase 5
**Requirements**: CONC-01, CONC-02, CONC-03, CONC-04, TYPE-07
**Success Criteria** (what must be TRUE):
  1. A Snow program can spawn 100,000 actors that each hold state and respond to messages, completing without crashing or exhausting memory (demonstrating lightweight processes)
  2. Sending a message of the wrong type to a typed `Pid[MessageType]` is rejected at compile time
  3. An actor running an infinite computation does not prevent other actors from making progress (preemptive scheduling via yield points)
  4. A `receive` block with pattern matching correctly dispatches incoming messages to the matching arm
  5. Process linking works: when a linked actor crashes, the linked partner receives an exit signal
**Plans**: TBD

Plans:
- [ ] 06-01: Standalone actor runtime library (libsnowrt) with scheduler and process model
- [ ] 06-02: Message passing infrastructure (typed mailboxes, MPSC queues, per-actor heaps)
- [ ] 06-03: Compiler integration (spawn/send/receive codegen, yield point insertion)
- [ ] 06-04: Typed actor PIDs (Pid[MessageType]) and compile-time protocol checking
- [ ] 06-05: Process linking, monitoring, and exit signal propagation

### Phase 7: Supervision & Fault Tolerance
**Goal**: OTP-style supervision trees with restart strategies, enabling the let-it-crash philosophy with automatic recovery from actor failures
**Depends on**: Phase 6
**Requirements**: CONC-05, CONC-06, CONC-07
**Success Criteria** (what must be TRUE):
  1. A supervisor configured with `one_for_one` strategy automatically restarts a crashed child actor while leaving siblings running
  2. A supervisor configured with `one_for_all` restarts all children when any one crashes
  3. Restart limits prevent infinite crash loops (supervisor itself terminates after exceeding max restarts in a time window)
  4. Typed supervision validates child specifications at compile time (the supervisor knows the message types of its children)
**Plans**: TBD

Plans:
- [ ] 07-01: Supervisor actor implementation (strategies, child specs, restart logic)
- [ ] 07-02: Exit signal propagation, trap exits, and restart policies (permanent/transient/temporary)
- [ ] 07-03: Typed supervision and compile-time child spec validation

### Phase 8: Standard Library
**Goal**: A core standard library providing I/O, string operations, collections, file access, HTTP, and JSON -- enough to build real web backends and CLI tools
**Depends on**: Phase 5 (sequential stdlib), Phase 6 (for any actor-based stdlib)
**Requirements**: STD-01, STD-02, STD-03, STD-04, STD-05, STD-06, STD-09
**Success Criteria** (what must be TRUE):
  1. A Snow program can read a file, process its contents with string and list operations, and write output to another file
  2. A Snow program can start an HTTP server that accepts requests and returns JSON responses
  3. List operations (map, filter, reduce) and Map (hash map) operations work with full type inference and pipe operator chaining
  4. Standard I/O (print, read from stdin) works for interactive CLI programs
**Plans**: TBD

Plans:
- [ ] 08-01: Core types and I/O (print, read, String operations)
- [ ] 08-02: Collections (List with map/filter/reduce, Map/HashMap)
- [ ] 08-03: File I/O
- [ ] 08-04: HTTP client and server
- [ ] 08-05: JSON encoding/decoding

### Phase 9: Concurrency Standard Library
**Goal**: High-level concurrency abstractions (GenServer, Task) built on the actor primitives, providing ergonomic patterns for common concurrent programming needs
**Depends on**: Phase 7, Phase 8
**Requirements**: STD-07, STD-08
**Success Criteria** (what must be TRUE):
  1. A GenServer can be defined with init/handle_call/handle_cast callbacks, started under a supervisor, and interacted with via synchronous calls and asynchronous casts
  2. A Task can be spawned to perform work asynchronously and its result awaited, with supervision ensuring the task is restarted on failure
  3. Both GenServer and Task are fully type-checked (message types, return types) with inference
**Plans**: TBD

Plans:
- [ ] 09-01: GenServer behavior (init, handle_call, handle_cast, typed interface)
- [ ] 09-02: Task abstraction (async spawn, await, supervision integration)

### Phase 10: Developer Tooling
**Goal**: Developer tools that make Snow practical for daily use -- polished error messages, code formatter, REPL, package manager, and LSP server
**Depends on**: Phase 5 (compiler must work), Phase 8 (stdlib for REPL usability)
**Requirements**: TOOL-02, TOOL-03, TOOL-04, TOOL-05, TOOL-06
**Success Criteria** (what must be TRUE):
  1. Compiler error messages consistently identify the problem, show the relevant source code with underlined spans, and suggest fixes where possible (Elm/Rust quality standard)
  2. `snowc fmt` formats Snow source code to a canonical style, and formatting is idempotent (formatting already-formatted code produces identical output)
  3. `snowc repl` starts an interactive session where expressions can be evaluated, types are displayed, and previous results are accessible
  4. A package manager can initialize a project, declare dependencies, and resolve/fetch them
  5. An LSP server provides diagnostics, go-to-definition, and type-on-hover in editors
**Plans**: TBD

Plans:
- [ ] 10-01: Error message polish (multi-span diagnostics, suggestions, Elm/Rust quality)
- [ ] 10-02: Code formatter (AST-based pretty printer)
- [ ] 10-03: REPL (interpreter mode or incremental compilation)
- [ ] 10-04: Package manager
- [ ] 10-05: LSP server (salsa + rowan for incremental parsing)

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7 -> 8 -> 9 -> 10

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Project Foundation & Lexer | 3/3 | Complete | 2026-02-06 |
| 2. Parser & AST | 0/5 | Not started | - |
| 3. Type System | 0/5 | Not started | - |
| 4. Pattern Matching & ADTs | 0/3 | Not started | - |
| 5. LLVM Codegen & Native Binaries | 0/5 | Not started | - |
| 6. Actor Runtime | 0/5 | Not started | - |
| 7. Supervision & Fault Tolerance | 0/3 | Not started | - |
| 8. Standard Library | 0/5 | Not started | - |
| 9. Concurrency Standard Library | 0/2 | Not started | - |
| 10. Developer Tooling | 0/5 | Not started | - |
