<div align="center">

# Mesh Language

![Version](https://img.shields.io/badge/version-v14.0-blue.svg?style=flat-square)
![License](https://img.shields.io/badge/license-MIT-green.svg?style=flat-square)

**A statically typed, actor-based language for native services and distributed systems.**

Elixir-inspired syntax · Hindley–Milner-style inference · LLVM native binaries · Runtime-owned clustering

[Get started](https://meshlang.dev/docs/getting-started/) ·
[Read the docs](https://meshlang.dev/docs/language-basics/) ·
[Browse the reference](https://meshlang.dev/docs/reference/) ·
[Contribute](CONTRIBUTING.md)

</div>

> [!IMPORTANT]
> Mesh 14 is in active development. The language, runtime, package format, and
> autonomous-cluster protocol can change before a stable release.

## What Mesh includes

Mesh is one toolchain for writing ordinary programs, concurrent services, and
multi-node backends:

| Area | Current surface |
| --- | --- |
| Language | Immutable bindings, inference, generics, transparent generic aliases, structs, algebraic data types, exhaustive matching, guards, closures, comprehensions, pipes, slot pipes, direct self-tail-call elimination, and `Option`/`Result` with `?` |
| Type system | Interfaces, default methods, associated types, `where` bounds, operator traits, `From`/`Into`, `TryFrom`/`TryInto`, and selective deriving for `Eq`, `Ord`, `Display`, `Debug`, `Hash`, `Json`, `Row`, and `Schema` |
| Concurrency | Typed `Pid<M>`, actors and mailboxes, links, monitors, services, supervision trees, jobs, timers, bounded channels, process registries, and graceful shutdown signals |
| Web and data | HTTP and WebSocket servers, scheduler-aware HTTP and WebSocket clients, structured JSON, SQLite, PostgreSQL, pooling, query builders, repositories, changesets, and migrations |
| Distribution | Remote actors, global registration, `@cluster` work declarations, clustered HTTP handlers, request continuity, adaptive routing, capacity drivers, and bounded runtime telemetry |
| Core data | Immutable lists, maps, sets, queues, ranges, tuples, lazy iterators, binary-safe `Bytes`, checked `U64`/`U128`/`I128`, checked `Int` arithmetic, crypto, time, regex, files, environment access, and deterministic random streams |
| Native interop | Manifest-gated ABI 1 declarations with `@native`, exact-target static archives, SHA-256 verification, and no package build-script execution |
| Ecosystem | Source, git, path, registry, and native packages; official Borsh, Anchor-account, and Solana packages are developed in this repository |
| Tooling | Compiler, formatter, test runner, REPL, package manager, migrations, LSP, VS Code and Neovim support, cluster operations, and release-proof commands |

The [complete feature and module index](https://meshlang.dev/docs/reference/)
is the quickest way to find an exact language or library surface.

## A small Mesh program

```mesh
type Message do
  Add(Int)
  Reset
end

actor counter(total :: Int) do
  receive do
    message -> case message do
      Add(value) -> counter(total + value)
      Reset -> counter(0)
    end
  end
end

fn main() do
  let pid :: Pid<Message> = spawn(counter, 0)
  send(pid, Add(2))
  send(pid, Add(40))
  send(pid, Reset)
end
```

Actors have isolated state and typed mailboxes. Services add synchronous calls
and casts; supervisors add restart policy; jobs and bounded channels cover
short-lived work and explicit backpressure. See the
[concurrency guide](https://meshlang.dev/docs/concurrency/).

Cluster-eligible work is explicit:

```mesh
@cluster(2)
pub fn load_account(id :: Int) -> String ! String do
  Ok("account-#{id}")
end
```

`@cluster` declares the work and replica requirement. Node bootstrap, routing,
continuity, security, and capacity policy still come from the runtime and the
project manifest; the annotation is not a substitute for deployment
configuration. Start with the
[clustered walkthrough](https://meshlang.dev/docs/getting-started/clustered-example/)
and continue to [Autonomous Clusters](https://meshlang.dev/docs/autonomous-clusters/).

## Install

### macOS and Linux

```bash
curl -sSf https://meshlang.dev/install.sh | sh
```

### Windows PowerShell

```powershell
irm https://meshlang.dev/install.ps1 | iex
```

Verify both installed tools:

```bash
meshc --version
meshpkg --version
```

Refresh an installer-managed toolchain with either command:

```bash
meshc update
# or
meshpkg update
```

The [tooling guide](https://meshlang.dev/docs/tooling/) documents supported
targets, installer options, source-build prerequisites, and editor setup.

## Build and run

```bash
meshc init hello_mesh
cd hello_mesh
meshc build .
./output
```

For a named artifact, select it explicitly:

```bash
meshc build . --output hello_mesh
./hello_mesh
```

`main.mpl` is the default executable entrypoint. Override it when needed:

```toml
[package]
name = "hello_mesh"
version = "0.1.0"
entrypoint = "lib/start.mpl"
```

Useful development commands:

```bash
meshc fmt .
meshc fmt . --check
meshc test .
meshc deps .
meshc repl
meshc migrate . status
```

See [Developer Tools](https://meshlang.dev/docs/tooling/) for build targets,
LLVM output, JSON diagnostics, migrations, the LSP, the registry CLI, and the
full command reference.

## Pick a starter

```bash
# Minimal clustered function scaffold
meshc init --clustered hello_cluster

# Local, single-node SQLite API
meshc init --template todo-api --db sqlite todo_api

# Shared PostgreSQL API and clustered deployment path
meshc init --template todo-api --db postgres shared_todo
```

- The SQLite starter is deliberately local and single-node.
- The PostgreSQL starter uses shared transactional state and is the base for
  deployable cluster proofs.
- A generated starter is a starting point, not necessarily byte-for-byte
  identical to the repository's evolving proof application.

Follow the generated README, then use the
[getting-started guide](https://meshlang.dev/docs/getting-started/) to choose
the next guide.

## Language guide

- [Language Basics](https://meshlang.dev/docs/language-basics/) — literals,
  bindings, functions, patterns, control flow, modules, JSON, and errors
- [Type System](https://meshlang.dev/docs/type-system/) — generics, aliases,
  structs, sum types, interfaces, associated types, traits, and deriving
- [Iterators](https://meshlang.dev/docs/iterators/) — lazy list-backed
  pipelines, custom `Iterable` types, terminal operations, and collection
- [Concurrency](https://meshlang.dev/docs/concurrency/) — actors, receive
  timeouts, links, monitors, services, supervision, jobs, timers, channels,
  registries, and shutdown
- [Syntax Cheatsheet](https://meshlang.dev/docs/cheatsheet/) — compact syntax
  examples
- [Complete Reference](https://meshlang.dev/docs/reference/) — supported
  declarations, operators, types, modules, and feature boundaries

## Runtime and standard library

The built-in modules are grouped below. The
[standard-library guide](https://meshlang.dev/docs/stdlib/) contains the
function-level reference.

| Group | Modules |
| --- | --- |
| Collections and text | `String`, `List`, `Map`, `Set`, `Tuple`, `Range`, `Queue`, `Iter`, `Regex` |
| Values and encoding | `Bytes`, `U64`, `U128`, `I128`, `Checked`, `JSON`/`Json`, `Base64`, `Hex` |
| System and time | `IO`, `Env`, `File`, `Math`, `Int`, `Float`, `DateTime`, `Monotonic`, `Duration`, `Random`, `Crypto` |
| Concurrency | `Job`, `Timer`, `Channel`, `Process`, `Test` |
| Web | `HTTP`, `Request`, `Ws`, `Http`, `WsClient` |
| Databases | `Sqlite`, `Pg`, `Pool`, `Orm`, `Expr`, `Query`, `Repo`, `Changeset`, `Migration` |
| Distribution | `Node`, `Global`, `Continuity`, `Cluster` |

Important operational distinctions:

- `Bytes` is the binary-safe value type. `String` is UTF-8 text.
- Wide integers use parse/compare/checked arithmetic functions; they are not
  ordinary `Int` literals with unchecked operators.
- `Checked` returns `Result` for overflow, division, rescaling, and explicit
  rounding.
- `Monotonic` is for elapsed time; `DateTime` is for wall-clock timestamps.
- `Random` is deterministic and state-threaded. It is not a cryptographic RNG.
- Bounded channels currently carry `Int` payloads and never wait on the
  producer path.
- SQLite is for local process state. Use PostgreSQL for shared multi-node
  application state.

## Web, database, and distributed services

- [Web](https://meshlang.dev/docs/web/) covers HTTP routing, middleware,
  request identity, responses, JSON, inbound WebSockets, outbound HTTP and
  WebSocket clients, limits, cancellation, retries, and shutdown.
- [Databases](https://meshlang.dev/docs/databases/) covers direct SQLite and
  PostgreSQL connections, pools, transactions, typed row conversion, query
  composition, repositories, changesets, and migrations.
- [Distributed Actors](https://meshlang.dev/docs/distributed/) covers node
  bootstrap, remote spawn/link, global names, node monitors, and continuity.
- [Autonomous Clusters](https://meshlang.dev/docs/autonomous-clusters/) covers
  `@cluster`, clustered HTTP work, routing, continuity, telemetry, and
  horizontal capacity.
- [Cluster Operations](https://meshlang.dev/docs/cluster-operations/) and
  [Capacity Drivers](https://meshlang.dev/docs/capacity-drivers/) document
  operator commands and deployment responsibilities.

## Native and official packages

Native declarations are public, concrete, fully annotated, and bodyless:

```mesh
@native("mesh_math_add")
pub fn add(left :: Int, right :: Int) -> Int
```

ABI 1 supports `Int`, `Float`, `Bool`, `String`, `Bytes`, `U64`, `U128`, and
`I128`; returns may also use `Option` or `Result` over supported values. Native
archives are target-specific and checksum-pinned. Mesh does not execute build
scripts while resolving or publishing a package.

Read [Native Packages](https://meshlang.dev/docs/native-packages/) before
crossing the ABI boundary.

The current official package families are:

- [`mesh-borsh`](packages/mesh-borsh/README.md) — bounded Borsh readers and
  writers
- [`mesh-anchor`](packages/mesh-anchor/README.md) — Anchor discriminator,
  owner, and versioned-layout validation
- [`mesh-solana` 0.2](packages/mesh-solana/README.md) — typed Solana RPC,
  account decoding, subscriptions, instruction inspection, legacy/v0 message
  construction, and unsigned simulation

`mesh-solana` intentionally does not hold secrets, sign transactions, submit
transactions, or expose transaction bytes in its allowlist reports. See
[Packages and Registry](https://meshlang.dev/docs/packages/) for installation,
publishing, provenance, and package-specific boundaries.

## Project status and boundaries

Mesh already has end-to-end compiler and runtime coverage, but it is not a
stable production release. In particular:

- `meshc test --coverage` exits with an unsupported-feature error.
- Native packages must provide an archive for the exact compilation target;
  the package manager does not build one for you.
- Cross-compilation still requires a working target LLVM/linker toolchain.
- Legacy/manual node bootstrap and autonomous protocol-two clusters have
  different security and compatibility contracts.
- Safe replay of mutating clustered HTTP work requires stable operation
  identity and the configured continuity guarantees.
- Proof commands validate repository-owned scenarios; they do not certify an
  arbitrary deployment or provider account.

Current compatibility and release evidence lives in:

- [Autonomous Cluster Release Notes](https://meshlang.dev/docs/autonomous-release-notes/)
- [Cluster Migration](https://meshlang.dev/docs/cluster-migration/)
- [Distributed Proof](https://meshlang.dev/docs/distributed-proof/)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow and
verification commands. Please keep feature changes, compiler tests, runtime
tests, examples, the README, and the public docs in sync.

## License

Mesh is licensed under the [MIT License](LICENSE).
