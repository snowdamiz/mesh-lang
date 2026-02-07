# Roadmap: Snow

## Overview

Snow is a statically typed, LLVM-compiled programming language with Elixir-style syntax and BEAM-style actor concurrency. The roadmap follows the critical dependency chain: Lexer -> Parser -> Type System -> Pattern Matching -> Codegen -> Actor Runtime -> Supervision -> Standard Library -> Tooling. Each phase delivers a verifiable capability that the next phase builds on. The compiler pipeline (Phases 1-5) must produce working native binaries for sequential code before actor concurrency is introduced (Phases 6-7), because debugging two hard problems simultaneously (type system + runtime) causes compounding failures.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Project Foundation & Lexer** - Reproducible build, test infrastructure, and tokenization of Snow source code
- [x] **Phase 2: Parser & AST** - Recursive descent parser producing a complete abstract syntax tree for all Snow syntax
- [x] **Phase 3: Type System** - Hindley-Milner type inference with generics, structs, traits, Option/Result types
- [x] **Phase 4: Pattern Matching & Algebraic Data Types** - Exhaustive pattern matching, sum types, and guards
- [x] **Phase 5: LLVM Codegen & Native Binaries** - Full compilation pipeline producing native single-binary executables for sequential code
- [x] **Phase 6: Actor Runtime** - Lightweight actor processes with typed message passing, standalone runtime library integrated with compiler
- [x] **Phase 7: Supervision & Fault Tolerance** - Supervision trees with restart strategies and let-it-crash semantics
- [x] **Phase 8: Standard Library** - Core library for I/O, strings, collections, file operations, HTTP, and JSON
- [x] **Phase 9: Concurrency Standard Library** - Service (GenServer) and Job (Task) high-level concurrency abstractions
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
- [x] 02-01-PLAN.md -- Parser crate scaffolding, SyntaxKind enum, rowan CST types, event-based Parser struct
- [x] 02-02-PLAN.md -- Pratt expression parser with operator precedence, literals, calls, field access, pipe
- [x] 02-03-PLAN.md -- Compound expressions (if/else, case/match, closures, blocks) and let/return statements
- [x] 02-04-PLAN.md -- Declarations (fn, module, import, struct), patterns, type annotations, visibility
- [x] 02-05-PLAN.md -- Typed AST wrappers, public parse() API, comprehensive snapshot tests

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
**Plans**: 5 plans

Plans:
- [x] 03-01-PLAN.md -- Parser migration (angle brackets, interface/impl/type alias syntax), snow-typeck crate with type representation, unification, environment, builtins
- [x] 03-02-PLAN.md -- Algorithm J inference engine with let-polymorphism and occurs check (TDD)
- [x] 03-03-PLAN.md -- Structs, Option<T>, Result<T, E>, type aliases (TDD)
- [x] 03-04-PLAN.md -- Traits/interfaces with where-clause constraints and compiler-known operator traits (TDD)
- [x] 03-05-PLAN.md -- ariadne diagnostic rendering, fix suggestions, end-to-end phase verification

### Phase 4: Pattern Matching & Algebraic Data Types
**Goal**: Exhaustive pattern matching compilation with algebraic data types (sum types), guards, and compile-time warnings for missing or redundant patterns
**Depends on**: Phase 3
**Requirements**: LANG-05, LANG-06, LANG-11
**Success Criteria** (what must be TRUE):
  1. Sum types (enums with data variants) can be defined, constructed, and destructured via pattern matching with full type inference
  2. The compiler warns when a match expression does not cover all variants of a sum type (exhaustiveness checking)
  3. The compiler warns when a pattern arm is unreachable (redundancy checking)
  4. Guards (`when` clauses) work in match arms and function heads, with the type checker understanding guard implications
**Plans**: 5 plans

Plans:
- [x] 04-01-PLAN.md -- Lexer BAR token, parser sum type definitions, extended pattern syntax (constructor/or/as), typed AST wrappers
- [x] 04-02-PLAN.md -- Type checking for sum types, variant constructors, constructor/or/as pattern inference
- [x] 04-03-PLAN.md -- Maranget's exhaustiveness and redundancy algorithm (TDD)
- [x] 04-04-PLAN.md -- Exhaustiveness/redundancy wiring, guard validation, multi-clause functions
- [x] 04-05-PLAN.md -- Diagnostic rendering, Option/Result migration to sum types, end-to-end integration

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
**Plans**: 5 plans

Plans:
- [x] 05-01-PLAN.md -- Runtime crate (snow-rt), codegen crate scaffolding, TypeckResult exposure, LLVM build config
- [x] 05-02-PLAN.md -- MIR type system, AST-to-MIR lowering, pipe/interpolation desugaring, closure conversion, monomorphization
- [x] 05-03-PLAN.md -- Pattern match compilation to decision trees (TDD)
- [x] 05-04-PLAN.md -- LLVM codegen via Inkwell (type layouts, expressions, control flow, closures, pattern matches)
- [x] 05-05-PLAN.md -- snowc build CLI, linking, end-to-end integration tests

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
**Plans**: 7 plans

Plans:
- [x] 06-01-PLAN.md -- M:N work-stealing scheduler, Process Control Block, corosensei coroutines, reduction counting
- [x] 06-02-PLAN.md -- Compiler frontend: actor keyword, parser for actor/spawn/send/receive/self, MIR extensions
- [x] 06-03-PLAN.md -- Per-actor heaps, FIFO mailbox, message deep-copy, send/receive with scheduler blocking
- [x] 06-04-PLAN.md -- Typed Pid<M> in type checker, compile-time send validation, actor type errors
- [x] 06-05-PLAN.md -- AST-to-MIR lowering for actors, LLVM codegen for actor primitives, reduction check instrumentation
- [x] 06-06-PLAN.md -- Process linking, exit signal propagation, named process registry
- [x] 06-07-PLAN.md -- E2E integration tests, 100K actor benchmark, success criteria verification

### Phase 7: Supervision & Fault Tolerance
**Goal**: OTP-style supervision trees with restart strategies, enabling the let-it-crash philosophy with automatic recovery from actor failures
**Depends on**: Phase 6
**Requirements**: CONC-05, CONC-06, CONC-07
**Success Criteria** (what must be TRUE):
  1. A supervisor configured with `one_for_one` strategy automatically restarts a crashed child actor while leaving siblings running
  2. A supervisor configured with `one_for_all` restarts all children when any one crashes
  3. Restart limits prevent infinite crash loops (supervisor itself terminates after exceeding max restarts in a time window)
  4. Typed supervision validates child specifications at compile time (the supervisor knows the message types of its children)
**Plans**: 3 plans

Plans:
- [x] 07-01-PLAN.md -- Supervisor runtime: ExitReason expansion, child spec types, SupervisorState, all four strategies, restart limits, ordered shutdown, extern "C" ABI
- [x] 07-02-PLAN.md -- Compiler integration: parser (supervisor blocks), AST, type checker, MIR SupervisorStart, LLVM codegen, intrinsics, E2E smoke test
- [x] 07-03-PLAN.md -- Typed supervision: compile-time child spec validation (E0018-E0021), E2E tests for all success criteria

### Phase 8: Standard Library
**Goal**: A core standard library providing I/O, string operations, collections, file access, HTTP, and JSON -- enough to build real web backends and CLI tools
**Depends on**: Phase 5 (sequential stdlib), Phase 6 (for any actor-based stdlib)
**Requirements**: STD-01, STD-02, STD-03, STD-04, STD-05, STD-06, STD-09
**Success Criteria** (what must be TRUE):
  1. A Snow program can read a file, process its contents with string and list operations, and write output to another file
  2. A Snow program can start an HTTP server that accepts requests and returns JSON responses
  3. List operations (map, filter, reduce) and Map (hash map) operations work with full type inference and pipe operator chaining
  4. Standard I/O (print, read from stdin) works for interactive CLI programs
**Plans**: 7 plans

Plans:
- [x] 08-01-PLAN.md -- String operations, console I/O, Env access, and module/import namespace resolution
- [x] 08-02-PLAN.md -- Collections (List with map/filter/reduce, Map/HashMap, Set)
- [x] 08-03-PLAN.md -- File I/O with Result types
- [x] 08-04-PLAN.md -- JSON encoding/decoding with serde_json
- [x] 08-05-PLAN.md -- HTTP client and server with thread-per-connection model
- [x] 08-06-PLAN.md -- Gap closure: pipe chain E2E test with closures, IO.read_line E2E test
- [x] 08-07-PLAN.md -- Gap closure: HTTP server runtime E2E test (start server, make request, verify response)

### Phase 9: Concurrency Standard Library
**Goal**: High-level concurrency abstractions (Service and Job) built on the actor primitives, providing ergonomic patterns for common concurrent programming needs
**Depends on**: Phase 7, Phase 8
**Requirements**: STD-07, STD-08
**Success Criteria** (what must be TRUE):
  1. A Service can be defined with init/call/cast callbacks, started under a supervisor, and interacted with via synchronous calls and asynchronous casts
  2. A Job can be spawned to perform work asynchronously and its result awaited, with supervision ensuring the job is restarted on failure
  3. Both Service and Job are fully type-checked (message types, return types) with inference
**Plans**: 5 plans

Plans:
- [x] 09-01-PLAN.md -- Service syntax: lexer keywords (service/call/cast), parser for service blocks, AST wrappers
- [x] 09-02-PLAN.md -- Type checking: infer_service_def with state unification, per-variant reply types, Job module registration
- [x] 09-03-PLAN.md -- Service runtime and codegen: snow_service_call/reply, MIR desugaring to actor primitives, LLVM wiring
- [x] 09-04-PLAN.md -- Job runtime and codegen: snow_job_async/await/map, closure argument handling, Result returns
- [x] 09-05-PLAN.md -- E2E integration tests: Service counter/state management, Job async/await, type error verification

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
**Plans**: 10 plans

Plans:
- [ ] 10-01-PLAN.md -- Error message polish (multi-span diagnostics, --json output, fix suggestions)
- [ ] 10-02-PLAN.md -- Formatter core (snow-fmt crate, FormatIR, CST walker, printer)
- [ ] 10-03-PLAN.md -- Formatter CLI (snowc fmt, --check mode, idempotency tests)
- [ ] 10-04-PLAN.md -- REPL core (snow-repl crate, JIT engine, session management, multi-line)
- [ ] 10-05-PLAN.md -- REPL actor support and snowc repl CLI integration
- [ ] 10-06-PLAN.md -- Package manager core (snow-pkg crate, snow.toml, dependency resolution)
- [ ] 10-07-PLAN.md -- Package manager CLI (snowc init, snowc deps)
- [ ] 10-08-PLAN.md -- LSP server core (snow-lsp crate, diagnostics, type-on-hover)
- [ ] 10-09-PLAN.md -- LSP go-to-definition and integration tests
- [ ] 10-10-PLAN.md -- E2E integration tests and human verification

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7 -> 8 -> 9 -> 10

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Project Foundation & Lexer | 3/3 | Complete | 2026-02-06 |
| 2. Parser & AST | 5/5 | Complete | 2026-02-06 |
| 3. Type System | 5/5 | Complete | 2026-02-06 |
| 4. Pattern Matching & ADTs | 5/5 | Complete | 2026-02-06 |
| 5. LLVM Codegen & Native Binaries | 5/5 | Complete | 2026-02-06 |
| 6. Actor Runtime | 7/7 | Complete | 2026-02-07 |
| 7. Supervision & Fault Tolerance | 3/3 | Complete | 2026-02-06 |
| 8. Standard Library | 7/7 | Complete | 2026-02-07 |
| 9. Concurrency Standard Library | 5/5 | Complete | 2026-02-07 |
| 10. Developer Tooling | 0/10 | Not started | - |
