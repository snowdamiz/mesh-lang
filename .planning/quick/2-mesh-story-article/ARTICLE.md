# I Let Claude Opus 4.6 Build a Programming Language. Here's How It Went.

*111,000 lines of Rust. 7,200 lines of Mesh. A custom LLVM backend. A distributed actor runtime. One very sleep-deprived human who mostly just steered.*

---

**By [@andrew_da_miz](https://x.com/andrew_da_miz)**
*February 17, 2026*

---

Twelve days ago, I had an empty directory. Today, I have a compiled programming language with a distributed actor runtime, a PostgreSQL driver written from scratch, a full ORM, and a SaaS backend written in the language itself. 111,000 lines of Rust. I didn't write most of it. Claude Opus 4.6 did.

The model handled the implementation velocity. [GSD](https://github.com/gsd-build/get-shit-done), an open-source spec-driven development system, kept it on the rails. Every feature followed a strict pipeline from research to plan to atomic commit. The codebase was shippable after every single step.

---

## So What the Hell is Mesh?

The elevator pitch: imagine Elixir and Rust had a kid, and that kid was raised by OCaml.

Mesh is a statically typed functional language with Hindley-Milner type inference. It has the expressive, readable syntax of Ruby and Elixir (think `do`/`end` blocks, pattern matching everywhere, minimal noise). But under the hood, it compiles to native code via LLVM, and it ships as a single binary. No VM. No runtime install. Just `./mesh run your_app.mpl` and you're live.

The real headline feature is the actor system. If you've used Erlang or Elixir, you know the model: lightweight processes that communicate via message passing, supervised and isolated so one crash doesn't bring down the whole system. Mesh bakes that into the language itself. `spawn`, `send`, `receive` aren't library calls, they're *primitives*.

Here's a complete HTTP server:

```mesh
service Counter do
  fn init(start_val :: Int) -> Int do
    start_val
  end

  call Increment() :: Int do |count|
    (count + 1, count + 1)
  end

  cast Reset() do |_count|
    0
  end
end

fn handle_count(request) do
  let pid = Process.whereis("counter")
  let count = Counter.increment(pid)
  HTTP.response(200, "Count is: ${count}")
end

fn handle_reset(request) do
  let pid = Process.whereis("counter")
  Counter.reset(pid)
  HTTP.response(200, "Counter reset")
end

fn main() do
  let pid = Counter.start(0)
  let _ = Process.register("counter", pid)

  HTTP.serve((HTTP.router()
    |> HTTP.on_get("/", handle_count)
    |> HTTP.on_post("/reset", handle_reset)), 8080)
end
```

Notice what's *not* there. No type annotations on the handler parameters,the compiler infers them. No framework boilerplate. No dependency injection ceremony. The `service` block defines a stateful actor with typed synchronous and asynchronous operations in twelve lines. Routes compose through the pipe operator. The whole thing is a working concurrent HTTP server in thirty-something lines.

---

## The Workflow That Made It Possible

Let me be blunt: Opus 4.6 did not "figure out" how to build a language on its own. If I had opened a chat and typed "build me a programming language," I would have gotten a toy lexer and a lot of apologies. The model is fast, not clairvoyant.

What I did instead was adopt a workflow built for exactly this problem. It's called **[Get Shit Done (GSD)](https://github.com/gsd-build/get-shit-done)**, an open-source spec-driven development system created by **TACHES**. The core insight is dead simple: *the AI has no long-term memory, so you have to be its brain.* GSD gives you the scaffolding to do that systematically instead of hoping for the best.

Three markdown files traveled with me for the entire project:

- **`PROJECT.md`**: The constitution. Tech stack, design philosophy, non-negotiable constraints. This never changed.
- **`ROADMAP.md`**: The master plan. Every milestone, every phase, every feature, laid out in advance.
- **`STATE.md`**: The "you are here" dot on the map. Updated after every single commit. "We are on Milestone 4, Phase 2, Step 3. The last thing we did was X. The next thing we're doing is Y. Do not skip ahead."

That last file was the secret weapon. Without it, the AI would constantly try to "help" by jumping ahead three steps, introducing features I hadn't built foundations for, or refactoring code that was about to change anyway. `STATE.md` was a leash.

Every feature followed a strict hierarchy: **Milestone -> Phase -> Plan -> Atomic Commit**. A "plan" was never more than one meaningful unit of work. Write the code, run the tests, commit. Update `STATE.md`. Start the next plan.

I didn't "add features." I *executed phases.* Phase 53: SQLite driver. Phase 54: PostgreSQL driver. Phase 60: Actor integration. The whole thing moved like clockwork.

And I *never broke the build.* Roughly 300 tests ran after every single plan. If something failed, I fixed it before moving on. No tech debt. No "I'll come back to that." The codebase was always shippable.

---

## Twelve Days, Milestone by Milestone

### Days 1-2: Bootstrap (v1.0)

I started where every language starts: a lexer and a parser. By the end of Day 2, Mesh could tokenize source files, parse them into an AST, type-check with Algorithm W (the classic Hindley-Milner algorithm), and emit LLVM IR that compiled to native code.

It could add two numbers. I was unreasonably excited.

### Days 3-4: Things Get Real (v2.0-v3.0)

This is where the project went from "cute weekend hack" to "oh, I'm actually doing this."

I needed actors, which meant I needed green threads. I used `corosensei` for coroutines to spin up thousands of lightweight processes without the OS noticing. I wrote a mark-and-sweep garbage collector. Nothing fancy, but it worked and it meant I wasn't leaking memory in long-running actor systems.

Then I decided I needed database access. The sane move would have been to bind to `libpq`. Instead, I wrote a **pure Rust PostgreSQL driver** by implementing the wire protocol from scratch. Why? Partially because FFI would have complicated the build. Partially because I wanted to see if I could.

I could.

### Days 5-7: The Distributed Part (v4.0-v5.0)

This was the "hold my beer" stretch.

Mesh was always meant to be a distributed language, not just a concurrent one. Actors needed to talk across machines, not just across threads. If you've used Erlang or Elixir, you know the model: location-transparent PIDs are a solved problem on the BEAM. The challenge was implementing that same model from scratch in a compiled-to-native language, without a VM to lean on.

v4.0 added WebSocket support with TLS encryption. v5.0 introduced the *mesh* itself: a custom binary protocol called **Snow Term Format** for fast serialization (JSON was a bottleneck in benchmarks), and location-transparent PIDs wired into the runtime.

The end result feels like what you'd expect from Erlang. A PID is a PID whether the actor is local or remote, and the code is identical:

```mesh
# Works the same locally or across the network
send(pid, "Hello from wherever I am")
```

The runtime figures out routing. You don't think about it.

### Days 8-9: Ship It and Expand (v6.0-v7.0)

A language nobody can learn is a language nobody uses. I built a documentation site with VitePress, wrote nine guides covering everything from installation to distributed deployment, created a landing page, and made it all look like something you wouldn't be embarrassed to share.

Then v7.0 pushed the language's expressiveness further: associated types on traits, a lazy iterator protocol, `From`/`Into` conversion traits, numeric trait hierarchies, and `Collect` for materializing iterators into collections. The kind of type system machinery that makes real code feel natural instead of fighting the compiler at every turn.

### Day 10: Developer Tooling (v8.0)

Features don't matter if the onboarding is painful. I built the boring stuff that makes a language actually usable: one-command install scripts with prebuilt binaries, a TextMate grammar for syntax highlighting, an LSP server with code completion, signature help, formatting, and document symbols, and a VS Code extension published to the Marketplace.

None of it is glamorous. All of it is mandatory.

### Days 10-11: The SaaS App (v9.0)

This is the payoff.

Earlier in this article, I talked about how language demos always work. Fibonacci, Hello World, a counter endpoint. You learn nothing from those. The only test that matters is whether the language survives contact with real requirements. So I built a real application.

Mesher is a multi-tenant event monitoring and alerting platform, built entirely in Mesh. Organizations, projects, teams, API key authentication with Bearer token middleware, an event ingestion pipeline with background processing, real-time WebSocket alerts with room-based fan-out, and a configurable alert rules engine with threshold-based triggers. It's 4,020 lines of pure Mesh code. 38 plans across 14 phases. Zero Rust escape hatches.

Mesher isn't 100% finished yet. But the core functionality is there: multi-tenant auth, event ingestion, real-time alerting, background processing, database queries. Enough to prove that the language actually works for real applications, not just toy demos.

### Days 11-12: The ORM (v10.0-v10.1)

Most languages bolt on an ORM as a library. I built one into the language and added language features to make it feel native.

Mesh's ORM takes direct inspiration from Elixir's Ecto. A `deriving(Schema)` macro generates metadata from struct definitions. Queries compose through pipe operators. The Repo pattern provides `insert`, `get`, `all`, `delete`, and transactions. Changesets validate data through a pipeline of cast, validate, and constraint mapping functions. Relationships declare associations with batch preloading. A migration system with a DDL builder handles schema evolution.


Then came stabilization. Integrating the ORM into Mesher surfaced 47 compilation errors and an ABI segfault when returning structs inside Result types. v10.1 fixed all of them. Every Mesher endpoint was verified working end-to-end. That's the reality of a new language meeting its first real application: you find bugs. The point is you fix them and keep moving.

---

## What I Actually Learned

Before GSD, I tried the obvious approach: open a chat, describe what I want, let the model run. It worked for thirty minutes. Then context drifted, assumptions compounded, and the output was impressive in isolation and useless in aggregate. The AI was fast. It was also completely lost.

GSD fixed that. Each plan was small enough for the model to execute cleanly. Each commit was atomic and tested. When something broke, the git history told me exactly which plan introduced the regression. 1,399 commits across 311 plans, 105 phases, 21 milestones. The process scaled from "emit correct LLVM IR for integer addition" to "implement a multi-tenant event ingestion pipeline with real-time WebSocket alerting." Same workflow, same results.

The discourse around AI coding focuses on the model. After twelve days of letting Opus 4.6 ship a compiler, a runtime, a database driver, an ORM, and a SaaS backend, I'm convinced the tooling *around* the model matters just as much. [GSD](https://github.com/gsd-build/get-shit-done) is lightweight. A few markdown files, a command structure, some commit conventions. But it was the difference between "neat demo" and "111,000 lines of working Rust."

---

## Try It

Mesh is open source. The language compiled a SaaS backend and an ORM without dropping into Rust. Download the binary, read the docs, and break things. If you find bugs (and you will), open an issue. Benchmarks are coming soon.


---

**Mesh**: [meshlang.dev](https://meshlang.dev)
**GitHub**: [snowdamiz](https://github.com/snowdamiz)
**X**: [@andrew_da_miz](https://x.com/andrew_da_miz)
