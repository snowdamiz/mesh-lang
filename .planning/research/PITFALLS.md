# Pitfalls Research: Snow Language

**Domain:** Programming language design and implementation (compiler in Rust, HM type inference, actor runtime, LLVM codegen)
**Researched:** 2026-02-05
**Overall Confidence:** HIGH (well-established domain with decades of literature and multiple verified sources)

---

## Critical Pitfalls (Will Derail Project)

These mistakes cause rewrites, multi-month delays, or fundamental architectural dead-ends.

### C1: Building the Actor Runtime Before the Compiler Works

**What goes wrong:** Attempting to build the preemptive actor runtime and the compiler/type system simultaneously. Both are individually hard; doing both at once means neither stabilizes, bugs in one mask bugs in the other, and you cannot test the runtime without the compiler or vice versa.

**Why it happens:** Excitement about the "full vision" leads to parallel development of interdependent systems that each need focused, sequential attention.

**Consequences:** Nothing works end-to-end for months. Motivation dies. Debugging becomes impossible because you cannot isolate whether a failure is in codegen, type checking, or runtime scheduling.

**Prevention:**
- Phase the project strictly: Lexer/Parser first, then type checker, then LLVM codegen for sequential code, THEN actor runtime. Each phase must produce a working, testable artifact.
- The actor runtime should be developed as a standalone Rust library first, tested independently with handwritten Rust actors, before the compiler targets it.
- Treat the first milestone as "Snow compiles and runs sequential pure functions to native code." Actors come later.

**Detection:** If after 3 months you cannot compile and run `fn add(a, b) = a + b`, you have this problem.

**Phase mapping:** Phase 1 should be purely sequential. Actor runtime should not appear before Phase 3 at the earliest.

**Severity:** CRITICAL

**Confidence:** HIGH -- this is a universal pattern in ambitious language projects. The Erlang VM itself was first a sequential Prolog implementation before actors were added.

---

### C2: Getting the Core Type System Wrong and Having to Rewrite It

**What goes wrong:** Implementing Hindley-Milner type inference without deeply understanding the algorithm leads to subtle bugs that do not surface until the language grows. Classic bugs: forgetting the occurs check (causing infinite types), incorrect substitution composition order, failing to generalize in let-bindings, and not properly instantiating type schemes.

**Why it happens:** HM inference appears deceptively simple in textbooks. The core algorithm is elegant, but a correct implementation requires meticulous attention to: (1) the occurs check in unification, (2) proper composition and application order of substitutions, (3) the distinction between generalization (let-polymorphism) and instantiation, and (4) handling of free variables during substitution into type schemes.

**Consequences:** Subtle type unsoundness. Programs that should fail to type-check are accepted. Programs that should be accepted are rejected. Infinite loops in the type checker. Error messages that point to the wrong location. All of these erode trust in the type system, and once users learn to distrust it, the language is dead.

**Prevention:**
- Study the canonical references: Damas and Milner's original paper, and the "Write You a Haskell" tutorial by Stephen Diehl.
- Use the constraint-generation-then-solving approach rather than interleaving constraint generation with unification (Algorithm W style). Separating these concerns makes the system easier to debug and extend, and produces better error messages.
- Write a comprehensive test suite for the type checker BEFORE implementing it. Cover: polymorphic identity, let-polymorphism (`let id = fn x -> x in (id 1, id true)`), occurs check rejection (`fn x -> x x`), nested let-bindings, higher-order functions.
- Start with a minimal core (no records, no modules, no type classes) and get it provably correct before extending.

**Detection:** If `let id = fn x -> x in (id 1, id "hello")` does not type-check, or if `fn x -> x x` type-checks without error, your implementation has fundamental bugs.

**Phase mapping:** Phase 2 (type system) is the highest-risk phase. Budget 2x the time you think it needs. Do not move to codegen until the type checker passes a comprehensive test suite.

**Severity:** CRITICAL

**Confidence:** HIGH -- verified across multiple sources including academic literature, implementation tutorials, and post-mortems from language projects.

**Sources:**
- [Implementing a Hindley-Milner Type System](https://blog.stimsina.com/post/implementing-a-hindley-milner-type-system-part-1)
- [Write You a Haskell - HM Inference](http://dev.stephendiehl.com/fun/006_hindley_milner.html)
- [Damas-Hindley-Milner inference two ways](https://bernsteinbear.com/blog/type-inference/)
- [A reckless introduction to Hindley-Milner](https://reasonableapproximation.net/2019/05/05/hindley-milner.html)

---

### C3: Preemptive Scheduling of Native Code Without Reduction Counting

**What goes wrong:** Attempting to build BEAM-style preemptive scheduling for natively compiled code. The BEAM achieves preemption because it controls the bytecode interpreter -- every function call decrements a reduction counter, and when it hits zero, the process yields. With native code compiled via LLVM, you do not have this luxury. Tight loops with no function calls will never yield.

**Why it happens:** The design says "BEAM-style concurrency" but the execution strategy says "native compilation via LLVM." These two goals are fundamentally in tension. The BEAM's preemption model depends on interpretive control that does not exist in AOT-compiled native code.

**Consequences:** Either actors can starve each other (a tight compute loop blocks all other actors on that OS thread), or you must add yield points that degrade performance and complicate codegen, or you must use OS-level signal-based preemption (like Go 1.14+) which adds enormous complexity.

**Prevention:** Choose ONE of these strategies and commit to it early:

1. **Compiler-inserted yield points (recommended for Snow).** At every function call and loop back-edge, the compiler inserts a check of a per-actor reduction counter. This is what Erlang does conceptually. The cost is a branch instruction per function call and loop iteration -- measurable but acceptable for a language prioritizing concurrency over raw compute speed. This is the simplest approach that preserves the BEAM mental model.

2. **Signal-based preemption (Go's approach).** Use OS signals (SIGURG on Unix) to asynchronously interrupt goroutines/actors. Go 1.14 added this after years of cooperative-only scheduling. It requires the compiler to emit safepoint metadata (PCDATA tables in Go's case) so the runtime knows where it is safe to preempt and where GC roots are. This is extremely complex to implement correctly.

3. **Cooperative scheduling with yield intrinsics.** Require the programmer to explicitly yield in long-running computations. This is the simplest to implement but breaks the "BEAM-style" promise -- BEAM users expect that no single process can starve others.

4. **Hybrid: cooperative with a watchdog.** Cooperative scheduling with a background monitor thread that detects starvation and sends signals to stuck threads. This is a pragmatic middle ground but still requires safepoint metadata.

**Detection:** Write a test: one actor runs `loop(n) = loop(n+1)` (infinite loop with no IO), another actor tries to receive a message. If the second actor never gets scheduled, you have this problem.

**Phase mapping:** This decision must be made in Phase 1 (architecture). The choice affects every subsequent phase: codegen (yield point insertion), runtime (scheduler design), and even the type system (if you need to track "this function may not yield").

**Severity:** CRITICAL

**Confidence:** HIGH -- verified from BEAM internals documentation, Go scheduler source code, and academic literature on actor scheduling.

**Sources:**
- [BEAM Book: Scheduling](https://github.com/happi/theBeamBook/blob/master/chapters/scheduling.asciidoc)
- [Erlang Scheduler Details](https://hamidreza-s.github.io/erlang/scheduling/real-time/preemptive/migration/2016/02/09/erlang-scheduler-details.html)
- [Go Non-cooperative Preemption Proposal](https://go.googlesource.com/proposal/+/master/design/24543-non-cooperative-preemption.md)
- [Go Preemption in Go](https://hidetatz.github.io/goroutine_preemption/)
- [Signal-based preemptive scheduling in Go](https://www.sobyte.net/post/2022-01/go-scheduling/)

---

### C4: Typing Actor Message Passing Incorrectly

**What goes wrong:** Bolting static types onto actor message passing without a coherent design leads to either an unsound system (messages can crash at runtime despite type-checking), an unusable system (too many type annotations required, defeating the purpose of inference), or a system that cannot express common actor patterns (supervision trees, dynamic actor creation, protocol evolution).

**Why it happens:** Actor message passing is inherently dynamic -- the BEAM was designed around dynamically typed messages. Combining static HM inference with actor protocols is a research-level problem. Erlang itself never successfully added static types, and Dialyzer uses "success typing" (a weaker guarantee) precisely because full static typing conflicted with the actor model.

**Consequences:** Users either cannot express the patterns they need, or the type system silently allows invalid messages, or the annotation burden is so high that users rebel against the type system.

**Prevention:**
- Study NVLang's approach: use algebraic data types (sum types) as actor message protocols. Each actor declares the sum type of messages it accepts. Typed process identifiers `Pid[MessageType]` encode what an actor expects. This is the cleanest known solution as of late 2024.
- Require each actor to declare its message type explicitly (as a sum type). This is one place where type inference should NOT be attempted -- message protocols should be manifest.
- Use typed futures `Future[ResponseType]` for request-reply patterns, so the compiler knows what type `await` returns.
- Accept that some patterns (fully dynamic message routing, protocol evolution at runtime) are outside the scope of the static type system. Provide an escape hatch (e.g., `Any` message type) for advanced users.
- Design the message typing system on paper, with worked examples, before implementing. Cover: basic send/receive, request-reply, supervision trees, dynamic actor creation, actor discovery.

**Detection:** Try to type-check a basic supervision tree pattern. If you cannot express "a supervisor that can spawn actors of different message types," your design has a gap.

**Phase mapping:** This must be designed (on paper) during Phase 1 architecture, but implemented during Phase 3 (actor system). The type system (Phase 2) should be designed with awareness that actor typing is coming, but the actual actor typing extensions happen later.

**Severity:** CRITICAL

**Confidence:** HIGH -- verified from NVLang paper (arXiv:2512.05224), Erlang community discussions on typing, and session types literature.

**Sources:**
- [NVLang: Unified Static Typing for Actor-Based Concurrency on the BEAM](https://arxiv.org/abs/2512.05224)
- [Why isn't Erlang strongly typed? (erlang-questions)](http://erlang.org/pipermail/erlang-questions/2008-October/039261.html)
- [Types (or lack thereof) - Learn You Some Erlang](https://learnyousomeerlang.com/types-or-lack-thereof)

---

## High-Risk Pitfalls (Significant Rework)

These mistakes cost weeks to months and cause significant technical debt, but are recoverable.

### H1: LLVM Version Coupling and Build Complexity

**What goes wrong:** LLVM's C and C++ APIs are unstable across versions. The Rust bindings (inkwell/llvm-sys) require an exact LLVM version match via Cargo feature flags. Binary LLVM packages often lack `llvm-config`. Building LLVM from source takes significant time and disk space (2.5M+ lines of C++). Contributors and CI need matching LLVM installations.

**Why it happens:** LLVM is a living project that regularly breaks API compatibility. Inkwell is pre-1.0 and may make breaking changes. The C API does not cover everything, so you may need custom C++ glue code.

**Consequences:** "Works on my machine" syndrome. CI breaks when LLVM updates. New contributors cannot build the project. Cross-platform builds become a nightmare.

**Prevention:**
- Pin to a specific LLVM version (e.g., LLVM 18) and do not update until there is a specific reason to.
- Use inkwell with a locked feature flag (`features = ["llvm18-0"]`). Do not track LLVM HEAD.
- Document exact build instructions including LLVM installation. Provide a Dockerfile or Nix flake for reproducible builds.
- Plan for LLVM version bumps as explicit, scheduled maintenance tasks (once or twice per year at most).
- Consider using the LLVM C API where possible for stability, resorting to C++ glue only when necessary.
- Set `LLVM_SYS_<version>_PREFIX` environment variables in CI and document them.

**Detection:** If a clean checkout on a new machine takes more than 30 minutes to build, or if it fails on a different LLVM version, you have this problem.

**Phase mapping:** Address in Phase 1 (project setup). The build system and LLVM integration must be rock-solid before any codegen work begins.

**Severity:** HIGH

**Confidence:** HIGH -- verified from inkwell and llvm-sys documentation, LLVM official FAQ, and "LLVM: The bad parts" blog post.

**Sources:**
- [LLVM: The Bad Parts (nikic)](https://www.npopov.com/2026/01/11/LLVM-The-bad-parts.html)
- [Inkwell GitHub](https://github.com/TheDan64/inkwell)
- [llvm-sys crate documentation](https://lib.rs/crates/llvm-sys)
- [How Bad is LLVM Really? (C3 compiler)](https://c3.handmade.network/blog/p/8852-how_bad_is_llvm_really)

---

### H2: Terrible Type Error Messages

**What goes wrong:** The standard HM inference algorithm (Algorithm W) has an inherent left-to-right bias. When a type error occurs, the error is reported at the point where unification fails, which is often far from the actual mistake. Users see errors like "expected Int, got String" on line 47, when the real mistake was on line 12. This is the single biggest complaint about ML-family languages.

**Why it happens:** Algorithm W interleaves constraint generation with solving, so when it encounters a contradiction, it blames the latest constraint. The actual error might be several unification steps back. Studies show that grouping errors (parentheses, brackets) account for 45-60% of type errors in functional programs, and these are the hardest for standard algorithms to localize.

**Consequences:** Users blame the type system, not their code. Beginners abandon the language. Even experienced users waste 12+ minutes per error (Google study). The type system becomes the language's reputation.

**Prevention:**
- Use constraint generation followed by separate solving (not Algorithm W's interleaved approach). This gives you the full constraint set to reason about when an error occurs.
- When a type error is detected, compute the minimal set of constraints that are contradictory. Report ALL locations involved, not just the last one.
- Invest heavily in error message quality from day one. Treat error messages as a first-class product feature, not an afterthought. Study Elm and Rust's approaches.
- For each error, show: (1) what the compiler expected, (2) what it found, (3) where the expectation came from (the other location that constrained the type), and (4) a suggestion if possible.
- Use the "data flow" approach to error explanation: describe errors as faulty data flows ("this value flows from line 12 where it's a String, to line 47 where an Int is expected").
- Build an error message test suite alongside the type checker. For each type error test case, assert not just that an error is produced, but that the error message mentions the correct location and type names.

**Detection:** Show your error messages to someone who has never seen the language. If they cannot understand what went wrong within 30 seconds, the message is bad.

**Phase mapping:** Must be considered from Phase 2 (type system) but will be iteratively improved throughout. The constraint-solving architecture decision (Phase 2) determines how good errors CAN be.

**Severity:** HIGH

**Confidence:** HIGH -- extensive academic literature and industry evidence.

**Sources:**
- [Getting into the Flow: Better Type Error Messages](https://doi.org/10.1145/3622812)
- [Repairing Type Errors in Functional Programs (McAdam thesis)](https://www.lfcs.inf.ed.ac.uk/reports/02/ECS-LFCS-02-427/ECS-LFCS-02-427.pdf)
- [Type Inference (Brown PAPL)](https://papl.cs.brown.edu/2020/Type_Inference.html)
- [Writing Good Compiler Error Messages](https://calebmer.com/2019/07/01/writing-good-compiler-error-messages.html)

---

### H3: AST and IR Representation Fights with Rust's Ownership Model

**What goes wrong:** Compilers are fundamentally about tree and graph transformations. Rust's ownership model makes mutable tree structures painful. Multiple passes need to read and annotate the same AST. Inherited attributes (types flowing down), synthesized attributes (types flowing up), and cross-references (symbol tables) all fight the borrow checker.

**Why it happens:** In Java/Python you would use shared mutable references everywhere. In Rust, you cannot have multiple mutable references. Self-recursive structs (for nested scopes, linked symbol tables) are notoriously difficult. Even `Rc<RefCell<...>>` patterns become unwieldy.

**Consequences:** Excessive `clone()` calls that hide design problems. `Rc<RefCell>` everywhere, losing Rust's safety guarantees in practice. Massive boilerplate for AST transformations. Slow iteration speed because every small change to the AST type ripples through every pass.

**Prevention:**
- Use an arena allocator for AST nodes. The `bumpalo` or `typed-arena` crates let you allocate nodes in an arena and use references freely within that arena's lifetime. This is the standard approach in production Rust compilers (rustc uses arenas extensively).
- Represent the AST as a flat vector of nodes with index-based references (the "ECS-style" or "data-oriented" approach). Each node stores indices into the vector rather than Box/Rc pointers. This sidesteps ownership entirely and enables cache-friendly iteration.
- Use separate annotation maps (`HashMap<NodeId, Type>`) rather than trying to annotate the AST in-place. Each compiler pass reads the AST immutably and writes to its own output map.
- Do NOT implement linked lists or self-recursive structs for symbol tables. Use a `Vec`-based scope stack or a `HashMap` with scope IDs.
- Make AST node types generic with an annotation parameter that changes through compilation stages. But use `Option` fields for optional annotations rather than rebuilding the entire AST per pass.

**Detection:** If you find yourself writing `Rc<RefCell<Box<dyn ...>>>`, or if a simple AST change requires modifications in 10+ files, you have this problem.

**Phase mapping:** The AST representation must be designed in Phase 1. Changing it later is extremely expensive because every pass depends on it.

**Severity:** HIGH

**Confidence:** HIGH -- verified from multiple Rust compiler implementation accounts and the Rust compiler development guide.

**Sources:**
- [Writing a Compiler in Rust (Tristan Hume)](https://thume.ca/2019/04/18/writing-a-compiler-in-rust/)
- [Writing Interpreters in Rust](https://rust-hosted-langs.github.io/book/chapter-interp-compiler-design.html)
- [Rust Compiler Development Guide](https://rustc-dev-guide.rust-lang.org/overview.html)

---

### H4: Per-Actor Garbage Collection Is a Research Problem

**What goes wrong:** BEAM-style concurrency uses per-process heaps with independent garbage collection -- when an actor is collected, no other actor is affected (no global stop-the-world). Implementing this for native code is a research-level problem. You need either: (1) a per-actor heap with copying GC (like BEAM), which requires deep integration with the memory allocator and codegen, or (2) reference counting with cycle detection across actors (like Pony's ORCA protocol), which requires a data-race-free type system.

**Why it happens:** The "single binary native executable" goal means you cannot use BEAM's existing GC. You must implement your own. Garbage collection for concurrent actor systems is fundamentally harder than sequential GC because of cross-actor references, message passing creating new roots, and the need to avoid global pauses.

**Consequences:** Memory leaks (if GC is too conservative), crashes (if GC is too aggressive), or global pauses that destroy the concurrency benefits (if you fall back to stop-the-world).

**Prevention:**
- Start with reference counting per actor, not tracing GC. Reference counting is simpler, deterministic, and does not require stop-the-world pauses. Accept that cycles within an actor's heap will leak initially.
- Enforce that messages between actors are deep-copied or use move semantics (not shared references). This keeps actor heaps isolated, which is the prerequisite for independent GC. Snow's functional-first design helps here -- immutable values can be safely shared.
- Study Pony's ORCA protocol for the long-term solution: it combines per-actor tracing with inter-actor reference counting, using message-based GC coordination with no locks or barriers.
- Consider: do you even need GC? If Snow is functional-first with immutable values and linear/affine types for actors, you might be able to use Rust-style ownership for most values and only need GC for actors themselves.
- Defer cycle detection to a later phase. Ship with reference counting + cycle leaks, then add a cycle detector as a separate background actor (this is what Pony does).

**Detection:** Write a test that creates and destroys 1 million short-lived actors. If memory usage grows without bound, you have a GC problem.

**Phase mapping:** Phase 3 (actor runtime). The GC strategy must be designed alongside the runtime, but a perfect GC is not needed for initial release -- reference counting with known limitations is acceptable for v0.1.

**Severity:** HIGH

**Confidence:** HIGH -- verified from Pony ORCA paper, BEAM documentation, and Crafting Interpreters GC chapter.

**Sources:**
- [Ownership and Reference Counting based GC in the Actor World (Pony ORCA)](https://www.doc.ic.ac.uk/~scd/icooolps15_GC.pdf)
- [Fully concurrent garbage collection of actors](https://www.researchgate.net/publication/311472609_Fully_concurrent_garbage_collection_of_actors_on_many-core_machines)
- [BEAM Book](https://blog.stenmans.org/theBeamBook/)

---

### H5: Slow Unoptimized Compile Times from LLVM

**What goes wrong:** LLVM is optimized for optimization. Even at -O0, LLVM codegen is slow. For the C3 compiler, LLVM codegen and linking takes over 98% of total compilation time when codegen is single-threaded with no optimizations. This means that during development, the edit-compile-run cycle is painfully slow, which destroys developer experience.

**Why it happens:** LLVM's architecture has inherent overhead even when no optimization passes run: IR construction, instruction selection, register allocation, and machine code emission all have baseline costs. The LLVM TPDE (Trivial Program Development Engine) alternative backend demonstrated that it is possible to be an order of magnitude faster for -O0 builds.

**Consequences:** Developers waiting 5-10 seconds for a small program to compile during iteration. This compounds into hours of wasted time daily. Users may abandon the language for this reason alone.

**Prevention:**
- Build an interpreter or bytecode VM for development mode from the start. Use LLVM only for release builds. This is the approach many modern language implementations take (e.g., Swift has an interpreter mode, Zig uses a separate backend for debug builds).
- Alternatively, implement a simple, fast code generator for debug builds that emits unoptimized native code directly (like Cranelift, which is designed for fast compilation at the expense of optimization quality).
- If using LLVM for all builds, parallelize compilation by compiling each function/module independently on separate threads, then linking.
- Do NOT use the -O2/-O3 pass pipelines as-is. These are tuned for C/C++, not your language. Start with a minimal set of passes and add only the ones that matter for your language's patterns.
- Measure compile time from day one. Set a budget (e.g., "100-line program compiles in under 1 second at -O0") and track it.

**Detection:** Time how long it takes to compile a trivial "hello world" program. If it exceeds 500ms, investigate where the time goes. Use LLVM's `-time-passes` flag.

**Phase mapping:** The debug-mode compilation strategy should be decided in Phase 1 (architecture). A bytecode interpreter for dev mode could be Phase 4 (tooling), but awareness of the problem must inform Phase 2 (codegen) design.

**Severity:** HIGH

**Confidence:** HIGH -- verified from LLVM documentation, C3 compiler benchmarks, and nikic's "LLVM: The bad parts."

**Sources:**
- [LLVM: The Bad Parts](https://www.npopov.com/2026/01/11/LLVM-The-bad-parts.html)
- [How Bad is LLVM Really?](https://c3.handmade.network/blog/p/8852-how_bad_is_llvm_really)
- [LLVM Performance Tips for Frontend Authors](https://llvm.org/docs/Frontend/PerformanceTips.html)

---

## Medium-Risk Pitfalls (Pain but Recoverable)

These cause days to weeks of rework but are fixable without fundamental redesign.

### M1: Extensions Breaking Type Inference

**What goes wrong:** Starting with a clean HM core, then adding features (records, type classes, GADTs, higher-kinded types, subtyping) that destroy the ability to infer principal types. Each extension seems harmless in isolation but the combination quickly makes inference undecidable or requires annotations that defeat the purpose.

**Why it happens:** HM inference lives in a "sweet spot" of the design space. Nearly any major addition either destroys principal types, requires annotations, or severely complicates the algorithm. Record types, ad-hoc polymorphism, and subtyping are the most common culprits.

**Prevention:**
- Define the language's type system scope BEFORE implementation. Decide which extensions you will and will not support. Write this down and do not deviate.
- For records: use row polymorphism (integrates cleanly with HM) rather than structural subtyping (destroys principal types). Study Elm's record system as a practical example.
- For ad-hoc polymorphism: type classes (Haskell-style) preserve inference but add complexity. Consider deferring type classes to a later version and using module-level functions instead.
- For each proposed extension, ask: "Does this preserve principal types?" If not, it requires annotations. Decide if that trade-off is acceptable.
- Test inference thoroughly after each extension. Maintain a suite of programs that must infer without annotations.

**Detection:** If users start needing type annotations to make code compile that "should obviously work," an extension has broken inference.

**Phase mapping:** Phase 2 (type system design). The scope of extensions must be locked before implementation begins.

**Severity:** MEDIUM

**Confidence:** HIGH -- well-established in type theory literature.

**Sources:**
- [Write You a Haskell - HM](http://dev.stephendiehl.com/fun/006_hindley_milner.html)
- [HM Wikipedia](https://en.wikipedia.org/wiki/Hindley%E2%80%93Milner_type_system)

---

### M2: Error Handling Design as an Afterthought

**What goes wrong:** Not designing the error handling model (how Snow programs handle errors -- not compiler errors) upfront. This was the biggest regret reported by the Ink language author. Without a clear distinction between recoverable errors (file not found) and unrecoverable errors (type error at runtime), programs cannot gracefully handle failures.

**Why it happens:** Error handling feels like a "detail" compared to the type system and concurrency model. But it pervades every API and standard library function.

**Consequences:** Programs crash irrecoverably on any error. No way to write robust servers or long-running processes. Users invent ad-hoc error conventions that fragment the ecosystem.

**Prevention:**
- Decide on the error model in Phase 1 language design: Result types (Rust-style `Result<T, E>`), exceptions (with or without checked), or a hybrid (like Elixir's `{:ok, value} | {:error, reason}` tuples encoded as sum types).
- For a functional-first language with static types, Result types are the natural fit. They compose well with pattern matching and are explicit in function signatures.
- Design how errors interact with actors: if an actor crashes, what happens? BEAM has "let it crash" with supervision trees. Snow needs an equivalent philosophy backed by the type system.
- Ensure errors carry source location and stack trace information. This requires the runtime to maintain a call stack or equivalent metadata.

**Detection:** Try to write a program that reads a file and handles "file not found" gracefully. If you cannot, the error model is missing.

**Phase mapping:** Phase 1 (language design). The error model affects the type system (Phase 2), standard library (Phase 4), and actor system (Phase 3).

**Severity:** MEDIUM

**Confidence:** HIGH -- verified from Ink language post-mortem and general language design literature.

**Sources:**
- [A retrospective on toy programming language design mistakes (Ink)](https://dotink.co/posts/pl-design-mistakes/)

---

### M3: Exhaustiveness Checking Gaps in Pattern Matching

**What goes wrong:** Pattern matching is a core feature of functional languages, but exhaustiveness checking (ensuring all cases are handled) is algorithmically complex, especially with nested patterns, guards, GADTs, and or-patterns. Incomplete checking means the compiler either misses non-exhaustive matches (crashes at runtime) or produces false warnings (annoying users).

**Why it happens:** The basic exhaustiveness algorithm is straightforward for flat enums but becomes complex with nested sum types, literal patterns, and guard expressions. Guards in particular make exhaustiveness undecidable in general.

**Prevention:**
- Start with the algorithm from Maranget's "Warnings for Pattern Matching" paper (2007) -- this is the standard reference, used in OCaml, and adapted for Rust and Haskell.
- Handle guards conservatively: treat guarded patterns as non-exhaustive (the guard might be false). This is what most production compilers do.
- Implement exhaustiveness checking as a separate pass, not interleaved with code generation. This makes it testable and replaceable.
- Write tests for: nested patterns, wildcard patterns, literal patterns (integers -- note you cannot enumerate all integers), or-patterns, and patterns on user-defined sum types.

**Detection:** If `match x { Some(true) -> ..., Some(false) -> ... }` does not warn about the missing `None` case, exhaustiveness checking is broken.

**Phase mapping:** Phase 2 (type system / pattern matching). Should be implemented alongside the pattern matching compiler.

**Severity:** MEDIUM

**Confidence:** HIGH -- well-established algorithms exist.

**Sources:**
- [Rust Compiler: Pattern and Exhaustiveness Checking](https://rustc-dev-guide.rust-lang.org/pat-exhaustive-checking.html)
- [Warnings for Pattern Matching (Maranget, 2007)](https://www.cambridge.org/core/journals/journal-of-functional-programming/article/warnings-for-pattern-matching/3165B75113781E2431E3856972940347)

---

### M4: Not Expressing Language Guarantees in LLVM IR

**What goes wrong:** LLVM IR is designed for C/C++. Snow's functional, immutable-by-default semantics provide guarantees that LLVM cannot automatically discover. If you do not encode these guarantees in the IR, LLVM cannot optimize as well as it could.

**Why it happens:** The path of least resistance is to emit "C-like" LLVM IR without leveraging metadata, attributes, or flags that express Snow-specific invariants.

**Consequences:** Missed optimizations. Snow programs are slower than equivalent C for no fundamental reason, just because the compiler does not tell LLVM what it knows.

**Prevention:**
- Mark function arguments and return values with `noalias` where Snow's ownership semantics guarantee no aliasing.
- Use `nsw` (no signed wrap) and `nuw` (no unsigned wrap) flags on arithmetic operations where the type system guarantees no overflow.
- Mark pure functions with `readnone` or `readonly` attributes. Snow's functional-first design means most functions should be pure.
- Use `!invariant.load` metadata for immutable data.
- Do NOT overuse the `assume` intrinsic -- it can hurt both compile time and optimization quality. Express constraints through IR attributes/flags instead.
- Study the LLVM Performance Tips for Frontend Authors document thoroughly before writing the codegen pass.

**Detection:** Compare the generated LLVM IR for a simple function against what Clang generates for an equivalent C function. If Snow's IR has fewer annotations, you are leaving performance on the table.

**Phase mapping:** Phase 2 (codegen). This is an optimization concern, not a correctness concern, so it can be iteratively improved.

**Severity:** MEDIUM

**Confidence:** HIGH -- verified from LLVM official documentation.

**Sources:**
- [LLVM Performance Tips for Frontend Authors](https://llvm.org/docs/Frontend/PerformanceTips.html)
- [LLVM: The Bad Parts](https://www.npopov.com/2026/01/11/LLVM-The-bad-parts.html)

---

### M5: Calling Convention and ABI Mismatches

**What goes wrong:** LLVM's calling convention documentation is incomplete. The contract between frontends and LLVM for calling conventions is largely undocumented -- implementers must "look at what Clang does and copy that (invariably with errors, because the rules can be very subtle)." Target features can alter the ABI (e.g., enabling AVX means additional float/vector registers are used for argument passing), so functions compiled with different feature flags may be ABI-incompatible.

**Why it happens:** LLVM expects you to know C/C++ calling conventions intimately. The rules differ per platform (x86-64 SystemV vs. Windows x64 vs. ARM AAPCS). Snow's data types (tagged unions, closures, actor references) do not map cleanly to C calling conventions.

**Consequences:** Segfaults, data corruption, or silently wrong results when calling between Snow functions and C FFI, or between Snow functions compiled with different settings.

**Prevention:**
- Define Snow's calling convention explicitly in a design document. Decide how tagged unions, closures, and multi-return values are passed.
- Start with the C calling convention for FFI compatibility, then consider a custom convention for internal Snow-to-Snow calls if performance demands it.
- Set the calling convention on both the function definition AND every call site. LLVM silently produces undefined behavior if these do not match.
- Test FFI calls thoroughly on every target platform. Use a test suite that calls C functions with every Snow type and vice versa.
- Avoid enabling target features (like AVX) per-function unless you understand the ABI implications.

**Detection:** If calling a C function from Snow (or vice versa) produces wrong results or crashes on one platform but works on another, you have a calling convention mismatch.

**Phase mapping:** Phase 2 (codegen). Must be addressed before FFI support is added.

**Severity:** MEDIUM

**Confidence:** HIGH -- verified from LLVM FAQ, nikic's blog post, and Mono LLVM backend documentation.

**Sources:**
- [LLVM FAQ](https://llvm.org/docs/FAQ.html)
- [LLVM: The Bad Parts](https://www.npopov.com/2026/01/11/LLVM-The-bad-parts.html)

---

## Language Design Pitfalls

### L1: Syntax Decisions That Seem Small but Compound

**What goes wrong:** Individual syntax choices (significant whitespace vs. braces, operator precedence, how to denote type annotations, expression vs. statement distinction) feel like bikeshedding, but they interact combinatorially. A language with 10 "minor" syntax quirks has 100+ quirk interactions.

**Prevention:**
- Use an LL(1) or LALR parseable grammar. Ambiguous or context-dependent grammars make tooling (IDEs, formatters, linters) much harder to build. C++ is the canonical example of a language whose grammar is so complex that parsing requires type information.
- Define operator precedence from day one and NEVER change it. C's precedence of `&` and `|` below `==` is a "hundred-year mistake" that cannot be fixed.
- Use one consistent rule for expression termination (always expressions, like Rust/Elixir, or always statements with explicit return, like Go). Do not mix.
- Design the grammar formally (BNF/EBNF) before implementing the parser. Use a parser generator or parser combinator library for the first implementation; hand-written recursive descent can come later for better error recovery.

**Severity:** MEDIUM

**Sources:**
- [5 Mistakes in Programming Language Design (Zwinkau)](https://beza1e1.tuxen.de/articles/proglang_mistakes.html)
- [Hundred Year Mistakes (Eric Lippert)](https://ericlippert.com/2020/02/27/hundred-year-mistakes/)

---

### L2: Null/Nil in Any Form

**What goes wrong:** Introducing any form of null/nil reference. Tony Hoare's "billion-dollar mistake." Even optional types can be misused if the language allows implicit unwrapping.

**Prevention:**
- Snow should have `Option[T]` (or `Maybe[T]`) as the only way to represent absence. No null pointers, no nil values, no implicit unwrapping.
- Pattern matching on Option should be the standard way to handle absence. The type system should enforce exhaustive matching.
- This is already implied by the functional-first, statically-typed design, but it must be an explicit, non-negotiable design principle.

**Severity:** MEDIUM (if Snow avoids it) / CRITICAL (if Snow introduces it)

---

### L3: Unclear Semantics and Undefined Behavior

**What goes wrong:** Leaving behavior "implementation-defined" or "undefined" creates a language where programmers cannot reason about their code. C's undefined behavior enables powerful optimizations but has caused decades of security vulnerabilities and confusing bugs.

**Prevention:**
- Every operation in Snow should have defined behavior. Integer overflow: either trap, wrap, or saturate, but PICK ONE and document it.
- No undefined behavior. If something cannot be given defined behavior, make it a compile-time error.
- Write a language specification (even informal) that covers every operation's behavior. Test against it.

**Severity:** MEDIUM

**Sources:**
- [5 Mistakes in Programming Language Design](https://beza1e1.tuxen.de/articles/proglang_mistakes.html)

---

## Type System Pitfalls

### T1: Monomorphization Explosion

**What goes wrong:** If Snow uses monomorphization (like Rust) for generics, each instantiation of a polymorphic function at a different type generates a separate copy. This leads to code bloat and long compile times for heavily generic code.

**Prevention:**
- Consider boxed/erased generics (like Java/Haskell) for the initial implementation. This is simpler, avoids code bloat, and is easier to implement. Monomorphization can be added as an optimization later for hot paths.
- If using monomorphization, implement a deduplication pass that shares identical instantiations.
- Set limits on instantiation depth to prevent pathological cases.

**Severity:** MEDIUM

---

### T2: Recursive Types Without Explicit Boxing

**What goes wrong:** Allowing types like `type List = Cons(Int, List) | Nil` without requiring explicit indirection (`Box`, `Ref`, etc.) for the recursive case. This type has infinite size and cannot be stack-allocated.

**Prevention:**
- Either require explicit boxing for recursive types (Rust's approach: `Cons(Int, Box<List>)`), or automatically box recursive positions (Haskell's approach -- everything is heap-allocated by default).
- For a functional-first language, automatic heap allocation with the compiler optimizing stack allocation where possible is likely the right default.

**Severity:** MEDIUM

---

### T3: Forgetting the Value Restriction

**What goes wrong:** In ML-family languages, unrestricted polymorphism for mutable references is unsound. The "value restriction" limits generalization to syntactic values (not arbitrary expressions) to prevent this. If Snow has any mutable state (even for actor-internal state), this applies.

**Prevention:**
- If Snow is purely immutable within actors, the value restriction may not apply. But if actors have mutable state (mailboxes, internal state), the type system must account for it.
- Study OCaml's value restriction as the standard approach.
- Test: `let r = ref [] in r := [1]; hd(!r) + "hello"` should be a type error, not accepted.

**Severity:** MEDIUM (depends on whether Snow has mutable references)

---

## Runtime/Concurrency Pitfalls

### R1: Work-Stealing Scheduler Without Understanding Lock-Free Data Structures

**What goes wrong:** Building a work-stealing scheduler (like BEAM's or Go's) requires lock-free or fine-grained concurrent data structures for run queues, message queues, and load balancing. Getting lock-free algorithms wrong causes extremely hard-to-debug race conditions, deadlocks, or data corruption.

**Prevention:**
- Start with a simple scheduler: one OS thread per core, each with its own run queue, no work stealing. This is correct and simple. Work stealing is an optimization.
- Use well-tested concurrent queue implementations (crossbeam in Rust) rather than rolling your own.
- If implementing work stealing, study BEAM's migration logic and Go's scheduler, both of which are well-documented and battle-tested.
- Use loom or similar tools for testing concurrent data structures under all possible interleavings.

**Phase mapping:** Phase 3 (actor runtime). Start simple, add work stealing only when benchmarks show it is needed.

**Severity:** MEDIUM

---

### R2: Message Queue Backpressure and Mailbox Overflow

**What goes wrong:** Unbounded actor mailboxes. A fast producer can fill a slow consumer's mailbox, exhausting memory. BEAM handles this by slowing down the sender (implicit backpressure through scheduling) and by providing process monitoring, but this is emergent behavior, not a designed solution.

**Prevention:**
- Design mailbox overflow policy from the start: bounded mailboxes with configurable behavior on overflow (block sender, drop oldest, drop newest, crash).
- Consider typed mailboxes that distinguish between different message priorities.
- Provide monitoring: allow querying mailbox depth, and provide supervisor-level alerts when mailboxes grow too large.

**Phase mapping:** Phase 3 (actor runtime design).

**Severity:** MEDIUM

---

### R3: Actor Supervision Trees Without Proper Isolation

**What goes wrong:** If one actor crashing can corrupt another actor's state (shared memory, shared file handles, shared network connections), the "let it crash" philosophy becomes "let it corrupt." BEAM's per-process isolation is absolute -- processes share nothing. This is what makes supervision trees safe.

**Prevention:**
- Enforce strict isolation between actors. No shared mutable state, period. Messages must be copied or moved, not shared.
- Actor-internal state must be fully encapsulated. No references to one actor's state from another.
- Design supervision trees as a first-class concept, not a library afterthought.
- Test: crash one actor, verify that its supervisor can restart it and no other actor is affected.

**Phase mapping:** Phase 3 (actor runtime). Isolation must be enforced from the start.

**Severity:** MEDIUM

---

## LLVM/Codegen Pitfalls

### G1: Using LLVM's Optimization Pipelines Without Understanding Them

**What goes wrong:** Passing Snow's IR through LLVM's -O2 pipeline produces worse code than expected, or even incorrect code, because the pipeline assumes C/C++ semantics (e.g., strict aliasing, no-wrap arithmetic, UB exploitation).

**Prevention:**
- Start with NO optimization passes. Get correct code generation first.
- Add optimization passes one at a time, testing after each addition.
- Understand what each pass assumes about the input IR. Some passes (like TBAA-based alias analysis) assume C/C++ type-based aliasing rules that may not apply to Snow.
- Write a pass pipeline specifically for Snow, starting from LLVM's pipeline and removing/modifying passes that assume C semantics.

**Phase mapping:** Phase 2 (codegen), after correctness is established.

**Severity:** MEDIUM

---

### G2: Exception/Panic Handling Model Mismatch

**What goes wrong:** LLVM's exception handling is designed around C++ ABI-based exceptions (Itanium, SEH). Snow's error handling model (likely Result types + actor crash semantics) does not map cleanly to these mechanisms. Trying to use LLVM's EH for Snow's semantics leads to mismatches, especially for nested handlers and cross-actor crashes.

**Prevention:**
- Do NOT use LLVM's exception handling for Snow's normal error flow. Result types should be compiled as tagged returns (sum types), not exceptions.
- Reserve LLVM's exception handling only for truly unrecoverable panics (e.g., out-of-memory, stack overflow) that need to unwind the stack for cleanup.
- If actors need stack unwinding on crash, implement this in the runtime (longjmp-style or manual stack walking) rather than through LLVM EH.

**Phase mapping:** Phase 2 (codegen), but the error model must be decided in Phase 1.

**Severity:** MEDIUM

---

### G3: `undef` and `poison` Values in Generated IR

**What goes wrong:** LLVM's `undef` and `poison` values are the IR equivalent of undefined behavior. If Snow's codegen accidentally produces `undef` values (e.g., uninitialized variables), LLVM can optimize in surprising ways -- including "deleting" code that appears correct.

**Prevention:**
- Snow should not have uninitialized variables (functional-first design helps here). Every variable binding should have an initializer.
- In the codegen, explicitly initialize all LLVM values. Never rely on `undef` as a default.
- Use LLVM's verification pass (`llvm::verifyModule`) after generating IR to catch malformed IR early.
- Understand the difference between `undef` (any value, chosen independently each use) and `poison` (the result of UB, propagates through operations). Avoid both.

**Phase mapping:** Phase 2 (codegen).

**Severity:** MEDIUM

---

## Project Management Pitfalls

### P1: Trying to Build Everything at Once

**What goes wrong:** Attempting to build the lexer, parser, type checker, codegen, runtime, standard library, package manager, and LSP server simultaneously. Nothing reaches a usable state.

**Prevention:**
- Strict phasing: each phase produces a working, testable artifact.
- Phase 1: Lexer + Parser + AST printer (can parse Snow files and pretty-print them).
- Phase 2: Type checker + LLVM codegen for sequential code (can compile and run pure functions).
- Phase 3: Actor runtime + message passing (can run concurrent actors).
- Phase 4: Standard library + tooling (usable for real programs).
- Each phase should take 1-3 months. Do not start the next phase until the current one is solid.

**Severity:** HIGH

---

### P2: Premature Self-Hosting

**What goes wrong:** Attempting to rewrite the compiler in Snow before Snow is mature enough. This creates a chicken-and-egg problem and means you are debugging the compiler and the language simultaneously. Rust started in OCaml and only self-hosted after years of development.

**Prevention:**
- Keep the compiler in Rust indefinitely. Self-hosting is a milestone, not a goal.
- Self-hosting should only be attempted when: (1) the language is stable enough that the compiler written in Snow would not need constant changes due to language evolution, (2) the standard library is rich enough to support a compiler (file I/O, string manipulation, data structures), and (3) there is a clear bootstrap path documented.

**Phase mapping:** Not before v1.0, if ever.

**Severity:** MEDIUM

---

### P3: Not Having a Working Test Suite from Day One

**What goes wrong:** Writing the compiler without a comprehensive test suite. Each change introduces regressions that are not caught, leading to a "two steps forward, one step back" development pattern.

**Prevention:**
- Write snapshot tests (input program -> expected output/error) before implementing features.
- For the type checker, test both acceptance (programs that should type-check) and rejection (programs that should be errors, with expected error messages).
- For codegen, test compiled output against expected runtime behavior.
- Run the full test suite on every commit (CI).
- Use property-based testing (quickcheck) for the type checker: generate random well-typed programs and verify they type-check, generate random ill-typed programs and verify they are rejected.

**Phase mapping:** Phase 0 (project setup). The test framework should exist before the first line of compiler code.

**Severity:** HIGH

---

### P4: Designing in Isolation Without User Feedback

**What goes wrong:** Building the entire language without ever getting feedback from potential users. The language ends up theoretically elegant but practically unusable.

**Prevention:**
- Share the language design document early. Get feedback on syntax, semantics, and the overall "feel" before implementation is complete.
- Release a minimal working version (even if it only supports integers and basic functions) and let people try it.
- Write real programs in Snow as early as possible. "Dogfood" the language for its own build scripts, test harness, or documentation generator.

**Phase mapping:** Ongoing from Phase 2 onwards.

**Severity:** MEDIUM

---

## Phase-Specific Warning Summary

| Phase | Topic | Likely Pitfall | Mitigation |
|-------|-------|---------------|------------|
| Phase 0 | Project Setup | H1: LLVM build complexity | Pin LLVM version, provide Docker/Nix, document everything |
| Phase 0 | Project Setup | P3: No test suite | Set up test framework before writing compiler code |
| Phase 1 | Language Design | C3: Preemption strategy | Decide compiler-inserted yield points vs. signals upfront |
| Phase 1 | Language Design | L1: Syntax compounding | Formal grammar, LL(1) parseable, lock operator precedence |
| Phase 1 | Language Design | M2: Error handling model | Design Result types + actor crash semantics before coding |
| Phase 1 | Architecture | H3: AST representation | Choose arena + index-based AST before writing any passes |
| Phase 2 | Type System | C2: HM implementation bugs | Constraint-gen-then-solve, comprehensive test suite, occurs check |
| Phase 2 | Type System | H2: Bad error messages | Separate constraint solving, multi-location errors |
| Phase 2 | Type System | M1: Extensions breaking inference | Lock type system scope, no scope creep |
| Phase 2 | Type System | M3: Exhaustiveness checking | Maranget's algorithm, handle guards conservatively |
| Phase 2 | Codegen | H5: Slow -O0 compile times | Consider interpreter/fast backend for dev mode |
| Phase 2 | Codegen | M4: Missing IR annotations | Study LLVM performance tips, mark purity/aliasing |
| Phase 2 | Codegen | M5: ABI mismatches | Define Snow calling convention, test FFI on all platforms |
| Phase 3 | Actor Runtime | C1: Building too much at once | Runtime is standalone Rust library, tested independently |
| Phase 3 | Actor Runtime | C4: Typed message passing | NVLang approach: ADTs as protocols, Pid[T], Future[T] |
| Phase 3 | Actor Runtime | H4: Per-actor GC | Start with refcounting, defer cycle detection |
| Phase 3 | Actor Runtime | R1: Work-stealing complexity | Start simple (no stealing), use crossbeam, add later |
| Phase 3 | Actor Runtime | R2: Mailbox overflow | Bounded mailboxes, configurable overflow policy |
| Phase 3 | Actor Runtime | R3: Isolation failures | Strict isolation, copy/move messages, no shared state |
| Phase 4 | Tooling | P2: Premature self-hosting | Stay in Rust, self-host only after v1.0 if ever |
| Phase 4 | Tooling | P4: No user feedback | Release early, dogfood, gather feedback from Phase 2 |

---

## Sources

### Academic / Authoritative
- [Hindley-Milner type system (Wikipedia)](https://en.wikipedia.org/wiki/Hindley%E2%80%93Milner_type_system)
- [NVLang: Unified Static Typing for Actor-Based Concurrency on the BEAM](https://arxiv.org/abs/2512.05224)
- [Warnings for Pattern Matching (Maranget, 2007)](https://www.cambridge.org/core/journals/journal-of-functional-programming/article/warnings-for-pattern-matching/3165B75113781E2431E3856972940347)
- [Ownership and Reference Counting based GC in the Actor World (Pony ORCA)](https://www.doc.ic.ac.uk/~scd/icooolps15_GC.pdf)
- [Getting into the Flow: Better Type Error Messages](https://doi.org/10.1145/3622812)
- [Repairing Type Errors in Functional Programs (McAdam)](https://www.lfcs.inf.ed.ac.uk/reports/02/ECS-LFCS-02-427/ECS-LFCS-02-427.pdf)

### Official Documentation
- [LLVM Performance Tips for Frontend Authors](https://llvm.org/docs/Frontend/PerformanceTips.html)
- [LLVM FAQ](https://llvm.org/docs/FAQ.html)
- [Rust Compiler Development Guide](https://rustc-dev-guide.rust-lang.org/overview.html)
- [Rust Pattern and Exhaustiveness Checking](https://rustc-dev-guide.rust-lang.org/pat-exhaustive-checking.html)
- [BEAM Book: Scheduling](https://github.com/happi/theBeamBook/blob/master/chapters/scheduling.asciidoc)
- [Inkwell GitHub](https://github.com/TheDan64/inkwell)
- [Go Non-cooperative Preemption Proposal](https://go.googlesource.com/proposal/+/master/design/24543-non-cooperative-preemption.md)

### Implementation Accounts / Post-Mortems
- [LLVM: The Bad Parts (nikic, 2026)](https://www.npopov.com/2026/01/11/LLVM-The-bad-parts.html)
- [How Bad is LLVM Really? (C3 compiler)](https://c3.handmade.network/blog/p/8852-how_bad_is_llvm_really)
- [Writing a Compiler in Rust (Tristan Hume)](https://thume.ca/2019/04/18/writing-a-compiler-in-rust/)
- [A retrospective on toy PL design mistakes (Ink)](https://dotink.co/posts/pl-design-mistakes/)
- [5 Mistakes in Programming Language Design (Zwinkau)](https://beza1e1.tuxen.de/articles/proglang_mistakes.html)
- [Hundred Year Mistakes (Eric Lippert)](https://ericlippert.com/2020/02/27/hundred-year-mistakes/)
- [Write You a Haskell - HM Inference](http://dev.stephendiehl.com/fun/006_hindley_milner.html)
- [Writing Good Compiler Error Messages](https://calebmer.com/2019/07/01/writing-good-compiler-error-messages.html)

### Erlang/Actor Model
- [Erlang Scheduler Details](https://hamidreza-s.github.io/erlang/scheduling/real-time/preemptive/migration/2016/02/09/erlang-scheduler-details.html)
- [Deep Diving Into the Erlang Scheduler (AppSignal)](https://blog.appsignal.com/2024/04/23/deep-diving-into-the-erlang-scheduler.html)
- [Types (or lack thereof) - Learn You Some Erlang](https://learnyousomeerlang.com/types-or-lack-thereof)
- [Why isn't Erlang strongly typed? (mailing list)](http://erlang.org/pipermail/erlang-questions/2008-October/039261.html)
