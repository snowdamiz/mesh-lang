# Stack Research: Snow Language

**Project:** Snow Programming Language
**Researched:** 2026-02-05
**Mode:** Ecosystem survey -- compiler toolchain, runtime, and testing infrastructure

---

## Compiler Toolchain

### Lexer

**Recommendation: Hand-written lexer (not a generator)**

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Hand-written lexer | N/A | Tokenization | Full control over error recovery, span tracking, whitespace handling; simpler than it sounds for a language with known syntax |

**Rationale:** For a production language compiler, a hand-written lexer gives you complete control over error recovery, Unicode handling, and diagnostics. The Rust compiler itself uses a hand-written lexer (`rustc_lexer`). A lexer is the simplest part of a compiler -- typically 300-800 lines of Rust for a language like Snow. Generators like Logos add a dependency and abstraction layer for a problem that doesn't need one.

**If you want a generator anyway:** Logos v0.16 (released Dec 2025) is the clear choice. It rewrote its engine from scratch for correctness, compiles token definitions into a deterministic state machine at compile time, and is ridiculously fast. However, for a language with `do`/`end` blocks, significant indentation awareness (if desired later), string interpolation, and heredocs, you will outgrow a generator quickly.

| Alternative | Version | Why Not |
|-------------|---------|---------|
| Logos | 0.16 | Excellent for simple token sets but becomes limiting for complex lexer needs (string interpolation, context-sensitive tokens). Adds a proc-macro dependency for a problem that's straightforward to solve by hand. |
| lexgen | N/A | Less popular, fewer users to learn from |

**Confidence: HIGH** -- This is well-established consensus. Every major production compiler (rustc, GCC, Clang, Go, Swift) uses a hand-written lexer.

---

### Parser

**Recommendation: Hand-written recursive descent parser**

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Hand-written recursive descent | N/A | Parsing source into AST/CST | Best error recovery, best diagnostics, best LSP support, no black-box dependencies |
| rowan | latest | CST (Concrete Syntax Tree) representation | Red-green tree pattern used by rust-analyzer; enables lossless syntax trees for IDE support |

**Rationale:** The overwhelming consensus in the language implementation community is that hand-written recursive descent parsers are the right choice for production compilers. Reasons:

1. **Error recovery:** You control exactly how to recover from malformed input, which is critical for IDE integration and helpful diagnostics. Parser combinators and generators make this much harder.
2. **Diagnostics:** You can emit exactly the error message you want at exactly the right point. "Expected `end` to close `do` block started on line 5" is trivial in a hand-written parser, painful in a combinator.
3. **Performance:** No abstraction overhead. A recursive descent parser for Snow will be extremely fast.
4. **IDE support:** If Snow ever gets an LSP server, you need incremental, error-tolerant parsing. Hand-written parsers with rowan-based CSTs are the proven path (this is exactly what rust-analyzer does).
5. **Simplicity:** A recursive descent parser is ~1500-3000 lines of clear, debuggable Rust code. Parser combinators create opaque type soup.

The Rust compiler, Go compiler, Swift compiler, GCC, and Clang all use hand-written parsers.

**CST vs AST strategy:** Start with a direct AST approach (simpler for v1). Adopt rowan-based CST later when you need LSP/IDE support. The CST-then-lower-to-AST pattern used by rust-analyzer is the gold standard but adds complexity you don't need on day one.

**Pratt parsing for expressions:** Use a Pratt parser (precedence climbing) for expression parsing within the recursive descent framework. This handles operator precedence elegantly and is about 50-100 lines of code.

| Alternative | Version | Why Not |
|-------------|---------|---------|
| chumsky | 0.11.1 | Excellent combinator library with error recovery. The best choice IF you go the combinator route. But compile times suffer with complex grammars, and you're fighting Rust's type system instead of writing straightforward parsing code. Good for prototyping, not for a production compiler. |
| LALRPOP | latest | Grammar-driven, generates Rust code. Nice for simple languages. But the built-in lexer is a "toy" (author's words), writing custom lexers for it is painful, and you lose control over error messages. |
| winnow | latest | Fast parser combinator (nom fork). Good for binary protocols and data formats. Not designed for programming language parsers with rich error recovery needs. |
| tree-sitter | latest | Designed for editors, not compilers. Generates C code. Wrong tool for a compiler's parser. |

**Confidence: HIGH** -- This is the strongest consensus in language implementation. The survey "Parser generators vs handwritten parsers" (2021) confirmed that virtually all major language implementations use hand-written parsers.

---

### Type System

**Recommendation: Custom implementation, inspired by existing Rust HM implementations**

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Custom type checker | N/A | Hindley-Milner type inference with extensions | No off-the-shelf crate provides what Snow needs; type systems are inherently custom |
| ena | latest | Union-find data structure for type unification | Used by rustc itself for type inference; battle-tested |

**Rationale:** There is no "type system library" you can drop in. Type inference and checking are deeply intertwined with your language's semantics. However, the implementation approach is well-understood:

1. **Algorithm W / Algorithm J** for core HM inference (constraint generation + unification)
2. **Union-find** (ena crate) for efficient type variable unification
3. **Bidirectional type checking** as an extension for better error messages than pure HM

Key references for implementation:
- `tcr/rust-hindley-milner` -- clean Rust HM implementation
- `zdimension/hm-infer-rs` -- HM for Scheme in Rust, with currying support
- `nwoeanhinnogaehr/algorithmw-rust` -- Algorithm W in Rust
- The Rust compiler's own type inference (based on HM with extensions for subtyping, regions)

**Implementation approach:**
- Phase 1: Basic HM inference (let-polymorphism, function types, ADTs)
- Phase 2: Row polymorphism or similar for records/maps
- Phase 3: Typeclass/trait-like dispatch for ad-hoc polymorphism (if desired)

The `polytype` crate (crates.io) provides HM polymorphic typing primitives, but it's better to understand and implement this yourself since the type system is the heart of the language and you'll need to extend it in ways no library anticipates.

**Confidence: MEDIUM** -- The approach is well-understood (HM is textbook material), but implementation complexity is high. The specific extensions needed for Snow (actor types? message types? supervision types?) are novel and will require design work.

---

### LLVM Code Generation

**Recommendation: Inkwell 0.8.0 with LLVM 18 (stable target)**

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| inkwell | 0.8.0 | Safe Rust wrapper for LLVM | Type-safe LLVM IR generation; catches many errors at Rust compile time rather than LLVM runtime. Active development, supports LLVM 11-21. |
| LLVM | 18 (target), 19-21 (future) | Code generation backend | LLVM 18 is the sweet spot: widely available via package managers, well-tested, stable. |

**Why inkwell over llvm-sys:**
- `inkwell` wraps `llvm-sys` with Rust's type system, turning many runtime LLVM crashes into compile-time errors
- Creating basic blocks, building instructions, managing types -- all have safe Rust APIs
- The Kaleidoscope tutorial is implemented in inkwell, providing a clear learning path
- Still provides escape hatches to raw llvm-sys when needed

**Why LLVM 18 as the initial target:**
- Available via `brew install llvm@18` on macOS, apt packages on Linux
- Well-tested, stable API
- inkwell 0.8.0 supports it with feature flag `llvm18-0`
- Can upgrade to LLVM 19/20/21 later by changing one feature flag
- LLVM 21 is current bleeding edge (Homebrew default), but targeting the latest adds install friction for contributors

**Installation:**
```toml
[dependencies]
inkwell = { version = "0.8.0", features = ["llvm18-0"] }
```

**Note:** Inkwell 0.8.0 was released 2026-01-09 on GitHub. The crates.io published version may still be 0.7.1. If 0.8.0 is not yet on crates.io, use a git dependency:
```toml
[dependencies]
inkwell = { git = "https://github.com/TheDan64/inkwell", features = ["llvm18-0"] }
```

| Alternative | Version | Why Not |
|-------------|---------|---------|
| llvm-sys | 211.0.0 | Raw C bindings. No type safety. Every mistake is a segfault or silent corruption. Only use if inkwell can't do something (rare). |
| Cranelift | latest | Compiles faster than LLVM but generates slower code (~2x slower in some benchmarks). No mature optimization pipeline. Missing many targets. Good for debug builds of Rust itself, but Snow needs LLVM's optimization quality for "compiled to native" to be compelling. |
| QBE | latest | Minimalist backend. ~10% of LLVM's code. Interesting for a learning project but lacks optimization power and target coverage. |

**Confidence: HIGH** -- inkwell is the standard choice for Rust-based LLVM projects. Verified on GitHub that 0.8.0 supports LLVM 18-21.

---

### Build and Package Management

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Cargo | (Rust toolchain) | Build system for the compiler itself | Standard Rust build tool. No alternative needed. |
| Cargo workspace | N/A | Multi-crate project structure | Split compiler into `snow-lexer`, `snow-parser`, `snow-typeck`, `snow-codegen`, `snow-runtime` crates |

**Recommended workspace structure:**
```
snow/
  Cargo.toml              # workspace root
  crates/
    snow-driver/          # CLI entry point, orchestration
    snow-lexer/           # Tokenization
    snow-parser/          # Parsing (AST types live here)
    snow-ast/             # AST type definitions (shared)
    snow-typeck/          # Type inference and checking
    snow-hir/             # High-level IR (desugared, typed)
    snow-codegen/         # LLVM IR generation
    snow-runtime/         # Rust runtime library that gets compiled and linked into Snow binaries
  tests/                  # Integration/golden tests
  stdlib/                 # Snow standard library source files
```

**Confidence: HIGH** -- Cargo workspaces for multi-crate Rust projects is standard practice.

---

### Supporting Libraries

| Library | Version | Purpose | Why |
|---------|---------|---------|-----|
| ariadne | 0.6.0 | Error/diagnostic reporting | Beautiful Rust-style compiler error messages. Sister project of chumsky, but works independently. Produces the best-looking diagnostics in the Rust ecosystem. |
| lasso | 0.7.3 | String interning | Fast, concurrent string interner. Turns repeated string comparisons into integer comparisons. Essential for compiler performance. Used widely in language tooling. |
| ena | latest | Union-find for type inference | Used by rustc itself. The standard choice for HM unification. |
| salsa | 0.25.2 | Incremental computation (FUTURE) | Framework used by rust-analyzer for incremental recompilation. Not needed for v1 but is the path to fast IDE support. Defer until LSP work begins. |

**Why ariadne over miette:**
- ariadne is purpose-built for compiler diagnostics -- multi-line labels, span overlaps, color generation
- miette is a broader error-handling framework (great for CLI apps, overkill for a compiler)
- ariadne is made by the same author as chumsky and is designed for language tooling
- Lighter dependency footprint

**Why lasso for string interning:**
- Compilers compare identifiers millions of times; interning makes this O(1)
- lasso provides both single-threaded (`Rodeo`) and multi-threaded (`ThreadedRodeo`) interners
- Arena-allocated, minimal memory overhead
- 6M+ downloads, well-maintained

**Confidence: HIGH** for ariadne and lasso (verified versions, widely used). MEDIUM for salsa (defer to later phase, API still evolving).

---

## Actor Runtime

This is the most architecturally significant decision for Snow. The runtime gets compiled as a Rust library and statically linked into every Snow binary.

### Architecture Decision: Custom Runtime on Tokio

**Recommendation: Build a custom actor runtime on top of Tokio's work-stealing scheduler**

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| tokio | 1.49.0 (LTS: 1.47.x) | Async runtime foundation | Work-stealing task scheduler, I/O reactor, timers. The foundation upon which Snow's actor runtime is built. |
| Custom runtime | N/A | Actor semantics, supervision, mailboxes | Neither ractor nor kameo match Snow's exact needs. The runtime IS the language. |

**Why custom over ractor/kameo:**
1. **The runtime is the language.** Snow's runtime isn't a library users import -- it's the execution model. Every Snow program runs on this runtime. Using someone else's actor framework means your language semantics are constrained by their API decisions.
2. **BEAM-style semantics require specific guarantees** that existing Rust actor frameworks approximate but don't deliver:
   - Per-actor mailboxes with selective receive (pattern matching on messages)
   - Reduction-counting preemption (neither ractor nor kameo do this)
   - Per-actor garbage collection (Rust doesn't GC, but Snow's runtime may need to manage Snow-level heap objects)
   - Process linking and monitoring (ractor has this; kameo has a different model)
   - Process registry (name-based lookup)
3. **Control over the wire format.** When Snow compiles pattern matching on messages, the codegen needs to know exactly how messages are laid out. With a third-party framework, you're constrained to their message types.

**Why Tokio as the foundation (not from-scratch):**
- Tokio provides the hard parts: epoll/kqueue integration, work-stealing scheduler, timer wheel, cross-platform I/O
- Writing a scheduler from scratch is a multi-year effort (Pony's took years)
- Tokio's scheduler is battle-tested at massive scale (used by AWS, Discord, Cloudflare)
- `tokio::task::spawn` maps naturally to "spawn actor" -- each actor is a Tokio task
- `tokio::sync::mpsc` provides the mailbox primitive (bounded channels with backpressure)
- LTS releases (1.43.x until March 2026, 1.47.x until September 2026) ensure stability

### Scheduler Architecture

**Approach: Tokio work-stealing scheduler + cooperative yielding**

The BEAM uses reduction counting for preemptive scheduling. Snow can approximate this:

1. **Tokio's cooperative task budgeting:** Tokio already has a cooperative scheduling mechanism where tasks yield back to the scheduler after a budget of operations. This is not reduction counting, but it prevents any single actor from monopolizing a thread.
2. **Yield points in generated code:** The Snow compiler can insert explicit yield points (calls to `tokio::task::yield_now()`) at strategic locations in generated code: loop back-edges, function calls, message receives. This is how Go's goroutine scheduler works and is the pragmatic approach.
3. **One scheduler thread per core:** Tokio's `Runtime::new()` creates a multi-threaded work-stealing scheduler with one worker thread per CPU core, matching BEAM's architecture.

**What Snow's runtime needs to provide (as a Rust library linked into binaries):**
- `snow_rt::spawn(actor_fn)` -- spawn a new actor
- `snow_rt::send(pid, message)` -- send a message to an actor's mailbox
- `snow_rt::receive(patterns)` -- selective receive with pattern matching
- `snow_rt::link(pid)` / `snow_rt::monitor(pid)` -- process linking and monitoring
- `snow_rt::supervisor::start(strategy, children)` -- supervision tree primitives
- `snow_rt::registry::register(name, pid)` -- process name registry

### Message Passing

| Component | Implementation | Notes |
|-----------|---------------|-------|
| Mailbox | `tokio::sync::mpsc::unbounded_channel` | Each actor gets one. Unbounded to match BEAM semantics (BEAM mailboxes are unbounded). |
| Message type | Enum or tagged union | Snow-level messages are compiled to a Rust enum representing the message variants the actor can receive. |
| Selective receive | Pattern match on channel | The actor loops over the channel, matching messages against patterns. Non-matching messages are deferred (re-queued). |
| Send semantics | Async, non-blocking | `send` never blocks the sender. If the mailbox is full (if bounded), messages are queued. |
| Request-reply | `tokio::sync::oneshot` | For call-style interactions where a response is expected. |

### Supervision Trees

**Model: Erlang OTP-style supervision**

| Component | Implementation |
|-----------|---------------|
| Supervisor | A special actor that manages child actors |
| Restart strategies | `one_for_one`, `one_for_all`, `rest_for_one` |
| Child specs | Declarative: how to start, max restarts, restart type (permanent/transient/temporary) |
| Links | Bidirectional: if linked actor dies, linked partner gets exit signal |
| Monitors | Unidirectional: monitoring actor gets notified of monitored actor's death |
| Exit signals | Propagated through links; can be trapped by supervisors |

The supervision implementation doesn't need external libraries. It's ~500-1000 lines of Rust code built on the mailbox and linking primitives.

### What NOT to use for the runtime

| Technology | Why Not |
|------------|---------|
| ractor 0.15.10 | Closest to BEAM semantics of any Rust actor framework. Has supervision, linking, monitoring, registry. BUT: you'd be coupling your language to a third-party framework's API. If ractor changes its `Actor` trait, your language breaks. The runtime IS the language -- own it. **Study ractor's design** (it's excellent), but implement your own. |
| kameo 0.19.2 | More Rust-idiomatic than ractor, but further from BEAM semantics. Actor linking model differs from Erlang's. Supervision is lifecycle-hook-based rather than tree-based. |
| actix | Older framework, pre-async/await design. Not a good foundation for a new project. |
| bastion | Designed as a fault-tolerant runtime but has seen less maintenance. |

**Confidence: HIGH** for Tokio as foundation. **MEDIUM** for the specific runtime architecture -- this will need iterative refinement as you discover what Snow's semantics actually require.

---

## Testing Infrastructure

### Compiler Testing

| Tool | Version | Purpose | When |
|------|---------|---------|------|
| insta | latest | Snapshot testing | Test lexer output, AST structure, type inference results, generated LLVM IR. THE standard tool for compiler testing in Rust. |
| cargo-insta | latest | CLI for reviewing/updating snapshots | `cargo insta test` to run, `cargo insta review` to update |
| goldentests | latest | End-to-end golden tests | Compile Snow source files, run the binary, compare output against expected. Perfect for testing the full pipeline. |
| datatest-stable | latest | Data-driven tests on stable Rust | Generate test cases from directories of `.snow` files |

**Testing strategy by compiler phase:**

| Phase | Testing Approach |
|-------|-----------------|
| Lexer | Snapshot tests: input source -> token stream |
| Parser | Snapshot tests: input source -> AST (debug-printed) |
| Type checker | Snapshot tests: input source -> inferred types per expression |
| Error reporting | Snapshot tests: malformed source -> diagnostic output (stderr) |
| Code generation | Snapshot tests: input source -> LLVM IR |
| End-to-end | Golden tests: `.snow` source + expected stdout + expected exit code |
| Runtime | Unit tests for actor spawn/send/receive/supervision using `#[tokio::test]` |

**Why insta for compiler testing:**
- The Rust project itself adopted insta for bootstrap snapshot testing (2025)
- `cargo insta review` provides a TUI for reviewing changes, which is critical when compiler output changes
- Supports redaction (hide non-deterministic parts like memory addresses)
- VS Code extension for inline snapshot review
- Adrian Sampson (Cornell) explicitly recommends snapshot testing for "compilers and compiler-like things"

**Test directory structure:**
```
tests/
  lexer/
    snapshots/              # insta snapshot files
    test_lexer.rs
  parser/
    snapshots/
    test_parser.rs
  typeck/
    snapshots/
    test_typeck.rs
  codegen/
    snapshots/
    test_codegen.rs
  ui/                       # end-to-end golden tests
    hello_world.snow
    hello_world.stdout       # expected output
    pattern_match.snow
    pattern_match.stdout
    type_error.snow
    type_error.stderr        # expected error output
  runtime/
    test_actors.rs
    test_supervision.rs
```

**Confidence: HIGH** -- insta is the de facto standard for snapshot testing in Rust, verified by its adoption in the Rust compiler's own test infrastructure.

### Runtime Testing

| Tool | Purpose |
|------|---------|
| `#[tokio::test]` | Async test harness for actor/supervision tests |
| `tokio::time::pause()` | Deterministic time control for timeout/timer tests |
| `tracing` + `tracing-test` | Structured logging assertions for runtime behavior |

---

## Recommended Stack Summary

### Core (Day 1)

```toml
[workspace]
members = ["crates/*"]

# In crates/snow-driver/Cargo.toml
[dependencies]
snow-lexer = { path = "../snow-lexer" }
snow-parser = { path = "../snow-parser" }
snow-ast = { path = "../snow-ast" }
snow-typeck = { path = "../snow-typeck" }
snow-codegen = { path = "../snow-codegen" }
snow-runtime = { path = "../snow-runtime" }
clap = { version = "4", features = ["derive"] }

# In crates/snow-lexer/Cargo.toml
[dependencies]
lasso = "0.7"
# (hand-written lexer, minimal dependencies)

# In crates/snow-parser/Cargo.toml
[dependencies]
snow-ast = { path = "../snow-ast" }
lasso = "0.7"
# (hand-written parser, minimal dependencies)

# In crates/snow-ast/Cargo.toml
[dependencies]
lasso = "0.7"

# In crates/snow-typeck/Cargo.toml
[dependencies]
snow-ast = { path = "../snow-ast" }
ena = "0.14"  # union-find for type unification

# In crates/snow-codegen/Cargo.toml
[dependencies]
snow-ast = { path = "../snow-ast" }
inkwell = { version = "0.8.0", features = ["llvm18-0"] }

# In crates/snow-runtime/Cargo.toml
[dependencies]
tokio = { version = "1.49", features = ["full"] }

# In workspace Cargo.toml (shared dev dependencies)
[workspace.dependencies]
insta = { version = "1", features = ["yaml"] }

# Dev dependencies in each crate
[dev-dependencies]
insta = { workspace = true }
```

### Diagnostic Reporting

```toml
# In crates/snow-driver/Cargo.toml (or a shared snow-diagnostics crate)
[dependencies]
ariadne = "0.6"
```

### For Later Phases

| Library | When | Purpose |
|---------|------|---------|
| salsa 0.25.2 | LSP/IDE phase | Incremental recompilation for language server |
| rowan | LSP/IDE phase | Lossless CST for IDE support |
| tower-lsp | LSP/IDE phase | Language Server Protocol implementation |
| serde + serde_json | Package manager phase | Config file parsing |

---

## What NOT to Use (and Why)

| Technology | Why Not |
|------------|---------|
| **Cranelift** (as primary backend) | Generates code ~2x slower than LLVM in benchmarks. Snow's value prop includes "compiled to native" -- that needs LLVM's optimization quality. Cranelift is great for debug builds of Rust, not for a language whose selling point is fast compiled binaries. |
| **Any parser generator** (LALRPOP, pest, tree-sitter) | You will fight the generator for error recovery, diagnostics, and incremental parsing. Every major production compiler uses a hand-written parser. The "time savings" of a generator are illusory -- you spend the saved time learning the generator's quirks instead. |
| **chumsky** (for production parser) | Excellent library, but Rust compile times for complex grammars are painful, and debugging combinator type errors is worse than debugging a hand-written parser. Fine for prototyping. |
| **miette** (for compiler diagnostics) | It's a general error-handling framework. ariadne is purpose-built for compiler diagnostics with better multi-span label rendering. |
| **Any ORM/database** | Snow is a compiler, not a web app. No database needed. |
| **ractor/kameo/actix** (as the runtime) | The runtime IS the language. Using a third-party actor framework means your language semantics are determined by someone else's API. Study them (especially ractor), but build your own on Tokio. |
| **async-std** | Tokio won. 437M downloads vs. ~30M for async-std. The ecosystem, tooling, and community are all on Tokio. |
| **Logos** (for production lexer) | Overkill dependency for what should be a simple hand-written component. Logos shines for complex regex-based tokenization, but a language lexer is straightforward state machine code. |

---

## Confidence Levels

| Area | Confidence | Rationale |
|------|------------|-----------|
| **Lexer approach** (hand-written) | HIGH | Verified: all major compilers (rustc, GCC, Clang, Go, Swift) use hand-written lexers. Multiple authoritative sources agree. |
| **Parser approach** (hand-written RD) | HIGH | Verified: 2021 survey of major language implementations confirms. rustc, Go, Swift, GCC all hand-written. Community consensus overwhelming. |
| **LLVM via inkwell** | HIGH | Verified: inkwell 0.8.0 released 2026-01-09, supports LLVM 11-21. Standard choice for Rust LLVM projects. GitHub confirms version and features. |
| **LLVM version** (target 18) | HIGH | Verified: LLVM 18 available via `brew install llvm@18`. Well-tested. inkwell supports it. |
| **Type system approach** (custom HM) | MEDIUM | Approach is textbook, but implementation complexity is high. No off-the-shelf solution. Multiple reference implementations exist in Rust but none are production-grade libraries. |
| **Tokio as runtime foundation** | HIGH | Verified: Tokio 1.49.0 current, LTS 1.47.x until Sept 2026. 437M+ downloads. Work-stealing scheduler confirmed. |
| **Custom runtime over ractor/kameo** | MEDIUM | Architecturally sound reasoning, but adds significant implementation work. Risk: might underestimate effort and wish you'd used ractor. Mitigation: study ractor's source code closely before implementing. |
| **ariadne for diagnostics** | HIGH | Verified: v0.6.0. Sister project of chumsky. Purpose-built for compiler diagnostics. |
| **lasso for string interning** | HIGH | Verified: v0.7.3, 6M+ downloads. Used widely in language tooling. |
| **insta for testing** | HIGH | Verified: adopted by Rust compiler's own bootstrap. De facto standard for snapshot testing in Rust. |
| **Workspace structure** | HIGH | Standard Rust practice for multi-crate projects. |

---

## Open Questions for Later Phases

1. **Memory management for Snow values:** Snow is functional (immutable by default), so reference counting (like Swift) may work. But actors need isolated heaps (like BEAM). How does the generated code manage Snow-level objects? This needs dedicated research.

2. **Message serialization format:** How are Snow-level values serialized into actor mailbox messages? Zero-copy? Deep copy? This affects both codegen and runtime design.

3. **Preemption granularity:** How aggressively should the compiler insert yield points? Every function call? Every loop iteration? Every N reductions? Needs benchmarking.

4. **Standard library scope:** What ships with Snow? TCP/UDP? HTTP? JSON? File I/O? Timer APIs? Each has implications for the runtime.

5. **FFI story:** How does Snow call C/Rust libraries? This affects the codegen significantly and needs design work.

6. **Distribution/clustering:** ractor_cluster and kameo both support distributed actors. Snow might want this eventually. The runtime architecture should not preclude it.

---

## Sources

### Verified (HIGH confidence)
- [inkwell GitHub -- v0.8.0, LLVM 11-21 support](https://github.com/TheDan64/inkwell)
- [llvm-sys crates.io -- v211.0.0](https://crates.io/crates/llvm-sys)
- [Tokio -- v1.49.0, LTS releases](https://tokio.rs/)
- [ractor GitHub -- v0.15.10, Erlang gen_server model](https://github.com/slawlor/ractor)
- [kameo GitHub -- v0.19.2, supervision and distribution](https://github.com/tqwewe/kameo)
- [Logos -- v0.16, engine rewrite Dec 2025](https://github.com/maciejhirsz/logos)
- [chumsky -- v0.11.1](https://github.com/zesterer/chumsky)
- [ariadne -- v0.6.0](https://github.com/zesterer/ariadne)
- [lasso -- v0.7.3](https://github.com/Kixiron/lasso)
- [insta -- snapshot testing](https://github.com/mitsuhiko/insta)
- [salsa -- v0.25.2](https://github.com/salsa-rs/salsa)
- [LLVM releases -- 21.1.8 latest stable, LLVM 22 in RC](https://github.com/llvm/llvm-project/releases)
- [Homebrew LLVM -- default is LLVM 21, versioned 18/19/20 available](https://formulae.brew.sh/formula/llvm)

### Verified (MEDIUM confidence)
- [Comparing Rust Actor Libraries -- Actix, Coerce, Kameo, Ractor, Xtra](https://tqwewe.com/blog/comparing-rust-actor-libraries/)
- [Actors with Tokio -- Alice Ryhl's canonical blog post](https://ryhl.io/blog/actors-with-tokio/)
- [BEAM architecture -- The BEAM Book](https://blog.stenmans.org/theBeamBook/)
- [Pony runtime architecture -- lock-free, work-stealing, per-actor GC](https://www.ponylang.io/)
- [Enigma -- Erlang VM in Rust](https://github.com/archseer/enigma)
- [Cranelift vs LLVM comparison](https://cranelift.dev/)
- [Parser generators vs handwritten parsers survey (2021)](https://lobste.rs/s/10pkib/parser_generators_vs_handwritten)

### Reference implementations
- [tcr/rust-hindley-milner](https://github.com/tcr/rust-hindley-milner)
- [algorithmw-rust](https://github.com/nwoeanhinnogaehr/algorithmw-rust)
- [hm-infer-rs](https://github.com/zdimension/hm-infer-rs)
- [Rust langdev libraries collection](https://github.com/Kixiron/rust-langdev)
- [Create Your Own Programming Language with Rust](https://createlang.rs/)
