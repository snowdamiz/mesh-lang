# How GSD and Opus 4.6 Turned 9 Days Into a Full Programming Language

*93,000 lines of Rust. A custom LLVM backend. A distributed actor runtime. One very sleep-deprived human.*

---

**By [Your Name/Handle]**
*February 13, 2026*

---

Nine days ago, I had an empty directory. Today, I have a compiled, statically typed, functional programming language with a distributed actor runtime, TLS networking, and a PostgreSQL wire-protocol driver written from scratch.

Two things made this possible. The first was Opus 4.6, which handled the raw implementation velocity. The second, and honestly the more important one, was [GSD](https://github.com/gsd-build/get-shit-done): an open-source spec-driven development system that gave the entire project a spine. Without GSD, I'd have had a fast AI and no way to keep it on track. With it, every feature followed a strict pipeline from research to plan to atomic commit, and the codebase was shippable after every single step.

This is the story of Mesh, and the process that made it possible.

---

## So What the Hell is Mesh?

The elevator pitch: imagine Elixir and Rust had a kid, and that kid was raised by OCaml.

Mesh is a statically typed functional language with Hindley-Milner type inference. It has the expressive, readable syntax of Ruby and Elixir (think `do`/`end` blocks, pattern matching everywhere, minimal noise). But under the hood, it compiles to native code via LLVM, and it ships as a single binary. No VM. No runtime install. Just `./mesh run your_app.mpl` and you're live.

The real headline feature is the actor system. If you've used Erlang or Elixir, you know the model: lightweight processes that communicate via message passing, supervised and isolated so one crash doesn't bring down the whole system. Mesh bakes that into the language itself. `spawn`, `send`, `receive` aren't library calls, they're *primitives*.

Here's a complete HTTP server:

```mesh
import Http
import Log

struct State {
  count: Int
}

type Msg =
  | Increment
  | Reset

pub fn main() {
  let counter_pid = spawn(init_counter)

  Http.serve(8080, fn(req) {
    match (req.method, req.path) {
      ("GET", "/") -> {
        let count = call(counter_pid, Increment)
        Http.response(200, "Count is: ${count}")
      }
      ("POST", "/reset") -> {
        send(counter_pid, Reset)
        Http.response(200, "Counter reset")
      }
      _ -> Http.response(404, "Not Found")
    }
  })
}

fn init_counter() {
  loop(0)
}

fn loop(count) {
  receive {
    Increment -> {
      reply(count + 1)
      loop(count + 1)
    }
    Reset -> loop(0)
  }
}
```

Notice what's *not* there. No type annotations on `count` because the compiler infers it. No framework boilerplate. No dependency injection ceremony. And that `match` on `(req.method, req.path)` is doing HTTP routing in four lines without a router library.

---

## The Workflow That Made It Possible

Let me be blunt: the AI did not "figure out" how to build a language. If I had opened a chat and typed "build me a programming language," I would have gotten a toy lexer and a lot of apologies.

What I did instead was adopt a workflow built for exactly this problem. It's called **[Get Shit Done (GSD)](https://github.com/gsd-build/get-shit-done)**, an open-source spec-driven development system created by **TÂCHES**. The core insight is dead simple: *the AI has no long-term memory, so you have to be its brain.* GSD gives you the scaffolding to do that systematically instead of hoping for the best.

Three markdown files traveled with us for the entire project:

- **`PROJECT.md`**: The constitution. Tech stack, design philosophy, non-negotiable constraints. This never changed.
- **`ROADMAP.md`**: The master plan. Every milestone, every phase, every feature, laid out in advance.
- **`STATE.md`**: The "you are here" dot on the map. Updated after every single commit. "We are on Milestone 4, Phase 2, Step 3. The last thing we did was X. The next thing we're doing is Y. Do not skip ahead."

That last file was the secret weapon. Without it, the AI would constantly try to "help" by jumping ahead three steps, introducing features we hadn't built foundations for, or refactoring code that was about to change anyway. `STATE.md` was a leash.

Every feature followed a strict hierarchy: **Milestone → Phase → Plan → Atomic Commit**. A "plan" was never more than one meaningful unit of work. We'd write the code, run the tests, and commit. Then update `STATE.md`. Then start the next plan.

We didn't "add features." We *executed phases.* Phase 53: SQLite driver. Phase 54: PostgreSQL driver. Phase 60: Actor integration. The whole thing moved like clockwork.

And we *never broke the build.* Roughly 300 tests ran after every single plan. If something failed, we fixed it before moving on. No tech debt. No "we'll come back to that." The codebase was always shippable.

---

## Nine Days, Milestone by Milestone

### Days 1–2: Bootstrap (v1.0)

We started where every language starts: a lexer and a parser. By the end of Day 2, Mesh could tokenize source files, parse them into an AST, type-check with Algorithm W (the classic Hindley-Milner algorithm), and emit LLVM IR that compiled to native code.

It could add two numbers. I was unreasonably excited.

### Days 3–4: Things Get Real (v2.0–v3.0)

This is where the project went from "cute weekend hack" to "oh, we're actually doing this."

We needed actors, which meant we needed green threads. We used `corosensei` for coroutines so we could spin up thousands of lightweight processes without the OS noticing. We wrote a mark-and-sweep garbage collector. Nothing fancy, but it worked and it meant we weren't leaking memory in long-running actor systems.

Then we decided we needed database access. The sane move would have been to bind to `libpq`. Instead, we wrote a **pure Rust PostgreSQL driver** by implementing the wire protocol from scratch. Why? Partially because FFI would have complicated the build. Partially because I wanted to see if we could.

We could.

### Days 5–7: The Distributed Part (v4.0–v5.0)

This was the "hold my beer" stretch.

Mesh was always meant to be a distributed language, not just a concurrent one. Actors needed to talk across machines, not just across threads. If you've used Erlang or Elixir, this is familiar territory. Location-transparent PIDs are a solved problem on the BEAM. The challenge was implementing that same model from scratch in a compiled-to-native language, without a VM to lean on.

v4.0 added WebSocket support with TLS encryption. v5.0 introduced the *mesh* itself: a custom binary protocol called **Snow Term Format** for fast serialization (JSON was a bottleneck in benchmarks), and location-transparent PIDs wired into the runtime.

The end result feels like what you'd expect from Erlang. A PID is a PID whether the actor is local or remote, and the code is identical:

```mesh
# Works the same locally or across the network
send(pid, "Hello from wherever I am")
```

The runtime figures out routing. You don't think about it.

### Days 8–9: Ship It (v6.0)

A language nobody can learn is a language nobody uses. We built a documentation site with VitePress, wrote nine guides covering everything from installation to distributed deployment, created a landing page, and made it all look like something you wouldn't be embarrassed to share.

---

## Three Technical Bets That Paid Off

**Rust for the compiler.** Not just for the "fast and safe" memes. Rust's type system caught dozens of bugs at compile time that would have been runtime panics in C or subtle logic errors in Go. And `inkwell` gave us a gorgeous Rust-native interface to LLVM. Worth every minute spent fighting the borrow checker.

**Actor-per-connection.** Every HTTP request is an actor. Every WebSocket connection is an actor. Every database query spawns a short-lived actor. When one crashes, it crashes alone. The supervisor restarts it. The rest of the system doesn't even notice. This pattern made the entire networking stack radically simpler to reason about.

**No null.** Mesh has `Option<T>`. If a value might be absent, the type system forces you to handle it. This isn't a new idea (Rust, Haskell, and OCaml all do it), but the experience of building a language where null literally doesn't exist was a reminder of how much incidental complexity it causes everywhere else.

---

## The Real Test: Building a SaaS Product on Mesh

Here's the thing about language demos: they always work. "Look, a Fibonacci function! Look, a web server that returns 'Hello World!'" That tells you nothing about whether a language can survive contact with real requirements.

So we're not stopping at v6.0. **The next milestone is building a fully functional SaaS product with Mesh as the backend language.** Not a toy. Not a demo. A product that handles user authentication, persistent data, billing integration, background jobs, and all the boring-but-essential plumbing that separates a weekend project from something people actually pay for.

The working plan is a **collaborative project management tool**, something in the neighborhood of a simplified Linear or Basecamp. It'll exercise every part of the Mesh runtime:

- **User auth and sessions.** OAuth 2.0 flows, JWT handling, middleware for protected routes. This will stress-test the HTTP server and prove (or break) the actor-per-request model under realistic session management.
- **Real-time collaboration.** Live updates over WebSockets. When someone moves a task, every connected client sees it immediately. This is the distributed actor system's moment to shine: each user connection is an actor, each project room is an actor, and the mesh protocol handles the fan-out.
- **PostgreSQL under load.** Not just "can we run a SELECT." We're talking migrations, connection pooling, transactions, and the kind of query patterns that expose driver bugs fast. Our from-scratch Postgres driver is about to earn its keep.
- **Background job processing.** Scheduled emails, webhook retries, data aggregation. We'll build a lightweight job queue on top of the actor system, which should be a natural fit but has a hundred sharp edges we haven't hit yet.
- **Stripe integration.** Subscription billing, webhook verification, invoice generation. The unglamorous reality of SaaS, and a great test of Mesh's FFI boundaries and JSON parsing in a protocol that does not forgive sloppiness.

If Mesh can handle all of that without us needing to drop into Rust for escape hatches, then it's a real tool. If it can't, we'll know exactly where the gaps are and what v7.0 needs to fix.

I'll be documenting the entire build publicly: every architecture decision, every bug, every moment where the language fought back. Expect a follow-up post with either a victory lap or a brutally honest autopsy.

---

## What I Actually Learned

The biggest lesson from this project wasn't about AI capabilities. It was about process.

Early on, before I adopted GSD, I tried the obvious approach: open a chat, describe what I wanted, and let the model run. It worked for about thirty minutes. Then context drifted. The model started making assumptions about code it could no longer see. It would refactor something that was fine while ignoring something that was broken. The output was impressive in isolation and useless in aggregate.

GSD fixed that, and it fixed it in a way that taught me something about where AI development is actually headed. The models are good enough. That's no longer the bottleneck. What's missing, and what GSD provides, is *structure*. A system that breaks work into atomic, verifiable units. That keeps state externalized so the model always knows where it is. That enforces the discipline of "research, plan, execute, verify" when the temptation is to just keep prompting and praying.

The compound effect was staggering. Each plan was small enough that the model could execute it cleanly in a fresh context window. Each commit was atomic and tested. When something broke, we knew exactly which plan introduced the regression because the git history was surgical. I never once found myself in the "everything is broken and I don't know why" spiral that plagues most AI-assisted projects at scale.

There's a broader point here. The discourse around AI coding tends to focus on the model: which one is smartest, which benchmark matters, who has the biggest context window. But after nine days of shipping a compiler, a runtime, a networking stack, and a database driver, I'm convinced that the tooling *around* the model matters just as much. GSD is a lightweight system (a few markdown files, a command structure, some conventions around commits) but it was the difference between "neat demo" and "93,000 lines of working Rust."

If you're building anything non-trivial with AI, the model is only half the equation. The other half is the process that keeps it honest. [GSD](https://github.com/gsd-build/get-shit-done) was that process for me, and I can't imagine going back to working without it.

---

## Try It

Mesh is open source. Download the binary, read the docs, and break things. If you find bugs (and you will), open an issue.

This is one human and one very fast AI, signing off. v7.0 has a SaaS app to build.