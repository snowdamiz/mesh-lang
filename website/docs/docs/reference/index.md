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
| `foo/bar_baz.mpl` | A file module named `Foo.BarBaz`: each path segment in PascalCase |
| `module Name do ... end` | Declare a module named exactly `Name` inside a file; import it to use it, from that file only |
| `pub module Name do ... end` | Declare a module any file may import |
| `import Foo.Bar` | Import a module for qualified access through its last segment: `Bar.run()` |
| `from Foo.Bar import one, two` | Import selected public symbols |
| `pub` | Export a function, module, struct, interface, supervisor, sum type, alias, or resource |

Standard-library modules need no import. A module block may not share its name
with a file module. Qualified names work in calls, type annotations
(`p :: Geo.Point`), struct literals (`Geo.Point { x: 1, y: 2 }`), constructors
and constructor patterns (`Shapes.Circle(r)`), and `impl` headers
(`impl Geo.Describe for Geo.Point`).

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
| Statements | A newline ends a statement; two statements on one line need `;` between them |
| Identifier | Starts with `_` or a Unicode alphabetic code point; later characters may be `_` or a Unicode alphanumeric code point |
| Integer | Decimal, `0x` hexadecimal, `0b` binary, or `0o` octal (prefixes may be uppercase); `_` separators are accepted; at most `9223372036854775807` in any radix, and `-9223372036854775808` may be written |
| Float | Decimal and scientific notation; any literal with an exponent, such as `1e3`, is a `Float` |
| Boolean | `true`, `false` |
| Unit | `nil` or `()` |
| String | `"text"`, with interpolation and the escapes `\n` `\t` `\r` `\0` `\\` `\"`, `\$` and `\#` (a literal `${` or `#{`), and `\u{1F389}` (1 to 6 hex digits); any other escape is an error |
| Heredoc | `"""multiline text"""`; dedented to the closing `"""`, with escapes and interpolation as in strings |
| Atom | `:name`; the first character after `:` is a lowercase ASCII letter or `_`, later ones may be `_` or any Unicode letter or digit |
| Regex | `~r/pattern/ims`; the pattern may span physical source lines until an unescaped `/`; supported flags are `i`, `m`, and `s` |
| List | `[one, two]` |
| Tuple | `(one, two)` |
| Map | `%{key => value}` |
| JSON object | `json { bare_key: expression }` |
| Struct | `Point { x: 1, y: 2 }` |
| Struct update | `%{point | x: 3}` |

Both `"#{expression}"` and `"${expression}"` interpolate. The `#{...}` form is
preferred in new code.

The compiler rejects malformed numeric literals, a digit outside the radix
(`0b102`), an integer literal beyond the `Int` range (`0xffffffffffffffff`),
and a float literal that overflows (`1e999`).

A heredoc drops the newline after its opening `"""` and the final line holding
only the closing indentation, removes that indentation from every line, and
turns source `\r\n` line endings into `\n`. In a run of more than three quotes,
the last three close it: `"""say "hi""""` is `say "hi"`. A heredoc may also be
a pattern. See [Heredoc Strings](/docs/language-basics/#heredoc-strings).

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
| `Atom` | Named atom such as `:ok`; has `Eq`, `Display`, `Debug`, and `Hash`, and matches as a literal pattern |
| `Regex` | Compiled regular expression |
| `()` | Unit |
| `Option<T>` / `T?` | `Some(T)` or `None`; a tuple takes the shorthand too: `(Int, String)?` |
| `Result<T, E>` / `T!E` | `Ok(T)` or `Err(E)`; for example `(Int, Int)!String` |
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

fn join_display<A, B>(left :: A, right :: B) -> String where A: Display, B: Display do
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

Function parameters may be patterns: literals, constructors, tuples, lists
(`len([])`), cons (`len(_ :: rest)`), and or-patterns (`small(1 | 2)`). In a
parameter, `name :: Type` is an annotation; a lowercase name, `_`, or a list
pattern after `::` makes a cons pattern. A clause may use `= expression` or a
`do ... end` body. The first clause owns the public, generic, return-type, and
`where` metadata for a same-name/arity clause group. Put a catch-all clause
last. A function clause's `when` guard may be any `Bool` expression.

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
- a trailing `do |x| ... end` closure after the argument list, which becomes
  the last argument of a call, method call, or pipe step;
- field and method chaining;
- postfix `?` on `Option` and `Result`.

Positional arguments cannot follow keyword arguments. The heads of `if`,
`while`, `case`, and `for` do not take a trailing closure, so their `do` always
opens the body.

On a `String`, `List`, `Map`, `Set`, or `Range` value, a method call falls back
to that module's function: `xs.contains(x)` is `List.contains(xs, x)`,
`m.put(k, v)` is `Map.put(m, k, v)`, and `"s".length()` is `String.length("s")`.
An interface method of the same name is preferred.

## Built-in functions

These functions need no module prefix and no import:

| Function | Behavior |
| --- | --- |
| `println(text)`, `print(text)` | Write a `String` to standard output, with or without a newline; interpolate other values |
| `panic(message)` | `String -> Never`; end the current actor, or the program from `main` |
| `default()` | The `Default` value of the type the context expects |
| `compare(a, b)` | An `Ordering` (`Less`, `Equal`, or `Greater`) through `Ord` |
| `map`, `filter`, `reduce`, `head`, `tail` | The `List` functions of the same names |
| `spawn`, `send`, `self`, `link` | Actor operations; see [Actors](#actors-and-concurrent-processes) |
| `test`, `describe`, `setup`, `teardown`, `assert`, `assert_eq`, `assert_ne`, `assert_raises` | The test DSL; see [Testing](/docs/testing/) |

`inspect(value)` and `to_string(value)` call the `Debug` and `Display`
methods as functions, the same as `value.inspect()` and `value.to_string()`.

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
- integer, float, string, atom, boolean, and `nil` literals;
- negative numeric literals;
- tuple patterns;
- qualified and unqualified constructors;
- constructor payload destructuring;
- cons patterns such as `head :: tail`;
- list patterns such as `[]` and `[a, b]`, which match exactly that length;
- or-patterns such as `one | two`;
- alias patterns such as `pattern as whole`;
- optional `when` guards on function, closure, receive, and match arms.

Both sides of an or-pattern must bind the same names. Struct-field patterns are
not currently supported; use tuple and constructor patterns. A heredoc literal
is also a pattern.

An arm body is one expression, a `-> do ... end` block, or statements starting
on the next line and indented under the arm. `return` is an expression, so
`None -> return -1` leaves the function.

A guard on a `case`, `match`, or `receive` arm, a function clause, or a
multi-clause closure may be any `Bool` expression.

A `case` or `match` arm with no `->` is its pattern alone and passes the
matched value through, rebuilt: `Ok(value)` means `Ok(value) -> Ok(value)`.
The rebuilt value takes the type of the whole `case`, so another arm may change
the rest of it, as `Err(e) -> Err(wrap(e))` changes the error type. The pattern
must name a whole value: names, literals, constructors, and nullary
constructors such as `None`; `_` and a bare `Ok` are rejected (E0056).

## Control flow

| Form | Result |
| --- | --- |
| `if condition do ... else ... end` | Unified branch type |
| `if a do ... else if b do ... else ... end` | `else if` chain closed by one `end` |
| `if condition do ... end` | `()` |
| `case value do ... end` | Unified arm type; exhaustiveness checked |
| `match value do ... end` | Synonym for `case` |
| `for value in iterable [when guard] do ... end` | `List<body type>` |
| `while condition do ... end` | Unit |
| `break` | Exit the enclosing loop |
| `continue` | Continue the enclosing loop |

`for` supports ranges, lists, maps, sets, and user implementations of
`Iterable`/`Iterator`. Ranges are end-exclusive: `0..5` yields `0` through `4`,
and `a..b` is a `Range` value anywhere, not only in a `for` head. A map loop
binds each entry with `{key, value}` or a tuple pattern `(key, value)`.

An `if` without `else` has type `()`, so use it only for its effects.

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

Arithmetic and comparisons dispatch through built-in interfaces. `<>` and `++`
are interchangeable: either joins two strings or two lists of the same type.

`Int` division truncates toward zero (`-7 / 2` is `-3`), and `%` takes the
dividend's sign (`7 % -2` is `1`). Integer division or remainder by zero
panics; `-9223372036854775808 / -1` wraps. `Float` follows IEEE 754, so NaN is
unequal to itself. `Float.to_int`, `Math.floor`, `Math.ceil`, and `Math.round`
saturate at the `Int` bounds and turn NaN into `0`.

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
fn load() -> Account!AppError do
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
- `panic(message)` (`String -> Never`) prints `Mesh panic: message` and ends
  the current actor, which a supervisor can restart, or ends the program with
  exit status 101 when called from `main`.
- Runtime errors are panics: `List.get` out of range, `Map.get` of a missing
  key, a call no function clause matches, and integer division by zero.
- Recursion deeper than the stack ends the whole program with
  `error: stack overflow`.

See [Panics](/docs/language-basics/#panics).

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

Actor definitions accept arguments. Receive arms match the next message like
`case` arms: patterns, guards, and multi-line bodies are allowed, and the arms
must cover the whole message type. An actor that calls itself continues with
new arguments; in tail position that call is a loop. See [Concurrency](/docs/concurrency/) for service syntax,
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

## Exported functions

```mesh
@export("mesh_mobile_echo")
pub fn echo(request :: Bytes) -> Bytes!String do
  Ok(request)
end
```

`@export("c_symbol")` exposes a function under a C symbol when the project is
built with `meshc build --artifact staticlib` or `--artifact cdylib`. The
function must be `pub`, the symbol a C identifier, and the signature exactly
`(Bytes) -> Bytes!String`, without generic parameters, `where`, or a guard;
anything else is error E0055. See [Library Builds](/docs/library-builds/).

## Standard-library module index

This table lists every compiler-recognized built-in module on the current
branch.

| Area | Modules | Detailed guide |
| --- | --- | --- |
| Text and collections | `String`, `List`, `Map`, `Set`, `Tuple`, `Range`, `Queue`, `Iter`, `Regex` | [Standard Library](/docs/stdlib/), [Iterators](/docs/iterators/) |
| Binary and numbers | `Bytes`, `BytesBuilder`, `U64`, `U128`, `I128`, `Checked`, `Math`, `Int`, `Float` | [Standard Library](/docs/stdlib/) |
| Encoding and JSON | `JSON`, `Json`, `Base64`, `Hex` | [Web](/docs/web/), [Standard Library](/docs/stdlib/) |
| System and time | `IO`, `Env`, `File`, `DateTime`, `Monotonic`, `Duration`, `Random`, `Crypto` | [Standard Library](/docs/stdlib/) |
| Secrets and keys | `Secret`, `SecretMap`, `StorageKey`, `X25519PrivateKey`, `SigningPrivateKey`, `MlKemPrivateKey` | [Standard Library](/docs/stdlib/) |
| Host callbacks | `Host` | [Standard Library](/docs/stdlib/), [Library Builds](/docs/library-builds/) |
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
| `meshc build` | Compile and link a project as an executable, static library, or dynamic library |
| `meshc init` | Generate hello, clustered, or Todo API starters |
| `meshc deps` | Resolve git and path dependencies and fetch git checkouts |
| `meshc fmt` | Format or check `.mpl` files |
| `meshc lint` | Report deep nesting and other lint findings in `.mpl` files |
| `meshc test` | Run `.test.mpl` tests |
| `meshc repl` | Start the LLVM-backed REPL |
| `meshc lsp` | Run the language server over stdio |
| `meshc migrate` | Generate, inspect, apply, or roll back migrations |
| `meshc cluster` | Inspect or control a cluster |
| `meshc proof` | Run repository-owned proof scenarios |
| `meshc update` | Refresh an installer-managed toolchain |
| `meshpkg login` | Store a registry token |
| `meshpkg search` | Search the registry |
| `meshpkg install` | Install the latest release of one package, or the manifest's exact registry dependencies |
| `meshpkg publish` | Publish an immutable package version |
| `meshpkg update` | Refresh the toolchain |

See [Developer Tools](/docs/tooling/) for arguments, output paths, supported
targets, editor integration, and limitations, [Library Builds](/docs/library-builds/)
for `--artifact staticlib|cdylib`, and
[Environment Variables](/docs/environment-variables/) for the variables the
tools and runtime read.

## Keywords

These words are reserved and cannot name a variable or function:

`actor`, `after`, `alias`, `and`, `break`, `call`, `case`, `cast`, `cond`,
`continue`, `def`, `do`, `else`, `end`, `false`, `fn`, `for`, `if`, `impl`,
`import`, `in`, `interface`, `json`, `let`, `link`, `match`, `module`,
`monitor`, `nil`, `not`, `or`, `pub`, `receive`, `return`, `self`, `send`,
`service`, `spawn`, `struct`, `supervisor`, `terminate`, `trait`, `trap`,
`true`, `type`, `when`, `where`, `while`, `with`.

Some words have a meaning only in one position and remain ordinary names
elsewhere:

| Word | Position |
| --- | --- |
| `from` | `from Module import name` |
| `as` | After a pattern: `pattern as whole` |
| `deriving` | After a struct or sum type's `end` |
| `resource` | Before a type declaration: `resource Name`, `resource struct Name do ... end` |
| `borrow`, `consume` | Parameter ownership after `::`: `handle :: borrow Handle` |
| `table`, `primary_key`, `timestamps`, `belongs_to`, `has_many`, `has_one` | ORM schema metadata inside a struct |
| `strategy`, `max_restarts`, `max_seconds`, `child`, `start`, `restart`, `shutdown` | Fields of a `supervisor` block and its `child` blocks |
| `cluster`, `native`, `export` | Decorator names after `@` |

### Reserved words that are not features

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
- There are no module-level bindings: a `let` outside a function is error E0080
  in a build. Use a function, such as `fn limit() -> Int do 10 end`. The REPL
  keeps its `let` bindings between inputs.
- There is no bracket indexing: an index expression is error E0078. Use
  module functions such as `List.get`, `Map.get`, `Tuple.nth`, and
  `Json.array_get`.
- Struct-field patterns are not implemented.
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
