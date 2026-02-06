# Architecture Research: Snow Language

**Domain:** Programming language compiler + actor runtime
**Researched:** 2026-02-05
**Overall Confidence:** HIGH (well-established domain with multiple reference implementations)

---

## System Overview

Snow is a compiled language with two major subsystems that must interoperate:

1. **The Compiler** -- a Rust program that transforms Snow source code into native executables
2. **The Runtime** -- a Rust static library (`libsnowrt`) that gets linked into every compiled Snow binary, providing the actor scheduler, GC, and core primitives

```
                          COMPILE TIME                                    RUN TIME
  +---------------------------------------------------------------------------+
  |                                                                           |
  |  snow source                                                              |
  |      |                                                                    |
  |      v                                                                    |
  |  [Lexer] --> tokens                                                       |
  |      |                                                                    |
  |      v                                                                    |
  |  [Parser] --> AST                                                         |
  |      |                                                                    |
  |      v                                                                    |
  |  [Type Checker] --> Typed AST                                             |
  |      |                                                                    |
  |      v                                                                    |
  |  [IR Lowering] --> Snow IR (mid-level)                                    |
  |      |                                                                    |
  |      v                                                                    |
  |  [LLVM Codegen] --> LLVM IR --> object file(s)                            |
  |      |                                                                    |
  |      v                                                                    |
  |  [Linker] ---+--- object file(s)                                          |
  |              +--- libsnowrt.a  -----> native binary                       |
  |                                           |                               |
  |                                           v                               |
  |                                  +--[Runtime Bootstrap]--+                |
  |                                  |  - init scheduler     |                |
  |                                  |  - spawn main actor   |                |
  |                                  |  - run event loop     |                |
  |                                  |  - cleanup & exit     |                |
  |                                  +-----------------------+                |
  +---------------------------------------------------------------------------+
```

The compiler is a batch tool that runs once. The runtime lives inside every produced binary and runs for the lifetime of the program.

---

## Compiler Pipeline

### Phase 1: Lexer (Tokenizer)

**Input:** Raw source text (UTF-8)
**Output:** Stream of tokens with source locations (spans)

**Responsibility:**
- Break source into atomic tokens: keywords (`do`, `end`, `def`, `fn`, `match`, `spawn`, `send`, `receive`), identifiers, literals (integers, floats, strings, atoms), operators, delimiters, newlines/indentation
- Track source positions (line, column, byte offset) for error reporting
- Handle string interpolation (emit token sequences for interpolated segments)
- Handle comments (strip or preserve for doc-comments)
- Unicode identifier support

**Architecture notes:**
- Implement as a hand-written lexer, not a generator. Hand-written lexers are standard for production languages (Rust, Go, Clang all use them) because they give full control over error recovery, performance, and edge cases like string interpolation.
- The lexer should be lazy/streaming -- produce tokens on demand rather than materializing the entire token stream upfront. This enables the parser to request tokens as needed and keeps memory usage bounded.
- Tokens should carry `Span` metadata (start/end byte offsets into the source). All later phases use spans for error messages.

**Key type:**
```rust
pub struct Token {
    pub kind: TokenKind,  // enum of all token types
    pub span: Span,       // byte range in source
}

pub struct Span {
    pub start: u32,  // byte offset
    pub end: u32,    // byte offset
    pub file_id: u16, // index into file table
}
```

**Confidence:** HIGH -- this is standard compiler architecture. Clang, rustc, and Go all use hand-written lexers. ([Rust Compiler Dev Guide](https://rustc-dev-guide.rust-lang.org/overview.html))

---

### Phase 2: Parser

**Input:** Token stream
**Output:** Untyped Abstract Syntax Tree (AST)

**Responsibility:**
- Recursive descent parsing with operator-precedence (Pratt parsing) for expressions
- Build AST nodes for all language constructs: modules, functions, expressions, pattern matching, actor operations (spawn, send, receive)
- Exhaustive error recovery -- report multiple errors per parse, don't bail on first error
- Preserve enough information for good error messages (all AST nodes carry spans)

**Architecture notes:**
- Use **recursive descent** for statements/declarations and **Pratt parsing** for expressions. This is the standard approach for languages with infix operators and is used by Rust, Clang, and V8. Pratt parsing handles precedence and associativity elegantly without grammar ambiguity.
- The parser should produce a **concrete** AST that faithfully represents the source, including parenthesization, before any desugaring. Desugaring happens in a separate pass (AST -> AST transform) or during IR lowering.
- Pattern matching syntax gets parsed into pattern AST nodes. Exhaustiveness and redundancy checking happens during type checking, not parsing. This follows Maranget's approach: the parser builds pattern trees, the type checker validates them, and IR lowering compiles them to decision trees. ([Maranget, "Compiling Pattern Matching to Good Decision Trees"](https://www.cs.tufts.edu/~nr/cs257/archive/luc-maranget/jun08.pdf))
- **Error recovery strategy:** Use synchronization tokens (like `end`, newlines, `def`). When the parser hits an error, skip tokens until it finds a synchronization point, emit an error node in the AST, and continue parsing.

**Key types:**
```rust
pub enum Expr {
    Literal(LitExpr),
    Ident(IdentExpr),
    BinaryOp(BinaryOpExpr),
    UnaryOp(UnaryOpExpr),
    Call(CallExpr),
    Block(BlockExpr),
    If(IfExpr),
    Match(MatchExpr),
    Fn(FnExpr),       // anonymous function / lambda
    Spawn(SpawnExpr),
    Send(SendExpr),
    Receive(ReceiveExpr),
    // ...
}

pub enum Pattern {
    Wildcard(Span),
    Literal(LitPattern),
    Variable(IdentPattern),
    Constructor(ConstructorPattern),  // e.g., {:ok, value}
    Tuple(TuplePattern),
    List(ListPattern),               // [head | tail]
    // ...
}
```

**Confidence:** HIGH -- recursive descent + Pratt parsing is the proven standard.

---

### Phase 3: Type Checker / Inference

**Input:** Untyped AST
**Output:** Typed AST (every expression annotated with its resolved type)

**Responsibility:**
- Hindley-Milner type inference (Algorithm W or Algorithm J) extended with:
  - Row polymorphism for structs/records (if desired)
  - Actor/process types (Pid parameterized by message type)
  - Algebraic data types and pattern matching exhaustiveness
- Type unification and constraint solving
- Error reporting with inference chain (show the user *why* the types don't match, not just *that* they don't)
- Trait/protocol resolution (if Snow has traits)
- Exhaustiveness and redundancy checking for pattern matching

**Architecture notes:**
- Start with classic **Hindley-Milner Algorithm W**. Multiple Rust implementations exist to reference: [rust-hindley-milner](https://github.com/tcr/rust-hindley-milner), [algorithmw-rust](https://github.com/nwoeanhinnogaehr/algorithmw-rust), and the [polytype](https://crates.io/crates/polytype) crate. Rust's own type inference is based on extended HM. ([Rust Dev Guide: Type Inference](https://rustc-dev-guide.rust-lang.org/type-inference.html))
- **Two-phase approach:** First, generate type constraints from the AST (constraint generation). Second, solve constraints via unification. This separation makes the system easier to test and debug.
- **Actor type safety:** The key innovation for Snow is typing actor message channels. Each actor should have a typed mailbox -- `Pid<MessageType>` -- so that `send` is type-checked at compile time. Gleam demonstrates this is achievable on the BEAM: "Gleam's type system is powerful when coupled with OTP -- you cannot send any message to your process that isn't type-checked at compile time." ([Gleam](https://gleam.run/frequently-asked-questions/))
- **Pattern matching exhaustiveness:** Implement Maranget's algorithm for checking that match expressions cover all cases and flag redundant arms. This runs during type checking because it needs type information (which constructors exist for a type).
- **Where generics get monomorphized:** Snow should monomorphize generic functions during or after type checking, before IR lowering. This means each concrete instantiation of a generic function becomes a separate function in the IR. Monomorphization avoids boxing and virtual dispatch at runtime, matching Rust's approach over Haskell's dictionary-passing approach.

**Key types:**
```rust
pub enum Type {
    Int,
    Float,
    String,
    Bool,
    Atom(String),
    Tuple(Vec<Type>),
    List(Type),
    Function(Vec<Type>, Box<Type>),  // arg types -> return type
    Pid(Box<Type>),                  // process ID typed by message type
    TypeVar(TypeVarId),              // unresolved inference variable
    Named(String, Vec<Type>),        // user-defined types with params
}
```

**Confidence:** HIGH for HM inference (well-studied, multiple Rust implementations). MEDIUM for actor-typed message passing (Gleam proves it works on BEAM; doing it with native compilation + LLVM is less explored but architecturally straightforward).

---

### Phase 4: IR Generation (Snow IR)

**Input:** Typed AST
**Output:** Snow IR (mid-level intermediate representation)

**Responsibility:**
- Lower high-level constructs to simpler operations
- Desugar pattern matching into decision trees (following Maranget's algorithm)
- Desugar `receive` blocks into runtime calls with pattern-matching closures
- Flatten nested expressions into SSA-like form
- Make control flow explicit (no implicit returns from blocks -- use explicit jumps/continuations)
- Insert GC safepoint markers
- Make actor operations into explicit runtime calls (`snow_rt_spawn`, `snow_rt_send`, `snow_rt_receive`)

**Architecture notes:**
- **Why a custom IR?** Going directly from AST to LLVM IR loses too much information. A mid-level IR allows Snow-specific optimizations (e.g., tail call detection for recursive actors, message batching, dead actor elimination) before lowering to LLVM's general-purpose IR. This is the approach taken by Rust (MIR between HIR and LLVM IR), Lumen/Firefly (EIR between AST and LLVM IR), and recommended by the ISPC project. ([ISPC Wiki](https://github.com/ispc/ispc/wiki/Additional-ir-step-between-ast-and-llvm-ir))
- **Pattern match compilation:** The typed AST's `match` expressions get compiled into **decision trees** during IR lowering. A decision tree never tests the same sub-term twice, which is optimal for runtime performance. The Maranget algorithm produces good decision trees using heuristics based on "necessity" -- testing columns that are most likely to narrow down the match. ([Maranget 2008](https://www.cs.tufts.edu/~nr/cs257/archive/luc-maranget/jun08.pdf))
- **Actor operations become function calls:** `spawn do ... end` lowers to `snow_rt_spawn(function_ptr, args)`. `send pid, msg` lowers to `snow_rt_send(pid, msg_ptr)`. `receive do ... end` lowers to `snow_rt_receive(patterns_fn_ptr, timeout)`.
- **Continuations for receive:** A `receive` block suspends the actor until a matching message arrives. In the IR, this is modeled as saving the actor's continuation (what to do after the receive) and yielding to the scheduler. Lumen/Firefly uses a continuation-passing style IR for exactly this reason. ([Firefly Readme](https://github.com/GetFirefly/firefly))

**Key IR structure:**
```rust
pub enum IrInst {
    // Basic operations
    Assign(VarId, IrValue),
    BinaryOp(VarId, BinOp, VarId, VarId),
    Call(VarId, FuncId, Vec<VarId>),

    // Control flow
    Jump(BlockId),
    Branch(VarId, BlockId, BlockId),  // condition, then, else
    Switch(VarId, Vec<(Pattern, BlockId)>, BlockId),  // decision tree
    Return(VarId),

    // Actor operations (calls into libsnowrt)
    RuntimeCall(VarId, RuntimeFunc, Vec<VarId>),
    Yield,              // give up time slice to scheduler

    // GC
    GcSafepoint,        // mark where GC can safely pause
}
```

**Confidence:** HIGH -- mid-level IR is established best practice. Rust's MIR and Lumen/Firefly's EIR validate this pattern.

---

### Phase 5: LLVM Code Generation

**Input:** Snow IR
**Output:** LLVM IR (via Inkwell), then object files

**Responsibility:**
- Translate Snow IR to LLVM IR using Inkwell
- Emit function declarations that match the runtime's calling convention
- Generate LLVM IR for data structure layouts (tuples, tagged unions/variants, closures)
- Insert `gc` attributes and GC root markers for LLVM's GC infrastructure
- Emit debug info (DWARF) for source-level debugging
- Invoke LLVM optimization passes
- Produce object files for the target platform

**Architecture notes:**
- **Use Inkwell** as the LLVM binding. Inkwell wraps `llvm-sys` with Rust's type safety and supports LLVM 11-21. It is the de facto standard for Rust-based compilers targeting LLVM. Pin to a specific LLVM version via Cargo feature flag (e.g., `features = ["llvm18-0"]`). ([Inkwell GitHub](https://github.com/TheDan64/inkwell))
- **Tagged union representation:** Algebraic data types in Snow should use a tagged representation. Each variant has an integer tag, and the payload follows. LLVM struct types with a tag field and a union-sized payload work well. Alternatively, use pointer tagging for small types (atoms, small ints) to avoid heap allocation.
- **Closure representation:** A closure is a pair of (function pointer, environment pointer). The environment is a heap-allocated struct containing captured variables. LLVM IR represents this as a struct `{ fn_ptr, env_ptr }`.
- **Calling convention for actor behaviors:** Actor message handlers (behaviors) must follow a specific calling convention that the runtime understands. The entry point for each behavior takes `(actor_state_ptr, message_ptr)` and returns a new state or a continuation. This is similar to Pony's approach.
- **GC integration:** Use LLVM's `gc` function attribute and `llvm.gcroot` intrinsics to mark stack roots. The runtime's GC uses these stack maps to find live references during collection. ([LLVM GC Documentation](https://llvm.org/docs/GarbageCollection.html))
- **Runtime bitcode optimization (advanced):** Pony compiles its runtime (`libponyrt`) to LLVM bitcode and performs link-time optimization (LTO) between user code and the runtime. This enables interprocedural optimizations across the user/runtime boundary. Snow should consider this as an advanced optimization -- compile `libsnowrt` to bitcode and use LLVM LTO when the user requests maximum performance. ([Pony Performance Cheat Sheet](https://www.ponylang.io/reference/pony-performance-cheatsheet/))

**Confidence:** HIGH -- Inkwell is mature, LLVM codegen is well-documented. Pony and Lumen/Firefly prove this works for actor languages.

---

## Actor Runtime Architecture (`libsnowrt`)

The runtime is a static library written in Rust that gets linked into every Snow binary. It provides the scheduler, process management, message passing, garbage collection, and I/O primitives.

### Reference Design Sources

| System | What Snow Should Learn From It |
|--------|-------------------------------|
| **BEAM (Erlang/OTP)** | Process model, supervision trees, reduction-based preemption, per-process heaps. BEAM has 30+ years of production validation. |
| **Pony (`libponyrt`)** | Lock-free runtime, work-stealing scheduler, per-actor GC (Orca protocol), actor GC (no poison pills), runtime-as-bitcode. Pony is the closest architectural relative to Snow's goals. |
| **Lumen/Firefly** | Rust-based BEAM runtime, ahead-of-time compilation via LLVM, continuation-based process scheduling. Directly validates Snow's approach of "BEAM semantics + LLVM + Rust." |
| **Tokio** | Work-stealing scheduler implementation in Rust, thread-per-core model. Tokio's scheduler rewrite demonstrated 10x improvement and is well-documented. |

### Process Model

Each Snow "process" (actor) is a lightweight entity with:

| Component | Size Target | Purpose |
|-----------|-------------|---------|
| Mailbox (message queue) | Pointer + metadata (~64 bytes) | Incoming messages. MPSC lock-free queue. |
| Local heap | Starting at ~2KB, growable | Private memory for the actor's data |
| Stack/continuation | Varies | Current execution state |
| Process metadata | ~100-200 bytes | Status, priority, reduction counter, supervisor link |
| **Total overhead** | **~300-500 bytes per actor** | For comparison: Pony is ~240 bytes, Erlang is ~2KB |

**Key design decisions:**

1. **Isolated heaps (per-actor).** Each actor owns its heap. This enables per-actor GC without stop-the-world pauses. Erlang and Pony both use this model. The tradeoff is that messages must be either copied between heaps or use reference-counted shared memory for large binaries. ([Erlang GC Documentation](https://www.erlang.org/doc/apps/erts/garbagecollection.html))

2. **Message passing copies data by default.** When sending a message, the data is copied into the receiver's heap. This maintains heap isolation and avoids cross-actor references (which would complicate GC). For large binaries, use reference-counted shared buffers (like Erlang does for binaries >64 bytes). ([Erlang Process Memory](https://hamidreza-s.github.io/erlang%20garbage%20collection%20memory%20layout%20soft%20realtime/2015/08/24/erlang-garbage-collection-details-and-why-it-matters.html))

3. **Unbounded mailboxes.** Following Pony's reasoning: "if the queue was bounded then, when the queue is full, you have to either block or fail." Blocking can introduce deadlocks. Unbounded queues with backpressure mechanisms (monitoring queue depth, shedding load) are the pragmatic choice. ([Pony FAQ](https://www.ponylang.io/faq/))

### Scheduler

**Model:** N:M scheduling -- many lightweight Snow processes multiplexed onto a fixed number of OS threads (one per CPU core by default).

**Algorithm:** Work-stealing, following Tokio and Pony's approach.

```
                    OS Thread 1              OS Thread 2              OS Thread N
                  +-------------+          +-------------+          +-------------+
                  | Scheduler 1 |          | Scheduler 2 |          | Scheduler N |
                  |             |          |             |          |             |
                  | [Run Queue] |  steal   | [Run Queue] |  steal   | [Run Queue] |
                  | P1, P4, P7  | <------> | P2, P5      | <------> | P3, P6      |
                  |             |          |             |          |             |
                  | Currently:  |          | Currently:  |          | Currently:  |
                  | executing P1|          | executing P2|          | idle (steal)|
                  +-------------+          +-------------+          +-------------+
```

**Preemption strategy:** Reduction counting, following BEAM's approach. Each actor gets a budget of reductions (e.g., 2000 function calls). When the budget is exhausted, the actor yields and goes to the back of the run queue. This ensures fair scheduling without OS-level thread preemption.

**Implementation approach:**
- Each scheduler thread has a local run queue (deque).
- When a process yields or blocks on `receive`, the scheduler picks the next process from its local queue.
- When the local queue is empty, the scheduler attempts to steal from a sibling's queue (steal from the back to maintain cache locality).
- When no work is available anywhere, the scheduler thread parks itself (suspends) until new work appears.
- **Reduction counting is implemented via the compiler:** The compiler inserts reduction counter decrements at function calls and loop back-edges. When the counter hits zero, the compiled code calls `snow_rt_yield()` to return control to the scheduler. This is how BEAM achieves preemption within a cooperative multitasking model. ([BEAM Scheduling](https://hamidreza-s.github.io/erlang/scheduling/real-time/preemptive/migration/2016/02/09/erlang-scheduler-details.html))

**Confidence:** HIGH -- work-stealing with reduction counting is battle-tested in BEAM for 30 years and validated in Pony and Tokio.

### Message Passing

**Mechanism:** Lock-free MPSC (multi-producer, single-consumer) queue per actor.

Each actor's mailbox is an intrusive, lock-free MPSC queue. Multiple actors can send messages concurrently (multi-producer). Only the owning actor reads from its mailbox (single-consumer).

**Implementation notes:**
- Use atomic operations (CAS) for the producer side. The consumer side needs no synchronization since only one thread (the scheduler running this actor) reads from it.
- Messages are copied into the receiver's heap during delivery. The sender allocates the message in its own heap, copies it into a transfer buffer, and the receiver copies from the buffer into its heap when processing.
- For large binary data, use a reference-counted shared binary heap (like Erlang's refc binaries) to avoid copying large payloads.
- Rust's `std::sync::mpsc` is a reasonable starting point for prototyping, but a custom lock-free MPSC queue will be needed for production performance. ([Tokio Scheduler Blog](https://tokio.rs/blog/2019-10-scheduler))

**`receive` semantics:**
- `receive` is a blocking operation from the actor's perspective (the actor suspends).
- The runtime implements `receive` by: (1) saving the actor's continuation, (2) checking the mailbox for matching messages, (3) if none match, suspending the actor until a new message arrives, (4) when a matching message is found, resuming the actor with the matched value.
- Selective receive (matching specific patterns) requires scanning the mailbox. Implement as a pattern-matching function pointer that the runtime calls on each message.
- Timeout support: `receive` with `after` timeout is implemented via the scheduler's timer wheel.

### Supervision Trees

**Model:** Directly follows Erlang/OTP's supervision architecture.

```
                         [Application]
                              |
                       [Root Supervisor]
                        /            \
               [Supervisor A]    [Supervisor B]
               /     |    \          |      \
           [Worker] [Worker] [Worker] [Worker] [Supervisor C]
                                                  |
                                              [Worker]
```

**Supervision strategies:**
- `:one_for_one` -- if a child crashes, only that child is restarted
- `:one_for_all` -- if any child crashes, all children are restarted
- `:rest_for_one` -- if a child crashes, that child and all children started after it are restarted

**Implementation:**
- A supervisor is just a special actor that monitors its children.
- When a child process crashes (panics, throws an unhandled error), it sends an exit signal to its supervisor.
- The supervisor's restart logic decides whether to restart the child, escalate to its own supervisor, or shut down.
- **Restart limits:** Supervisors track restart frequency. If a child crashes more than N times in M seconds, the supervisor itself crashes (escalating to its parent). This prevents infinite restart loops.

**Compiler support needed:**
- The `spawn` construct needs to accept a `link: supervisor_pid` option.
- Process exit signals need to be part of the runtime's message system.
- The standard library should provide a `Supervisor` module with the restart strategies built in.

**Confidence:** HIGH -- OTP supervision is well-documented and proven at scale (WhatsApp, Discord, telecom systems).

### Garbage Collection Strategy

**Recommended approach:** Per-actor generational copying GC, inspired by Erlang's design.

| Aspect | Design Choice | Rationale |
|--------|--------------|-----------|
| Scope | Per-actor | No stop-the-world. Each actor GCs independently. |
| Algorithm | Generational copying (young + old generation) | Most actor data is short-lived. Generational GC handles this efficiently. Erlang uses this. |
| Trigger | Heap usage threshold | GC when the actor's heap reaches N bytes (start at 2KB, grow as needed). |
| Cross-actor refs | Copy on send (default) | Maintains heap isolation. No cross-actor reference tracking needed. |
| Large binaries | Reference-counted shared heap | Avoid copying large data. Separate from per-actor heaps. |
| Actor GC | Cycle detection (like Pony's Orca) | Detect unreachable actor cycles and collect them. |

**LLVM integration:**
- Mark Snow functions with LLVM's `gc "snow"` attribute.
- Insert `llvm.gcroot` for stack variables holding heap references.
- The runtime uses LLVM's stack maps to find live references during GC.
- GC safepoints are inserted by the compiler at function calls, loop back-edges, and allocation sites.

**Alternative considered:** Pony's mark-and-don't-sweep. Simpler but non-generational -- performance degrades with many long-lived objects. Erlang's generational approach is better for general-purpose languages where actors may hold significant state.

**Confidence:** MEDIUM -- the individual pieces (per-actor heaps, generational copying GC, LLVM stack maps) are well-understood, but integrating them all together in a new runtime is non-trivial. Lumen/Firefly attempted this and it's the hardest part of the project.

---

## Runtime-Binary Integration

### How the Runtime Links Into Binaries

The runtime (`libsnowrt`) is compiled as a **static library** (`libsnowrt.a`) and linked into every Snow binary at compile time.

```
  snowc compilation:

  1. Compile Snow source -> LLVM IR -> object files (.o)
  2. Link: object files + libsnowrt.a + system libs -> single native binary

  Result:
  +---------------------------+
  |     Native Binary         |
  |                           |
  |  [User Code (.o)]         |  <-- compiled Snow functions
  |  [libsnowrt.a]            |  <-- scheduler, GC, message passing
  |  [libc / system libs]     |  <-- OS interface (pthreads, mmap, etc.)
  +---------------------------+
```

**Implementation:**
- The Snow compiler (`snowc`) invokes the system linker (or LLD) as the final step, linking user object files with `libsnowrt.a`.
- The runtime is distributed as a precompiled static library alongside the compiler. Building from source should also be supported.
- For cross-compilation, precompile `libsnowrt.a` for each target triple (x86_64-linux, aarch64-macos, etc.).
- **Optional: Runtime as LLVM bitcode.** Following Pony's approach, `libsnowrt` can also be compiled to LLVM bitcode (`.bc`). When the user requests maximum optimization (`--release` or `--lto`), the compiler performs LTO across user code and the runtime, enabling interprocedural optimization. ([Pony Runtime Bitcode](https://github.com/ponylang/ponyc/blob/main/BUILD.md))

### Entry Point / Bootstrap Sequence

Every Snow binary has a fixed entry point generated by the compiler:

```rust
// Pseudo-code for the generated main() function:
fn main() {
    // 1. Initialize the runtime
    snow_rt_init(num_schedulers, gc_config);

    // 2. Spawn the user's main module as the root actor
    let main_pid = snow_rt_spawn(user_main_fn, args);

    // 3. Set up the root supervisor (if configured)
    snow_rt_set_root_supervisor(main_pid, supervisor_config);

    // 4. Start the scheduler loop (blocks until all actors complete)
    let exit_code = snow_rt_run();

    // 5. Cleanup
    snow_rt_shutdown();

    std::process::exit(exit_code);
}
```

**Key design points:**
- The compiler generates a thin `main()` wrapper that calls runtime initialization functions.
- The user's `main` module/function is spawned as the first actor.
- `snow_rt_run()` blocks until all actors have exited or the root supervisor decides to shut down.
- Exit code comes from the main actor's return value or from an explicit `System.exit(code)` call.

**Confidence:** HIGH -- this is exactly how Go, Pony, and Lumen/Firefly work. The compiler generates a shim `main()` that initializes the runtime and starts the user's code.

---

## Standard Library Architecture

The standard library is split into two layers:

### Layer 1: Runtime Primitives (in Rust, part of `libsnowrt`)

These are implemented in Rust as part of the runtime because they need direct access to runtime internals or OS APIs:

| Module | Contains | Why in Rust |
|--------|----------|-------------|
| `Process` | `spawn`, `send`, `receive`, `self()`, `link`, `monitor` | Direct scheduler/mailbox access |
| `Supervisor` | Supervision strategies, child specs | Needs process lifecycle hooks |
| `IO` | `print`, `read`, file operations | OS syscall wrappers |
| `System` | `exit`, `argv`, environment variables | OS interface |
| `Timer` | `sleep`, `send_after`, `interval` | Scheduler timer wheel integration |
| `Binary` | Raw byte operations | Memory management |

These are exposed to Snow code as built-in functions via the compiler (the compiler knows their signatures and emits direct calls to the runtime).

### Layer 2: Snow Standard Library (written in Snow)

Once the language can compile itself, higher-level standard library modules are written in Snow:

| Module | Contains | Depends On |
|--------|----------|------------|
| `Enum` | `map`, `filter`, `reduce`, `each` | Core types |
| `String` | String manipulation, formatting | Binary |
| `List` | List operations, comprehensions | Core types |
| `Map` | Hash map (persistent/immutable) | Core types |
| `Result` | `{:ok, value}` / `{:error, reason}` patterns | Core types |
| `GenServer` | Generic server behavior | Process, Supervisor |
| `Task` | Async task abstraction | Process |
| `Agent` | Simple state-holding actor | GenServer |
| `HTTP` | HTTP client and server | IO, Process, Binary |
| `JSON` | JSON parsing/encoding | String, Map |

**Build order for stdlib:** Runtime primitives first (in Rust), then core Snow modules (Enum, String, List, Map, Result), then OTP-style behaviors (GenServer, Task, Agent, Supervisor), then application-level modules (HTTP, JSON).

**Confidence:** HIGH for the architecture. The two-layer approach (Rust primitives + Snow-written library) is how Go (runtime in C/assembly, stdlib in Go) and Rust (core in assembly/Rust, std in Rust) work.

---

## Component Dependencies and Build Order

### Dependency Graph

```
  [common]          Shared types: Span, AST nodes, Type definitions, errors
     |
     +---> [lexer]       Token definitions, tokenizer
     |        |
     |        v
     +---> [parser]      AST construction from tokens
     |        |
     |        v
     +---> [typechecker] Type inference, constraint solving, exhaustiveness
     |        |
     |        v
     +---> [ir]          Snow IR definition and lowering from typed AST
     |        |
     |        v
     +---> [codegen]     LLVM IR generation via Inkwell
     |        |
     |        v
     +---> [driver]      Orchestrates the pipeline, invokes linker
     |
     +---> [snowrt]      Runtime library (separate compilation unit)
```

### Suggested Cargo Workspace Structure

```
snow/
  Cargo.toml                    # workspace root
  crates/
    snow-common/                # shared types, errors, diagnostics
    snow-lexer/                 # tokenization
    snow-parser/                # AST construction
    snow-typechecker/           # type inference + checking
    snow-ir/                    # mid-level IR + lowering
    snow-codegen/               # LLVM codegen via Inkwell
    snow-driver/                # pipeline orchestration
    snowc/                      # binary entry point (the compiler CLI)
  runtime/
    snowrt/                     # runtime library (compiled to .a and optionally .bc)
  stdlib/
    core/                       # Snow source files for standard library
  tests/
    fixtures/                   # .snow test files
    integration/                # end-to-end compilation tests
```

### Suggested Build Order (Development Phases)

This is the critical ordering for roadmap phases, driven by strict dependencies:

```
  Phase 1: Foundation
  ==================
  [common] + [lexer] + [parser]
  - Can tokenize and parse Snow source to AST
  - Testable in isolation (parse Snow files, dump AST)
  - No LLVM dependency yet

  Phase 2: Type System
  ====================
  [typechecker]
  - Depends on: common, parser (for AST types)
  - Produces typed AST
  - Testable: feed ASTs, verify type assignments
  - This is the hardest intellectual challenge

  Phase 3: Code Generation (Minimal)
  ===================================
  [ir] + [codegen] + [driver]
  - Depends on: everything above + Inkwell/LLVM
  - Start with a minimal runtime (just main() + print)
  - Goal: compile "Hello, World" to a native binary
  - Defer actor features to Phase 4

  Phase 4: Actor Runtime
  ======================
  [snowrt] (scheduler, processes, message passing)
  - Can be developed partially in parallel with Phase 3
  - Integration: codegen emits calls to runtime functions
  - Goal: spawn actors, send/receive messages

  Phase 5: Supervision & Fault Tolerance
  =======================================
  Extends [snowrt] with supervision trees, monitors, links
  - Depends on: working actor runtime from Phase 4
  - Goal: actors can crash and be restarted

  Phase 6: Standard Library
  =========================
  [stdlib] -- mostly written in Snow itself
  - Depends on: working compiler (Phases 1-3)
  - Can be developed incrementally
  - Priority: GenServer, Task, then HTTP
```

**Phase ordering rationale:**
- Lexer/Parser first because everything depends on them, and they have zero external dependencies.
- Type checker before codegen because you want to catch errors early and because typed AST is the input to IR lowering.
- Minimal codegen before the full actor runtime because you need the ability to compile *something* to validate the pipeline end-to-end. A "Hello World" that calls `printf` through the C FFI validates lexer -> parser -> typechecker -> IR -> LLVM -> binary without needing the actor runtime.
- Actor runtime can be developed in parallel with codegen once the interface (which runtime functions exist, what their signatures are) is defined.
- Supervision builds on actors -- you need working actors before you can supervise them.
- Standard library is last because it requires the compiler to work.

---

## Reference Implementations

Languages that solved similar problems, ordered by relevance to Snow:

### Tier 1: Directly Relevant (study these closely)

| Project | Language | Relevance | GitHub |
|---------|----------|-----------|--------|
| **Pony** | Pony (C/C++ compiler) | Actor model + LLVM + native binaries + per-actor GC. Closest architectural match to Snow. | [ponylang/ponyc](https://github.com/ponylang/ponyc) |
| **Lumen/Firefly** | Rust | BEAM semantics + LLVM + Rust compiler + ahead-of-time compilation. Validates Snow's exact approach. | [GetFirefly/firefly](https://github.com/GetFirefly/firefly) |
| **Gleam** | Rust compiler, BEAM target | Statically typed + BEAM + Rust compiler. Type system design reference. | [gleam-lang/gleam](https://github.com/gleam-lang/gleam) |

### Tier 2: Valuable for Specific Components

| Project | What to Learn |
|---------|---------------|
| **Erlang/OTP (BEAM)** | Process model, supervision, scheduler, per-process GC. The gold standard for actor runtimes. ([The BEAM Book](https://blog.stenmans.org/theBeamBook/)) |
| **Rust (rustc)** | Compiler pipeline (HIR -> MIR -> LLVM IR), HM type inference, Cargo workspace structure. ([Rust Compiler Dev Guide](https://rustc-dev-guide.rust-lang.org/overview.html)) |
| **Tokio** | Work-stealing scheduler implementation in Rust. Study the scheduler rewrite blog post. ([Tokio Scheduler](https://tokio.rs/blog/2019-10-scheduler)) |

### Tier 3: Useful Background

| Project | What to Learn |
|---------|---------------|
| **Go** | Static binary linking, goroutine scheduler (M:N threading), single-binary deployment UX |
| **OCaml** | HM type inference implementation, pattern matching compilation |
| **Elm** | Simple type system with good error messages, HM inference in a practical language |
| **LAM** | Lightweight actor machine, compiling BEAM to native/WebAssembly ([LAM Blog](https://blog.lambdaclass.com/lam-an-actor-model-vm-for-webassembly-and-native/)) |

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: Building the Runtime in Snow

**What:** Attempting to write the actor runtime in Snow itself.
**Why bad:** Circular dependency. The runtime must exist before Snow programs can run. Additionally, the runtime needs direct access to OS primitives (threads, memory mapping, atomics) that a new language won't have bindings for initially.
**Instead:** Write the runtime in Rust. It's the compiler language, has excellent systems programming support, and can be compiled to a static library that links naturally with LLVM-produced object files.

### Anti-Pattern 2: Skipping the Mid-Level IR

**What:** Lowering directly from AST to LLVM IR.
**Why bad:** LLVM IR is too low-level for language-specific optimizations. You lose the ability to optimize pattern matching, detect tail calls in receive loops, or perform actor-specific transformations. You also make the codegen phase enormously complex because it must handle all desugaring simultaneously.
**Instead:** Lower AST -> Snow IR (handles desugaring, decision trees, actor operation lowering) -> LLVM IR (straightforward mechanical translation).

### Anti-Pattern 3: Global GC

**What:** Using a single garbage collector for the entire program.
**Why bad:** Stop-the-world GC pauses affect all actors. This defeats the purpose of the actor model (independent, isolated units of computation). With thousands of actors, GC pause times become the performance bottleneck.
**Instead:** Per-actor heaps with per-actor GC. Each actor collects independently. Cross-actor data is handled by copying or reference counting.

### Anti-Pattern 4: OS Threads per Actor

**What:** Creating an OS thread for each actor.
**Why bad:** OS threads cost ~1MB stack each and context switching is expensive. BEAM can run millions of processes because they're scheduled in userspace. With OS threads, you're limited to thousands.
**Instead:** M:N scheduling. A fixed number of OS threads (one per core) each run a scheduler that multiplexes many lightweight actors.

### Anti-Pattern 5: Shared Mutable State Between Actors

**What:** Allowing actors to share references to mutable data.
**Why bad:** Destroys the safety guarantees of the actor model. Introduces data races, requires locks (defeating the purpose), and makes per-actor GC impossible (can't collect an actor's heap if other actors have references into it).
**Instead:** Copy on send (for small data) or reference-counted immutable shared data (for large binaries). Pony solves this with reference capabilities; Snow should solve it by making all Snow data immutable-by-default and copying on send.

---

## Sources

### Official Documentation (HIGH confidence)
- [LLVM Kaleidoscope Tutorial](https://llvm.org/docs/tutorial/MyFirstLanguageFrontend/index.html)
- [LLVM GC Documentation](https://llvm.org/docs/GarbageCollection.html)
- [LLVM GC Statepoints](https://llvm.org/docs/Statepoints.html)
- [Rust Compiler Development Guide](https://rustc-dev-guide.rust-lang.org/overview.html)
- [Rust Type Inference](https://rustc-dev-guide.rust-lang.org/type-inference.html)
- [Erlang Garbage Collection](https://www.erlang.org/doc/apps/erts/garbagecollection.html)
- [Pony GitHub](https://github.com/ponylang/ponyc)
- [Pony Performance Cheat Sheet](https://www.ponylang.io/reference/pony-performance-cheatsheet/)
- [Pony GC Gotchas](https://tutorial.ponylang.io/gotchas/garbage-collection.html)
- [Gleam FAQ](https://gleam.run/frequently-asked-questions/)
- [Inkwell GitHub](https://github.com/TheDan64/inkwell)

### Research Papers (HIGH confidence)
- [Maranget, "Compiling Pattern Matching to Good Decision Trees" (2008)](https://www.cs.tufts.edu/~nr/cs257/archive/luc-maranget/jun08.pdf)
- [Maranget, "Warnings for Pattern Matching"](http://moscova.inria.fr/~maranget/papers/warn/warn.pdf)
- [Clebsch et al., "Ownership and Reference Counting Based GC in the Actor World"](https://www.doc.ic.ac.uk/~scd/icooolps15_GC.pdf)

### Project References (MEDIUM confidence)
- [Lumen/Firefly GitHub](https://github.com/GetFirefly/firefly)
- [The BEAM Book](https://blog.stenmans.org/theBeamBook/)
- [BEAM Scheduler Deep Dive](https://blog.appsignal.com/2024/04/23/deep-diving-into-the-erlang-scheduler.html)
- [Erlang Scheduler Details](https://hamidreza-s.github.io/erlang/scheduling/real-time/preemptive/migration/2016/02/09/erlang-scheduler-details.html)
- [Tokio Scheduler Blog](https://tokio.rs/blog/2019-10-scheduler)
- [Create Your Own Programming Language with Rust](https://createlang.rs/)
- [LAM Actor Machine](https://blog.lambdaclass.com/lam-an-actor-model-vm-for-webassembly-and-native/)

### Community / Blog (LOW-MEDIUM confidence)
- [Pony, Actors, Causality, Types, and Garbage Collection (InfoQ)](https://www.infoq.com/presentations/pony-types-garbage-collection/)
- [Sylvan Clebsch on Pony (InfoQ Interview)](https://www.infoq.com/interviews/clebsch-pony/)
- [Building a Compiler with Rust and LLVM](https://lyle.dev/2025/01/05/cyclang.html)
- [Rust HM Implementation](https://github.com/tcr/rust-hindley-milner)
- [Algorithm W in Rust](https://github.com/nwoeanhinnogaehr/algorithmw-rust)

### HM Type Inference Implementations in Rust (MEDIUM confidence)
- [rust-hindley-milner](https://github.com/tcr/rust-hindley-milner)
- [algorithmw-rust](https://github.com/nwoeanhinnogaehr/algorithmw-rust)
- [polytype crate](https://crates.io/crates/polytype)
- [hm-infer-rs](https://github.com/zdimension/hm-infer-rs)
