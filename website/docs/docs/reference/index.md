---
title: Complete Reference
description: "Inventory of the implemented Mesh 14 language, runtime, standard-library, package, and toolchain surfaces."
---

# Complete Reference

This page is the index of features implemented on the current Mesh 14 branch.
It distinguishes supported syntax from reserved words, source-level APIs from
runtime-only symbols, and built-in modules from optional packages.

For examples and explanation, follow the linked guides. Use this page when you
need to answer “does Mesh support this?” or find the owning reference.

## Source files and modules

Mesh source files use the `.mpl` extension. `main.mpl` is the default
executable entrypoint; `[package].entrypoint` can select another source file.

```mesh
module Billing do
  pub fn total(items :: List<Int>) -> Int do
    List.reduce(items, 0, fn acc, item -> acc + item end)
  end
end
```

| Form | Meaning |
| --- | --- |
| `module Name do ... end` | Declare a module |
| `pub module Name do ... end` | Export a module |
| `import Foo.Bar` | Import a module for qualified access |
| `from Foo.Bar import one, two` | Import selected public symbols |
| `pub` | Export a function, module, struct, interface, supervisor, sum type, or alias |

Selective imports may be parenthesized. Only the parenthesized form may span
lines; an unparenthesized import list ends at the newline. Glob imports are not
supported.

## Comments, literals, and separators

| Surface | Syntax and behavior |
| --- | --- |
| Line comment | `# comment` |
| Documentation comment | `## item documentation` |
| Module documentation | `##! module documentation` |
| Nested block comment | `#= outer #= inner =# outer =#` |
| Statements | Significant newlines or `;` |
| Identifier | Starts with `_` or a Unicode alphabetic code point; later characters may be `_` or a Unicode alphanumeric code point |
| Integer | Decimal, `0x` hexadecimal, `0b` binary, or `0o` octal; `_` separators are accepted |
| Float | Decimal and scientific notation |
| Boolean | `true`, `false` |
| Unit | `nil` or `()` |
| String | `"text"`, with escapes and interpolation |
| Heredoc | `"""multiline text"""` |
| Atom | `:name`, using lowercase letters, digits, and underscores |
| Regex | `~r/pattern/ims`; the pattern may span physical source lines until an unescaped `/`; supported flags are `i`, `m`, and `s` |
| List | `[one, two]` |
| Tuple | `(one, two)` |
| Map | `%{key => value}` |
| JSON object | `json { bare_key: expression }` |
| Struct | `Point { x: 1, y: 2 }` |
| Struct update | `%{point | x: 3}` |

Both `"#{expression}"` and `"${expression}"` interpolate. The `#{...}` form is
preferred in new code.

Reserved keywords are exact ASCII identifiers: `let` is a keyword, but `letπ`
is a valid ordinary identifier.

`json { ... }` has static type `Json`. It is implicitly compatible with
`String` at APIs that consume encoded JSON, but structured access remains
available through `Json.parse`, `Json.object_get`, `Json.array_get`, and the
typed scalar accessors.

## Core types

| Type | Notes |
| --- | --- |
| `Int` | Signed runtime integer used by ordinary numeric literals and operators |
| `Float` | Floating-point value |
| `Bool` | Boolean value |
| `String` | UTF-8 text |
| `Bytes` | Binary-safe byte sequence |
| `U64`, `U128`, `I128` | Opaque checked wide integers; use their module functions |
| `Json` | Structured JSON value with implicit String compatibility |
| `Atom` | Named atom such as `:ok` |
| `Regex` | Compiled regular expression |
| `()` | Unit |
| `Option<T>` / `T?` | `Some(T)` or `None` |
| `Result<T, E>` / `T!E` | `Ok(T)` or `Err(E)` |
| `List<T>` | Immutable list |
| `Map<K, V>` | Immutable map |
| `Set` | Immutable set of `Int` values |
| `Queue` | Immutable queue of `Int` values |
| `Range` | End-exclusive integer range |
| `(A, B)` | Tuple type |
| `Fun(A, B) -> R` | Function type |
| `Pid<M>` | Process identifier whose mailbox accepts `M` |
| `Pid` | Untyped process-identifier escape hatch |

Additional opaque types are introduced by their modules, including database
connections, pools, HTTP values, WebSocket messages, dates, jobs, iterators,
and cluster bootstrap records.

The built-in algebraic types also include `Ordering`, whose constructors are
`Less`, `Equal`, and `Greater`.

## Bindings and functions

```mesh
let inferred = 42
let explicit :: Int = 42

fn identity<T>(value :: T) -> T do
  value
end

fn join_display<A, B>(left :: A, right :: B) -> String
where A: Display, B: Display do
  left.to_string() <> right.to_string()
end
```

| Form | Support |
| --- | --- |
| `let name = expression` | Immutable inferred binding |
| `let name :: Type = expression` | Annotated binding |
| `let (a, b) = pair` | Tuple destructuring binding |
| `fn name(...) ...` | Named function |
| `def name(...) ...` | Synonym for `fn` |
| `fn name(...) = expression` | One-expression function |
| Multiple same-name clauses | Pattern and guard dispatch; clauses must be consecutive |
| Different arities | Supported as separate overloads |
| `<T, U>` | Explicit generic parameters |
| `where T: Trait` | Generic interface bound; multiple bounds are comma-separated |
| `when guard` | Clause guard |
| `return expression` | Explicit early return |
| `return` | Early Unit return |

The final expression of a function or block is its value. Named functions are
registered before their bodies are inferred, so mutual recursion is
supported. A non-exhaustive function-clause group warns; a non-exhaustive
`case` or `match` is an error.

Function parameters may be patterns. The first clause owns the public,
generic, return-type, and `where` metadata for a same-name/arity clause group.
Put a catch-all clause last.

Direct calls to the current function in tail position are lowered to a loop,
including tail positions reached through blocks, `let` continuations,
`if` branches, `case`/`match` arms, explicit `return`, and actor `receive`
arms or timeouts. This does not eliminate mutual recursion, calls wrapped in
another operation, or any other non-tail call.

## Closures and calls

Supported closure forms include:

```mesh
fn(x, y) -> x + y end
fn x, y -> x + y end
fn -> 42 end
fn do
  let value = compute()
  value
end
fn value do
  transform(value)
end
fn 0 -> "zero" | n when n > 0 -> "positive" | _ -> "negative" end
```

Closures capture lexical bindings. Calls support:

- positional arguments;
- keyword arguments, which become one final map argument;
- trailing closures;
- field and method chaining;
- postfix `?` on `Option` and `Result`.

Positional arguments cannot follow keyword arguments.

## Pattern matching

`case` and `match` are synonyms:

```mesh
match value do
  Some(head :: tail) when head > 0 -> do
    use(head, tail)
  end
  None -> "missing"
  Some(items) -> inspect(items)
end
```

Supported patterns are:

- wildcard `_`;
- variable binding;
- integer, float, string, boolean, and `nil` literals;
- negative numeric literals;
- tuple patterns;
- qualified and unqualified constructors;
- constructor payload destructuring;
- cons patterns such as `head :: tail`;
- or-patterns such as `one | two`;
- alias patterns such as `pattern as whole`;
- optional `when` guards on function, closure, receive, and match arms.

Both sides of an or-pattern must bind the same names. List-literal patterns
such as `[a, b]` and struct-field patterns are not currently supported; use
cons and constructor patterns.

## Control flow

| Form | Result |
| --- | --- |
| `if condition do ... else ... end` | Unified branch type |
| `case value do ... end` | Unified arm type; exhaustiveness checked |
| `match value do ... end` | Synonym for `case` |
| `for value in iterable [when guard] do ... end` | `List<body type>` |
| `while condition do ... end` | Unit |
| `break` | Exit the enclosing loop |
| `continue` | Continue the enclosing loop |

`for` supports ranges, lists, maps, sets, and user implementations of
`Iterable`/`Iterator`. Ranges are end-exclusive: `0..5` yields `0` through `4`.
Map destructuring uses `{key, value}`.

An `if` without `else` is best used only when its result is discarded.

## Pipes and operators

Operators are listed from lower to higher precedence:

| Precedence group | Operators |
| --- | --- |
| Pipe | `\|>`, `\|N>` |
| Boolean OR | `or`, `\|\|` |
| Boolean AND | `and`, `&&` |
| Equality | `==`, `!=` |
| Ordering | `<`, `>`, `<=`, `>=` |
| Range | `..` |
| Concatenation | `<>`, `++` |
| Additive | `+`, `-` |
| Multiplicative | `*`, `/`, `%` |
| Prefix | `-`, `not`, `!` |
| Postfix | call, field, `?` |

`|>` supplies the left value as the first argument. `|N>` supplies it as
argument position N; N begins at 2. Leading and trailing multiline pipe forms
are both supported.

Arithmetic and comparisons dispatch through built-in interfaces. `<>` is the
String concatenation form and `++` is the List concatenation form.

## Structs, sum types, and aliases

```mesh
pub struct Box<T> do
  value :: T
end deriving(Eq, Debug, Json)

pub type Either<A, B> do
  Left(A)
  Right(value :: B)
end deriving(Eq, Debug, Json)

pub type Pair<A, B> = (A, B)
```

- Structs are product types with named fields.
- Sum types may have nullary, positional, or named-payload variants.
- Aliases are transparent and may be generic.
- Struct and sum constructors can be used qualified or unqualified.
- ORM schema metadata may be declared inside structs; see
  [Databases](/docs/databases/).

## Interfaces and implementations

```mesh
pub interface Container<T> do
  type Item
  fn first(self) -> Self.Item
  fn label(self) -> String do
    "container"
  end
end

struct IntBox do
  value :: Int
end

impl Container<Int> for IntBox do
  type Item = Int
  fn first(self) -> Int do
    self.value
  end
end
```

Interfaces support:

- generic parameters;
- required methods;
- methods with default bodies;
- instance and static methods;
- associated types;
- generic `where` constraints.

Implementations must provide required methods and associated types with
matching signatures. Extra associated types, duplicate structural
implementations, and ambiguous method resolution are rejected.

The declaration keyword is `interface`; `trait` is reserved but is not a
supported declaration form.

## Built-in interfaces

| Group | Interfaces |
| --- | --- |
| Arithmetic | `Add`, `Sub`, `Mul`, `Div`, `Mod`, each with `Output`; `Neg` with `Output` |
| Comparison and logic | `Eq`, `Ord`, `Not` |
| Presentation and identity | `Display`, `Debug`, `Hash`, `Default` |
| Iteration | `Iterator` with `Item`; `Iterable` with `Item` and `Iter` |
| Conversion | `From`, `Into`, `TryFrom`, `TryInto` |
| Generated serialization | `ToJson`, `FromJson`, `FromRow`, and schema metadata behind derives |

Implementing `From<Source>` synthesizes the matching `Into<Target>`.
Implementing `TryFrom<Source>` synthesizes `TryInto<Target>`. Result
propagation may use `From` to convert an error into the enclosing function's
error type.

## Deriving

An explicit `deriving(...)` clause is selective, and `deriving()` selects
nothing. For backward compatibility, omitting the clause derives
`Debug`, `Eq`, `Ord`, and `Hash` for a struct, and `Debug`, `Eq`, and `Ord`
for a sum type. `Display`, `Json`, `Row`, and `Schema` are never enabled by
omission.

| Type kind | Supported derives | Constraints |
| --- | --- | --- |
| Struct | `Eq`, `Ord`, `Display`, `Debug`, `Hash`, `Json`, `Row`, `Schema` | `Ord` requires `Eq`; `Row` and `Schema` are struct-only |
| Sum type | `Eq`, `Ord`, `Display`, `Debug`, `Hash`, `Json` | `Ord` requires `Eq`; `Schema` is rejected |

`Json` generates encoding and decoding. `Row` generates database row
conversion for supported scalar/optional fields. `Schema` generates ORM schema
metadata. See [Type System](/docs/type-system/#deriving) for field rules.

## Errors and propagation

```mesh
fn load() -> Account ! AppError do
  let raw = File.read("account.json")?
  let value = Json.parse(raw)?
  decode_account(value)
end
```

- `T?` is shorthand for `Option<T>`.
- `T!E` is shorthand for `Result<T, E>`.
- `?` unwraps `Some`/`Ok`.
- `?` returns `None`/`Err` early.
- A `Result` error may be converted through `From`.
- Use `case`/`match` for explicit handling.

## Actors and concurrent processes

| Surface | Purpose |
| --- | --- |
| `actor name(args) do ... end` | Define an actor |
| `spawn(function, args...)` | Start an actor and return `Pid<M>` |
| `send(pid, message)` | Type-check and enqueue a message, returning an observable `Int` status |
| `receive do ... end` | Wait for the next mailbox message |
| `after timeout -> ...` | Receive timeout arm |
| `self()` | Current actor PID; actor context only |
| `link(pid)` | Bidirectional failure link |
| `terminate do ... end` | One actor cleanup callback |
| `Process.monitor` / `Process.demonitor` | One-way process observation; actor context required (`0`/`1` failure sentinels) |
| `service` | Stateful call/cast actor with generated client functions |
| `supervisor` | Child restart tree |
| `Job` | Short-lived async result |
| `Timer` | Sleep and delayed message delivery |
| `Channel` | Bounded, nonblocking integer delivery |

Actor definitions accept arguments. Although the grammar accepts several
receive arms with patterns and guards, current native code generation executes
only the first arm. Use one variable or wildcard receive arm and dispatch with
`case` inside it. See [Concurrency](/docs/concurrency/) for service syntax,
supervisor strategies, channel policies, jobs, registries, and graceful
shutdown.

## Cluster declarations

```mesh
@cluster
pub fn read_model() -> String do
  "ready"
end

@cluster(3)
pub fn replicated_read() -> String do
  "ready"
end
```

`@cluster` and `@cluster(N)` may decorate `fn` or `def`. The target must resolve
to one uniquely named public work function. The omitted replication count is
currently 2.

`HTTP.clustered(...)` adapts declared work to an HTTP route. It does not make
an arbitrary private closure remotely executable. Runtime bootstrap,
continuity, identity, admission, routing, and capacity policy are documented
under [Distributed Actors](/docs/distributed/) and
[Autonomous Clusters](/docs/autonomous-clusters/).

The removed `clustered(work)` declaration form is rejected; use the decorator.

## Native declarations

```mesh
@native("mesh_math_add")
pub fn add(left :: Int, right :: Int) -> Int
```

A native declaration must be:

- in a manifest-listed bindings file;
- `pub`;
- bodyless;
- fully annotated, including an explicit return type;
- concrete, without generic parameters, `where`, or guards;
- bound to one literal C identifier.

ABI 1 value types are `Int`, `Float`, `Bool`, `String`, `Bytes`, `U64`,
`U128`, and `I128`. A return may additionally be `Option<T>` or
`Result<T, E>` when every payload is an ABI value. User structs, collections,
tuples, closures, actors, and generic native functions do not have ABI 1
layouts.

See [Native Packages](/docs/native-packages/) for manifests, archive
verification, ownership, errors, and target selection.

## Standard-library module index

This table lists every compiler-recognized built-in module on the current
branch.

| Area | Modules | Detailed guide |
| --- | --- | --- |
| Text and collections | `String`, `List`, `Map`, `Set`, `Tuple`, `Range`, `Queue`, `Iter`, `Regex` | [Standard Library](/docs/stdlib/), [Iterators](/docs/iterators/) |
| Binary and numbers | `Bytes`, `U64`, `U128`, `I128`, `Checked`, `Math`, `Int`, `Float` | [Standard Library](/docs/stdlib/) |
| Encoding and JSON | `JSON`, `Json`, `Base64`, `Hex` | [Web](/docs/web/), [Standard Library](/docs/stdlib/) |
| System and time | `IO`, `Env`, `File`, `DateTime`, `Monotonic`, `Duration`, `Random`, `Crypto` | [Standard Library](/docs/stdlib/) |
| Concurrent runtime | `Job`, `Timer`, `Channel`, `Process`, `Test` | [Concurrency](/docs/concurrency/), [Testing](/docs/testing/) |
| Web and sockets | `HTTP`, `Request`, `Ws`, `Http`, `WsClient` | [Web](/docs/web/) |
| Databases | `Sqlite`, `Pg`, `Pool`, `Orm`, `Expr`, `Query`, `Repo`, `Changeset`, `Migration` | [Databases](/docs/databases/) |
| Distribution | `Node`, `Global`, `Continuity`, `Cluster` | [Distributed Actors](/docs/distributed/), [Autonomous Clusters](/docs/autonomous-clusters/) |

`JSON` and `Json` address the same module family; `Json` is the conventional
name in new examples.

## Official packages

| Package | Current scope |
| --- | --- |
| `mesh-borsh` | Bounded Borsh readers and writers backed by a native archive |
| `mesh-anchor` | Pure-Mesh Anchor discriminator, owner, and versioned-layout validation |
| `mesh-solana` 0.2 | Typed RPC, account decoders, subscriptions, SPL/Jito helpers, instruction inspection, legacy/v0 unsigned messages, and bounded unsigned simulation |

See [Packages and Registry](/docs/packages/) for public functions, installation,
provenance, and explicit non-goals.

## Toolchain index

| Command | Purpose |
| --- | --- |
| `meshc build` | Compile and link a project |
| `meshc init` | Generate hello, clustered, or Todo API starters |
| `meshc deps` | Resolve source git/path dependencies |
| `meshc fmt` | Format or check `.mpl` files |
| `meshc test` | Run `.test.mpl` tests |
| `meshc repl` | Start the LLVM-backed REPL |
| `meshc lsp` | Run the language server over stdio |
| `meshc migrate` | Generate, inspect, apply, or roll back migrations |
| `meshc cluster` | Inspect or control a cluster |
| `meshc proof` | Run repository-owned proof scenarios |
| `meshc update` | Refresh an installer-managed toolchain |
| `meshpkg login` | Store a registry token |
| `meshpkg search` | Search the registry |
| `meshpkg install` | Install one exact package or manifest registry dependencies |
| `meshpkg publish` | Publish an immutable package version |
| `meshpkg update` | Refresh the toolchain |

See [Developer Tools](/docs/tooling/) for arguments, output paths, supported
targets, editor integration, and limitations.

## Reserved words that are not features

The lexer reserves `alias`, `cond`, `trait`, `trap`, and `with`, but the parser
does not implement those forms on the current branch. Do not use them as
language features. Use:

- `import` or `from ... import` instead of `alias`;
- `if`, `case`, or `match` instead of `cond`;
- `interface` instead of `trait`;
- `Result`, supervision, links, and monitors for the corresponding error and
  process flows.

`monitor` is available through qualified APIs such as `Process.monitor` and
`Node.monitor`; it is not a bare monitor expression.

## Current intentional limits

- Variables and collections are immutable.
- `Iter.from` currently accepts `List<T>`; `for ... in` has the wider
  iterable surface.
- Index expressions are parser placeholders and are not an executable
  source-level collection API; use module functions such as `List.get`,
  `Map.get`, and `Json.array_get`.
- List-literal and struct-field patterns are not implemented.
- Native `receive` currently dispatches only its first arm; branch on the
  received value with `case`.
- Wide integers use checked module functions instead of ordinary literal
  operators.
- `Channel` payloads are currently `Int`.
- `Random` is deterministic, not cryptographic.
- Inbound WebSocket TLS is not exposed at the Mesh source API.
- SQLite is a local/single-node application database.
- Native packages supply prebuilt, exact-target static archives.
- `meshc test --coverage` is explicitly unsupported.
- `mesh-solana` does not sign or submit transactions.

These are boundaries of the current public surface, not implied future
commitments.
