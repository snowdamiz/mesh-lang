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

The installers put both commands in `~/.mesh/bin` and the runtime libraries
that every program links against in `~/.mesh/lib` (`libmesh_rt.a` and
`libmesh_test_rt.a`; `mesh_rt.lib` and `mesh_test_rt.lib` on Windows). `meshc`
finds them in the `lib` directory beside its own `bin` directory, so keep the
two together if you move the installation.

If you extract the compiler archive manually, place the `meshc` executable in
`<prefix>/bin` and the two runtime archives in `<prefix>/lib`. The executable
and archives need this layout for `meshc build` and `meshc test`.

`meshc build` links with the system C compiler driver: `cc` on macOS and Linux
(the Xcode Command Line Tools, or `build-essential` and its equivalents), and
`clang` from LLVM 21 on Windows, taken from `LLVM_SYS_211_PREFIX\bin` when that
variable is set and from `PATH` otherwise.

The installers check both archives against the release's `SHA256SUMS`. A
manifest entry can use the archive basename or prefix it with `./`. A checksum
mismatch stops the install. Set `MESH_INSTALL_STRICT_PROOF=1` to require a valid
checksum entry for each archive. See [installer environment variables](/docs/environment-variables/#installers-and-updates).

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
`-Uninstall`, `-Yes`, or `-Help`. Uninstall removes the whole `~/.mesh`
directory (both commands, the runtime libraries, and the `meshpkg` registry
credentials) and the PATH changes managed by the installer.

If you are contributing to Mesh or need an unsupported target, see the
[source-build prerequisites](/docs/getting-started/#alternative-build-from-source).
Source builds require LLVM 21 and a working native linker; they are an
alternative workflow, not the primary public install contract.

## Read the offline guide and install the Mesh skill

Run `meshc guide` to list topics or `meshc guide syntax` for examples such as
the slot pipe. The guide comes from the running compiler and works offline
with just the executable. Its SHA-256 identifies the guide content, including
when two builds report the same compiler version.

After an interactive installation, Mesh offers to set up its skill for your
coding tools. You can run the picker at any time, including with a
source-built or manually extracted compiler.

```bash
meshc skills setup
```

Choose one or more hosts, select account or project scope, then confirm the
displayed destinations. Project scope uses the nearest enclosing Mesh
project or Git repository, falling back to the current directory. Hosts
that share a directory can discover the same installed copy.

| Host | Project directory | Account directory |
| --- | --- | --- |
| Codex | `.agents/skills/mesh` | `~/.agents/skills/mesh` |
| Claude Code | `.claude/skills/mesh` | `~/.claude/skills/mesh` |
| Cursor | `.agents/skills/mesh` | `~/.cursor/skills/mesh` |
| Gemini CLI | `.gemini/skills/mesh` | `~/.gemini/skills/mesh` |
| GitHub Copilot CLI | `.github/skills/mesh` | `~/.copilot/skills/mesh` |

On Windows, `~` refers to `%USERPROFILE%`. Your coding tool manages its own
workspace trust and permissions.

Fresh unattended installs, including `--yes` and `-Yes`, leave skill setup
optional. Upgrades refresh account copies that Mesh installed and whose
files match the recorded digest. Mesh reports paths with local edits, extra
files, symlinks, or an existing skill managed by another tool. Project
copies stay pinned until you rerun setup and confirm the project scope.

For unattended project setup, run `meshc skills setup --yes --target <agent>`.
Repeat `--target` to select multiple hosts. Run `meshc skills setup --help`
for the supported agent IDs.

Run `meshc skills refresh` to apply the same account refresh with your current
compiler without reinstalling it.

## Verify a pure policy

`meshc verify` checks laws and proofs for pure code. Write a result law to
compare a function with an independent expected result. Z3 checks supported
`Int` and `Bool` result laws automatically. Write a Mesh proof to check
recursive code or to prove a named theorem. Libraries can verify laws without
an executable entrypoint.

### Run the retry law

In an empty project directory, save the implementation as `policy.mpl`.

```mesh
pub fn retry(status :: Int, left :: Int) -> Bool do
  (status == 429 || status == 503) && left > 0
end
```

Create `laws/http/eligibility.law` with the expected result.

```text
from Policy import retry

law retry(status :: Int, left :: Int) -> Bool do
  case status do
    429 -> left > 0
    503 -> left > 0
    _ -> false
  end
end
```

For a private function in the same package, write
`from Policy import private retry` in the law. The function retains its
visibility in application imports.

List the resolved target without a solver.

```sh
meshc verify . --list
```

Put Z3 4.15.7 on `PATH` or set `MESH_VERIFY_Z3` to your trusted Z3 executable.
Then run `meshc verify .`. The result is `verified`. Builds and tests run
without Z3. Set `MESH_VERIFY_TIMEOUT_MS` to change the default solver budget of
5,000 milliseconds per law.

Append `&& left <= 8` to the implementation's expression and run
`meshc verify . --json` again. The command exits with code 1 and reports
`falsified`. The report's `inputs`, `expected`, and `actual` fields show a
counterexample. Call the compiled function with the reported inputs to replay
it. For instance, at status 429 with a budget of 9, the law expects `true` and
the capped function returns `false`. Restore the original implementation and
run `meshc verify .` again.

### Prove a recursive list function

Include a `proof` block in the same `.law` file as its matching `law`. Write
both in Mesh. Set `MESH_VERIFY_LEAN` to a trusted Lean 4.36.0 executable or
put it on `PATH`, then run `meshc verify .`.

Save this function as `flags.mpl`. It drops `false` values until the first
`true`.

```mesh
pub fn drop_false_prefix(xs :: List<Bool>) -> List<Bool> do
  case xs do
    [] -> []
    head :: tail ->
      if head do
        xs
      else
        drop_false_prefix(tail)
      end
  end
end
```

Save the law and its proof as `laws/flags.law`.

```text
from Flags import drop_false_prefix

law drop_false_prefix(xs :: List<Bool>) -> List<Bool> do
  case xs do
    [] -> []
    head :: tail ->
      if !head do
        drop_false_prefix(tail)
      else
        xs
      end
  end
end

proof drop_false_prefix do
  induction xs do
    [] -> refl
    head :: tail, ih ->
      cases head do
        false -> exact ih
        true -> refl
      end
  end
end
```

`xs` names the list parameter in the law. `induction` gives the nonempty
branch the element, tail, and induction hypothesis. `cases` splits a
Boolean value into its two possible values. `refl` checks equality by
reduction. `exact ih` applies the induction hypothesis after reduction.
Run `meshc verify .` and expect `verified`. Change the implementation's
`true` branch to return `[]`. The proof will fail because that branch
must preserve the nonempty list.

For a recursive append, `congr List.cons(head), ih` uses an equality from
`ih` to prove that prepending the same `head` to each side preserves equality.

The kernel checks the proof against the modeled implementation and the law.
Result proofs support scalars, nested lists, and tuples. They accept direct
recursion that decreases a list argument to its tail. An unsupported
operation, missing checker, or rejected proof returns `unverified`. Use
`meshc verify . --list` to see proof locations without running a checker.

### State a theorem and reuse its proof

Start a `.law` file with `logic` to use named propositions. A theorem can
refer to checked functions, define proof-side functions and inductive types,
or apply a theorem proved elsewhere in the same package. The theorem name
identifies the requirement. Its statement can use equality, implication,
conjunction, disjunction, and universal or existential quantification.

The following file proves a property of the native function from the
preceding example. It also gives a witness for a dependent predicate.

```text
logic
from Flags import drop_false_prefix

theorem keeps_true_prefix(tail :: List<Bool>) do
  drop_false_prefix(List.cons(true, tail)) == List.cons(true, tail)
end

proof keeps_true_prefix do refl end

theorem predicate_has_witness(P :: Nat -> Prop, at_seven :: P(7)) do
  exists value :: Nat do P(value) end
end

proof predicate_has_witness do
  witness 7 do exact at_seven end
end
```

Add paired `theorem` and `proof` declarations with the same name. Refer to a
proved theorem by applying it to its arguments in an `exact` or `rewrite`
step. The checker infers declaration order from dependencies across the
package's discovered `logic` files.

Use `inductive` to define a proof-side datatype with type parameters or
value indices. The constructors of an indexed family specify their result
types.

```text
logic

inductive Vector(T :: Type) -> Nat -> Type do
  Empty() -> Vector(T, 0)
  Push(n :: Nat, head :: T, tail :: Vector<T, n>) -> Vector(T, Nat.succ(n))
end

theorem empty_is_reflexive(T :: Type, empty :: Vector<T, 0>) do
  empty == empty
end

proof empty_is_reflexive do refl end
```

Use `induction` for a structural proof and `cases` to analyze a value or a
hypothesis. An empty `cases` block eliminates a hypothesis with no possible
constructors. Use `branch` to prove the goal from each outcome of a
decidable proposition. The `yes(h)` and `no(h)` arms receive proofs of the
proposition and its negation. `unfold` expands the named definition in the
goal. `change` states an equivalent goal by computation, and `rewrite`
uses a proved equality. Use `goal` in an unfinished proof to print the
current obligation. The law still reports `unverified`.

Mesh `Int` uses signed comparisons and wrapping 64-bit arithmetic. Use
`Nat` for mathematical natural numbers. Write `(value : Type)` to fix a
term's type when inference needs it. A typed `let name :: Type = value`
binds a value for the following expression. Write `Prod<A, Prod<B, C>>` for
the type of a three-element tuple and `(a, (b, c))` for its value. With an
expected type such as `Sigma(fn x :: T -> Family(x) end)`, the type of a
pair's second component depends on the first.

### Bind compiled types and generic functions

Use separate `from Module import name` lines for a function and any
public nominal types in its theorem. Use `Type.Variant(...)` for sum
constructors and `Type.mk(...)` for struct fields in declaration order.
The checker uses the compiler's checked field types, layouts, and emitted
function bodies. It follows supported direct calls to pure helpers,
including private helpers.

To check one specialization, give the concrete type arguments or function
signature. An alias gives the theorem a short name for that specialization.

```text
logic
from Trees import Tree<List<Int>> as IntTree
from Trees import mirror(Tree<List<Int>>) -> Tree<List<Int>> as mirror_int
```

The verifier uses the exact checked types. For example, `Tree<List<Int>>`
and `Tree<List<Bool>>` can share a native layout while remaining distinct
types in the proof.

To prove a theorem for all type arguments, import the generic type and
function without arguments. Pass the type explicitly when you call the
function in a theorem.

```text
logic
from Trees import Tree
from Trees import mirror

theorem mirror_twice(T :: Type, tree :: Tree<T>) do
  mirror(T, mirror(T, tree)) == tree
end

proof mirror_twice do
  induction tree do
    Tree.Leaf(value) -> refl
    Tree.Branch(left, right, ih_left, ih_right) ->
      change Tree.Branch(mirror(T, mirror(T, left)), mirror(T, mirror(T, right))) == Tree.Branch(left, right) do
        rewrite ih_left do rewrite ih_right do refl end end
      end
  end
end
```

A universal native import requires an unconstrained generic function in
the supported pure fragment. The verifier compares its generic lowering
with every concrete specialization in the checked project. Use a concrete
signature when type constraints determine which code runs.

Native indexed functions can quantify over Nat indices as well as types.
For example, this `.law` file proves that a recursive `copy` preserves the
contents of a compiled `Vec<T, n>` at every length.

```text
logic
from Vector import Vec
from Vector import copy

theorem copy_preserves_contents(T :: Type, n :: Nat, values :: Vec<T, n>) do
  copy(T, n, values) == values
end

proof copy_preserves_contents do
  induction values do
    Vec.Empty -> refl
    Vec.Push(head, tail, ih) -> exact congr(Vec.Push(head), ih)
  end
end
```

The compiled proof model supports a dependent case over one Nat index
with explicit, unguarded constructor arms.

### Use proofs in a Mesh program

Declare indexed sums and proof-bearing functions in ordinary `.mpl` files.
`Vec<Int, 1>` and `Vec<Int, 2>` have the same native representation, but the
type checker distinguishes their lengths. In `Vec<T, Succ<n>>`, the index
tells the checker that only `Push` can occur.

```mesh
pub type Vec<T, n :: Nat> do
  Empty -> Vec<T, 0>
  Push<k :: Nat>(T, Vec<T, k>) -> Vec<T, Succ<k>>
end

logic do
  fn FitsTwo(size :: Nat) -> Prop do size == 2 end
  pub theorem allowed_two() do FitsTwo(2) end
  proof allowed_two do refl end
end

pub fn deliver<T, n :: Nat>(values :: Vec<T, Succ<n>>, witness :: Proof<FitsTwo<Succ<n>>>) -> T do
  case values do
    Push(head, _) -> head
  end
end
```

Save the module as `vector.mpl`. Import the checked theorem as a value in
`main.mpl`.

```mesh
from Vector import Vec, deliver, allowed_two

fn main() do
  let single :: Vec<Int, 1> = Push(7, Empty)
  let double :: Vec<Int, 2> = Push(42, single)
  println("${deliver(double, allowed_two)}")
end
```

`meshc build .` checks embedded proofs before producing the executable.
The executable prints `42`. Replacing `double` with `single` fails type
checking because `allowed_two` proves `FitsTwo(2)`. Nat indices and proof
values carry no native payload. `meshc verify . --verdict` independently
rechecks the embedded proofs.

To apply a reusable theorem from normal code, add this `logic` block to
`vector.mpl`.

```mesh
logic do
  pub theorem confirm(size :: Nat, known :: FitsTwo(size)) do
    FitsTwo(size)
  end
  proof confirm do exact known end
end
```

Import `confirm` from `Vector`, then call
`deliver(double, confirm(2, allowed_two))`. Static Nat arguments accept
natural-number literals, declared indices, `Succ(...)`, and arithmetic
that the type checker can evaluate. Mesh checks proof arguments at those
indices. Expressions that produce proof arguments still run.

A theorem can also return a direct Nat equality. Write its evidence type
as `Proof<NatEq<Left, Right>>` when you pass or return it in ordinary code.
The operands can use literals, Nat indices, `Succ`, `NatAdd`, `NatSub`, or
`NatMul`.

```mesh
logic do
  pub theorem add_zero(n :: Nat) do n + 0 == n end
  proof add_zero do
    induction n do
      Nat.zero -> refl
      Nat.succ(previous, ih) -> refl
    end
  end
end

fn zero_at_two() -> Proof<NatEq<2, 2>> do
  add_zero(2)
end
```

### Recheck and export proofs

Run the independent checker with the full Lean 4.36.0 toolchain installed.

```sh
meshc verify . --verdict --emit-proof .proof-artifacts
```

Mesh checks the generated theorem and its proof with Lean. The only
permitted axiom is Lean's propositional extensionality (`propext`), which
some dependent pattern matches use. `--verdict` also checks that every
required theorem appears in the proof export and rechecks the export with
ConLeche in its mechanically verified mode. Automatic result laws continue
to use Z3. `--emit-proof` writes SHA-256-named `.lean` files and, with
`--verdict`, the corresponding `.ndjson` exports. Replay the exports with
`con-leche --verified <artifact.ndjson>`. Regenerate the artifacts after
changing the implementation, law, or proof.

The proof is about the definitions generated from the checked program.
ConLeche's consistency result covers its verified declaration checker.
Native execution also depends on faithful lowering, the runtime
implementation of admitted operations, and normal completion. For example,
the proof of a recursive list result gives no bound on the native stack
required to compute it. Effects, native bindings, and unsupported or
nonterminating recursion fail verification.

### Prove a compiled sort

The repository's insertion sort example proves that every `List<Int>`
sorts into signed ascending order and that the output preserves each
input element's multiplicity. It checks the compiled `sort` and `insert`
functions against separate Mesh requirements and proofs. [Requirements](https://github.com/snowdamiz/mesh-lang/blob/main/compiler/meshc/tests/fixtures/proofs/sort/requirements.law) · [Mesh proofs](https://github.com/snowdamiz/mesh-lang/blob/main/compiler/meshc/tests/fixtures/proofs/sort/proofs.law)

From the repository root, run the example with the independent checker.

```sh
meshc verify compiler/meshc/tests/fixtures/proofs/sort --verdict
```

Change `value <= head` to `value >= head` in the example's `sort.mpl`.
The requirement inventory stays the same and the proof fails. Restore
the comparator and rerun verification to check all ten theorems again.

### Choose which laws to check

By default, Mesh scans the project recursively for visible `.law` files. It
skips build output, hidden dependency directories, and nested packages. A
`[verify]` table in `mesh.toml` replaces the default scan with the listed
directories. Use existing package-relative directories. Overlapping roots
count each file once.

```toml
[verify]
law_roots = ["laws"]
```

### Read the verdict

The verifier reports `verified` when the modeled function agrees with the
result law for every input, or when the checker accepts a named theorem's
proof. The automatic result model handles signed 64-bit arithmetic,
Boolean expressions, conditionals, and direct calls to supported helpers.
A `falsified` result includes inputs and both values. An `unverified`
result gives the reason, such as an exhausted solver budget, division,
recursion, or a clock read. For unsupported source expressions, the text
report shows the file, line, and column. JSON includes these fields and byte
offsets under `unsupported_at`. Verification succeeds when it proves at
least one law and every discovered law verifies. `--list --json` emits an
inventory with each target, its source location, and its requirement digest.

### Preserve requirements in CI

For an implementation change in CI, generate the baseline with the same
trusted compiler and keep the inventory outside the candidate's writable
checkout.

```sh
meshc verify "$BASE_CHECKOUT" --list --json > "$TRUSTED_ARTIFACTS/base-laws.json"
meshc verify "$CANDIDATE_CHECKOUT" --preserve "$TRUSTED_ARTIFACTS/base-laws.json" --json
```

The preservation check allows law-file moves and requires new laws to verify.
It fails if the candidate drops a baseline target or changes its requirement
digest. The digest covers the resolved requirement. Moving a law or changing
its whitespace or comments keeps the digest. A change to the owning package,
module, name, or signature changes the target identity.

For named theorems, the digest includes referenced specification
definitions and checked import types. Proof scripts and implementation
bodies can change when the new proof still checks against the preserved
requirement.

Use the [retry regression test](/docs/testing/#test-a-change-to-a-retry-policy)
to exercise the executable alongside the law.

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

`meshc init` refuses a name whose directory already exists. `--clustered` and
`--template` cannot be combined, `todo-api` is the only template, and
`--db sqlite|postgres` applies only to that template (default `sqlite`).

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

The entrypoint must be a relative path that stays inside the project and ends
in `.mpl`.

| `[package]` key | Required | Meaning |
|-----------------|----------|---------|
| `name` | yes | Package name; a published package is scoped as `login/name` |
| `version` | yes | Package version |
| `description` | no | One-line summary shown by the registry |
| `authors` | no | List of author strings |
| `license` | no | License identifier, such as `"MIT"` |
| `entrypoint` | no | Executable entry file; default `main.mpl` |

Other keys under `[package]` and unknown top-level tables are ignored. Two
further tables have pages of their own: `[native]` declares a package's native
libraries ([Native Packages](/docs/native-packages/)), and `[cluster]` holds
autonomous-cluster deployment policy
([Autonomous Clusters](/docs/autonomous-clusters/)); unknown keys inside
`[cluster]` are errors, as are the removed `[cluster]` keys `enabled` and
`declarations`.

The manifest supports registry, git, and path dependencies:

```toml
[dependencies]
"your-login/your-package" = "1.0.0"
widget = { version = "2.1.0" }
my_lib = { path = "../my_lib" }
some_pkg = { git = "https://github.com/user/some_pkg", tag = "v1.0.0" }
```

Registry versions must be exact, written as a string or as
`{ version = "..." }`. Git dependencies support `rev` (a hex commit id),
`branch`, and `tag`; with none of them the remote's default branch is used.
Prefer an immutable `rev` for a release.

### Lockfile

`mesh.lock` records exact registry versions and checksums, git commits, and
local path entries. `meshc deps` writes the git and path entries and
`meshpkg install` the registry ones; each keeps the entries the other wrote,
so run them in either order:

```bash
meshc deps
meshpkg install
```

Commit `mesh.lock`. `meshc build` consumes installed and fetched dependencies
but does not fetch missing code. See [Packages and Registry](/docs/packages/)
for the complete dependency and publishing workflow.

## Build projects

Compile a project directory to a native executable:

```bash
meshc build .
./output
```

The directory must contain the entrypoint: `main.mpl`, or the file
`[package].entrypoint` names. `mesh.toml` is optional for a program with no
dependencies; without it the build compiles the directory's `.mpl` files and
nothing else. Every `.mpl` file under the directory is compiled, except hidden
files and directories, the top-level `tests/` directory, `*.test.mpl` files,
and `*.test-support.mpl` fragments. An executable build fails with
"`main.mpl` has no `fn main()`" when the entrypoint defines no `main`.

When the directory argument is `.`, the default output is `./output`. When it
is a named directory such as `apps/api`, the default is `apps/api/api`. Prefer
an explicit output when a script depends on the name:

```bash
meshc build . --output my_app
./my_app
```

Build options:

| Option | Behavior |
| --- | --- |
| `--opt-level 0` | Debug/default optimization |
| `--opt-level 1` | LLVM `O1` |
| `--opt-level 2` | Release optimization (`O2`); higher values also mean `O2` |
| `--emit-llvm` | Write a `.ll` file next to the executable |
| `-o, --output <path>` | Choose the executable path |
| `--target <triple>` | Generate and link for an explicit target triple |
| `--artifact <kind>` | `executable` (default), `staticlib`, or `cdylib`; see [Library Builds](/docs/library-builds/) |
| `--json` | Emit newline-delimited JSON diagnostics |
| `--no-color` | Disable color in human-readable diagnostics |
| `--plaintext-report <path>` | After the build, write the [plaintext report](#plaintext-report) there |

The build reports `  Compiled: <path>` (and `  LLVM IR: <path>` with
`--emit-llvm`) on standard error. Human-readable diagnostics are colored only
when standard error is a terminal and `NO_COLOR` is unset.

### Plaintext report

Every `declassify` call and every `@display` export of a project (see
[Plaintext](/docs/type-system/#plaintext)) is listed by

```bash
meshc plaintext-report .                                  # print it
meshc plaintext-report . --output plaintext-report.json   # write it
meshc plaintext-report . --check plaintext-report.json    # compare with a committed one
```

The report is JSON, sorted by file and line, so the same source gives the same
bytes. `--check` exits `1` when a site was added
(``+ declassify in `f` (main.mpl): reason``) or removed (`-`), and passes when
only line numbers moved.
Commit the report and run the check in CI, so no disclosure is added without
review. The command type-checks the project as `meshc build` does, and fails
on the same errors.

### Check and explain diagnostics

Run `meshc check [path]` to parse and type-check a project or a standalone
source file without generating a binary. The path defaults to the current
directory. Add `--tests` to check the project's `*.test.mpl` files without
running them. Add `--json` for machine-readable diagnostics.

Run `meshc explain <CODE>` for the catalog entry, including the repair and
any examples. You can also look up a diagnostic by its kebab-case name.
Run `meshc explain` to list the codes. `meshc explain --json` writes the
catalog object to standard output. Pass a code or name to get one entry
in that format.

### JSON diagnostics

With `--json`, the v2 diagnostic stream writes one JSON object per line to
standard error. For `let x :: Int = "abc"` inside `fn main()` in `main.mpl`,
`meshc build . --json` reports the following diagnostic.

```json
{"code":"E0001","severity":"error","message":"type mismatch: expected `Int`, found `String`","file":"./main.mpl","spans":[{"start":21,"end":27,"label":"expected Int from annotation","file":"./main.mpl","line":2,"col":9,"end_line":2,"end_col":15}],"fix":"Convert the value (`Int.to_float(...)`, `Some(...)`, `Ok(...)`), change it, or change the annotation so both sides name one type.","line":2,"col":9,"name":"type-mismatch","category":"semantic","notes":[],"guide":{"explain":"meshc explain E0001","skill":"references/diagnostics.md#e0001","topic":"types"},"docs_url":"https://meshlang.dev/docs/errors/#e0001","edits":[]}
```

| Fields | Meaning |
| --- | --- |
| `code`, `name`, `severity`, `category` | Stable ID, kebab-case name, severity, and diagnostic family. |
| `message`, `fix`, `notes` | The problem, a non-null repair string, and an array of extra details. |
| `file`, `line`, `col` | Primary location. Invocation-level diagnostics have an empty `file`. Unknown lines and columns are `null`. |
| `spans` | The primary span followed by related spans. Each span names its own file and carries start and end positions and a label. |
| `guide`, `docs_url` | The `meshc explain` command, local skill reference, optional guide topic, and website entry. |
| `edits` | Suggested edits, each with `file`, `start`, `end`, `replacement`, and `applicability`. |

Offsets are zero-based UTF-8 bytes. End positions are exclusive. Lines and
columns are one-based, and columns count Unicode scalar values. Apply
`machine-applicable` edits to the source that produced them. Review
`maybe-incorrect` edits before applying them.

Every failure reports its specific code. A failed build exits with status 1,
and the stream contains the individual diagnostics without a final summary
object. The [diagnostic catalog](/docs/errors/) covers syntax (`P`),
semantic errors and warnings (`E`, `W`), lint (`L`), project and cluster
configuration (`M`, `CFG`), verification (`V`), build and linking (`B`),
internal compiler diagnostics (`I`), runtime (`R`), tests (`T`), and
command-line tools (`X`). Run `meshc explain <CODE>` to look up any active code.

For example:

```bash
meshc build . --opt-level 2 --emit-llvm --output dist/my_app
meshc build . --target x86_64-unknown-linux-gnu --output dist/my_app-linux
```

`--target` selects code generation and native-package archives; it does not
install a cross-linker, sysroot, C runtime, or target system libraries. Supply
those separately. A native dependency must declare a checksummed archive for
the exact effective target. See [Native Packages](/docs/native-packages/).

The linker depends on the target:

| Target | Linker |
| --- | --- |
| macOS, Linux, and other Unix-like triples | `cc` |
| `*-apple-ios`, `*-apple-ios-sim` | `xcrun --sdk iphoneos` (`iphonesimulator` for `-sim`) |
| `*-linux-android` | `<triple>26-clang` and `llvm-ar` from the NDK in `ANDROID_NDK_HOME` or `ANDROID_NDK_ROOT` |
| `*-windows-msvc` | `clang.exe` from `LLVM_SYS_211_PREFIX\bin`, or `clang` on `PATH` |

Other Windows triples are rejected. A cross build also needs the Mesh runtime
compiled for that target: `meshc` looks for it in `lib/<triple>/` beside the
installed toolchain, then in a Cargo target directory (`CARGO_TARGET_DIR`, or a
`target/` above the `meshc` binary) under `<triple>/release` and
`<triple>/debug`. `MESH_RT_LIB_PATH` names the runtime library explicitly
(`MESH_TEST_RT_LIB_PATH` for `meshc test`); its file name must be the target's
(`libmesh_rt.a`, or `mesh_rt.lib` for Windows MSVC). See
[Environment Variables](/docs/environment-variables/) for the rest.

### Resolve git and path dependencies

Run the source dependency resolver from the project root:

```bash
meshc deps
```

Pass another project directory when needed:

```bash
meshc deps apps/api
```

The resolver walks transitive git and path dependencies, checks git packages
out into `.mesh/deps/<name>/`, and writes their commits and the local path
entries to `mesh.lock`, keeping the registry entries already there. It prints
`Resolved N dependencies` or `No dependencies`. When the manifest is not newer
than the lockfile and every git dependency is checked out, it prints
`Dependencies up to date` and fetches nothing.

A git dependency is fetched at its `rev`, the head of its `branch`, its `tag`,
or the remote's default branch. The build compiles each checkout like a path
dependency; building before `meshc deps` has fetched one fails with
"Git dependency `name` is not fetched; run `meshc deps`". A dependency name
reached from two different sources, or a cycle, is an error.

Registry downloads belong to `meshpkg install`, which `meshc deps` leaves
alone. For a mixed project, run both:

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
`down` functions that return `Ok(0)`, with example `Migration.*` and `Pg.*`
calls commented out inside them. Fill them in:

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

The runner creates and maintains `_mesh_migrations` (`version BIGINT PRIMARY
KEY`, `name`, `applied_at`) in PostgreSQL. Only files named
`<digits>_<name>.mpl` in `migrations/` are migrations; others are ignored.
Each migration is compiled on its own, as a program of that one file and a
generated `main`, so it cannot import the project's modules, and runs with a
pool of one to two connections (`Pool.open(url, 1, 2, 5000)`). Without
`DATABASE_URL`, `status`, `up`, and `down` fail with
`DATABASE_URL environment variable is required`. `down` needs the
corresponding source file to remain present. Use neutral `Migration.*` helpers
for portable DDL and explicit `Pg.*` helpers for PostgreSQL-only features. See
[Databases](/docs/databases/).

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
| `meshc cluster events <target>` | The same ordered log as `diagnostics`: control, drain, scaling, startup, and continuity transitions |
| `meshc cluster explain <target> <request-key>` | Retained placement and current candidates |

All read commands accept `--cookie-file <path>`, `--timeout-ms <number>`
(default `5000`), and `--json`. Without `--cookie-file`, the CLI reads
`MESH_CLUSTER_COOKIE`.
Continuity lists, diagnostics, and events also accept `--limit <number>`;
`continuity` rejects `--limit` together with a request key.

A cookie or operator-key file must be a regular file, not a symbolic link,
with no group or other permission bits (mode `0600`), and must not be blank;
the environment-variable fallbacks must not be blank either.

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

Each mutation is a signed request that expires 30 seconds after the CLI
creates it. Literal secret flags are intentionally unsupported. Put cookies
and signing keys in owner-only files or the documented environment variables.
Follow the
[Cluster Operations](/docs/cluster-operations/) runbook before changing live
capacity or drain state.

## Proof commands

`meshc proof` exposes four separate repository-owned gates:

| Command | Purpose |
| --- | --- |
| `docker-autoscaling` | Mandatory local Docker/PostgreSQL autonomous scaling and failover proof |
| `continuity-soak` | Bounded-retention soak; release default is 24 hours |
| `autonomous-performance` | Deterministic performance budgets |
| `autonomous-chaos` | Repeated deterministic fault/model suite |

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

All three accept `--evidence-dir`; without it, evidence goes to
`target/proof/<kind>/<unix-millis>`. Performance also accepts `--iterations`
(default `10000`) and `--budget` (default
`proof/autonomous-gates/performance-budget.json`); chaos accepts `--rounds`
(default `5`, from 1 to 100). The soak accepts `--duration-seconds` (default
`86400`), `--cycle-millis` (default `100`), and `--allow-short`, which a
duration under 24 hours requires. A shortened soak is a harness smoke result,
never a 24-hour release pass.

Proof deadlines stretch on slower machines: by 3 on four cores or fewer, by 2
on five to eight, and not at all above that. Set `MESH_PROOF_TIME_SCALE` (1 to
10) to choose the factor.

## Test Runner

Run all `*.test.mpl` files from a project root, a tests directory, or a specific test file with `meshc test`:

```bash
meshc test .
meshc test tests
meshc test tests/example.test.mpl
```

The path defaults to the current directory and must lie inside a project (a
directory with `mesh.toml`). The test runner discovers all files ending in `.test.mpl` under the requested target, skipping hidden directories and `target/`, compiles and executes each independently, and prints a line per test, the failures again, and each file's count:

```
  ✓ arithmetic is correct
  ✗ string operations > length
    assert_eq failed: String.length("hello") == 4
      left:  5
      right: 4

Failures:
  ✗ string operations > length
    assert_eq failed: String.length("hello") == 4
      left:  5
      right: 4

1 failed, 1 passed in 0.00s

1 test file failed, 0 passed in 1.52s
```

Output is colored on a terminal, unless `NO_COLOR` is set.

For private implementation setup, a colocated `foo.test-support.mpl` fragment
is merged into `foo.mpl` only in the temporary test project. Tests can import
`pub` bridge helpers from `Foo`; normal builds and bindings never see the
fragment. See the [Testing guide](/docs/testing/#private-module-test-support)
for the exact naming and isolation rules.

Exit code is 1 if any test fails or a test file does not compile, making `meshc test` suitable for CI pipelines; a run that finds no test files reports `T0103` and exits 1, so a CI step pointed at the wrong directory fails. Each test failure is also a diagnostic on standard error at its line of the test file (`T0001` to `T0005` for the assertions, a panic's own `R0001`), and `meshc test --json` writes them as JSON lines there.

Use compact output for a large suite, a `.` per passing test and an `F` per failing one, with the failures listed after them:

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
formatting walks nested directories recursively and skips hidden files and
directories, such as `.git` and the packages installed under `.mesh`. Override
the default 100-column width or two-space indentation when a project needs it:

```bash
meshc fmt . --line-width 120 --indent-size 4
meshc fmt . --check --line-width 120 --indent-size 4
```

The formatter uses the **Wadler-Lindig** pretty-printing algorithm with a CST-based approach. This means:

- **Comments are preserved** -- the formatter works on the concrete syntax tree, so comments stay exactly where you put them
- **Whitespace and indentation are rewritten** canonically according to Mesh style conventions
- **Formatting is idempotent** -- running the formatter twice produces the same output as running it once

Beyond whitespace, the formatter puts `;`-separated statements on lines of
their own and may drop trailing commas. It never changes what the code means:
it refuses a file it cannot reproduce exactly, and a file with parse errors.
Either refusal (`Cannot format '<file>': ...`) fails the whole run before any
file is written.

`meshc fmt` prints `Formatted N file(s)`, counting every file it read. With
`--check`, it lists each file as `  would reformat: <path>`, prints
`N file(s) would be reformatted`, and exits with status 1 when any would
change.

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

Formatting through the language server uses the editor's tab size as the
indent and a 100-column width, and leaves a document with parse errors
unchanged.

## Linter

`meshc lint` reports code that compiles but is harder to follow than it needs to be:

```bash
meshc lint .
```

The path may be one `.mpl` file or a directory, walked recursively, and defaults to the current directory. Each finding is an `L` warning from the [error index](/docs/errors/), reported on standard error in the format `meshc check` and `meshc build` use, and the run ends with a tally. The command exits with status 1 when it reports anything, warnings included, so it can gate CI next to `meshc fmt --check`. A file that does not parse is reported with its parse errors (`P` codes) and is not linted.

```text
[L0004] Warning: comparison with the Bool literal `true`
   ╭─[ ./main.mpl:3:6 ]
   │
 3 │   if ready == true do
   │      ──────┬──────
   │            ╰──────── the same as `ready`
   │
   │ Help: Replace the comparison with `ready`.
───╯
  = L0004: meshc explain L0004 · skill diagnostics.md#l0004 · https://meshlang.dev/docs/errors/#l0004
warning: 1 warning (L0004)
```

| Code | Reports | Write instead |
|------|---------|---------------|
| [`L0001`](/docs/errors/#l0001) `deep-nesting` | `if`, `case`/`match`, `for`, `while`, `receive`, or a closure nested more than four levels deep in one function | A helper function, an early `return`, or `?` in place of a `case` that only passes an error on |
| [`L0002`](/docs/errors/#l0002) `collapsible-else-if` | An `else` whose whole body is a single `if` | `else if` |
| [`L0003`](/docs/errors/#l0003) `pass-through-arm` | An arm that returns exactly what it matched, such as `Ok(value) -> Ok(value)` | The pattern on its own: `Ok(value)` |
| [`L0004`](/docs/errors/#l0004) `bool-comparison` | A comparison with `true` or `false`, such as `ready == true` or `done != false` | The value itself, or `not` it |

Nesting is counted from each named function, service handler, `terminate`
clause, actor body, and top-level `test`, `describe`, `setup`, or `teardown`
block. An `else if` chain stays at the level of its first `if`; a trailing
`do |x| ... end` closure counts as a level. Only the outermost construct past
the limit is reported, labelled with each level it is nested in, so fixing it
may reveal a deeper one.

`pass-through-arm` fires only where the pattern alone would compile as the
arm's value: a name, a literal, or a constructor of those. A tuple or list
pattern such as `(0, b) -> (0, b)` is left alone.

`meshc lint --json` writes one JSON object per finding to standard error, in
the schema of [`--json`](#json-diagnostics) diagnostics, and nothing to
standard output. A `bool-comparison` finding carries a machine-applicable edit
that replaces the comparison, which the language server offers as a quick fix.
The language server reports the same findings as `meshc lint` while you edit.
Like the formatter, the linter skips hidden files and directories. It has no
options, configuration file, or suppression comments yet.

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
`String` (quoted, as in `"ab" :: String`), and `Unit` render as values, while
other pointer-backed values such as `Bytes`, collections, and structs render as
`<Type at 0x...>` placeholders. Use ordinary functions such as `println`,
`inspect`, `Bytes.to_hex`, or a package-specific renderer when inspecting those
values.

Input that starts with `fn`, `def`, `let`, `type`, `struct`, `module`, `actor`,
`service`, `interface`, `trait`, `impl`, or `supervisor` is kept as a
definition for the rest of the session; anything else is evaluated.

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
| `:load <file>` | | Type-check a Mesh source file and keep its definitions; nothing in it runs |

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

Input history is loaded from and saved to `~/.mesh_repl_history` when `HOME`
is set. `Ctrl-C` cancels the current input without exiting; `Ctrl-D`,
`:quit`, and `:q` exit.

## meshpkg — Package Registry CLI

The `meshpkg` binary provides commands for publishing and consuming packages from the Mesh package registry.

### Authentication

Open [the package publishing page](https://packages.meshlang.dev/publish), sign
in with GitHub, and save the generated token:

```bash
meshpkg login --token <your-token>
```

Without `--token`, `meshpkg login` prompts on standard input. The token is
stored in `~/.mesh/credentials` (`[registry]` / `token = "..."`), readable by
its owner only. `meshpkg publish` sends it as a bearer token to whichever
registry it publishes to, so point `--registry` only at a registry you trust.

Every command talks to `https://api.packages.meshlang.dev` unless given
`--registry <url>`.

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
modules and an override entrypoint. Hidden paths, `*.test.mpl` files, and
`*.test-support.mpl` fragments are excluded. The archive includes `.law` files
selected by [law discovery](#choose-which-laws-to-check), plus all explicitly
configured law roots. Manifest-declared native bindings and static libraries
are included and their hashes are verified before upload.
`README.md` is not currently included by `meshpkg publish`.

Target another compatible registry with `--registry`:

```bash
meshpkg publish --registry https://registry.example.com
```

### Installing a Package

Install the latest release of a package from the registry into the current project:

```bash
meshpkg install your-login/your-package
```

This fetches the latest published release, verifies its SHA-256 checksum, extracts it into `.mesh/packages/<name>@<version>/`, and updates mesh.lock to pin the exact version. Named install does not edit mesh.toml; add the dependency yourself when you want it declared in the manifest. A named install always takes the latest release; to use another version, declare it in `mesh.toml` and run `meshpkg install` without a name.

Omit the name to install every exact registry dependency already declared in
`mesh.toml`:

```bash
meshpkg install
```

A dependency whose `mesh.lock` entry pins the declared version is downloaded
at that pin and checked against its recorded checksum; after you change the
version in `mesh.toml`, the next install resolves the new version and updates
the pin. Installing a version removes any other installed version of the same
package.

`meshc build` compiles every package installed under `.mesh/packages`, whether
or not `mesh.toml` declares it, so a named install is importable at once.
Registry dependencies support exact versions only. `meshpkg install` leaves
git and path dependencies to `meshc deps`.

### Searching

Search the registry by name or keyword:

```bash
meshpkg search json
```

Prints a `NAME`, `VERSION`, `DESCRIPTION` table of the matching packages.

Search, install, and publish accept `--registry <url>`. Every meshpkg command
except `update` accepts the global `--json` flag:

```bash
meshpkg --json search json
meshpkg --json install
```

| Command | JSON on standard output |
| --- | --- |
| `search` | `[{"name", "version", "description"}]` |
| `install` | `{"status": "ok", "lockfile": "mesh.lock"}` |
| `install <name>` | `{"status": "ok", "name", "version", "lockfile": "mesh.lock", "manifest_changed": false}` |
| `publish` | `{"status": "ok", "name", "version", "sha256"}` |
| `login` | `{"status": "ok", "message"}` |

A failure prints `{"error": "..."}` on standard error and exits with status 1.
`meshpkg update` refuses `--json`, because it hands over to the installer.

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
| **Diagnostics** | Project, syntax, type, and lint diagnostics as you edit |
| **Quick fixes** | Code actions that apply a diagnostic's suggested edits |
| **Hover** | Hover over identifiers to see inferred type information |
| **Go-to-definition** | Jump to definitions in the current document |
| **Completion** | Keywords, built-in types, snippets, and names visible in the current scope |
| **Document symbols** | Functions, types, and other declarations for editor outline/symbol views |
| **Document formatting** | Format the current document through the same formatter used by `meshc fmt` |
| **Signature help** | Parameter hints for function calls, including active-parameter tracking |

The language server receives full-document changes and runs the same
`frontend::check` as `meshc check`. It also runs `mesh-lint` on files that
parse. It checks a file under `mesh.toml` in its project context, including
imports from path and git dependencies and installed packages. It uses the
unsaved text of other open documents. Files outside a project are checked
individually.

The server publishes each diagnostic to the file it concerns. Editing one
file can refresh or clear diagnostics in other files. The diagnostic
message includes `help:` for the repair and `note:` for each note. Its
`code_description` links to the catalog entry, and `related_information`
points to secondary labels, including labels in other files.

For diagnostics with applicable source edits, the server offers a
`QuickFix` code action that applies the edits together. Editors can mark
the action as preferred when all edits are `machine-applicable`.

Hover shows the inferred type of the expression under the cursor. Completion
has no trigger characters, so request it explicitly or let the editor ask as
you type; signature help triggers on `(` and `,`. Document symbols cover
functions, structs, sum types, type aliases, modules, actors, services, their
call and cast handlers, supervisors, interfaces, impls, and top-level `let`
bindings. References, rename, workspace symbols, and semantic
tokens are not implemented.

`meshc lsp` takes no arguments and always speaks over stdio; configure clients
not to pass `--stdio`.

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

### Syntax-highlighting model

VS Code and this documentation site use the same repo-owned TextMate grammar:
`tools/editors/vscode-mesh/syntaxes/mesh.tmLanguage.json`. VitePress loads that
grammar into Shiki, so fenced `mesh` examples and VS Code are checked against
the same scopes. Neovim does not consume that file; its repo-owned support pack
uses a separate classic Vim grammar at
`tools/editors/neovim-mesh/syntax/mesh.vim`.

Both editor grammars cover the current audited language surface:

- every compiler keyword and every visible operator, delimiter, and punctuation
  token, with regression probes derived from
  `compiler/mesh-common/src/token.rs`; the significant `Newline` punctuation
  token is tracked as vocabulary but has no visible glyph to scope
- declarations, imports and multi-segment module paths, calls, Unicode
  identifiers, current built-in types, and constructors
- `@cluster`, `@cluster(N)`, and `@native(...)`
- ORM schema clauses and relationships, supervisor clauses and values, maps and
  struct updates, result and optional types, patterns and wildcards, pipes and
  slot pipes, atoms, single- and physical-multiline regular expressions, nested
  block comments, and string interpolation

Qualified calls are recognized by their structure, not by an exhaustive list
of module or method names. This gives calls such as `Query.where(...)` and
`Repo.one(...)` appropriate module/function scopes and also accommodates the
database, web, concurrency, distributed-runtime, numeric, binary, and
multi-segment official or user package namespaces as their APIs evolve.

These are lexical grammars. `meshc lsp` does not currently advertise a
semantic-tokens provider, so syntax colors do not resolve whether a name is a
compiler built-in, an official package, a user module, or another symbol with
the same spelling. Mesh also does not currently ship a Tree-sitter grammar.
Diagnostics, hover, navigation, completion, symbols, formatting, and signature
help remain the separate LSP capabilities listed above.

An unterminated multiline token such as `~r/...` remains open to the end of the
document in these lexical grammars; `meshc lsp` supplies the corresponding
compiler diagnostic.

Declaration-name scoping is deliberately conservative where item and expression
contexts have the same token shape. Private parameterless forms beginning
`fn name do`, `fn name when ...`, or `fn name -> ...` keep the name's ordinary
identifier scope, while uppercase `fn Name(...)` forms that overlap
constructor-pattern closures retain non-declaration constructor/type scopes.
Their keywords, guards, annotations, types,
operators, and bodies are still highlighted, and the compiler/LSP parse them
normally. Public functions, `def` declarations, interface methods,
conventional lowercase private `fn name(...)` declarations, generic
declarations, and direct `fn name = ...` declarations receive declaration-name
scopes.

### VS Code

VS Code is a first-class editor host in the public Mesh tooling contract. The
official Mesh extension provides syntax highlighting, diagnostics, hover,
same-file go-to-definition, completion, document symbols, document formatting,
and signature help. Its shared grammar provides the TextMate side of the
syntax-highlighting model described above.

#### VS Code features

- **Syntax highlighting** via the shared TextMate grammar used by VS Code and the docs, including the compiler-derived token vocabulary and the current core, ORM, supervisor, native-package, and namespaced-module forms
- **Language configuration** for line and nested-block comment commands, bracket matching, auto-closing pairs, and automatic indentation and folding of multiline `do`/`end` blocks
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

Neovim is a first-class editor host in the public Mesh tooling contract for the audited classic syntax plus native `meshc lsp` path already proven in `scripts/verify-m036-s02.sh`. That audited syntax now covers the current structural language surface described above. Its separate classic Vim grammar is not the shared TextMate grammar used by VS Code and the docs. The repo-owned support pack lives in `tools/editors/neovim-mesh/` and requires **Neovim 0.11+**.

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

That proof exercises the interpolation/decorator corpus, the current language-surface fixture, compiler-derived keyword/operator/delimiter/visible-punctuation probes, and the native `meshc lsp` path. It does not imply semantic-token or Tree-sitter support, or support for third-party Neovim plugin-manager packaging.

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
| Compiler | `meshc build <dir>` | Compile a project to a native executable, or with `--artifact` to a static or dynamic library |
| Plaintext report | `meshc plaintext-report [dir] [--output <file> \| --check <file>]` | List every `declassify` and `@display` export; `--check` fails on an unreviewed change |
| Project scaffolding | `meshc init [--clustered \| --template todo-api --db <backend>] <name>` | Create hello-world, clustered, SQLite Todo, or PostgreSQL Todo projects |
| Source dependencies | `meshc deps [dir]` | Resolve git/path dependencies, fetch git checkouts, and write lock entries |
| Registry dependencies | `meshpkg install [name]` | Install all declared exact registry dependencies or one latest named package |
| Migrations | `meshc migrate [dir] [up \| down \| status \| generate]` | Generate and run PostgreSQL migrations |
| Formatter | `meshc fmt <path>` | Recursively format Mesh source or use `--check` in CI |
| Frontend check | `meshc check [path] [--tests]` | Parse and type-check a project or file without building it |
| Diagnostic catalog | `meshc explain [CODE] [--json]` | Look up repairs and examples |
| Linter | `meshc lint [path]` | Report deep nesting and other code that reads worse than it needs to; fails when it finds anything |
| Test Runner | `meshc test [path]` | Run `*.test.mpl` files from a project root, tests directory, or specific test file |
| REPL | `meshc repl` | Interactive LLVM JIT evaluation |
| Language Server | `meshc lsp` | Diagnostics, hover, navigation, completion, symbols, formatting, signature help, and quick fixes over stdio JSON-RPC |
| Cluster operations | `meshc cluster <command>` | Inspect or mutate runtime-owned cluster state |
| Release gates | `meshc proof <command>` | Run Docker, chaos, performance, and continuity proof commands |
| Toolchain update | `meshc update` or `meshpkg update` | Refresh both installed commands |
| Offline guide | `meshc guide [topic]` | Read guidance from the running compiler |
| Coding tool setup | `meshc skills setup` | Choose hosts and install the Mesh skill |
| Skill refresh | `meshc skills refresh` | Update eligible account copies of the Mesh skill |
| Policy verification | `meshc verify [dir]` | List or verify pure policy laws |
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
