# Research Summary: Snow Language

**Project:** Snow Programming Language
**Domain:** Compiled programming language with static types, actor concurrency, Elixir-like syntax
**Researched:** 2026-02-05
**Overall Confidence:** HIGH

---

## Executive Summary

Snow is a statically typed, compiled programming language combining Hindley-Milner type inference with BEAM-style actor concurrency, compiled to native binaries via LLVM. The research validates this approach as technically feasible but architecturally complex, requiring strict phasing to succeed.

The recommended architecture follows proven patterns: hand-written compiler in Rust (lexer, parser, type checker, LLVM codegen via Inkwell), with a custom actor runtime built on Tokio's work-stealing scheduler. The type system follows established Hindley-Milner inference with typed actor message protocols (inspired by NVLang). The runtime provides per-actor heaps, preemptive scheduling via compiler-inserted yield points, and OTP-style supervision trees. Multiple reference implementations validate this approach: Pony (actor model + LLVM), Lumen/Firefly (BEAM semantics + LLVM + Rust), and Gleam (static typing on BEAM).

The critical risk is complexity: building a production-grade type system AND a preemptive actor runtime simultaneously is a multi-year undertaking with high failure risk. Success requires strict phasing: compiler first (sequential code only), then actor runtime integration. The type system (Phase 2) is the highest intellectual risk—Hindley-Milner inference has subtle implementation pitfalls (occurs check, let-polymorphism, substitution ordering) that cause rewrites if wrong. The preemption strategy must be decided upfront: compiler-inserted yield points (recommended) vs. signal-based preemption (Go's approach, much harder). Actor message typing is a research-level problem requiring careful design before implementation.

---

## Key Findings

### Recommended Stack

**Summary:** The stack is proven and well-vetted. Hand-written compiler components (lexer, parser) give full control over error recovery and diagnostics. Inkwell provides type-safe LLVM bindings. Tokio provides production-grade scheduling infrastructure. The risk is not "will these technologies work" but "can we integrate them correctly."

**Core technologies:**

- **Hand-written lexer and parser** — Full control over error messages and recovery, simpler than generators for this use case. Every production compiler (rustc, GCC, Clang) uses hand-written parsers.
- **Inkwell 0.8.0 + LLVM 18** — Type-safe Rust wrapper for LLVM, supports LLVM 11-21. Target LLVM 18 for stability and wide availability. Inkwell catches many LLVM errors at Rust compile time.
- **Custom actor runtime on Tokio 1.49** — Build Snow-specific actor primitives (spawn, send, receive, supervision) on Tokio's work-stealing scheduler. Tokio provides the hard parts (epoll/kqueue, timers, work-stealing); Snow adds actor semantics, typed mailboxes, and supervision trees.
- **ena (union-find)** — Used by rustc itself for type unification. Standard choice for Hindley-Milner inference.
- **ariadne 0.6** — Purpose-built for compiler diagnostics with multi-line labels and span rendering. Better than miette for compilers.
- **insta (snapshot testing)** — De facto standard for compiler testing in Rust. Adopted by the Rust compiler's bootstrap tests.

**Critical version notes:**
- Pin LLVM 18 (not latest 21) for contributor stability. LLVM API breaks across versions.
- Inkwell 0.8.0 may require git dependency if not yet on crates.io.

**What NOT to use:**
- Parser generators (LALRPOP, pest) — lose control over error recovery
- ractor/kameo as-is — study them but build custom runtime (the runtime IS the language)
- Cranelift as primary backend — generates ~2x slower code than LLVM

**Confidence:** HIGH for all core choices (verified via GitHub, crates.io, official docs, and multiple reference implementations).

---

### Expected Features

**Summary:** Snow's feature set is well-defined but ambitious. The must-haves span three hard problems: type inference, pattern matching compilation, and actor runtime. The differentiators are clear: static types (vs Elixir), ergonomic concurrency (vs Rust), and native binaries (vs Elixir/BEAM). The MVP scope is achievable in 6-12 months for sequential code, 12-24 months for full actor system with supervision.

**Must have (table stakes):**

These features are non-negotiable—without them, the language is incomplete or dismissed immediately.

- **Hindley-Milner type inference** — Users expect to rarely write type annotations. This is the core value proposition over Elixir.
- **Algebraic data types (sum + product)** — Foundation for type-safe domain modeling. Every modern functional language has them.
- **Pattern matching (exhaustive)** — Must be exhaustive with compiler warnings for missing cases. Core to language identity.
- **Lightweight actor processes** — Core value proposition. Must support millions of actors like BEAM. Preemptive scheduling required.
- **Typed message passing** — `Pid[MessageType]` ensures compile-time protocol checking. This is Snow's advantage over Elixir's dynamic messages.
- **Supervision trees** — "Let it crash" philosophy requires language-level support. Strategies: one_for_one, one_for_all, rest_for_one.
- **Option[T] and Result[T, E]** — Modern error handling. No null/nil values. Forces explicit error handling via pattern matching.
- **Closures and first-class functions** — Expected in any functional language.
- **String interpolation** — Every modern language has it. Missing = constant annoyance.
- **Module system** — Code organization for real projects.

**Should have (competitive advantage):**

These differentiate Snow from competitors.

- **Pipe operator (`|>`)** — High-value, low-cost feature from Elixir. Makes data transformation pipelines readable. Type-checked at each step.
- **`?` operator for Result propagation** — Rust proved this is essential for ergonomic error handling. Eliminates boilerplate.
- **Guards in pattern matching** — `case value do :ok, n when n > 0 -> ... end` — extends pattern matching expressiveness.
- **Immutability by default** — Aligns with functional paradigm, prevents entire categories of bugs.
- **Single binary deployment** — No VM required (vs Elixir). Copy binary and run (like Go).
- **Native performance** — LLVM optimization beats BEAM for CPU-bound work.

**Defer (v2+):**

Explicitly out of scope for initial release.

- **Macros** — Powerful but create tooling problems and language complexity. Gleam deliberately excluded them.
- **Hot code reloading** — Requires VM infrastructure Snow avoids. Use rolling deploys instead.
- **Distributed actors across nodes** — Defer to post-v1. Focus on single-node concurrency first.
- **REPL** — Strongly expected given Ruby/Elixir heritage, but not strictly needed for v1. Requires interpreter mode or JIT.
- **LSP/IDE support** — Defer to post-v1 but architect for it (salsa + rowan for incremental parsing).
- **Package manager** — Defer to post-v1. Simple projects don't need it initially.

**Feature dependencies (critical ordering):**
```
Lexer/Parser → Type System → ADTs → Pattern Matching → LLVM Codegen → Actor Runtime → Supervision Trees
```

Each layer must be solid before the next. Shortcuts in type system cause rewrites. Shortcuts in actor runtime cause correctness bugs.

**Confidence:** HIGH (well-established domain, extensive prior art from Rust, Gleam, Elixir, Pony).

---

### Architecture Approach

**Summary:** The architecture follows a proven five-phase compiler pipeline (Lexer → Parser → Type Checker → IR Lowering → LLVM Codegen) plus a custom actor runtime compiled as a static library linked into every binary. The mid-level IR (Snow IR) between typed AST and LLVM IR is critical for language-specific optimizations and pattern matching compilation. The runtime's work-stealing scheduler with per-actor heaps matches BEAM's model but requires careful implementation of preemption, GC, and isolation.

**Major components:**

1. **Compiler pipeline (Rust workspace):**
   - **Lexer** — Hand-written, lazy/streaming token production. Tracks spans for error reporting.
   - **Parser** — Recursive descent + Pratt parsing for expressions. Produces concrete AST with full source fidelity.
   - **Type Checker** — Hindley-Milner Algorithm W with extensions (row polymorphism, actor types). Constraint generation then solving. Exhaustiveness checking via Maranget's algorithm.
   - **IR Lowering** — Typed AST → Snow IR. Desugars pattern matching into decision trees, lowers actor operations to runtime calls, inserts GC safepoints and yield points.
   - **LLVM Codegen** — Snow IR → LLVM IR via Inkwell. Emits tagged unions, closures, function declarations matching runtime calling convention. Integrates with LLVM's GC infrastructure.

2. **Actor runtime (`libsnowrt` — static library in Rust):**
   - **Scheduler** — N:M work-stealing scheduler (N OS threads, M actors). One thread per CPU core. Reduction-based preemptive scheduling via compiler-inserted yield points at function calls and loop back-edges.
   - **Process model** — ~300-500 bytes per actor (mailbox pointer, heap metadata, stack/continuation, process metadata). Isolated heaps enable independent GC. Unbounded mailboxes (like Pony).
   - **Message passing** — Lock-free MPSC queue per actor. Messages copied between heaps (maintains isolation). Reference-counted shared binaries for large data.
   - **Supervision trees** — Directly follows Erlang/OTP. Supervisors monitor children, restart on crash with strategies (one_for_one, one_for_all, rest_for_one). Restart limits prevent infinite loops.
   - **Garbage collection** — Per-actor generational copying GC (young + old generation like Erlang). Triggered by heap threshold. LLVM stack maps track live references. Cross-actor cycle detection deferred (start with reference counting + known leaks).

3. **Runtime-binary integration:**
   - Compiler links user object files with `libsnowrt.a` to produce single native binary.
   - Generated `main()` function initializes runtime, spawns user's main module as root actor, runs scheduler loop until completion.
   - Optional: compile runtime as LLVM bitcode for link-time optimization (Pony's approach).

**Key architectural decisions:**
- **Custom runtime over ractor/kameo** — The runtime IS the language. Must own it for full control over semantics.
- **Mid-level IR** — Enables Snow-specific optimizations (tail call detection, message batching, actor elimination) before LLVM.
- **Preemption via compiler-inserted yields** — Simplest approach preserving BEAM mental model. Cost is a branch per function call/loop iteration.
- **Per-actor heaps** — No stop-the-world GC. Each actor collects independently. Maintains "let it crash" isolation.
- **Typed actor protocols** — Each actor declares its message type (sum type). `Pid[MessageType]` enforces compile-time protocol checking (NVLang approach).

**Build order (critical dependencies):**
1. Lexer + Parser (no dependencies, pure AST construction)
2. Type Checker (depends on AST, most intellectually complex)
3. Codegen for sequential code (depends on type checker, validates pipeline before actors)
4. Actor runtime (can develop in parallel with codegen once interface is defined)
5. Supervision and fault tolerance (extends runtime)
6. Standard library (written in Snow, requires working compiler)

**Reference implementations to study:**
- **Pony** — Actor model + LLVM + per-actor GC. Closest architectural match.
- **Lumen/Firefly** — BEAM semantics + LLVM + Rust. Validates exact approach Snow is taking.
- **Gleam** — Statically typed + BEAM. Type system design reference.
- **BEAM/OTP** — 30+ years of production validation for actor runtime patterns.
- **rustc** — Compiler pipeline (HIR → MIR → LLVM IR), HM inference, workspace structure.

**Confidence:** HIGH for architecture (multiple proven implementations exist). MEDIUM for runtime complexity (per-actor GC and preemption are hard engineering problems).

---

### Critical Pitfalls

**Top pitfalls from research (severity: CRITICAL):**

1. **Building actor runtime before compiler works** — Attempting compiler and runtime simultaneously means neither stabilizes. Both are hard; doing both at once creates compounding debugging complexity. The first milestone must be "Snow compiles and runs sequential pure functions" with LLVM. Actors come later. Detection: if after 3 months you cannot compile `fn add(a, b) = a + b`, you have this problem.

2. **Type system implementation bugs (HM inference)** — Hindley-Milner appears simple in textbooks but has subtle traps: occurs check (prevents infinite types), substitution composition order, let-polymorphism generalization, type scheme instantiation. Wrong implementation causes type unsoundness (accepts invalid programs, rejects valid ones). Prevention: use constraint-generation-then-solving (not Algorithm W's interleaved approach), comprehensive test suite BEFORE implementation, study canonical references (Damas-Milner paper, "Write You a Haskell"). Detection: if `let id = fn x -> x in (id 1, id "hello")` fails to type-check, or `fn x -> x x` type-checks without error, your implementation is broken.

3. **Preemptive scheduling without reduction counting** — BEAM achieves preemption via bytecode interpretation (every function call decrements reduction counter, yields at zero). With native LLVM code, tight loops never yield unless the compiler inserts yield points. Decision must be made in Phase 1: (a) compiler-inserted yield points at function calls/loop back-edges (recommended, simple, measurable cost), (b) signal-based preemption (Go's approach, extremely complex, requires safepoint metadata), or (c) cooperative scheduling (breaks BEAM promise). Detection test: one actor runs infinite loop, another tries to receive message—if second actor never runs, you have this problem.

4. **Typing actor messages incorrectly** — Static typing for actor message passing is a research problem. Erlang never successfully added static types; Dialyzer uses "success typing" (weaker guarantee). Prevention: use NVLang's approach (sum types as protocols, `Pid[MessageType]`, explicit message type declarations per actor). Accept that some patterns (fully dynamic routing, runtime protocol evolution) are outside static type system scope. Design on paper with worked examples (basic send/receive, request-reply, supervision trees, dynamic actor creation) before implementation. Detection: try to type-check a supervision tree—if you cannot express "supervisor that spawns actors of different message types," design has a gap.

**High-risk pitfalls (weeks to months of rework):**

- **LLVM version coupling** — LLVM APIs break across versions. Inkwell requires exact version match. Solution: pin LLVM 18, document installation, provide Docker/Nix, never track HEAD.
- **Terrible type error messages** — Algorithm W has left-to-right bias. Error reported at unification failure point, often far from actual mistake. Solution: constraint-generation-then-solving architecture, multi-location error messages, invest heavily in message quality (Elm/Rust standards).
- **AST representation fights Rust's ownership** — Mutable trees fight borrow checker. Solution: arena allocators (bumpalo/typed-arena), index-based references (not Box/Rc), separate annotation maps (`HashMap<NodeId, Type>`).
- **Per-actor GC is hard** — No stop-the-world requires per-actor heaps with copying GC or reference counting with cycle detection. Solution: start with reference counting + known cycle leaks, study Pony's ORCA protocol, defer cycle detection to later phase.
- **Slow LLVM compile times** — Even at -O0, LLVM is slow. Solution: build interpreter/bytecode VM for dev mode, use LLVM only for release builds. Measure from day one, set budget (100-line program < 1 second at -O0).

**Phase-specific warnings:**
- **Phase 0 (setup):** LLVM build complexity, no test suite from day one
- **Phase 1 (design):** preemption strategy, syntax decisions, error handling model
- **Phase 2 (type system):** HM bugs, bad error messages, extensions breaking inference
- **Phase 3 (actors):** typed message passing, per-actor GC, work-stealing complexity, isolation failures

**Confidence:** HIGH (verified from academic literature, official docs, implementation post-mortems, NVLang paper, BEAM documentation, Go scheduler proposals).

---

## Implications for Roadmap

Based on dependency analysis across all research, the roadmap must follow strict sequential phases. The critical path is: `Lexer → Parser → Type Checker → Codegen → Actor Runtime → Supervision`. Each phase must reach "working, testable" before starting the next.

### Suggested Phase Structure

#### Phase 1: Compiler Foundation (Sequential Code)
**Duration:** 2-4 months
**Rationale:** Lexer and parser have zero dependencies and are the foundation for everything. Type checker is the highest intellectual risk and must be solid before codegen. This phase produces a working compiler for pure, sequential Snow code—no actors, no concurrency, just functions, types, and pattern matching. Validates the entire pipeline end-to-end before adding actor complexity.

**Delivers:**
- Hand-written lexer with span tracking and error recovery
- Recursive descent parser + Pratt parsing for expressions
- Hindley-Milner type inference (Algorithm W with constraint solving)
- Exhaustiveness checking for pattern matching (Maranget's algorithm)
- AST representation (arena-based or index-based to avoid Rc<RefCell> hell)
- Comprehensive test suite (snapshot tests with insta)

**Addresses features:**
- Variables and bindings, functions, control flow (if/else, match)
- Basic types (Int, Float, Bool, String), structs, enums (ADTs)
- Pattern matching, closures, pipe operator
- Option[T] and Result[T, E] types
- String interpolation, comments

**Avoids pitfalls:**
- C2 (HM implementation bugs) — comprehensive test suite before implementation, constraint-gen-then-solve architecture
- H2 (terrible error messages) — design error reporting strategy upfront
- H3 (AST representation fights Rust) — choose arena/index approach in phase planning

**Stack used:** lasso (string interning), ena (union-find), ariadne (diagnostics)

**Research flag:** Standard patterns. Parser and type checker are well-documented. No additional research needed beyond implementing Algorithm W correctly.

---

#### Phase 2: LLVM Code Generation (Native Binaries)
**Duration:** 2-4 months
**Rationale:** With type checker working, add LLVM backend to produce native executables. Starts with sequential code only—no actors yet. This validates the full compilation pipeline (source → tokens → AST → typed AST → IR → LLVM IR → binary) before adding runtime complexity. A "Hello World" program that calls printf through FFI validates everything without needing the actor runtime.

**Delivers:**
- Mid-level IR (Snow IR) with lowering from typed AST
- Pattern matching compilation to decision trees
- LLVM codegen via Inkwell (functions, control flow, data structures)
- Linking to single native binary
- Tagged union representation for ADTs
- Closure compilation (function pointer + environment)
- Basic FFI for C interop (printf, file I/O via libc)

**Addresses features:**
- LLVM code generation backend
- Single binary deployment
- Native performance
- Build system (`snow build`)

**Avoids pitfalls:**
- H1 (LLVM version coupling) — pin LLVM 18, document setup, provide Docker/Nix
- H5 (slow compile times) — measure from day one, consider bytecode VM for dev mode in future
- M4 (missing IR annotations) — mark purity, aliasing, arithmetic flags
- M5 (ABI mismatches) — define calling convention, test FFI on all platforms

**Stack used:** Inkwell 0.8.0 + LLVM 18, Cargo workspace structure

**Research flag:** Standard patterns. LLVM codegen is well-documented (Kaleidoscope tutorial, inkwell examples). May need phase-level research for optimization pass selection and GC integration later.

---

#### Phase 3: Actor Runtime Integration
**Duration:** 4-8 months
**Rationale:** With a working compiler for sequential code, add actor concurrency. This is the hardest engineering phase—building a preemptive scheduler, per-actor heaps, message passing, and typed mailboxes. The runtime is developed as a standalone Rust library first (tested independently with handwritten Rust actors), then integrated with the compiler (codegen emits calls to runtime functions, inserts yield points).

**Delivers:**
- Actor runtime (`libsnowrt`) as static library
- Work-stealing scheduler (N:M threading, one thread per core)
- Reduction-based preemption (compiler inserts yield points at function calls and loop back-edges)
- Lightweight process creation (spawn)
- Typed message passing (send, receive with pattern matching)
- Lock-free MPSC mailboxes
- Per-actor heaps with isolation
- Reference-counted GC per actor (defer cycle detection)
- Process linking and monitoring
- Codegen for actor operations (spawn → snow_rt_spawn, send → snow_rt_send, receive → runtime call with continuation)

**Addresses features:**
- Lightweight actor processes
- Typed message passing with `Pid[MessageType]`
- Message passing with pattern matching in receive blocks
- Process isolation (crash containment)

**Avoids pitfalls:**
- C1 (building runtime before compiler works) — runtime developed as standalone library, tested independently
- C3 (preemption strategy) — compiler-inserted yield points at function calls and loop back-edges
- C4 (typed message passing) — NVLang approach: sum types as protocols, explicit message type per actor
- H4 (per-actor GC) — start with reference counting + known cycle leaks
- R1 (work-stealing complexity) — start simple (no stealing), use crossbeam queues, add stealing later
- R2 (mailbox overflow) — unbounded mailboxes with backpressure monitoring
- R3 (isolation failures) — strict isolation, copy/move messages, no shared state

**Stack used:** Tokio 1.49 (foundation), custom runtime on top

**Research flag:** HIGH. Actor runtime is complex. May need phase-level research for:
- Scheduler implementation details (work-stealing algorithms, thread-per-core coordination)
- Per-actor GC strategies (copying GC vs. reference counting, LLVM GC integration)
- Message serialization and copying between heaps
- Preemption safepoint insertion in codegen

**Recommendation:** Plan for 1-2 weeks of focused research at phase start. Study Pony's runtime source, BEAM scheduler documentation, Tokio scheduler blog posts, and Lumen/Firefly's EIR compilation.

---

#### Phase 4: Supervision and Fault Tolerance
**Duration:** 2-3 months
**Rationale:** With actors working, add supervision trees to complete the "let it crash" philosophy. This is well-understood (OTP has 30 years of documentation) but requires careful state machine implementation for restart strategies and exit signal propagation.

**Delivers:**
- Supervisor actors with restart strategies (one_for_one, one_for_all, rest_for_one)
- Child specifications and restart policies (permanent, transient, temporary)
- Restart limits (max restarts in time window)
- Exit signal propagation through links
- Trap exits for supervisors
- Typed supervision (compile-time child spec validation against actor types)

**Addresses features:**
- Supervision trees (core value proposition)
- "Let it crash" philosophy with automatic restart
- Process linking and monitoring

**Avoids pitfalls:**
- R3 (supervision without isolation) — already addressed in Phase 3 by enforcing strict actor isolation

**Stack used:** Extensions to `libsnowrt` from Phase 3

**Research flag:** LOW. OTP supervision is well-documented. Standard patterns apply. No additional research needed—implementation is straightforward state machine logic following Erlang/OTP design.

---

#### Phase 5: Standard Library (Minimal Viable)
**Duration:** 2-4 months
**Rationale:** With the compiler and runtime complete, build the minimal standard library needed for real programs. This is mostly sequential work (implementing functions in Snow once the compiler works) with some Rust FFI for OS primitives.

**Delivers:**
- Core modules written in Snow: Enum, String, List, Map, Result patterns
- Runtime primitives in Rust (exposed as built-in functions): IO, System, Timer, Binary
- OTP-style behaviors: GenServer, Task, Agent (built on actor primitives)
- File I/O, standard I/O
- Basic math functions

**Addresses features:**
- Standard library (List/Map operations, String manipulation, File I/O, standard I/O, math)
- GenServer pattern (generic server behavior)

**Avoids pitfalls:**
- M2 (error handling as afterthought) — already addressed in Phase 1 by designing Result[T, E] types

**Research flag:** LOW. Standard library is straightforward implementation once the language works. No research needed.

---

#### Phase 6: Developer Experience (Post-MVP)
**Duration:** 3-6 months
**Rationale:** With a working language, add tooling that makes Snow practical for daily use. Defer to post-MVP because the language must work first before investing in tooling.

**Delivers:**
- Formatter (AST-based pretty printer)
- Better error messages (ongoing investment from Phase 1, but polish here)
- Package manager (optional, defer if needed)
- REPL (interpreter mode or incremental compilation)
- LSP server (requires salsa for incremental parsing + rowan for CST)

**Research flag:** MEDIUM for LSP (needs research into incremental parsing with salsa and rowan). LOW for formatter/package manager (standard patterns).

---

### Phase Ordering Rationale

1. **Lexer/Parser first** — Zero external dependencies, pure AST construction. Everything depends on them. Cannot proceed without working parser.

2. **Type checker before codegen** — Want to catch errors early. Typed AST is input to IR lowering. Type system is the highest intellectual risk—must be solid before moving to codegen.

3. **Sequential codegen before actor runtime** — Need ability to compile *something* to validate pipeline end-to-end. "Hello World" with printf through C FFI proves lexer → parser → type checker → IR → LLVM → binary without needing actor runtime.

4. **Actor runtime as standalone library** — Develop and test runtime independently from compiler integration. Define interface (which runtime functions exist, their signatures) upfront. Runtime can be developed partially in parallel with Phase 2 codegen once interface is locked.

5. **Supervision after actors** — Cannot supervise what does not exist yet. Need working actors before adding supervision trees.

6. **Standard library last** — Requires working compiler. Can be developed incrementally. Priority order: core types → OTP behaviors → application libs (HTTP, JSON).

**Critical path:** Phases 1-4 are sequential and cannot be parallelized. Phase 5 (stdlib) can begin once Phase 3 (actors) is stable. Phase 6 (tooling) is fully post-MVP.

---

### Research Flags by Phase

**Phases with standard patterns (no research-phase needed):**
- **Phase 1** — Parser and HM type inference are textbook material. Implement from canonical sources (Damas-Milner, "Write You a Haskell," Maranget for pattern matching).
- **Phase 2** — LLVM codegen is well-documented (Kaleidoscope tutorial, Inkwell examples, LLVM docs).
- **Phase 4** — OTP supervision is well-documented with 30 years of production validation. Follow Erlang/OTP design directly.
- **Phase 5** — Standard library is straightforward implementation once language works.

**Phases needing deeper research during planning:**
- **Phase 3 (Actor Runtime)** — HIGH complexity. Plan 1-2 weeks focused research at phase start for:
  - Work-stealing scheduler implementation (study BEAM, Tokio, Go)
  - Per-actor GC strategy (Pony ORCA paper, BEAM GC documentation)
  - LLVM GC integration (stack maps, safepoints)
  - Message copying and heap isolation
  - Reduction counting and yield point insertion in codegen
- **Phase 6 (LSP Server)** — MEDIUM complexity. Incremental parsing with salsa + rowan is complex. Plan 1 week research when starting LSP work.

---

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| **Stack** | HIGH | All technologies verified (Inkwell 0.8.0 on GitHub, LLVM 18 available, Tokio 1.49 released, ena/ariadne/insta on crates.io). Multiple reference implementations use same stack (Pony, Lumen/Firefly, Gleam). |
| **Features** | HIGH | Well-established domain. Feature list matches proven languages (Rust for types, Gleam for typed BEAM, Elixir for actors). MVP scope is achievable but ambitious (6-12 months for sequential, 12-24 months for full actors). |
| **Architecture** | HIGH | Compiler pipeline follows rustc's proven pattern (HIR → MIR → LLVM IR). Actor runtime architecture validated by BEAM (30+ years), Pony (10+ years), and Lumen/Firefly (proves BEAM semantics + LLVM works). |
| **Pitfalls** | HIGH | Verified from academic literature (HM inference, pattern matching), official documentation (LLVM, BEAM, Go scheduler), and implementation post-mortems (nikic's LLVM critique, Ink language retrospective, Tristan Hume's Rust compiler blog). |

**Overall confidence:** HIGH with clear understanding of high-complexity areas (HM type system implementation, per-actor GC, preemptive scheduling).

---

### Gaps to Address During Planning/Execution

1. **Memory management for Snow values** — Snow is functional (immutable by default), so reference counting (like Swift) may work. But actors need isolated heaps (like BEAM). How does generated code manage Snow-level objects? Decision: start with reference counting per actor, accept cycle leaks initially, study Pony's ORCA for long-term solution. Gap will be addressed during Phase 3 planning.

2. **Message serialization format** — How are Snow-level values serialized into actor mailbox messages? Zero-copy? Deep copy? Decision: deep copy by default (maintains isolation), reference-counted shared buffers for large binaries (like Erlang). Gap will be addressed during Phase 3 architecture design.

3. **FFI design** — How does Snow call C/Rust libraries? This affects codegen significantly. Decision: defer to Phase 2 planning. Start with minimal FFI (printf, basic libc calls) to validate approach.

4. **Standard library scope** — What ships with Snow? TCP/UDP? HTTP? JSON? Each has runtime implications. Decision: defer to Phase 5. Minimum viable is I/O, string, list, map. HTTP can be post-v1.

5. **Distribution/clustering** — The runtime architecture should not preclude distributed actors (ractor_cluster and kameo both support it). Decision: explicitly deferred to post-v1. Focus on single-node concurrency. Document extension points for future distribution work.

6. **Optimization pass selection** — LLVM's -O2 pipeline assumes C/C++ semantics. Which passes are appropriate for Snow? Decision: defer to Phase 2 codegen. Start with no optimizations (-O0), add passes incrementally based on benchmarks.

7. **Type system extensions** — Row polymorphism for records? Typeclasses for ad-hoc polymorphism? GADTs? Each has trade-offs for type inference. Decision: lock scope in Phase 1 planning. Start with pure HM + ADTs. Extensions are post-MVP.

---

## Sources

### Primary (HIGH confidence)

**Stack:**
- [Inkwell GitHub v0.8.0](https://github.com/TheDan64/inkwell) — LLVM bindings, version support
- [LLVM Official Documentation](https://llvm.org/docs/) — Codegen, GC, optimization
- [Tokio v1.49.0](https://tokio.rs/) — Async runtime, work-stealing scheduler
- [ena](https://crates.io/crates/ena), [ariadne](https://crates.io/crates/ariadne), [insta](https://crates.io/crates/insta), [lasso](https://crates.io/crates/lasso) — crates.io verified versions

**Architecture:**
- [Rust Compiler Development Guide](https://rustc-dev-guide.rust-lang.org/) — Compiler pipeline, type inference, MIR
- [BEAM Book](https://blog.stenmans.org/theBeamBook/) — BEAM VM internals, scheduler, process model
- [Pony GitHub](https://github.com/ponylang/ponyc) — Actor runtime architecture, ORCA GC
- [Lumen/Firefly GitHub](https://github.com/GetFirefly/firefly) — BEAM semantics + LLVM + Rust (validates Snow's approach)
- [Gleam FAQ](https://gleam.run/frequently-asked-questions/) — Static typing on BEAM

**Type System:**
- Damas-Hindley-Milner inference papers (canonical references)
- [Write You a Haskell - HM Inference](http://dev.stephendiehl.com/fun/006_hindley_milner.html)
- [Implementing a Hindley-Milner Type System](https://blog.stimsina.com/post/implementing-a-hindley-milner-type-system-part-1)

**Pattern Matching:**
- [Maranget, "Compiling Pattern Matching to Good Decision Trees" (2008)](https://www.cs.tufts.edu/~nr/cs257/archive/luc-maranget/jun08.pdf)
- [Maranget, "Warnings for Pattern Matching" (2007)](https://www.cambridge.org/core/journals/journal-of-functional-programming/article/warnings-for-pattern-matching/)

**Actor Typing:**
- [NVLang: Unified Static Typing for Actor-Based Concurrency on the BEAM (arXiv:2512.05224, Dec 2025)](https://arxiv.org/abs/2512.05224)

**Pitfalls:**
- [LLVM: The Bad Parts (nikic, Jan 2026)](https://www.npopov.com/2026/01/11/LLVM-The-bad-parts.html)
- [Go Non-cooperative Preemption Proposal](https://go.googlesource.com/proposal/+/master/design/24543-non-cooperative-preemption.md)
- [Ownership and Reference Counting Based GC (Pony ORCA)](https://www.doc.ic.ac.uk/~scd/icooolps15_GC.pdf)
- [Getting into the Flow: Better Type Error Messages](https://doi.org/10.1145/3622812)

### Secondary (MEDIUM confidence)

- [Tokio Scheduler Blog](https://tokio.rs/blog/2019-10-scheduler) — Work-stealing implementation details
- [BEAM Scheduler Deep Dive](https://hamidreza-s.github.io/erlang/scheduling/real-time/preemptive/migration/2016/02/09/erlang-scheduler-details.html)
- [Writing a Compiler in Rust (Tristan Hume)](https://thume.ca/2019/04/18/writing-a-compiler-in-rust/)
- [A retrospective on toy PL design mistakes (Ink)](https://dotink.co/posts/pl-design-mistakes/)
- [5 Mistakes in Programming Language Design](https://beza1e1.tuxen.de/articles/proglang_mistakes.html)
- [Comparing Rust Actor Libraries](https://tqwewe.com/blog/comparing-rust-actor-libraries/)
- [Actors with Tokio (Alice Ryhl)](https://ryhl.io/blog/actors-with-tokio/)
- Community blog posts on HM inference, LLVM usage, pattern matching compilation

### Reference Implementations

- **Pony** ([ponylang/ponyc](https://github.com/ponylang/ponyc)) — Actor model + LLVM + per-actor GC
- **Lumen/Firefly** ([GetFirefly/firefly](https://github.com/GetFirefly/firefly)) — BEAM semantics + LLVM + Rust
- **Gleam** ([gleam-lang/gleam](https://github.com/gleam-lang/gleam)) — Static types + BEAM
- **rust-hindley-milner**, **algorithmw-rust** — HM implementations in Rust

---

**Research completed:** 2026-02-05
**Ready for roadmap:** Yes
**Recommended first action:** Proceed to requirements definition (Phase 1 planning). Lock down type system scope and preemption strategy before any implementation begins.
