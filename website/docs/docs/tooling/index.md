---
title: Developer Tools
description: Complete meshc and meshpkg command reference, including builds, dependencies, migrations, formatting, tests, REPL, LSP, editors, and proof commands.
---

# Developer Tools

Mesh ships a developer toolchain centered on the `meshc` compiler plus the
companion `meshpkg` registry CLI. This page is the command reference for
building projects, resolving dependencies, running migrations and tests,
formatting code, exploring in the REPL, integrating editors, operating
clusters, and running release proof gates.

> **Autonomous cluster proof:** This page stays focused on the public day-one CLI workflow. Use [Autonomous Clusters](/docs/autonomous-clusters/) for capacity configuration and [Distributed Proof](/docs/distributed-proof/) for the repository-owned release gates.

## Install the CLI tools

The staged release proof covers that installer pair for both `meshc` and `meshpkg` on these targets:

- macOS `x86_64` and `arm64`
- Linux `x86_64` and `arm64` (GNU libc)
- Windows `x86_64`

**macOS and Linux:**

```bash
curl -sSf https://meshlang.dev/install.sh | sh
```

**Windows x86_64 (PowerShell):**

```powershell
irm https://meshlang.dev/install.ps1 | iex
```

Verify the installed binaries before using the tooling below:

```bash
meshc --version
meshpkg --version
```

### Update an installed toolchain

If you installed Mesh through the public installers, refresh both binaries in place with either command:

```bash
meshc update
meshpkg update
```

Both commands rerun the canonical installer path and refresh both `meshc` and `meshpkg` together.

For the clustered release proof behind this install contract, see [Distributed Proof](/docs/distributed-proof/).

The installer also supports a specific version, non-interactive confirmation,
uninstallation, and help. Download it first when you need those controls:

```bash
curl -sSf https://meshlang.dev/install.sh -o /tmp/mesh-install.sh
sh /tmp/mesh-install.sh --version <version>
sh /tmp/mesh-install.sh --uninstall
```

On Windows, invoke the downloaded PowerShell script with `-Version`,
`-Uninstall`, `-Yes`, or `-Help`. Uninstall removes both commands and the PATH
changes managed by the installer.

If you are contributing to Mesh or need an unsupported target, see the
[source-build prerequisites](/docs/getting-started/#alternative-build-from-source).
Source builds require LLVM 21 and a working native linker; they are an
alternative workflow, not the primary public install contract.

## Create a project

Mesh includes a built-in package manager for creating and managing projects.

Keep the public CLI workflow explicit and examples-first: hello world first, then the clustered scaffold, then the honest local SQLite starter or the serious shared/deployable PostgreSQL starter, and only after that the maintainer-facing backend proof page. SQLite stays local-only and single-node only here; the generated PostgreSQL starter is the serious shared/deployable path and the handoff into the staged deploy + failover proof chain, with the repo-boundary product handoff beginning only once you leave the public starter ladder.

### Creating a New Project

Use `meshc init` to scaffold a new project:

```bash
meshc init my_app
```

This creates the following structure:

```
my_app/
  mesh.toml
  main.mpl
```

Use the supported top-level `println` function for the minimal
`main.mpl` program:

```mesh
fn main() do
  println("Hello from Mesh!")
end
```

Use `meshc init --clustered` when you want the public clustered-app scaffold instead of the hello-world starter:

```bash
meshc init --clustered my_clustered_app
```

That scaffold adds:

- a package-only `mesh.toml`
- an `@cluster pub fn add()` boundary in `work.mpl`
- the generic `MESH_CLUSTER_COOKIE`, `MESH_NODE_NAME`, `MESH_DISCOVERY_SEED`, `MESH_CLUSTER_PORT`, `MESH_CONTINUITY_ROLE`, and `MESH_CONTINUITY_PROMOTION_EPOCH` contract in the generated README
- built-in operator guidance that points at the runtime-owned CLI instead of app-authored control-plane surfaces
- follow-on guidance that points at [`examples/todo-postgres/README.md`](https://github.com/hyperpush-org/mesh-lang/blob/main/examples/todo-postgres/README.md) for the serious shared/deployable starter and [`examples/todo-sqlite/README.md`](https://github.com/hyperpush-org/mesh-lang/blob/main/examples/todo-sqlite/README.md) for the honest local starter instead of internal proof fixtures

If you are migrating older clustered code, move `clustered(work)` into source-first `@cluster`, delete the removed placement stanza, and rename helper-shaped entries such as `execute_declared_work(...)` / `Work.execute_declared_work` to ordinary verbs like `add()` or `sync_todos()`. Keep source-declared `@cluster` surfaces canonical: the PostgreSQL Todo starter clusters `GET /todos`, `GET /todos/:id`, and idempotent `POST /todos`; `GET /health` plus unsafe-keyless `PUT` and `DELETE` stay local. The autonomous cluster manifest owns deployment policy rather than handler identity.

If you want the honest local Todo starter, generate SQLite explicitly:

```bash
meshc init --template todo-api --db sqlite my_local_todo
```

The SQLite Todo starter is the honest local-only starter: a single-node SQLite Todo API with generated package tests, local `/health`, actor-backed write rate limiting, and Docker packaging around `meshc build .`. It keeps SQLite single-node only and does not claim `work.mpl`, `HTTP.clustered(...)`, `meshc cluster`, or clustered/operator proof surfaces.

When you need the serious shared or deployable Todo starter, generate Postgres instead:

```bash
meshc init --template todo-api --db postgres my_shared_todo
```

The PostgreSQL Todo starter keeps the clustered-function contract source-first: `work.mpl` stays on `@cluster pub fn sync_todos()`, `main.mpl` boots through `Node.start_from_env()`, shared reads and idempotent `POST /todos` use `HTTP.clustered(...)`, `GET /health` plus unsafe-keyless `PUT` and `DELETE` stay local, and the Dockerfile packages the binary produced by `meshc build .`. It is also the generated starter that owns the staged deploy + failover proof chain once you leave this first-contact tooling page for the proof pages. Keep the SQLite starter on its honest single-node contract instead of treating it as a clustered/operator proof surface.

Inspect a running clustered app with the same operator order used by the scaffold and [`examples/todo-postgres/README.md`](https://github.com/hyperpush-org/mesh-lang/blob/main/examples/todo-postgres/README.md):

```bash
meshc cluster status <node-name@host:port> --json
meshc cluster continuity <node-name@host:port> --json
meshc cluster continuity <node-name@host:port> <request_key> --json
meshc cluster diagnostics <node-name@host:port> --json
```

Use the list form first to discover startup or request keys, then inspect a single continuity record. Continue with:

- [Clustered Example](/docs/getting-started/clustered-example/) — the scaffold-first clustered app story
- [SQLite Todo starter](https://github.com/hyperpush-org/mesh-lang/blob/main/examples/todo-sqlite/README.md) — the honest local-only single-node starter
- [PostgreSQL Todo starter](https://github.com/hyperpush-org/mesh-lang/blob/main/examples/todo-postgres/README.md) — the serious shared/deployable starter and the proof-page handoff for staged deploy + failover
- [Autonomous Clusters](/docs/autonomous-clusters/) — production routing, continuity, and capacity configuration
- [Distributed Proof](/docs/distributed-proof/) — Docker/PostgreSQL and release-gate verification

Keep the starter split explicit here too: [`examples/todo-sqlite/README.md`](https://github.com/hyperpush-org/mesh-lang/blob/main/examples/todo-sqlite/README.md) is the honest local starter with no `work.mpl`, `HTTP.clustered(...)`, or `meshc cluster` story, while [`examples/todo-postgres/README.md`](https://github.com/hyperpush-org/mesh-lang/blob/main/examples/todo-postgres/README.md) is the shared/deployable starter with clustered reads and an idempotent clustered mutation.

### Project Manifest

Every Mesh project has a `mesh.toml` file that describes the package and its dependencies:

```toml
[package]
name = "my_app"
version = "0.1.0"

[dependencies]
```

`main.mpl` stays the default executable entrypoint. When you need a different startup file, add the optional project-root-relative `[package].entrypoint = "lib/start.mpl"` override:

```toml
[package]
name = "my_app"
version = "0.1.0"
entrypoint = "lib/start.mpl"

[dependencies]
```

The manifest supports registry, git, and path dependencies:

```toml
[dependencies]
"your-login/your-package" = "1.0.0"
my_lib = { path = "../my_lib" }
some_pkg = { git = "https://github.com/user/some_pkg", tag = "v1.0.0" }
```

Registry versions must be exact. Git dependencies support `rev`, `branch`, and
`tag`; prefer an immutable `rev` for a release.

### Lockfile

When dependencies are resolved, `mesh.lock` records exact registry versions and
checksums, git revisions, and local path entries. Resolve git/path dependencies
first, then install registry dependencies so the registry command can merge
both sets of entries:

```bash
meshc deps
meshpkg install
```

Commit `mesh.lock`. `meshc build` consumes installed and locked dependencies but
does not fetch missing code. See [Packages and Registry](/docs/packages/) for
the complete dependency and publishing workflow.

## Build projects

Compile a project directory to a native executable:

```bash
meshc build .
./output
```

The directory must contain `mesh.toml` and the resolved entrypoint. When the
directory argument is `.`, the default output is `./output`. When it is a named
directory such as `apps/api`, the default is `apps/api/api`. Prefer an explicit
output when a script depends on the name:

```bash
meshc build . --output my_app
./my_app
```

Build options:

| Option | Behavior |
| --- | --- |
| `--opt-level 0` | Debug/default optimization |
| `--opt-level 2` | Release optimization |
| `--emit-llvm` | Write a `.ll` file next to the executable |
| `-o, --output <path>` | Choose the executable path |
| `--target <triple>` | Generate and link for an explicit target triple |
| `--json` | Emit newline-delimited JSON diagnostics |
| `--no-color` | Disable color in human-readable diagnostics |

For example:

```bash
meshc build . --opt-level 2 --emit-llvm --output dist/my_app
meshc build . --target x86_64-unknown-linux-gnu --output dist/my_app-linux
```

`--target` selects code generation and native-package archives; it does not
install a cross-linker, sysroot, C runtime, or target system libraries. Supply
those separately. A native dependency must declare a checksummed archive for
the exact effective target. See [Native Packages](/docs/native-packages/).

### Resolve git and path dependencies

Run the source dependency resolver from the project root:

```bash
meshc deps
```

Pass another project directory when needed:

```bash
meshc deps apps/api
```

The resolver walks transitive git and path dependencies, checks out git
packages under the project cache, and writes exact revisions and local entries
to `mesh.lock`. If the manifest is not newer than the lockfile, it reports that
dependencies are already up to date.

Registry downloads belong to `meshpkg install`, not `meshc deps`. For a mixed
project, run:

```bash
meshc deps
meshpkg install
```

## Database migrations

`meshc migrate` manages PostgreSQL migrations stored as timestamped `.mpl`
modules. Generate a migration from the project root:

```bash
meshc migrate generate create_users
```

Names may contain lowercase ASCII letters, digits, and underscores. The command
creates `migrations/YYYYMMDDHHMMSS_create_users.mpl` with public `up` and
`down` functions:

```mesh
pub fn up(pool :: PoolHandle) -> Int!String do
  Migration.create_table(pool, "users", [
    "id:UUID:PRIMARY KEY",
    "email:TEXT:NOT NULL UNIQUE"
  ])?
  Ok(0)
end

pub fn down(pool :: PoolHandle) -> Int!String do
  Migration.drop_table(pool, "users")?
  Ok(0)
end
```

Set `DATABASE_URL` for status, apply, and rollback:

```bash
export DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/my_app

meshc migrate status
meshc migrate up
meshc migrate down
```

Use an explicit project directory before the action when invoking the command
from elsewhere:

```bash
meshc migrate apps/api status
meshc migrate apps/api up
```

| Action | Behavior |
| --- | --- |
| `generate <name>` | Create a UTC timestamped migration; no database connection is needed |
| `status` | Show applied and pending files |
| `up` | Compile and run all pending migrations in timestamp order |
| `down` | Compile and run `down` for the most recently applied migration |
| no action | Same as `up` |

The runner creates and maintains `_mesh_migrations` in PostgreSQL. Each
migration is compiled as Mesh code and executed with a small connection pool.
`down` needs the corresponding source file to remain present. Use neutral
`Migration.*` helpers for portable DDL and explicit `Pg.*` helpers for
PostgreSQL-only features. See [Databases](/docs/databases/).

## Cluster operator commands

Every cluster command targets a runtime node as `name@host:port`. Read-only
commands are:

| Command | Result |
| --- | --- |
| `meshc cluster status <target>` | Membership and authority summary |
| `meshc cluster snapshot <target>` | Complete operator runtime snapshot |
| `meshc cluster continuity <target> [request-key]` | One continuity record, or a recent-record list |
| `meshc cluster diagnostics <target>` | Recent failover and continuity diagnostics |
| `meshc cluster capacity <target>` | Desired, observed, Ready, and draining capacity |
| `meshc cluster pressure <target>` | Cluster/per-node pressure and dominant signals |
| `meshc cluster routing <target>` | Eligibility, load reports, and reservations |
| `meshc cluster scaling <target>` | Scheduler and horizontal scaling state |
| `meshc cluster events <target>` | Ordered control, scaling, and continuity events |
| `meshc cluster explain <target> <request-key>` | Retained placement and current candidates |

All read commands accept `--cookie-file <path>`, `--timeout-ms <number>`
(default `5000`), and `--json`. Without `--cookie-file`, the CLI reads
`MESH_CLUSTER_COOKIE`.
Continuity lists, diagnostics, and events also accept `--limit <number>`.

Authenticated mutation commands are:

```bash
meshc cluster autoscale pause <target> --reason "investigating"
meshc cluster autoscale resume <target> --reason "resolved"
meshc cluster scale <target> <worker-count> --reason "planned load"
meshc cluster drain <target> <node-id> --reason "maintenance"
meshc cluster cancel-drain <target> <node-id> --reason "maintenance cancelled"
```

Mutation options:

| Option | Default or purpose |
| --- | --- |
| `--cookie-file <path>` | Otherwise `MESH_CLUSTER_COOKIE` |
| `--operator-key-file <path>` | Otherwise `MESH_OPERATOR_KEY` |
| `--cluster-id <id>` | `mesh` |
| `--actor <identity>` | `meshc`; retained in the audit event |
| `--reason <text>` | `operator request`; retained in the audit event |
| `--sequence <number>` | Explicit monotonic value for automation; otherwise current microseconds |
| `--timeout-ms <number>` | Bound the remote request; default `5000` |
| `--json` | Machine-readable result |

Literal secret flags are intentionally unsupported. Put cookies and signing
keys in owner-only files or the documented environment variables. Follow the
[Cluster Operations](/docs/cluster-operations/) runbook before changing live
capacity or drain state.

## Proof commands

`meshc proof` exposes seven separate repository-owned gates:

| Command | Purpose |
| --- | --- |
| `docker-autoscaling` | Mandatory local Docker/PostgreSQL autonomous scaling and failover proof |
| `continuity-soak` | Bounded-retention soak; release default is 24 hours |
| `autonomous-performance` | Deterministic performance budgets |
| `autonomous-chaos` | Repeated deterministic fault/model suite |
| `fly-driver-conformance` | Credential-free Fly Machines fake-API certification |
| `fly-driver-staging` | Credentialed create, Ready, cordon, delete, and removal gate |
| `fly-autoscaling-materialize` | Create owner-only TLS and signed-identity input for the full Fly proof |

### Docker autoscaling

```bash
meshc proof docker-autoscaling
```

Options:

- `--keep-running` retains the topology after evidence collection;
- `--evidence-dir <path>` chooses the evidence directory;
- `--no-build` reuses existing proof images;
- `--start-only` starts a healthy topology without fault injection; and
- `--connection-file <path>` writes an owner-only connection manifest and
  requires `--start-only`.

See [Distributed Proof](/docs/distributed-proof/) for prerequisites, topology,
assertions, and evidence.

### Deterministic and soak gates

```bash
meshc proof autonomous-performance \
  --iterations 10000 \
  --budget proof/autonomous-gates/performance-budget.json

meshc proof autonomous-chaos --rounds 5

meshc proof continuity-soak
```

All three accept `--evidence-dir`. Performance also accepts `--iterations` and
`--budget`; chaos accepts `--rounds`. The soak accepts
`--duration-seconds`, `--cycle-millis`, and `--allow-short`. A shortened soak
is a harness smoke result, never a 24-hour release pass.

### Fly driver gates

Credential-free conformance needs no Fly account:

```bash
meshc proof fly-driver-conformance
```

It accepts `--evidence-dir`.

The staging command creates and deletes one real Machine and refuses to run
without acknowledgement:

```bash
meshc proof fly-driver-staging \
  --app-name mesh-staging \
  --image registry.fly.io/mesh-staging@sha256:... \
  --cluster-id mesh-staging-cert \
  --template-revision release-42 \
  --worker-env DATABASE_URL \
  --confirm-create-and-delete
```

Required options are `--app-name`, `--image`, `--cluster-id`, and
`--confirm-create-and-delete`. Other options are:

- `--token-env` (default `FLY_API_TOKEN`);
- repeatable `--worker-env`;
- `--api-base-url` (default `https://api.machines.dev`);
- `--region`;
- `--pool` (default `workers`);
- `--template-revision`;
- `--cpu-kind` (default `shared`);
- `--cpus` (default `1`);
- `--memory-mb` (default `256`);
- `--deadline-seconds` (default `300`); and
- `--evidence-dir`.

The command reads token and worker values from environment-variable names; it
does not accept secret values on the command line.

Materialize the owner-only identity input for a full Fly autoscaling run:

```bash
meshc proof fly-autoscaling-materialize \
  --controller-app mesh-controller \
  --data-app mesh-data \
  --cluster-id mesh-proof \
  --output ./fly-proof-identity.json
```

The output path must be new; the command does not overwrite an existing
identity file. See [Capacity Drivers](/docs/capacity-drivers/) for the Fly
driver contract.

## Test Runner

Run all `*.test.mpl` files from a project root, a tests directory, or a specific test file with `meshc test`:

```bash
meshc test .
meshc test tests
meshc test tests/example.test.mpl
```

The test runner discovers all files ending in `.test.mpl` under the requested target, compiles and executes each independently, and prints a per-test pass/fail summary:

```
test arithmetic is correct ... ok
test string operations/length ... FAIL
  assert_eq failed: expected 5, got 6

2 tests, 1 failure
```

Exit code is non-zero if any test fails, making `meshc test` suitable for CI pipelines.

Use compact dot output for a large suite:

```bash
meshc test . --quiet
```

Coverage requests are intentionally honest today:

```bash
meshc test --coverage .
```

`--coverage` currently exits non-zero with an explicit unsupported message instead of claiming a stub report.

See the [Testing guide](/docs/testing/) for the full assertion API, grouping, mock actors, and receive expectations.

## Formatter

The Mesh formatter canonically formats your source code, enforcing a consistent style across your project:

```bash
meshc fmt main.mpl
```

To format a project directory:

```bash
meshc fmt .
```

To fail fast in CI or before committing if any file would change:

```bash
meshc fmt --check .
```

The path is required and may be one `.mpl` file or a directory. Directory
formatting walks nested directories recursively. Override the default
100-column width or two-space indentation when a project needs it:

```bash
meshc fmt . --line-width 120 --indent-size 4
meshc fmt . --check --line-width 120 --indent-size 4
```

The formatter uses the **Wadler-Lindig** pretty-printing algorithm with a CST-based approach. This means:

- **Comments are preserved** -- the formatter works on the concrete syntax tree, so comments stay exactly where you put them
- **Whitespace and indentation are rewritten** canonically according to Mesh style conventions
- **Formatting is idempotent** -- running the formatter twice produces the same output as running it once

### Example

Before formatting:

```mesh
fn add(a,b) do
a+b
end
```

After `meshc fmt`:

```mesh
fn add(a, b) do
  a + b
end
```

### Format on Save

Mesh only publishes repo-owned format-on-save guidance for the first-class editors in the [support tiers](#support-tiers) below. In VS Code, the Mesh extension routes document formatting through `meshc lsp`. In Neovim, the repo-owned pack attaches the native `meshc lsp` client, so save-time formatting should use your normal Neovim LSP formatting hook. Best-effort editors should invoke `meshc fmt <file>` directly and treat that integration as user-maintained.

## REPL

The Mesh REPL provides JIT-compiled interactive exploration for expressions and
definitions:

```bash
meshc repl
```

This starts an interactive session where you can evaluate expressions, define functions, and explore the language:

```
mesh> 1 + 2
3 :: Int

mesh> let answer = 40
Defined: answer

mesh> answer + 2
42 :: Int

mesh> fn double(x) do
  ...   x * 2
  ... end
Defined: double :: (Int) -> Int

mesh> double(21)
42 :: Int
```

The REPL runs parsing, type checking, MIR lowering, and LLVM JIT compilation for
each expression. It is not a separate interpreted language. Its current value
printer is narrower than compiled application output: `Int`, `Bool`, `Float`,
and `Unit` render as values, while pointer-backed values such as `String`,
`Bytes`, collections, and structs render as typed pointer placeholders. Use
ordinary functions such as `println`, `Bytes.to_hex`, or a package-specific
renderer when inspecting those values.

The REPL initializes the actor runtime, but it does not replace a project build
for manifest-gated native packages or deployment configuration.

### REPL Commands

| Command | Shorthand | Description |
|---------|-----------|-------------|
| `:help` | `:h` | Show available commands |
| `:type <expr>` | `:t` | Show the inferred type without evaluating |
| `:quit` | `:q` | Exit the REPL |
| `:clear` | | Clear the screen |
| `:reset` | | Reset session (clear all definitions and history) |
| `:load <file>` | | Load and evaluate a Mesh source file |

### Multi-line Input

The REPL automatically detects incomplete input. If you open a `do` block without closing it with `end`, the REPL switches to continuation mode (shown by `...`) until all blocks are balanced:

```
mesh> fn greet(name) do
  ...   println("Hello, ${name}!")
  ... end
Defined: greet :: (String) -> Unit

mesh> greet("world")
Hello, world!
```

Input history is loaded from and saved to `~/.mesh_repl_history`. `Ctrl-C`
cancels the current input without exiting; `Ctrl-D`, `:quit`, and `:q` exit.

## meshpkg — Package Registry CLI

The `meshpkg` binary provides commands for publishing and consuming packages from the Mesh package registry.

### Authentication

Open [the package publishing page](https://packages.meshlang.dev/publish), sign
in with GitHub, and save the generated token:

```bash
meshpkg login --token <your-token>
```

Without `--token`, `meshpkg login` prompts on standard input. Credentials are
stored in `~/.mesh/credentials`.

### Publishing a Package

Publish the current directory as a package:

```bash
meshpkg publish
```

This reads `mesh.toml`, creates a `.tar.gz` tarball, computes the SHA-256 checksum, and uploads to the registry. Publishing the same name+version twice is rejected (HTTP 409).

The authenticated GitHub login must match the package-name scope, such as
`your-login/your-package`. Versions are immutable and uploads are limited to
50 MiB.

The publish archive preserves package-relative `.mpl` paths, including nested
modules and an override entrypoint. Hidden paths and `*.test.mpl` files are
excluded. Manifest-declared native bindings and static libraries are included
and their hashes are verified before upload. `README.md` is not currently
included by `meshpkg publish`.

Target another compatible registry with `--registry`:

```bash
meshpkg publish --registry https://registry.example.com
```

### Installing a Package

Install the latest release of a package from the registry into the current project:

```bash
meshpkg install your-login/your-package
```

This fetches the latest published release, verifies its SHA-256 checksum, extracts it into the project's dependency directory, and updates mesh.lock to pin the exact version. Named install does not edit mesh.toml; add the dependency yourself when you want it declared in the manifest.

Omit the name to install every exact registry dependency already declared in
`mesh.toml`:

```bash
meshpkg install
```

Registry dependencies support exact versions only. Run `meshc deps` before
`meshpkg install` in a project that also has git or path dependencies.

### Searching

Search the registry by name or keyword:

```bash
meshpkg search json
```

Returns matching package names and descriptions.

Search and install accept `--registry <url>`. Every meshpkg command accepts the
global `--json` flag:

```bash
meshpkg --json search json
meshpkg --json install
```

### mesh.toml with Registry Dependencies

Declare registry dependencies in `mesh.toml`:

```toml
[package]
name = "my_app"
version = "1.0.0"
description = "A Mesh application"
license = "MIT"

[dependencies]
"your-login/your-package" = "1.0.0"                         # registry: exact version (quoted because scoped names contain '/')
my_lib = { path = "../my_lib" }                              # local path
utils = { git = "https://github.com/user/utils", tag = "v1.0.0" }  # git
```

Scoped registry package names include `/`, so TOML keys must be quoted in `mesh.toml`.

Browse and search available packages at [packages.meshlang.dev](https://packages.meshlang.dev).
See [Packages and Registry](/docs/packages/) for publishing rules, lockfile
behavior, native archives, and the shipped `mesh-borsh`, `mesh-anchor`, and
`mesh-solana` surfaces.

## Language Server (LSP)

Mesh includes a Language Server Protocol implementation that provides real-time feedback in your editor:

```bash
meshc lsp
```

This starts the language server on **stdin/stdout** using the **JSON-RPC** protocol (standard LSP transport). The server is built on the `tower-lsp` framework and provides:

### LSP capabilities

The transport-level regression suite for `meshc lsp` exercises these
editor-facing behaviors over real stdio JSON-RPC:

| Feature | Description |
|---------|-------------|
| **Diagnostics** | Parse errors and type errors displayed inline as you type |
| **Hover** | Hover over identifiers to see inferred type information |
| **Go-to-definition** | Jump to definitions in the current document |
| **Completion** | Keywords, built-in types, snippets, and names visible in the current scope |
| **Document symbols** | Functions, types, and other declarations for editor outline/symbol views |
| **Document formatting** | Format the current document through the same formatter used by `meshc fmt` |
| **Signature help** | Parameter hints for function calls, including active-parameter tracking |

The language server receives full-document changes and reruns the Mesh lexer,
parser, and type checker before publishing diagnostics.

### LSP client configuration

The JSON-RPC transport is shared across editors, but Mesh only publishes repo-owned editor-host guidance for VS Code and Neovim. VS Code starts `meshc lsp` through the Mesh extension. Neovim uses the repo-owned pack in `tools/editors/neovim-mesh/`. Best-effort editors that support LSP can point their client at:

```json
{
  "command": "meshc",
  "args": ["lsp"]
}
```

## Editor Support

### Support tiers

| Tier | Editors | Mesh-owned contract |
|------|---------|---------------------|
| First-class | VS Code and Neovim | Public docs, editor-specific READMEs, and repo-owned proof cover the published install/run path. |
| Best-effort | Emacs, Helix, Zed, Sublime Text, TextMate reuse, and similar setups | Reuse the shared `meshc lsp` transport or VS Code TextMate grammar, but Mesh does not publish repo-owned editor-host smoke for these integrations. |

### VS Code

VS Code is a first-class editor host in the public Mesh tooling contract. The
official Mesh extension provides syntax highlighting, diagnostics, hover,
same-file go-to-definition, completion, document symbols, document formatting,
and signature help. Its shared grammar covers `@cluster`, `@cluster(N)`,
`#{...}`, and `${...}` in double- and triple-quoted strings.

#### VS Code features

- **Syntax highlighting** via the shared TextMate grammar used by VS Code and the docs, with verified coverage for Mesh keywords, operators, comments, and both `#{...}` plus `${...}` interpolation in double- and triple-quoted strings
- **Language configuration** for bracket matching, auto-closing pairs, and automatic indentation of `do`/`end` blocks
- **LSP integration** that starts `meshc lsp` automatically and exposes
  diagnostics, hover, go-to-definition, completion, document symbols,
  formatting, and signature help

#### VS Code installation

Install Mesh first so `meshc lsp` is already available. Install
[Mesh Language from the VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=OpenWorthTechnologies.mesh-lang)
for the normal editor path.

To build and install the current extension source instead:

```bash
cd tools/editors/vscode-mesh
npm install
npm run compile
npm run package
```

The package step writes `dist/mesh-lang-<version>.vsix`. To install that freshly built artifact into your local VS Code profile, run:

```bash
npm run install-local
```

Or open the `tools/editors/vscode-mesh/` folder in VS Code and press F5 to launch an Extension Development Host with the extension loaded.

When you need the full repo-root public proof chain instead of only the VS Code packaging/install loop, run:

```bash
bash scripts/verify-m036-s03.sh
```

That verifier keeps the public tooling contract honest by replaying the docs contract, VitePress build, existing VSIX/public README proof, real VS Code editor-host smoke, and the Neovim replay from one named-phase command.

#### VS Code configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `mesh.lsp.path` | `"meshc"` | Path to the `meshc` binary (must be in PATH, or provide an absolute path) |

Without an explicit override, the extension checks workspace-local
`target/debug/meshc` and `target/release/meshc`, then `~/.mesh/bin`,
`/usr/local/bin`, `/opt/homebrew/bin`, and finally `PATH`.

### Neovim

Neovim is a first-class editor host in the public Mesh tooling contract for the audited classic syntax plus native `meshc lsp` path already proven in `scripts/verify-m036-s02.sh`. The repo-owned support pack lives in `tools/editors/neovim-mesh/` and requires **Neovim 0.11+**.

#### Neovim installation

Install Mesh first so `meshc` is available, then place `tools/editors/neovim-mesh/` on an active `packpath` as `pack/*/start/mesh-nvim`. A direct repo-local install looks like this:

```bash
mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/mesh/start"
ln -s \
  "/absolute/path/to/mesh-lang/tools/editors/neovim-mesh" \
  "${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/mesh/start/mesh-nvim"
```

After installation, opening any `*.mpl` file should load the classic syntax runtime files and auto-enable the native `meshc lsp` config when the binary is available.

Override binary discovery with either:

```lua
vim.g.mesh_lsp_path = "/absolute/path/to/meshc"
```

```lua
require("mesh").setup({ lsp_path = "/absolute/path/to/meshc" })
```

The pack searches workspace-local debug/release builds, the same well-known
installer locations as the VS Code extension, and then `PATH`. Project root
selection prefers `mesh.toml`, then root `main.mpl`, then `.git`; otherwise the
client attaches in single-file mode.

#### Verification

For the full repo-root public tooling/editor proof chain, run:

```bash
bash scripts/verify-m036-s03.sh
```

Use the Neovim-specific verifier below when you only need to replay this pack's bounded proof surface:

```bash
NEOVIM_BIN="${NEOVIM_BIN:-nvim}" bash scripts/verify-m036-s02.sh
```

That proof is intentionally bounded to the shared syntax corpus plus the native `meshc lsp` path. It does not imply Tree-sitter support or support for third-party Neovim plugin-manager packaging.

### Best-effort editors

Editors outside the first-class tier can still reuse the shared Mesh surfaces, but those integrations are best-effort. For syntax highlighting, reuse `tools/editors/vscode-mesh/syntaxes/mesh.tmLanguage.json` anywhere that can ingest a TextMate grammar. For LSP, point your editor at `meshc lsp` over stdin/stdout JSON-RPC.

Best-effort examples include Emacs, Helix, Zed, Sublime Text, and TextMate-style consumers of the shared grammar. Mesh does not publish repo-owned editor-host smoke, packaging, or troubleshooting guides for those setups.

## Routine compatibility workflow

Normal PRs and `main` pushes now also fan out through `compatibility-matrix.yml`.
That workflow is the compile-only cross-platform signal: it builds `meshc` across the release target matrix and builds `meshpkg` everywhere except the musl-only lane, but it does **not** replace the tag/manual release packaging flow.
Use it when you want early platform breakage visibility without waiting for a version tag.

## Release Assembly Runbook

When you need the full public-release acceptance flow instead of an individual tool check, run the assembled verifier from the repo root with the repo `.env` loaded:

```bash
set -a && source .env && set +a && bash scripts/verify-m034-s05.sh
```

The candidate identity stays split on purpose:

- Binary release candidate tag: `v<Cargo version>` from `compiler/meshc/Cargo.toml` and `compiler/meshpkg/Cargo.toml`
- VS Code extension release candidate tag: `ext-v<extension version>` from `tools/editors/vscode-mesh/package.json`

Hosted rollout evidence must exist for these exact workflows:

- `deploy.yml`
- `deploy-services.yml`
- `authoritative-verification.yml`
- `release.yml`
- `extension-release-proof.yml`
- `publish-extension.yml`

The runbook stays tied to these exact public URLs:

- `https://meshlang.dev/install.sh`
- `https://meshlang.dev/install.ps1`
- `https://meshlang.dev/docs/getting-started/`
- `https://meshlang.dev/docs/tooling/`
- `https://packages.meshlang.dev/packages/snowdamiz/mesh-registry-proof`
- `https://packages.meshlang.dev/search?q=snowdamiz%2Fmesh-registry-proof`
- `https://api.packages.meshlang.dev/api/v1/packages?search=snowdamiz%2Fmesh-registry-proof`

The verifier persists the candidate and hosted-run evidence under:

- `.tmp/m034-s05/verify/candidate-tags.json`
- `.tmp/m034-s05/verify/remote-runs.json`

## Tool Summary

| Tool | Command | Description |
|------|---------|-------------|
| Compiler | `meshc build <dir>` | Compile a project to a native executable |
| Project scaffolding | `meshc init [--clustered \| --template todo-api --db <backend>] <name>` | Create hello-world, clustered, SQLite Todo, or PostgreSQL Todo projects |
| Source dependencies | `meshc deps [dir]` | Resolve git/path dependencies and write lock entries |
| Registry dependencies | `meshpkg install [name]` | Install all declared exact registry dependencies or one latest named package |
| Migrations | `meshc migrate [dir] [up \| down \| status \| generate]` | Generate and run PostgreSQL migrations |
| Formatter | `meshc fmt <path>` | Recursively format Mesh source or use `--check` in CI |
| Test Runner | `meshc test [path]` | Run `*.test.mpl` files from a project root, tests directory, or specific test file |
| REPL | `meshc repl` | Interactive LLVM JIT evaluation |
| Language Server | `meshc lsp` | Diagnostics, hover, navigation, completion, symbols, formatting, and signature help over stdio JSON-RPC |
| Cluster operations | `meshc cluster <command>` | Inspect or mutate runtime-owned cluster state |
| Release gates | `meshc proof <command>` | Run Docker, Fly, chaos, performance, and continuity proof commands |
| Toolchain update | `meshc update` or `meshpkg update` | Refresh both installed commands |
| Package CLI | `meshpkg <login \| publish \| install \| search \| update>` | Authenticate with and use a registry |
| VS Code Extension | Marketplace or VSIX | First-class VS Code host for the shared grammar and Mesh LSP |
| Neovim Pack | Native package runtime | First-class Neovim host for classic syntax and `meshc lsp` |

## Next Steps

- [Testing](/docs/testing/) -- write and run tests with `meshc test`
- [Packages and Registry](/docs/packages/) -- resolve, install, publish, and use packages
- [Native Packages](/docs/native-packages/) -- build and consume ABI 1 packages
- [Standard Library](/docs/stdlib/) -- Crypto, Encoding, and DateTime modules
- [Language Basics](/docs/language-basics/) -- core language features and syntax
- [Distributed Actors](/docs/distributed/) -- building distributed systems with Mesh
