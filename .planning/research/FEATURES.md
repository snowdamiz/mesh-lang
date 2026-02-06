# Features Research: Snow Language

**Domain:** Compiled programming language with static types, actor concurrency, expressive syntax
**Researched:** 2026-02-05
**Overall confidence:** HIGH (well-established domain with extensive prior art to learn from)

---

## Table Stakes (Must Have)

Features users expect from any modern compiled language. Missing any of these and the language is unusable or dismissed immediately.

### Core Language

| Feature | Why Expected | Complexity | Dependencies | Notes |
|---------|--------------|------------|--------------|-------|
| Variables and bindings (`let`) | Fundamental to any language | Low | Lexer, Parser, AST | Immutable by default (functional-first). Use `let mut` or similar for mutation if ever needed. |
| Functions with parameters and return values | Fundamental | Low | Parser, Type checker | First-class functions required for functional paradigm. |
| `do/end` block syntax | Core identity of Snow (Elixir/Ruby-style) | Medium | Parser | Distinguishes Snow from C-family syntax. This IS the language feel. |
| Control flow (`if/else`, `case/match`) | Fundamental | Low | Parser, Pattern matching | `if/else` as expressions (return values), not statements. |
| Pattern matching (exhaustive) | Table stakes for functional languages in 2025. Rust, Gleam, Elixir, OCaml all have it. | High | Type system, ADTs | Must be exhaustive -- compiler warns on missing cases. Core to the language identity, not an afterthought. |
| Algebraic Data Types (sum + product types) | Foundation for type-safe domain modeling. Every modern functional language has them. | High | Type system | Sum types (`enum`/`union`) + product types (structs/records). Enable "make illegal states unrepresentable." |
| String interpolation | Every modern language has it. Missing = constant annoyance. | Low | Parser, String type | `"Hello, #{name}"` style (Elixir/Ruby convention). |
| Closures / anonymous functions | Expected in any functional language | Medium | Parser, Type inference, Scope capture | `fn(x) -> x + 1 end` or similar. Must capture enclosing scope. |
| Comments (line and block) | Fundamental | Low | Lexer | `# line comment` (Elixir/Ruby style). |
| Module system | Code organization is table stakes for any real project | Medium | Parser, Name resolution | Modules for namespacing. File-based or explicit `module` declarations. |
| Imports | Fundamental for code reuse | Medium | Module system | `import ModuleName` or `use ModuleName`. |
| Integer and Float arithmetic | Fundamental | Low | Codegen | Standard operators: `+`, `-`, `*`, `/`, `%`. Integer division vs float division semantics. |
| Boolean logic | Fundamental | Low | Codegen | `and`, `or`, `not` (word-based, not `&&`/`||` -- Elixir style). |
| Comparison operators | Fundamental | Low | Codegen | `==`, `!=`, `<`, `>`, `<=`, `>=`. |

### Type System

| Feature | Why Expected | Complexity | Dependencies | Notes |
|---------|--------------|------------|--------------|-------|
| Static type checking | Core design decision. Without it, Snow is just another Elixir. | Very High | Type inference engine | The entire point: catch errors at compile time. |
| Hindley-Milner type inference | Users rarely write type annotations. Compiler figures it out. | Very High | Unification algorithm | HM is proven, well-understood, used by OCaml, Haskell, Gleam, NVLang. The right choice for Snow. |
| Parametric polymorphism (generics) | Can't write reusable code without it. `List[T]`, `Option[T]`. | High | Type inference | Inferred generics, not explicit angle-bracket annotation soup. |
| `Option[T]` / no null | "Billion dollar mistake" avoidance is table stakes in 2025. Rust, Gleam, Kotlin, Swift all enforce this. | Medium | ADTs, Pattern matching | `Option.some(value)` / `Option.none`. No null/nil at the language level. |
| `Result[T, E]` error type | Modern error handling standard. Rust proved the pattern. | Medium | ADTs, Pattern matching | Explicit error handling, no exceptions. Pairs with `?` operator or similar for ergonomic propagation. |
| Struct types (product types) | Need to define data shapes | Medium | Type checker | Named fields, type-checked at compile time. |
| Type aliases | Readability and domain modeling | Low | Type checker | `type UserId = Int` or similar. |
| Recursive types | Needed for trees, linked lists, ASTs | Medium | Type checker, Occurs check | Must handle occurs check properly in HM inference. |

### Concurrency

| Feature | Why Expected | Complexity | Dependencies | Notes |
|---------|--------------|------------|--------------|-------|
| Lightweight actor processes | Core value proposition of Snow. This is why the language exists. | Very High | Runtime scheduler, Codegen | Not OS threads. Must support millions of actors like BEAM. Preemptive scheduling. |
| Message passing (`send`) | Fundamental to actor model | High | Actor runtime, Type system | Typed messages via ADTs: each actor declares its message vocabulary as a sum type. Compile-time protocol checking (NVLang approach). |
| Typed process identifiers (`Pid[T]`) | Type safety for actors. Without this, Snow loses its advantage over Elixir. | High | Type system, Actor runtime | `Pid[ChatMessage]` means you can only send `ChatMessage` variants to this actor. Gleam and NVLang both do this. |
| `receive` blocks with pattern matching | Core actor interaction pattern | High | Pattern matching, Actor runtime | Exhaustive matching on message types. This is where Snow's expressiveness shines. |
| `spawn` for creating actors | Fundamental actor primitive | Medium | Actor runtime, Codegen | Returns typed `Pid[T]`. |
| Supervision trees | "Let it crash" is the killer feature of BEAM-style concurrency | Very High | Actor runtime, Process linking | Strategies: `one_for_one`, `one_for_all`, `rest_for_one`. Restart policies: permanent, transient, temporary. |
| Process isolation (crash isolation) | Without this, supervision trees are meaningless | Very High | Runtime memory management | Each actor has its own heap. Crash in one actor must not corrupt another. This is the hardest runtime engineering challenge. |

### Standard Library (Minimum Viable)

| Feature | Why Expected | Complexity | Dependencies | Notes |
|---------|--------------|------------|--------------|-------|
| String manipulation | Fundamental | Medium | UTF-8 support | `String.length`, `String.split`, `String.trim`, `String.contains?`, interpolation. |
| List/Array operations | Fundamental data structure | Medium | Generics | `map`, `filter`, `reduce`, `head`, `tail`, `append`, `length`. Linked list or array semantics TBD. |
| Map/Dictionary type | Key-value storage is essential | Medium | Generics, Hashing | `Map[K, V]` with `get`, `put`, `delete`, `keys`, `values`. |
| File I/O | Can't build CLI tools without it | Medium | OS bindings | `File.read`, `File.write`, `File.exists?`. Returns `Result[T, E]`. |
| Standard I/O (stdin/stdout/stderr) | Fundamental | Low | OS bindings | `IO.puts`, `IO.gets`, `IO.inspect` (debug printing). |
| Basic math | Fundamental | Low | Runtime | `Math.abs`, `Math.max`, `Math.min`, `Math.pow`. |
| Process/system operations | Needed for CLI tools | Low | OS bindings | `System.args` (CLI arguments), `System.env` (environment variables), `System.exit`. |

### Tooling (Day-One Requirements)

| Feature | Why Expected | Complexity | Dependencies | Notes |
|---------|--------------|------------|--------------|-------|
| Compiler with clear error messages | Poor errors = dead language. Elm and Rust set the standard. | High | All compiler phases | Invest heavily here. Point to the exact line, suggest fixes. "Did you mean X?" This is a differentiator disguised as table stakes. |
| Build system (single command compile) | `snow build` must just work | Medium | Compiler, Linker | Produces single binary. No Makefiles, no configuration for simple projects. |
| REPL | Expected for expressive/scripting-feel languages | Medium | Interpreter mode or JIT | Not strictly needed for v1 but strongly expected given Elixir/Ruby heritage. Could defer to post-v1. |

---

## Differentiators (Competitive Advantage)

Features that make Snow unique. Not expected by default, but these are why someone would choose Snow over Go, Rust, or Elixir.

### vs Go

Go's strengths: simplicity, fast compilation, goroutines, massive ecosystem, single binary deployment, comprehensive standard library.
Go's weaknesses: verbose error handling, no sum types (until very recently), no pattern matching, no generics until 1.18 (and still limited), primitive type system, goroutines lack actor isolation and supervision.

| Feature | Snow's Advantage | Complexity | Notes |
|---------|------------------|------------|-------|
| Expressive syntax (`do/end`, pattern matching) | Go is famously verbose. `if err != nil` everywhere. Snow's pattern matching and `Result` type with `?` propagation eliminate this boilerplate. | Already in table stakes | Go developers who want more expressiveness are a target audience. |
| Algebraic data types + exhaustive matching | Go added generics in 1.18 but still has no sum types or exhaustive pattern matching. Snow has both as core features. | Already in table stakes | This alone is a major draw for developers tired of Go's type system limitations. |
| Actor isolation with supervision | Goroutines share memory. Data races are possible (and common). Snow's actors are isolated by design -- no shared mutable state, no data races. Supervision trees auto-restart crashed actors. | Already in table stakes | Go has nothing comparable to supervision trees. This is Snow's biggest structural advantage over Go. |
| Typed message passing | Go channels are typed but channels themselves are a lower-level primitive than actors. Snow's typed `Pid[T]` ensures you can only send valid messages to an actor. | Already in table stakes | Channels can deadlock. Actors with message queues and receive blocks cannot (by design). |
| Pipe operator (`\|>`) | Chain function calls readably: `data \|> transform \|> validate \|> save`. Go has nothing like this. | Low | A high-value, low-cost differentiator. Directly from Elixir. Makes data transformation pipelines readable. |
| Immutability by default | Go defaults to mutable everything. Snow defaults to immutable, which prevents entire categories of bugs and aligns with the functional paradigm. | Medium | Functional-first means immutable data structures. Mutable state is the exception, not the rule. |

### vs Rust

Rust's strengths: zero-cost abstractions, memory safety without GC, borrow checker, fearless concurrency, massive ecosystem, incredible tooling.
Rust's weaknesses: steep learning curve (borrow checker), verbose for simple tasks, no built-in actor model, compile times, not expressive for concurrent workflows.

| Feature | Snow's Advantage | Complexity | Notes |
|---------|------------------|------------|-------|
| Dramatically simpler concurrency model | Rust's concurrency requires understanding lifetimes, `Send`/`Sync` traits, `Arc<Mutex<T>>`, async runtimes (Tokio). Snow: `spawn` an actor, `send` it a message. Done. | Already in table stakes | Snow sacrifices Rust's zero-cost abstractions for dramatically better concurrency ergonomics. The right tradeoff for web backends. |
| No borrow checker / ownership system | Snow uses GC (per-actor garbage collection like BEAM). Developers never fight the borrow checker. | Part of runtime design | The tradeoff: Snow won't match Rust's performance for CPU-bound work. But for I/O-bound web backends, the difference is negligible. |
| Faster development velocity | Rust's compile times are long. Its learning curve is steep. Snow aims for Go-like simplicity with Elixir-like expressiveness. | Holistic | Target developer: someone who finds Rust too complex for web backends but wants more safety than Go. |
| Built-in supervision and fault tolerance | Rust has no built-in supervisor trees. Libraries like `ractor` exist but they're not language-level. Snow makes this a core primitive. | Already in table stakes | "Let it crash" philosophy requires language-level support to work well. |
| Expressive, Ruby-like syntax | Rust's syntax is powerful but dense. Snow's `do/end` blocks, keyword arguments, and minimal punctuation aim for readability. | Already in table stakes | The Elixir community consistently cites syntax as a reason they love the language. |

### vs Elixir

Elixir's strengths: BEAM concurrency, supervision trees, incredible ecosystem (Phoenix, LiveView, Ecto), pipe operator, pattern matching, macros, hot code reloading, distributed systems.
Elixir's weaknesses: dynamic typing, BEAM VM required, no single binary deployment, runtime errors from type mismatches, slower raw performance than native code.

| Feature | Snow's Advantage | Complexity | Notes |
|---------|------------------|------------|-------|
| Static type system with inference | Elixir is dynamically typed. Type errors only surface at runtime. Snow catches them at compile time with HM inference, requiring minimal annotations. | Already in table stakes | This is Snow's #1 advantage over Elixir. Gleam fills this niche on BEAM, but Snow goes further with native compilation. |
| Native single-binary deployment | Elixir requires the BEAM VM. Snow compiles to a single native binary via LLVM. Deploy like Go: copy binary, run it. No runtime installation needed. | High (LLVM integration) | Massively simplifies deployment, container images, and distribution. The BEAM runtime is powerful but heavy for simple CLI tools. |
| Compile-time message protocol checking | Elixir sends arbitrary terms as messages. Snow types each actor's message protocol as an ADT and enforces it at compile time (NVLang approach). | Already in table stakes | Eliminates "wrong message sent to wrong process" bugs that only appear at runtime in Elixir. |
| Native performance | BEAM is optimized for concurrency, not raw computation. Snow compiles to native code via LLVM and will be faster for CPU-bound work. | Part of architecture | For I/O-bound work the difference is smaller, but for JSON parsing, crypto, data processing, native code wins significantly. |
| No VM dependency | BEAM applications require Erlang/OTP installed (or a release with embedded runtime). Snow binaries run anywhere with no dependencies. | Part of architecture | Critical for CLI tools and edge deployment where you can't install a VM. |

### Unique Snow Differentiators (vs ALL competitors)

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Concurrency that reads like sequential code | Snow's core promise: `spawn`, `send`, `receive` with do/end blocks should look as clean as a function call. No colored functions, no async/await split. | Very High (language + runtime design) | Go achieves this with goroutines but lacks supervision. Elixir achieves this but lacks static types. Rust fails here (async is complex). Snow is the only language combining ALL: expressive syntax + static types + transparent concurrency + supervision. |
| Typed supervision trees | Supervisors are typed constructs: the compiler verifies that child specs match actual actor types and message protocols. | Very High | NVLang demonstrated this is feasible. Snow should go further by making supervision a first-class syntax construct, not just a library call. |
| `?` operator for Result propagation | Rust proved this is essential for ergonomic error handling. `File.read(path)?` returns early on error. Combined with Snow's do/end syntax, this eliminates error-handling boilerplate. | Medium | Already proven in Rust. Low risk to adopt. |
| Pipe operator (`\|>`) with type inference | `data \|> parse \|> validate \|> save` -- fully type-checked at each step. Elixir has pipes but no type checking. Rust has no pipes. Go has neither. | Low | High-value, low-cost feature. Type inference across pipes is a natural fit for HM. |
| Guards in pattern matching | `case value do :ok, n when n > 0 -> ... end` -- Elixir-style guards that add runtime conditions to pattern matches. | Medium | Extends pattern matching expressiveness significantly. |

---

## Anti-Features (Deliberately Excluded)

Things Snow should explicitly NOT build. Each is a conscious design decision.

| Anti-Feature | Why Avoid | What to Do Instead | Confidence |
|--------------|-----------|-------------------|------------|
| **Classes and OOP** | Already decided. Functional-first. Classes add complexity (inheritance hierarchies, virtual dispatch, diamond problem) and conflict with actor model philosophy where behavior belongs to actors, not class hierarchies. | Structs for data, traits/protocols for polymorphism, actors for behavior + state. | HIGH -- project decision |
| **Null/nil values** | The "billion dollar mistake." Every modern language is moving away from null. Kotlin, Rust, Gleam, Swift all eliminate or restrict null. | `Option[T]` as the only way to represent absence. Pattern matching forces handling both cases. | HIGH -- universal consensus |
| **Exceptions (try/catch/throw)** | Exceptions create invisible control flow, hurt performance at scale, and conflict with actor isolation. The "let it crash" philosophy replaces defensive exception handling. | `Result[T, E]` for expected errors with `?` propagation. Actor crashes handled by supervisors. `panic` for truly unrecoverable programming bugs. | HIGH -- aligned with Rust + Elixir best practices |
| **Shared mutable state between actors** | Defeats the entire purpose of the actor model. Shared state = data races, locks, deadlocks. | Message passing only. Each actor owns its state. Immutable data can be shared freely (like Pony's `val` capability). | HIGH -- fundamental to actor model |
| **Inheritance** | Inheritance creates tight coupling, fragile base class problem, and "is-a" relationships that rarely map to reality. | Composition via structs containing other structs. Polymorphism via traits/protocols. | HIGH -- functional paradigm |
| **Macros (Elixir-style metaprogramming)** | Macros are powerful but create "languages within the language," hurt tooling (hard to analyze), and increase language complexity enormously. Gleam deliberately excluded them. | Generics + traits cover most macro use cases. If Snow needs compile-time code generation later, consider a simpler mechanism than full AST macros. | MEDIUM -- could revisit post-v1 |
| **Async/await colored functions** | Creates the function coloring problem: splits the ecosystem into sync and async code. Go and Elixir/BEAM prove you don't need it when the runtime handles concurrency transparently. | Actor runtime handles I/O scheduling. All code looks synchronous. The runtime multiplexes actors across OS threads (like BEAM and Go). | HIGH -- core design philosophy |
| **Manual memory management** | Snow targets web backends and CLI tools, not systems programming. Manual memory management adds complexity without benefiting the target use cases. | Per-actor garbage collection (like BEAM). Each actor has its own heap, collected independently. No stop-the-world GC pauses for the whole program. | HIGH -- aligns with target use cases |
| **Mutable variables by default** | Conflicts with functional-first paradigm. Mutable-by-default encourages bugs and makes reasoning about concurrent code harder. | Immutable by default. Explicit mutation only within actor state (actors are the place where mutable state lives, mediated by message passing). | HIGH -- functional paradigm |
| **Operator overloading** | Creates "clever" code that's hard to read. Math-heavy domains (Snow's anti-target) benefit, but web/CLI code suffers from hidden semantics behind `+` or `*`. | Named functions are clearer. Traits can define standard operations if needed (e.g., `Eq`, `Ord`, `Display`). | MEDIUM -- could selectively allow for numeric types |
| **Hot code reloading** | A powerful BEAM feature but requires the VM infrastructure that Snow deliberately avoids. Extremely complex to implement with native compilation. | Deploy new binaries. Use rolling deploys, blue/green deployment. This is how Go and Rust handle it, and it works fine for the target use cases. | HIGH -- not feasible with native compilation |
| **Distributed actors across nodes** | BEAM's distributed Erlang is powerful but extremely complex to implement and only useful at massive scale. | Focus on single-node actor concurrency first. Network communication via standard library HTTP/TCP. Distributed actors could be a post-v1 research area. | HIGH -- scope control |

---

## Feature Dependencies

Understanding dependencies is critical for phasing the roadmap. You cannot build feature B without feature A.

```
LAYER 1: Foundation (no dependencies)
  Lexer
  Parser (depends on: Lexer)
  AST representation

LAYER 2: Core Language
  Basic types (Int, Float, Bool, String) -- depends on: Parser, AST
  Variables and let bindings -- depends on: Parser, AST
  Functions -- depends on: Parser, AST
  Control flow (if/else) -- depends on: Parser, AST
  Operators (arithmetic, comparison, boolean) -- depends on: Basic types
  Comments -- depends on: Lexer
  String interpolation -- depends on: Parser, String type

LAYER 3: Type System
  Type inference engine (HM algorithm W) -- depends on: AST, Basic types
  Unification -- depends on: Type inference engine
  Type checking -- depends on: Unification
  Structs (product types) -- depends on: Type checker
  Enums/Sum types (ADTs) -- depends on: Type checker
  Pattern matching (basic) -- depends on: ADTs, Type checker
  Exhaustiveness checking -- depends on: Pattern matching, ADTs
  Generics (parametric polymorphism) -- depends on: Type inference, Unification
  Option[T] -- depends on: Generics, ADTs
  Result[T, E] -- depends on: Generics, ADTs
  Type aliases -- depends on: Type checker

LAYER 4: Functions & Composition
  Closures -- depends on: Functions, Type inference, Scope analysis
  Pipe operator (|>) -- depends on: Functions, Type inference
  Guards in pattern matching -- depends on: Pattern matching
  First-class functions -- depends on: Functions, Closures, Generics
  ? operator for Result propagation -- depends on: Result[T, E], Functions

LAYER 5: Code Organization
  Module system -- depends on: Parser, Name resolution
  Imports -- depends on: Module system
  Traits/Protocols -- depends on: Type system, Generics, Module system
  Visibility/access control (pub/private) -- depends on: Module system

LAYER 6: Collections & Standard Library
  List type -- depends on: Generics, ADTs
  Map type -- depends on: Generics, Hashing
  List operations (map, filter, reduce) -- depends on: List type, Closures, Generics
  String operations -- depends on: String type
  File I/O -- depends on: Result[T, E], String type
  Standard I/O -- depends on: String type
  System operations -- depends on: Module system

LAYER 7: Code Generation
  LLVM IR generation -- depends on: Type-checked AST
  Basic codegen (functions, control flow) -- depends on: LLVM IR generation
  Pattern matching compilation -- depends on: Basic codegen, Pattern matching
  Closure compilation -- depends on: Basic codegen, Closures
  Struct/ADT memory layout -- depends on: Basic codegen, Type system
  Linking to single binary -- depends on: LLVM codegen

LAYER 8: Actor Runtime
  Lightweight process/actor creation -- depends on: Runtime scheduler, Codegen
  Actor message queues -- depends on: Actor creation
  Message sending (typed) -- depends on: Actor creation, ADTs, Pid[T] type
  Receive blocks with pattern matching -- depends on: Message queues, Pattern matching compilation
  Actor scheduler (preemptive, work-stealing) -- depends on: Runtime design
  Per-actor garbage collection -- depends on: Runtime memory management
  Process isolation (crash containment) -- depends on: Per-actor GC, Error handling

LAYER 9: Fault Tolerance
  Process linking and monitoring -- depends on: Actor runtime
  Supervisor actors -- depends on: Process linking, Actor runtime
  Restart strategies -- depends on: Supervisor actors
  Supervision trees -- depends on: Supervisor actors, Restart strategies
  Typed supervision (compile-time child spec validation) -- depends on: Supervisors, Type system

LAYER 10: Tooling
  Error messages (human-readable) -- depends on: All compiler phases
  Build system (`snow build`) -- depends on: Compiler, Linker
  REPL -- depends on: Parser, Type checker (optional: interpreter or JIT)
  Package manager -- depends on: Module system, Build system
  Formatter -- depends on: Parser
  LSP server -- depends on: Parser, Type checker, Module system
```

### Critical Path

The longest dependency chain that determines minimum time to a working language:

```
Lexer -> Parser -> AST -> Type Inference -> ADTs -> Pattern Matching ->
LLVM Codegen -> Actor Runtime -> Supervision Trees
```

Each layer must be solid before the next. Shortcuts in the type system will cause rewrites. Shortcuts in the actor runtime will cause correctness bugs.

---

## Complexity Assessment

Summary of implementation difficulty for major feature groups.

| Feature Group | Complexity | Estimated Effort | Risk Level | Notes |
|---------------|------------|------------------|------------|-------|
| Lexer + Parser | Medium | 2-4 weeks | Low | Well-understood. Use Elixir-style grammar. Rust has good parser combinator libraries (nom, pest, lalrpop). |
| Basic type system (primitives, structs) | Medium | 2-3 weeks | Low | Straightforward type checking. |
| Hindley-Milner inference | Very High | 4-8 weeks | High | Algorithm W is well-documented but tricky to implement correctly. Occurs check, let-polymorphism, recursive types all have subtle edge cases. Research NVLang's implementation paper. |
| Algebraic Data Types | High | 2-4 weeks | Medium | Sum types + product types. Must integrate cleanly with HM inference. |
| Pattern matching + exhaustiveness | High | 3-6 weeks | Medium | Pattern compilation is a research topic in itself. Exhaustiveness checking for nested patterns is non-trivial. Look at Maranget's algorithm. |
| LLVM codegen (basic) | High | 4-8 weeks | Medium | Rust's `inkwell` crate provides LLVM bindings. Main challenge: mapping Snow's semantics to LLVM IR correctly. Closures and ADTs require careful representation. |
| Pipe operator | Low | 1-2 days | Low | Syntactic sugar: `a \|> f` desugars to `f(a)`. Type inference handles the rest. |
| Result/Option + ? operator | Medium | 1-2 weeks | Low | Well-understood pattern from Rust. |
| Actor runtime (scheduler) | Very High | 8-16 weeks | Very High | The hardest engineering challenge. Work-stealing scheduler, preemptive context switching at safe points, per-actor heaps. Study Tokio, Go's runtime, and BEAM's scheduler. |
| Message passing (typed) | High | 2-4 weeks | Medium | Integrating typed PIDs with the actor runtime. Type system must track message protocol types. |
| Process isolation + crash containment | Very High | 4-8 weeks | Very High | Each actor needs its own heap or allocation arena. Crashes must be contained without corrupting other actors' memory. This is where BEAM spends decades of engineering. |
| Supervision trees | High | 3-6 weeks | Medium | Complex state machine logic but well-documented by Erlang/OTP. Strategies are well-defined. |
| Per-actor GC | Very High | 4-8 weeks | Very High | Options: per-actor copying GC (like BEAM), reference counting (like Swift), or arena allocation. Each has major tradeoffs. |
| Standard library (minimal) | Medium | 4-8 weeks | Low | Strings, lists, maps, file I/O, stdio. Straightforward once codegen and types work. |
| Error messages | High | Ongoing | Medium | Not a single task but an ongoing investment. Budget time in every phase. |
| Module system | Medium | 2-4 weeks | Medium | Name resolution, imports, visibility. Can be simple initially. |
| Traits/Protocols | High | 3-6 weeks | Medium | Must integrate with HM inference. Look at Rust traits and Haskell typeclasses for inspiration. |
| Build system | Medium | 1-2 weeks | Low | Wrapper around compiler + LLVM linker. |
| REPL | Medium | 2-4 weeks | Medium | Requires either an interpreter mode or incremental compilation. Could defer. |
| Package manager | High | 4-8 weeks | Medium | Registry, dependency resolution, versioning. Defer to post-v1. |
| LSP server | High | 4-8 weeks | Medium | Requires incremental parsing and type checking. Defer to post-v1 but plan architecture for it. |
| Formatter | Medium | 1-2 weeks | Low | Pretty-printer based on AST. Can ship early. |

### Total Rough Estimate

**Minimum viable language (compile + run basic concurrent programs):** 6-12 months of focused work.
**Language usable for real web backends with supervision:** 12-24 months.
**Language with full tooling (LSP, package manager, formatter):** 24-36 months.

These estimates assume a single developer working full-time. They are intentionally rough -- the point is relative sizing, not scheduling.

---

## MVP Feature Recommendation

For a first milestone that proves the concept works end-to-end:

### Must Ship (Proves the vision)
1. Lexer + parser (do/end blocks, function definitions, let bindings)
2. Basic types (Int, Float, Bool, String) with HM inference
3. Structs and enums (ADTs)
4. Pattern matching with exhaustiveness checking
5. LLVM codegen to single binary
6. Basic actor spawn + send + receive
7. Standard I/O (print to stdout)

### Should Ship (Makes it compelling)
8. Pipe operator
9. Option[T] and Result[T, E]
10. Closures
11. Clear error messages
12. String interpolation

### Defer to Post-MVP
- Supervision trees (complex but not needed to prove the concept)
- Traits/Protocols (generics are enough initially)
- Standard library beyond basics
- Module system (single-file programs first)
- Package manager, LSP, formatter
- REPL
- Per-actor GC (use simpler allocation initially, accept limitations)

---

## Sources

### Official Documentation and Authoritative Sources
- [Go 1.26 Release Notes](https://go.dev/doc/go1.26) - HIGH confidence
- [Gleam Programming Language](https://gleam.run/) - HIGH confidence
- [Gleam FAQ](https://gleam.run/frequently-asked-questions/) - HIGH confidence
- [Pony Language](https://www.ponylang.io/) - HIGH confidence
- [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) - HIGH confidence
- [Rust Project Goals 2025](https://rust-lang.github.io/rust-project-goals/) - HIGH confidence

### Research Papers
- [NVLang: Unified Static Typing for Actor-Based Concurrency on the BEAM (arXiv:2512.05224, Dec 2025)](https://arxiv.org/abs/2512.05224) - HIGH confidence, directly relevant to Snow's design
- [Deny Capabilities for Safe, Fast Actors (Pony)](https://www.ponylang.io/media/papers/fast-cheap.pdf) - HIGH confidence

### Community and Analysis Sources
- [Earthly Blog: Slow March of Progress in Programming Language Tooling](https://earthly.dev/blog/programming-language-improvements/) - MEDIUM confidence
- [Gleam: The Functional BEAM Language to Watch](https://medium.com/@ThinkingLoop/gleam-the-functional-beam-language-to-watch-02a523752048) - MEDIUM confidence
- [Zig Defeats Function Coloring](https://byteiota.com/zig-defeats-function-coloring-the-async-problem-other-languages-cant-solve/) - MEDIUM confidence
- [Daniel Tan: Function Colors Represent Different Execution Contexts](https://danieltan.weblog.lol/2025/08/function-colors-represent-different-execution-contexts) - MEDIUM confidence
- [Result Pattern vs Exceptions Revisited (2025)](https://stevenstuartm.com/blog/2025/10/29/result-pattern-vs-exceptions-revisited.html) - MEDIUM confidence
- [2025 Stack Overflow Developer Survey](https://survey.stackoverflow.co/2025/technology) - MEDIUM confidence
- [Hindley-Milner Type System (Wikipedia)](https://en.wikipedia.org/wiki/Hindley%E2%80%93Milner_type_system) - MEDIUM confidence
- [Pony: Actor-Model Language for High-Safety Concurrency](https://dev.to/viz-x/pony-the-actor-model-language-built-for-high-safety-concurrency-c2a) - MEDIUM confidence
- [Typeclasses, Traits, and Protocols: Architecture Wars (BugsyBits, 2025)](https://medium.com/@bugsybits/typeclasses-traits-and-protocols-the-architecture-wars-between-haskell-rust-and-python-57a18e4f77fe) - MEDIUM confidence
