---
title: Mesh diagnostics
description: Look up Mesh diagnostic codes, repairs, and examples.
outline: [2, 2]
---

# Mesh diagnostics

Start with the diagnostic's code and message. The code identifies the kind of
problem. The message describes this occurrence. Use the primary location to
find the affected code, then read any related labels and notes. The repair
describes what to change. Run `meshc explain <CODE>` for the catalog entry
and `meshc guide <topic>` for the language feature it covers.

## Diagnostic JSON v2

The diagnostic stream uses NDJSON on stderr, with one JSON object per line.
Command results go to stdout. `meshc explain --json` emits a catalog object
on stdout with its own schema.

| Fields | Meaning |
| --- | --- |
| `code`, `name`, `severity`, `category` | Stable ID, kebab-case name, `error` or `warning`, and the code's family. |
| `message`, `fix`, `notes` | The observed problem, a repair instruction that is always a string, and an array of extra details. |
| `file`, `line`, `col` | Primary location. `file` is an empty string for invocation-level diagnostics. `line` and `col` can be null. |
| `spans` | The primary span followed by related spans. Each has `file`, `start`, `end`, `label`, `line`, `col`, `end_line`, and `end_col`. A diagnostic without a span has an empty array. |
| `guide`, `docs_url` | `guide.explain` is the CLI command, `guide.skill` is the local reference and anchor, and `guide.topic` is the guide topic or null. `docs_url` links to the website entry. |
| `edits` | Suggested edits with `file`, `start`, `end`, `replacement`, and `applicability`. |

Offsets are zero-based UTF-8 bytes. Lines and columns are one-based, and
columns count Unicode scalar values. End positions are exclusive. An edit
marked `machine-applicable` can be applied as written to the source that
produced it. Review `maybe-incorrect` edits before applying them.

## Repair loop

With a compiler that supports diagnostic JSON v2, use this loop.

1. Run `meshc check --json` and read diagnostics from stderr.
2. Start with the first occurrence of each code. Use its `fix` and source
   location to make the repair.
3. Run `meshc explain <CODE>` when you need the full repair or an example.
4. Repeat the check and repair cycle until the errors clear.
5. Run `meshc lint --json` and address the lint findings.
6. Run `meshc test`.

## Code families

| Prefix | Category | Covers |
| --- | --- | --- |
| `P` | `syntax` | Lexing and parsing |
| `E`, `W` | `semantic` | Semantic errors and warnings |
| `L` | `lint` | Lint warnings |
| `M`, `CFG` | `project` | Project and cluster configuration |
| `V` | `verify` | Verification and proofs |
| `B` | `build` | Targets, linking, and build artifacts |
| `R` | `runtime` | Runtime failures |
| `T` | `test` | Assertions and the test harness |
| `X` | `cli` | Command-line tools |
| `I` | `internal` | Internal compiler diagnostics |
| `C` | `uncategorized` | Transitional diagnostics |

## B0001

**target-unsupported** · error · build · guide: `meshc guide projects`

The target triple names a platform Mesh cannot compile or link for, or an artifact kind that the target does not support.

**Repair:** Pass `--target` a supported triple, a Unix-like one (macOS, iOS, Linux, Android, or a BSD) or a Windows MSVC one, or omit it to build for the host. Build an executable or a `cdylib` where the target has no `staticlib` support.

Docs: https://meshlang.dev/docs/errors/#b0001

## B0101

**linker-not-found** · error · build · guide: `meshc guide projects`

The C compiler, linker, or archiver the target links with could not be found or started.

**Repair:** Install the tool the message names and put it on `PATH`: a C compiler (`cc`) on Unix-like targets, `clang` from LLVM 21 for Windows MSVC (or set `LLVM_SYS_211_PREFIX` to that LLVM), or the Android NDK with `ANDROID_NDK_HOME` set.

Docs: https://meshlang.dev/docs/errors/#b0101

## B0102

**link-failed** · error · build · guide: `meshc guide projects`

The linker or archiver ran and reported a failure; its output is in the notes.

**Repair:** Fix what the linker output in the notes names: supply a missing native library or archive, and rebuild the Mesh runtime for the same target when its symbols are missing.

Docs: https://meshlang.dev/docs/errors/#b0102

## B0201

**runtime-library-missing** · error · build · guide: `meshc guide projects`

The Mesh runtime static library for the target was not found, or `MESH_RT_LIB_PATH` or `MESH_TEST_RT_LIB_PATH` does not name one.

**Repair:** Reinstall Mesh so the `lib` directory beside `meshc` holds the runtime, or in a source checkout run `cargo build -p mesh-rt` (`-p mesh-test-rt` for `meshc test`, with `--target` for another target). Point `MESH_RT_LIB_PATH` at the library file itself, or unset it.

Docs: https://meshlang.dev/docs/errors/#b0201

## B0301

**output-unwritable** · error · build · guide: `meshc guide projects`

The object file, LLVM IR, executable, or library could not be written at its output path.

**Repair:** Create the output directory, or pass `--output` a path in a writable directory where no directory of the same name exists.

Docs: https://meshlang.dev/docs/errors/#b0301

## B0302

**artifact-entry-missing** · error · build · guide: `meshc guide projects`

An executable build found no `fn main()` in the entry module, or a `staticlib` or `cdylib` build found no `@export` function for a host to call.

**Repair:** Define `fn main()` in the entrypoint to build an executable, or mark a public function `@export("symbol")` to build a library, or pass `--artifact` the kind of artifact the program provides.

Docs: https://meshlang.dev/docs/errors/#b0302

## C0001

**uncategorized** · error · uncategorized

A failure on a path that has no catalog code yet.

**Repair:** Fix what the message names. If the message does not say enough to act on, report it with the command that produced it.

Docs: https://meshlang.dev/docs/errors/#c0001

## CFG0001

**cluster-declaration-invalid** · error · project · guide: `meshc guide clustering`

A `@cluster` decorator is on a private function, on a name with several public definitions, or on a target already declared as clustered work.

**Repair:** Put `@cluster` on exactly one `pub fn` per name: make the function public, give overloaded entrypoints distinct names, and remove any second decorator for the same target.

Docs: https://meshlang.dev/docs/errors/#cfg0001

## E0001

**type-mismatch** · error · semantic · guide: `meshc guide types`

A value's type differs from the type its position requires.

**Repair:** Convert the value (`Int.to_float(...)`, `Some(...)`, `Ok(...)`), change it, or change the annotation so both sides name one type.

**Example**
```mesh
fn main() do
  let n :: Int = "one"
  println("#{n}")
end
```

**Fixed**
```mesh
fn main() do
  let n :: Int = 1
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0001

## E0002

**infinite-type** · error · semantic · guide: `meshc guide types`

Inference needs a type that contains itself, as when a function is applied to itself.

**Repair:** Stop passing the value to itself or storing it inside itself. For recursive data, define a struct or sum type that refers to itself by name.

**Example**
```mesh
fn self_apply(f) = f(f)

fn main() do
end
```

**Fixed**
```mesh
fn apply(f, x) = f(x)

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0002

## E0003

**arity-mismatch** · error · semantic · guide: `meshc guide syntax`

A call passes more or fewer arguments than the function takes, or a closure of several parameters is given where a function of one tuple is expected.

**Repair:** Pass exactly the arguments the function declares; a value piped with `|>` counts as the first. To take a tuple argument apart, put its pattern in parentheses: `fn ((a, b)) -> ... end`.

**Example**
```mesh
fn add(a :: Int, b :: Int) -> Int = a + b

fn main() do
  println("#{add(1)}")
end
```

**Fixed**
```mesh
fn add(a :: Int, b :: Int) -> Int = a + b

fn main() do
  println("#{add(1, 2)}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0003

## E0004

**unbound-variable** · error · semantic · guide: `meshc guide syntax`

A name is used where nothing of that name is defined, imported, or in scope.

**Repair:** Define the name before this use, import the module that defines it, or correct the spelling to a name in scope.

**Example**
```mesh
fn main() do
  let count = 1
  println("#{cuont}")
end
```

**Fixed**
```mesh
fn main() do
  let count = 1
  println("#{count}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0004

## E0005

**not-a-function** · error · semantic · guide: `meshc guide syntax`

A value that is not a function is called with parentheses.

**Repair:** Remove the parentheses to use the value itself, or call the function the value was meant to be.

**Example**
```mesh
fn main() do
  let n = 1
  println("#{n()}")
end
```

**Fixed**
```mesh
fn main() do
  let n = 1
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0005

## E0006

**trait-not-satisfied** · error · semantic · guide: `meshc guide traits`

A type is used where an interface it does not implement is required, such as interpolating a struct that does not implement `Display`.

**Repair:** Add the interface to the type's `deriving(...)` list, or write `impl Interface for Type do ... end`.

**Example**
```mesh
struct Point do
  x :: Int
end deriving(Eq)

fn main() do
  let p = Point { x: 1 }
  println("#{p}")
end
```

**Fixed**
```mesh
struct Point do
  x :: Int
end deriving(Eq, Display)

fn main() do
  let p = Point { x: 1 }
  println("#{p}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0006

## E0007

**missing-trait-method** · error · semantic · guide: `meshc guide traits`

An `impl` block leaves out a method its interface declares without a default body.

**Repair:** Add the missing `fn` to the `impl` block, with the signature the interface declares.

**Example**
```mesh
interface Shape do
  fn area(self) -> Int
end

struct Square do
  side :: Int
end

impl Shape for Square do
end

fn main() do
end
```

**Fixed**
```mesh
interface Shape do
  fn area(self) -> Int
end

struct Square do
  side :: Int
end

impl Shape for Square do
  fn area(self) -> Int do
    self.side * self.side
  end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0007

## E0008

**trait-method-signature-mismatch** · error · semantic · guide: `meshc guide traits`

A method in an `impl` block has other parameter or return types than its interface declares.

**Repair:** Change the method's parameter and return types to the ones the interface declares.

**Example**
```mesh
interface Shape do
  fn area(self) -> Int
end

struct Square do
  side :: Int
end

impl Shape for Square do
  fn area(self) -> String do
    "big"
  end
end

fn main() do
end
```

**Fixed**
```mesh
interface Shape do
  fn area(self) -> Int
end

struct Square do
  side :: Int
end

impl Shape for Square do
  fn area(self) -> Int do
    self.side * self.side
  end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0008

## E0009

**invalid-field** · error · semantic · guide: `meshc guide types`

A struct literal leaves out a field or names one the struct does not have, or a field is read from a type without it.

**Repair:** Give every field the struct declares exactly once, and use only field names from the struct's definition.

**Example**
```mesh
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  let p = Point { x: 1 }
  println("#{p.x}")
end
```

**Fixed**
```mesh
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  let p = Point { x: 1, y: 2 }
  println("#{p.x}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0009

## E0010

**unknown-variant** · error · semantic · guide: `meshc guide pattern-matching`

A pattern names a constructor that no sum type in scope defines.

**Repair:** Correct the spelling to a variant of the matched type, or import the module that defines the type. To split a list in a pattern, write `head :: tail` with a lowercase tail.

**Example**
```mesh
fn main() do
  let n = case Some(1) do
    Some(v) -> v
    Nothing -> 0
  end
  println("#{n}")
end
```

**Fixed**
```mesh
fn main() do
  let n = case Some(1) do
    Some(v) -> v
    None -> 0
  end
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0010

## E0011

**or-pattern-binding-mismatch** · error · semantic · guide: `meshc guide pattern-matching`

The alternatives of a `|` pattern bind different sets of variables, so the arm body cannot rely on any of them.

**Repair:** Bind the same names in every alternative, or split the alternatives into separate arms. A list's tail is written `head :: tail`, not `[head | tail]`.

**Example**
```mesh
fn main() do
  let n = case Some(1) do
    Some(a) | None -> 0
  end
  println("#{n}")
end
```

**Fixed**
```mesh
fn main() do
  let n = case Some(1) do
    Some(_) | None -> 0
  end
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0011

## E0012

**non-exhaustive-match** · error · semantic · guide: `meshc guide pattern-matching`

A `case` has no arm for some values of the matched type.

**Repair:** Add an arm for each missing pattern the diagnostic lists, or a final `_` arm for an intentional default.

**Example**
```mesh
fn main() do
  let n = case Some(1) do
    Some(v) -> v
  end
  println("#{n}")
end
```

**Fixed**
```mesh
fn main() do
  let n = case Some(1) do
    Some(v) -> v
    None -> 0
  end
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0012

## E0014

**message-type-mismatch** · error · semantic · guide: `meshc guide actors`

A message sent to a typed `Pid<M>` is not of type `M`.

**Repair:** Send a value of the pid's message type, or change the actor's message type to one that includes this message.

**Example**
```mesh
actor counter(total :: Int) do
  receive do
    n -> counter(total + n)
  end
end

fn main() do
  let pid :: Pid<Int> = spawn(counter, 0)
  send(pid, "five")
end
```

**Fixed**
```mesh
actor counter(total :: Int) do
  receive do
    n -> counter(total + n)
  end
end

fn main() do
  let pid :: Pid<Int> = spawn(counter, 0)
  send(pid, 5)
end
```

Docs: https://meshlang.dev/docs/errors/#e0014

## E0015

**self-outside-actor** · error · semantic · guide: `meshc guide actors`

`self()` names the running actor, and it is called where no actor is running.

**Repair:** Move the `self()` call into an `actor` body, or pass the pid it should name in as an argument.

**Example**
```mesh
fn main() do
  let _me = self()
end
```

**Fixed**
```mesh
actor worker() do
  let _me = self()
end

fn main() do
  let _ = spawn(worker)
end
```

Docs: https://meshlang.dev/docs/errors/#e0015

## E0016

**spawn-non-function** · error · semantic · guide: `meshc guide actors`

`spawn` is given a value that is not an actor.

**Repair:** Pass the actor to start as the first argument, followed by its arguments: `spawn(worker, arg)`.

**Example**
```mesh
fn main() do
  let _ = spawn(1)
end
```

**Fixed**
```mesh
actor worker() do
  println("started")
end

fn main() do
  let _ = spawn(worker)
end
```

Docs: https://meshlang.dev/docs/errors/#e0016

## E0017

**receive-outside-actor** · error · semantic · guide: `meshc guide actors`

A `receive` block appears outside an actor, where there is no mailbox to read.

**Repair:** Move the `receive` block into an `actor name(...) do ... end` body.

**Example**
```mesh
fn main() do
  receive do
    _ -> println("got one")
  end
end
```

**Fixed**
```mesh
actor worker() do
  receive do
    _ -> println("got one")
  end
end

fn main() do
  let _ = spawn(worker)
end
```

Docs: https://meshlang.dev/docs/errors/#e0017

## E0018

**invalid-child-start** · error · semantic · guide: `meshc guide supervisors`

A supervisor child's `start` function returns something other than the `Pid` of the process it spawns.

**Repair:** Make `start` return the result of `spawn`, as `start: fn -> spawn(worker, arg) end`.

**Example**
```mesh
actor worker() do
  nil
end

supervisor Workers do
  strategy: one_for_one
  child worker do
    start: fn -> 42 end
  end
end
```

**Fixed**
```mesh
actor worker() do
  nil
end

supervisor Workers do
  strategy: one_for_one
  child worker do
    start: fn -> spawn(worker) end
  end
end
```

Docs: https://meshlang.dev/docs/errors/#e0018

## E0019

**unknown-supervision-strategy** · error · semantic · guide: `meshc guide supervisors`

A supervisor's `strategy` is not one Mesh supports.

**Repair:** Set `strategy:` to `one_for_one`, `one_for_all`, `rest_for_one`, or `simple_one_for_one`.

**Example**
```mesh
actor worker() do
  receive do
    _ -> println("done")
  end
end

supervisor WorkerSup do
  strategy: one_for_none

  child w do
    start: fn -> spawn(worker) end
    restart: transient
    shutdown: 1_000
  end
end

fn main() do
  let _ = spawn(WorkerSup)
end
```

**Fixed**
```mesh
actor worker() do
  receive do
    _ -> println("done")
  end
end

supervisor WorkerSup do
  strategy: one_for_one

  child w do
    start: fn -> spawn(worker) end
    restart: transient
    shutdown: 1_000
  end
end

fn main() do
  let _ = spawn(WorkerSup)
end
```

Docs: https://meshlang.dev/docs/errors/#e0019

## E0020

**invalid-restart-type** · error · semantic · guide: `meshc guide supervisors`

A child spec's `restart` is not a restart type Mesh supports.

**Repair:** Set `restart:` to `permanent`, `transient`, or `temporary`.

**Example**
```mesh
actor worker() do
  receive do
    _ -> println("done")
  end
end

supervisor WorkerSup do
  strategy: one_for_one

  child w do
    start: fn -> spawn(worker) end
    restart: always
    shutdown: 1_000
  end
end

fn main() do
  let _ = spawn(WorkerSup)
end
```

**Fixed**
```mesh
actor worker() do
  receive do
    _ -> println("done")
  end
end

supervisor WorkerSup do
  strategy: one_for_one

  child w do
    start: fn -> spawn(worker) end
    restart: transient
    shutdown: 1_000
  end
end

fn main() do
  let _ = spawn(WorkerSup)
end
```

Docs: https://meshlang.dev/docs/errors/#e0020

## E0021

**invalid-shutdown-value** · error · semantic · guide: `meshc guide supervisors`

A child spec's `shutdown` is neither a positive number of milliseconds nor `brutal_kill`.

**Repair:** Set `shutdown:` to a positive integer of milliseconds, such as `1_000`, or to `brutal_kill`.

**Example**
```mesh
actor worker() do
  receive do
    _ -> println("done")
  end
end

supervisor WorkerSup do
  strategy: one_for_one

  child w do
    start: fn -> spawn(worker) end
    restart: transient
    shutdown: forever
  end
end

fn main() do
  let _ = spawn(WorkerSup)
end
```

**Fixed**
```mesh
actor worker() do
  receive do
    _ -> println("done")
  end
end

supervisor WorkerSup do
  strategy: one_for_one

  child w do
    start: fn -> spawn(worker) end
    restart: transient
    shutdown: 1_000
  end
end

fn main() do
  let _ = spawn(WorkerSup)
end
```

Docs: https://meshlang.dev/docs/errors/#e0021

## E0022

**catch-all-not-last** · error · semantic · guide: `meshc guide syntax`

A clause that matches every argument comes before other clauses of the same function, so they never run.

**Repair:** Move the catch-all clause after every more specific clause of the function.

**Example**
```mesh
fn label(n) = n
fn label(0) = 1
```

**Fixed**
```mesh
fn label(0) = 1
fn label(n) = n
```

Docs: https://meshlang.dev/docs/errors/#e0022

## E0023

**non-consecutive-clauses** · error · semantic · guide: `meshc guide syntax`

Clauses of one function name and arity are separated by other definitions.

**Repair:** Move every clause of the function together, one after another, or rename the later definition.

**Example**
```mesh
fn label(n) = "any"

fn main() do
  println(label(1))
end

fn label(n) = "other"
```

**Fixed**
```mesh
fn label(n) = "any"

fn main() do
  println(label(1))
end

fn other_label(n) = "other"
```

Docs: https://meshlang.dev/docs/errors/#e0023

## E0026

**duplicate-impl** · error · semantic · guide: `meshc guide traits`

Two `impl` blocks implement the same interface for the same type.

**Repair:** Remove one of the `impl` blocks, or merge their methods into one.

**Example**
```mesh
interface Named do
  fn name(self) -> String
end

struct User do
  id :: Int
end

impl Named for User do
  fn name(self) -> String do
    "user"
  end
end

impl Named for User do
  fn name(self) -> String do
    "person"
  end
end

fn main() do
end
```

**Fixed**
```mesh
interface Named do
  fn name(self) -> String
end

struct User do
  id :: Int
end

impl Named for User do
  fn name(self) -> String do
    "user"
  end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0026

## E0027

**ambiguous-method** · error · semantic · guide: `meshc guide traits`

Several interfaces implemented for the receiver's type provide a method of the called name.

**Repair:** Call the method through the interface you mean, as `Interface.method(value)`.

**Example**
```mesh
interface A do
  fn label(self) -> Int
end

interface B do
  fn label(self) -> Int
end

struct Item do
  n :: Int
end

impl A for Item do
  fn label(self) -> Int = self.n
end

impl B for Item do
  fn label(self) -> Int = self.n + 1
end

fn main() = Item { n: 0 }.label()
```

**Fixed**
```mesh
interface A do
  fn label(self) -> Int
end

interface B do
  fn label(self) -> Int
end

struct Item do
  n :: Int
end

impl A for Item do
  fn label(self) -> Int = self.n
end

impl B for Item do
  fn label(self) -> Int = self.n + 1
end

fn main() = A.label(Item { n: 0 })
```

Docs: https://meshlang.dev/docs/errors/#e0027

## E0028

**unsupported-derive** · error · semantic · guide: `meshc guide traits`

A `deriving(...)` list names an interface that cannot be derived for this kind of type.

**Repair:** Derive only Eq, Ord, Display, Debug, Hash, Json, Row, or Schema on a struct, and the same but Row and Schema on a sum type. Write an `impl` for any other interface.

**Example**
```mesh
struct Point do
  x :: Int
end deriving(Add)

fn main() do
end
```

**Fixed**
```mesh
struct Point do
  x :: Int
end deriving(Eq)

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0028

## E0029

**missing-derive-prerequisite** · error · semantic · guide: `meshc guide traits`

A derived interface depends on another that the `deriving` list leaves out, such as `Ord` without `Eq`.

**Repair:** Add the required interface to the same `deriving(...)` list.

**Example**
```mesh
struct Point do
  x :: Int
end deriving(Ord)

fn main() do
end
```

**Fixed**
```mesh
struct Point do
  x :: Int
end deriving(Eq, Ord)

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0029

## E0030

**no-such-method** · error · semantic · guide: `meshc guide traits`

A method is called on a type that neither defines it nor implements an interface that provides it.

**Repair:** Correct the method name, call the module function it stands for as `Module.function(value)`, or implement an interface that provides the method.

**Example**
```mesh
fn main() do
  let n = 1
  println(n.shout())
end
```

**Fixed**
```mesh
fn main() do
  let n = 1
  println(n.to_string())
end
```

Docs: https://meshlang.dev/docs/errors/#e0030

## E0031

**module-not-found** · error · semantic · guide: `meshc guide projects`

An `import` names a module that is neither in the project nor in the standard library.

**Repair:** Correct the module path, which follows the file path (`geo/points.mpl` is `Geo.Points`), or create the module's file.

**Example**
```mesh
import Maths

module Math do
  pub fn count() = 1
end
```

**Fixed**
```mesh
import Math

module Math do
  pub fn count() = 1
end
```

Docs: https://meshlang.dev/docs/errors/#e0031

## E0032

**break-outside-loop** · error · semantic · guide: `meshc guide syntax`

`break` appears where no `while` or `for` loop encloses it.

**Repair:** Move the `break` into a loop body, or leave the function with `return` instead.

**Example**
```mesh
fn main() do
  break
end
```

**Fixed**
```mesh
fn main() do
  while true do
    break
  end
end
```

Docs: https://meshlang.dev/docs/errors/#e0032

## E0033

**continue-outside-loop** · error · semantic · guide: `meshc guide syntax`

`continue` appears where no `while` or `for` loop encloses it.

**Repair:** Move the `continue` into a loop body, or remove it.

**Example**
```mesh
fn main() do
  continue
end
```

**Fixed**
```mesh
fn main() do
  while false do
    continue
  end
end
```

Docs: https://meshlang.dev/docs/errors/#e0033

## E0034

**import-name-not-found** · error · semantic · guide: `meshc guide projects`

An import names something its module does not define.

**Repair:** Correct the name to one the module exports (the diagnostic lists them), or define it in the module with `pub`.

**Example**
```mesh
from Math import coun

module Math do
  pub fn count() = 1
end
```

**Fixed**
```mesh
from Math import count

module Math do
  pub fn count() = 1
end
```

Docs: https://meshlang.dev/docs/errors/#e0034

## E0035

**private-item** · error · semantic · guide: `meshc guide projects`

An import names a definition its module does not mark `pub`.

**Repair:** Add `pub` to the definition in its module, or stop importing it.

**Example**
```mesh
from Math import count

module Math do
  fn count() = 1
end
```

**Fixed**
```mesh
from Math import count

module Math do
  pub fn count() = 1
end
```

Docs: https://meshlang.dev/docs/errors/#e0035

## E0036

**try-incompatible-return** · error · semantic · guide: `meshc guide error-handling`

`?` returns early with the operand's `Err` or `None`, and the enclosing function's return type cannot hold it.

**Repair:** For a `Result` operand, make the function return a `Result` whose error type matches or has a `From` conversion; for an `Option` operand, return an `Option`. Otherwise handle the value with `case` instead of `?`.

**Example**
```mesh
fn half(n :: Int) -> Option<Int> do
  if n % 2 == 0 do
    Some(n / 2)
  else
    None
  end
end

fn quarter(n :: Int) -> Int do
  let h = half(n)?
  h / 2
end

fn main() do
end
```

**Fixed**
```mesh
fn half(n :: Int) -> Option<Int> do
  if n % 2 == 0 do
    Some(n / 2)
  else
    None
  end
end

fn quarter(n :: Int) -> Option<Int> do
  let h = half(n)?
  half(h)
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0036

## E0037

**try-on-non-result** · error · semantic · guide: `meshc guide error-handling`

`?` unwraps only `Result` and `Option` values, and its operand is neither.

**Repair:** Remove the `?`, or apply it to an expression that returns a `Result` or an `Option`.

**Example**
```mesh
fn double(n :: Int) -> Int = n * 2

fn quadruple(n :: Int) -> Option<Int> do
  let d = double(n)?
  Some(d * 2)
end

fn main() do
end
```

**Fixed**
```mesh
fn double(n :: Int) -> Int = n * 2

fn quadruple(n :: Int) -> Option<Int> do
  let d = double(n)
  Some(d * 2)
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0037

## E0038

**non-serializable-field** · error · semantic · guide: `meshc guide strings`

A type deriving `Json` has a field whose type has no JSON form.

**Repair:** Add `deriving(Json)` to the field's type, or change the field to a JSON type: Int, Float, Bool, String, a tuple, Option, List, or a Map with String keys.

**Example**
```mesh
struct Point do
  x :: Int
end

struct Shape do
  origin :: Point
end deriving(Json)

fn main() do
end
```

**Fixed**
```mesh
struct Point do
  x :: Int
end deriving(Json)

struct Shape do
  origin :: Point
end deriving(Json)

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0038

## E0039

**non-mappable-field** · error · semantic · guide: `meshc guide database`

A struct deriving `Row` has a field of a type a database column cannot hold.

**Repair:** Change the field to Int, Float, Bool, String, or an Option of one of them, or remove `Row` from the `deriving` list.

**Example**
```mesh
struct User do
  tags :: List<String>
end deriving(Row)

fn main() do
end
```

**Fixed**
```mesh
struct User do
  tags :: String
end deriving(Row)

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0039

## E0040

**missing-associated-type** · error · semantic · guide: `meshc guide traits`

An `impl` block leaves out an associated type its interface declares.

**Repair:** Add `type Name = ConcreteType` to the `impl` block for each associated type the interface declares.

**Example**
```mesh
interface Container do
  type Item
  fn first(self) -> Self.Item
end

struct IntBox do
  value :: Int
end

impl Container for IntBox do
  fn first(self) -> Int do
    self.value
  end
end

fn main() do
end
```

**Fixed**
```mesh
interface Container do
  type Item
  fn first(self) -> Self.Item
end

struct IntBox do
  value :: Int
end

impl Container for IntBox do
  type Item = Int
  fn first(self) -> Int do
    self.value
  end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0040

## E0041

**extra-associated-type** · error · semantic · guide: `meshc guide traits`

An `impl` block binds an associated type its interface does not declare.

**Repair:** Remove the `type` binding from the `impl` block, or declare it in the interface with `type Name`.

**Example**
```mesh
interface Labeled do
  fn label(self) -> String
end

struct IntBox do
  value :: Int
end

impl Labeled for IntBox do
  type Item = Int
  fn label(self) -> String do
    "box"
  end
end

fn main() do
end
```

**Fixed**
```mesh
interface Labeled do
  fn label(self) -> String
end

struct IntBox do
  value :: Int
end

impl Labeled for IntBox do
  fn label(self) -> String do
    "box"
  end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0041

## E0042

**unresolved-associated-type** · error · semantic · guide: `meshc guide traits`

`Self.Name` refers to an associated type that no interface in scope declares.

**Repair:** Declare the associated type in the interface with `type Name`, and bind it in each `impl` with `type Name = ConcreteType`.

**Example**
```mesh
interface Container do
  fn first(self) -> Self.Item
end
```

**Fixed**
```mesh
interface Container do
  type Item
  fn first(self) -> Self.Item
end
```

Docs: https://meshlang.dev/docs/errors/#e0042

## E0044

**slot-pipe-out-of-range** · error · semantic · guide: `meshc guide syntax`

`|N>` passes the piped value as argument N, and the function takes fewer than N arguments.

**Repair:** Use a position from 2 up to the function's arity, or `|>` to pass the value as the first argument.

**Example**
```mesh
fn add(a :: Int, b :: Int) -> Int = a + b

fn main() do
  let n = 1 |3> add(2)
  println("#{n}")
end
```

**Fixed**
```mesh
fn add(a :: Int, b :: Int) -> Int = a + b

fn main() do
  let n = 1 |2> add(2)
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0044

## E0045

**undefined-alias-target** · error · semantic · guide: `meshc guide types`

A `type Name = Target` alias names a target type that does not exist.

**Repair:** Correct the target's spelling, import it, or define it as a struct, sum type, or alias.

**Example**
```mesh
type Count = Integer

fn main() do
end
```

**Fixed**
```mesh
type Count = Int

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0045

## E0046

**manual-continuity-promotion** · error · semantic · guide: `meshc guide clustering`

`Continuity.promote()` is no longer part of Mesh; failover between nodes is automatic.

**Repair:** Remove the `Continuity.promote()` call. Read authority state with `Continuity.authority_status()` where the program needs it.

**Example**
```mesh
fn main() = Continuity.promote()
```

**Fixed**
```mesh
fn main() = Continuity.authority_status()
```

Docs: https://meshlang.dev/docs/errors/#e0046

## E0047

**clustered-route-invalid-arguments** · error · semantic · guide: `meshc guide clustering`

`HTTP.clustered(...)` received arguments other than an optional replication count and a handler reference.

**Repair:** Write `HTTP.clustered(handler)` or `HTTP.clustered(count, handler)`, where `handler` names a public top-level route handler.

**Example**
```mesh
pub fn handle(req :: Request) -> Response = HTTP.response(200, "ok")

fn routes() = HTTP.on_get(HTTP.router(), "/", HTTP.clustered(0, handle))
```

**Fixed**
```mesh
pub fn handle(req :: Request) -> Response = HTTP.response(200, "ok")

fn routes() = HTTP.on_get(HTTP.router(), "/", HTTP.clustered(handle))
```

Docs: https://meshlang.dev/docs/errors/#e0047

## E0048

**clustered-route-private-handler** · error · semantic · guide: `meshc guide clustering`

`HTTP.clustered(...)` wraps a handler that is not `pub`, so other nodes cannot run it.

**Repair:** Add `pub` to the handler's definition, or remove the `HTTP.clustered(...)` wrapper.

**Example**
```mesh
fn handle(req :: Request) -> Response = HTTP.response(200, "ok")

fn routes() = HTTP.on_get(HTTP.router(), "/", HTTP.clustered(handle))
```

**Fixed**
```mesh
pub fn handle(req :: Request) -> Response = HTTP.response(200, "ok")

fn routes() = HTTP.on_get(HTTP.router(), "/", HTTP.clustered(handle))
```

Docs: https://meshlang.dev/docs/errors/#e0048

## E0049

**clustered-route-misplaced** · error · semantic · guide: `meshc guide clustering`

`HTTP.clustered(...)` appears somewhere other than the handler argument of `HTTP.route(...)` or `HTTP.on_*(...)`.

**Repair:** Pass `HTTP.clustered(handler)` directly as the handler argument of `HTTP.route(...)` or `HTTP.on_*(...)`.

**Example**
```mesh
pub fn handle(req :: Request) -> Response = HTTP.response(200, "ok")

fn routes() = HTTP.clustered(handle)
```

**Fixed**
```mesh
pub fn handle(req :: Request) -> Response = HTTP.response(200, "ok")

fn routes() = HTTP.on_get(HTTP.router(), "/", HTTP.clustered(handle))
```

Docs: https://meshlang.dev/docs/errors/#e0049

## E0050

**clustered-route-conflicting-count** · error · semantic · guide: `meshc guide clustering`

One clustered route handler is wrapped with different replication counts in different places.

**Repair:** Use one replication count for the handler everywhere it is wrapped in `HTTP.clustered(...)`.

**Example**
```mesh
pub fn handle(req :: Request) -> Response = HTTP.response(200, "ok")

fn routes() do
  let router = HTTP.on_get(HTTP.router(), "/a", HTTP.clustered(2, handle))
  HTTP.on_get(router, "/b", HTTP.clustered(3, handle))
end
```

**Fixed**
```mesh
pub fn handle(req :: Request) -> Response = HTTP.response(200, "ok")

fn routes() do
  let router = HTTP.on_get(HTTP.router(), "/a", HTTP.clustered(2, handle))
  HTTP.on_get(router, "/b", HTTP.clustered(2, handle))
end
```

Docs: https://meshlang.dev/docs/errors/#e0050

## E0051

**clustered-route-origin-missing** · error · semantic · guide: `meshc guide clustering`

A handler imported by bare name into `HTTP.clustered(...)` has lost the module that defines it, so its runtime name is unknown.

**Repair:** Import the handler's module and wrap the handler by its qualified name, as `HTTP.clustered(Module.handler)`.

Docs: https://meshlang.dev/docs/errors/#e0051

## E0052

**invalid-native-declaration** · error · semantic · guide: `meshc guide projects`

In a native binding listed in `mesh.toml`, an `@native` function is private, generic, missing a type annotation, or names a symbol that is not a C identifier.

**Repair:** Declare it as a public, non-generic function with every parameter and the return type annotated, and a C identifier as its symbol.

**Example**
```mesh
@native("mesh_count")
fn count() -> Int
```

**Fixed**
```mesh
@native("mesh_count")
pub fn count() -> Int
```

Docs: https://meshlang.dev/docs/errors/#e0052

## E0053

**resource-ownership-violation** · error · semantic · guide: `meshc guide types`

A resource is used after a move, moved while borrowed, or given more than one owner.

**Repair:** Use the resource before moving it. Pass it to a direct `borrow` parameter when the caller needs to keep ownership.

**Example**
```mesh
fn close(secret :: SecretBytes) do
  Secret.destroy(secret)
  Secret.destroy(secret)
end
```

**Fixed**
```mesh
fn close(secret :: SecretBytes) do
  Secret.destroy(secret)
end
```

Docs: https://meshlang.dev/docs/errors/#e0053

## E0054

**invalid-let-pattern** · error · semantic · guide: `meshc guide pattern-matching`

A `let` pattern could fail to match, or uses a form `let` cannot bind.

**Repair:** Bind only lowercase names, `_`, tuples, and struct patterns in `let`; match a value that may not fit with `case`.

**Example**
```mesh
fn main() do
  let (1, b) = (1, 2)
  println("#{b}")
end
```

**Fixed**
```mesh
fn main() do
  let (_, b) = (1, 2)
  println("#{b}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0054

## E0055

**invalid-export-declaration** · error · semantic · guide: `meshc guide projects`

An `@export` function does not have the binary request and response signature library exports use.

**Repair:** Declare the export as `@export("c_symbol") pub fn name(request :: Bytes) -> Bytes!String`.

**Example**
```mesh
@export("mesh_echo")
fn echo(request :: Bytes) -> Bytes!String = Ok(request)
```

**Fixed**
```mesh
@export("mesh_echo")
pub fn echo(request :: Bytes) -> Bytes!String = Ok(request)
```

Docs: https://meshlang.dev/docs/errors/#e0055

## E0056

**invalid-pass-through-arm** · error · semantic · guide: `meshc guide pattern-matching`

A match arm with no `->` gives its pattern as its value, and this pattern cannot stand for one.

**Repair:** Write the arm's value after `->`, as `pattern -> value`.

**Example**
```mesh
fn main() do
  case 1 do
    _
  end
end
```

**Fixed**
```mesh
fn main() do
  case 1 do
    _ -> 1
  end
end
```

Docs: https://meshlang.dev/docs/errors/#e0056

## E0057

**duplicate-binding** · error · semantic · guide: `meshc guide pattern-matching`

One pattern binds the same name in two places.

**Repair:** Rename one of the bindings. To require the two values to be equal, compare them in a `when` guard.

**Example**
```mesh
fn main() do
  let n = case (1, 2) do
    (a, a) -> a
  end
  println("#{n}")
end
```

**Fixed**
```mesh
fn main() do
  let n = case (1, 2) do
    (a, b) -> a + b
  end
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0057

## E0058

**duplicate-field** · error · semantic · guide: `meshc guide types`

A struct literal or update gives the same field more than once.

**Repair:** Give each field one value, and remove the repeated one.

**Example**
```mesh
struct Point do
  x :: Int
end

fn main() do
  let p = Point { x: 1, x: 2 }
  println("#{p.x}")
end
```

**Fixed**
```mesh
struct Point do
  x :: Int
end

fn main() do
  let p = Point { x: 1 }
  println("#{p.x}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0058

## E0059

**not-a-struct** · error · semantic · guide: `meshc guide types`

A struct literal or a `%{value | field: new}` update is applied to something that is not a struct, or whose type is not known there.

**Repair:** Use struct literals and updates only on struct types, and annotate the value with its struct type when nothing else fixes it.

**Example**
```mesh
fn main() = Int { n: 1 }
```

**Fixed**
```mesh
struct Count do
  n :: Int
end

fn main() = Count { n: 1 }
```

Docs: https://meshlang.dev/docs/errors/#e0059

## E0060

**underivable-function-field** · error · semantic · guide: `meshc guide traits`

A derived interface would compare, hash, or show a field that holds a function.

**Repair:** Remove the interface from the `deriving` list, or write an `impl` that leaves the function field out.

**Example**
```mesh
struct Handler do
  run :: Fun(Int) -> Int
end deriving(Eq)

fn main() do
end
```

**Fixed**
```mesh
struct Handler do
  run :: Fun(Int) -> Int
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0060

## E0061

**duplicate-variant** · error · semantic · guide: `meshc guide types`

Two sum types in one module declare a variant of the same name, so the name could mean either.

**Repair:** Rename one of the variants, or move one of the sum types to its own module.

**Example**
```mesh
type Light do
  Red
  Green
end

type Alert do
  Red
  Calm
end

fn main() do
end
```

**Fixed**
```mesh
type Light do
  Red
  Green
end

type Alert do
  Critical
  Calm
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0061

## E0062

**cyclic-alias** · error · semantic · guide: `meshc guide types`

A type alias expands into itself, directly or through other aliases, so its expansion never ends.

**Repair:** Point the alias at an existing type. For a recursive type, define a struct or sum type instead.

**Example**
```mesh
type Loop = Loop

fn main() do
end
```

**Fixed**
```mesh
type Loop = Int

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0062

## E0063

**unbounded-type-parameter** · error · semantic · guide: `meshc guide traits`

A generic function applies an operator to a type parameter that no bound says supports it.

**Repair:** Add `where T: Interface` to the function's signature, naming the interface the operator needs, such as `Add` for `+`.

**Example**
```mesh
fn double<T>(x :: T) -> T do
  x + x
end

fn main() do
end
```

**Fixed**
```mesh
fn double<T>(x :: T) -> T where T: Add do
  x + x
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0063

## E0064

**ambiguous-default** · error · semantic · guide: `meshc guide traits`

A `default()` call appears where nothing fixes which type of value it builds.

**Repair:** Annotate the binding with the type to build, as `let x :: Int = default()`.

**Example**
```mesh
fn main() do
  let _x = default()
end
```

**Fixed**
```mesh
fn main() do
  let _x :: Int = default()
end
```

Docs: https://meshlang.dev/docs/errors/#e0064

## E0065

**ambiguous-impl-method** · error · semantic · guide: `meshc guide traits`

Several `impl`s provide the called method with different return or argument types, and nothing at the call picks one.

**Repair:** Annotate the result, or the argument, with one of the types the diagnostic lists.

**Example**
```mesh
fn main() do
  let x = 5.into()
  println("#{x}")
end
```

**Fixed**
```mesh
fn main() do
  let x :: Float = 5.into()
  println("#{x}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0065

## E0066

**ambiguous-static-method** · error · semantic · guide: `meshc guide traits`

A static interface method, one without `self`, is called bare while several types provide it.

**Repair:** Call it on the type that should provide it, as `Type.method()`.

**Example**
```mesh
interface Versioned do
  fn version() -> Int
end

struct A do
  n :: Int
end

struct B do
  n :: Int
end

impl Versioned for A do
  fn version() -> Int = 1
end

impl Versioned for B do
  fn version() -> Int = 2
end

fn main() = version()
```

**Fixed**
```mesh
interface Versioned do
  fn version() -> Int
end

struct A do
  n :: Int
end

struct B do
  n :: Int
end

impl Versioned for A do
  fn version() -> Int = 1
end

impl Versioned for B do
  fn version() -> Int = 2
end

fn main() = A.version()
```

Docs: https://meshlang.dev/docs/errors/#e0066

## E0067

**rigid-type-parameter** · error · semantic · guide: `meshc guide types`

A generic function's body treats a type parameter as one particular type, though callers may pass any type.

**Repair:** Replace the type parameter with that type in the signature, or keep the parameter's values opaque in the body.

**Example**
```mesh
fn same<T>(x :: T) -> Int = x

fn main() do
end
```

**Fixed**
```mesh
fn same<T>(x :: T) -> T = x

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0067

## E0068

**duplicate-definition** · error · semantic · guide: `meshc guide types`

A field, variant, parameter, or type of one name is defined twice in the same scope.

**Repair:** Rename or remove one of the two definitions.

**Example**
```mesh
struct Point do
  x :: Int
  x :: Int
end

fn main() do
end
```

**Fixed**
```mesh
struct Point do
  x :: Int
  y :: Int
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0068

## E0069

**unknown-type** · error · semantic · guide: `meshc guide types`

An annotation names a type that is not defined, imported, or built in.

**Repair:** Correct the spelling, define the type, or import the module that defines it.

**Example**
```mesh
fn main() do
  let _n :: Integer = 1
end
```

**Fixed**
```mesh
fn main() do
  let _n :: Int = 1
end
```

Docs: https://meshlang.dev/docs/errors/#e0069

## E0070

**unknown-field-owner** · error · semantic · guide: `meshc guide types`

A field is read from a value whose type nothing fixes, so the struct that owns the field is unknown.

**Repair:** Annotate the value with its struct type, such as a parameter `p :: Point`.

**Example**
```mesh
fn get_x(p) = p.x

fn main() do
end
```

**Fixed**
```mesh
struct Point do
  x :: Int
end

fn get_x(p :: Point) -> Int = p.x

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0070

## E0071

**unknown-interface** · error · semantic · guide: `meshc guide traits`

An `impl` names an interface that is not defined, imported, or built in.

**Repair:** Correct the spelling, declare the interface, or import the module that defines it.

**Example**
```mesh
struct Point do
  x :: Int
end

impl Printable for Point do
  fn print(self) -> String do
    "point"
  end
end

fn main() do
end
```

**Fixed**
```mesh
interface Printable do
  fn print(self) -> String
end

struct Point do
  x :: Int
end

impl Printable for Point do
  fn print(self) -> String do
    "point"
  end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0071

## E0072

**invalid-literal** · error · semantic · guide: `meshc guide types`

A numeric literal is malformed or does not fit its type.

**Repair:** Write a value in range for its type, using only the digits and `_` separators its base allows.

**Example**
```mesh
fn main() do
  let n = 99999999999999999999
  println("#{n}")
end
```

**Fixed**
```mesh
fn main() do
  let n = 9_999
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0072

## E0073

**invalid-concat** · error · semantic · guide: `meshc guide syntax`

`<>` joins strings and `++` joins lists, and these operands are neither.

**Repair:** Convert the operands to strings first, as `"#{a}#{b}"`, or use the operator their type has, such as `+` for numbers.

**Example**
```mesh
fn main() do
  let a = 1
  let b = 2
  println(a <> b)
end
```

**Fixed**
```mesh
fn main() do
  let a = 1
  let b = 2
  println("#{a}#{b}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0073

## E0074

**no-such-module-function** · error · semantic · guide: `meshc guide stdlib`

`Module.name` names a function the module does not define.

**Repair:** Correct the name to a function the module defines; the diagnostic lists them.

**Example**
```mesh
fn main() do
  println(String.upper("hi"))
end
```

**Fixed**
```mesh
fn main() do
  println(String.to_upper("hi"))
end
```

Docs: https://meshlang.dev/docs/errors/#e0074

## E0075

**overloaded-function-value** · error · semantic · guide: `meshc guide syntax`

A function defined at several arities is referenced without a call, so the reference names no single function.

**Repair:** Call it with the arguments of one arity, or wrap the arity you mean in a closure, as `fn a -> name(a) end`.

**Example**
```mesh
fn area(w :: Int) -> Int = w * w
fn area(w :: Int, h :: Int) -> Int = w * h

fn main() do
  let _f = area
end
```

**Fixed**
```mesh
fn area(w :: Int) -> Int = w * w
fn area(w :: Int, h :: Int) -> Int = w * h

fn main() do
  let _f = fn w -> area(w) end
end
```

Docs: https://meshlang.dev/docs/errors/#e0075

## E0076

**generic-impl-target** · error · semantic · guide: `meshc guide traits`

An `impl` is written for a type that takes type parameters, which Mesh does not support.

**Repair:** Implement the interface for a type without type parameters, such as a struct that holds the generic one at a concrete type.

**Example**
```mesh
interface Named do
  fn name(self) -> String
end

struct Box<T> do
  value :: T
end

impl Named for Box do
  fn name(self) -> String do
    "box"
  end
end

fn main() do
end
```

**Fixed**
```mesh
interface Named do
  fn name(self) -> String
end

struct IntBox do
  value :: Int
end

impl Named for IntBox do
  fn name(self) -> String do
    "box"
  end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0076

## E0077

**assert-receive-outside-test** · error · semantic · guide: `meshc guide testing`

`assert_receive` is expanded only in test files that `meshc test` runs.

**Repair:** Move the assertion into a `.test.mpl` file, or write the `receive ... after TIMEOUT -> ...` block it stands for.

**Example**
```mesh
fn main() do
  assert_receive 42, 500
end
```

**Fixed**
```mesh
actor worker() do
  receive do
    42 -> nil
    _ -> nil
  after 500 -> nil
  end
end
```

Docs: https://meshlang.dev/docs/errors/#e0077

## E0078

**indexing-unsupported** · error · semantic · guide: `meshc guide collections`

`value[index]` is used, and Mesh has no indexing syntax.

**Repair:** Use `List.get(list, index)`, `Map.get(map, key)`, or `Tuple.nth(tuple, index)`.

**Example**
```mesh
fn main() do
  let xs = [1, 2, 3]
  let _first = xs[0]
end
```

**Fixed**
```mesh
fn main() do
  let xs = [1, 2, 3]
  let _first = List.get(xs, 0)
end
```

Docs: https://meshlang.dev/docs/errors/#e0078

## E0079

**actor-message-type-unknown** · error · semantic · guide: `meshc guide actors`

Nothing in the module tells which type of message an actor receives, because only untyped pids reach it.

**Repair:** Give the pid a message type where the actor is spawned, as `let pid :: Pid<Int> = spawn(worker)`.

**Example**
```mesh
actor worker() do
  receive do
    msg -> msg
  end
end
```

**Fixed**
```mesh
actor worker() do
  receive do
    msg -> msg + 1
  end
end
```

Docs: https://meshlang.dev/docs/errors/#e0079

## E0080

**top-level-let** · error · semantic · guide: `meshc guide syntax`

A `let` appears at the top level of a module, which has no global bindings.

**Repair:** Move the binding into the function that uses it, or define a function of no arguments for it, as `fn limit() = 10`.

**Example**
```mesh
let limit = 10

fn main() do
  println("#{limit}")
end
```

**Fixed**
```mesh
fn limit() = 10

fn main() do
  println("#{limit()}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0080

## E0081

**module-not-imported** · error · semantic · guide: `meshc guide projects`

Code calls a function of a project module that the file does not import.

**Repair:** Add `import Module` at the top of the file.

**Example**
```mesh
module Counters do
  pub fn count() = 1
end

fn main() = Counters.count()
```

**Fixed**
```mesh
import Counters

module Counters do
  pub fn count() = 1
end

fn main() = Counters.count()
```

Docs: https://meshlang.dev/docs/errors/#e0081

## E0082

**untyped-method-parameter** · error · semantic · guide: `meshc guide traits`

A method parameter has no annotation and nothing in the method fixes its type; a method is compiled once, for the type it is implemented for.

**Repair:** Annotate the parameter with its type, as `param :: Type`.

**Example**
```mesh
interface Labeler do
  fn label(self, x) -> String
end

struct Item do
  n :: Int
end

impl Labeler for Item do
  fn label(self, x) -> String = "#{x}"
end
```

**Fixed**
```mesh
interface Labeler do
  fn label(self, x) -> String
end

struct Item do
  n :: Int
end

impl Labeler for Item do
  fn label(self, x :: Int) -> String = "#{x}"
end
```

Docs: https://meshlang.dev/docs/errors/#e0082

## E0083

**type-not-value** · error · semantic · guide: `meshc guide types`

A type's name is used where a value is expected.

**Repair:** Build a value of the type (`Name { ... }` for a struct, a variant for a sum type), or call one of its functions, as `Name.function(...)`.

**Example**
```mesh
struct Point do
  x :: Int
end

fn main() do
  let _p = Point
end
```

**Fixed**
```mesh
struct Point do
  x :: Int
end

fn main() do
  let _p = Point { x: 1 }
end
```

Docs: https://meshlang.dev/docs/errors/#e0083

## E0084

**nested-definition** · error · semantic · guide: `meshc guide syntax`

A definition such as `fn`, `struct`, or `import` appears inside a function body, where only `let` binds names.

**Repair:** Move the definition to the top level of the module. For a local function, bind a closure: `let name = fn x -> ... end`.

**Example**
```mesh
fn main() do
  fn helper() = 1
end
```

**Fixed**
```mesh
fn helper() = 1

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0084

## E0085

**type-argument-count** · error · semantic · guide: `meshc guide types`

A generic type is named with another number of type arguments than it declares.

**Repair:** Pass exactly as many type arguments as the type declares, as `Option<Int>`.

**Example**
```mesh
fn main() do
  let _x :: Option<Int, String> = None
end
```

**Fixed**
```mesh
fn main() do
  let _x :: Option<Int> = None
end
```

Docs: https://meshlang.dev/docs/errors/#e0085

## E0086

**monitor-outside-actor** · error · semantic · guide: `meshc guide actors`

`Process.monitor` or `Node.monitor` is called outside an actor, so its message has no actor to go to.

**Repair:** Call `Process.monitor` or `Node.monitor` inside the actor body that should receive the message.

**Example**
```mesh
fn main() = Node.monitor("node@localhost:9000", 1)
```

**Fixed**
```mesh
actor watcher() do
  Node.monitor("node@localhost:9000", 1)
  receive do
    n -> n + 1
  end
end
```

Docs: https://meshlang.dev/docs/errors/#e0086

## E0087

**top-level-statement** · error · semantic · guide: `meshc guide syntax`

An expression appears at the top level of a module, where nothing runs it.

**Repair:** Move the expression into `main` or into the function that should run it.

**Example**
```mesh
println("hi")

fn main() do
end
```

**Fixed**
```mesh
fn main() do
  println("hi")
end
```

Docs: https://meshlang.dev/docs/errors/#e0087

## E0088

**method-return-unknown** · error · semantic · guide: `meshc guide traits`

A method an `impl` provides has a return type nothing fixes, so a call to it cannot be typed.

**Repair:** Annotate the method's return type in the `impl` or the interface, as `fn name(self) -> Type`.

**Example**
```mesh
interface Maker do
  fn make(self)
end

struct Item do
  n :: Int
end

impl Maker for Item do
  fn make(self) = self.make()
end

fn main() = Item { n: 0 }.make()
```

**Fixed**
```mesh
interface Maker do
  fn make(self) -> Int
end

struct Item do
  n :: Int
end

impl Maker for Item do
  fn make(self) -> Int = self.n
end

fn main() = Item { n: 0 }.make()
```

Docs: https://meshlang.dev/docs/errors/#e0088

## E0089

**spawn-computed** · error · semantic · guide: `meshc guide actors`

`spawn` is given a function computed at run time, and a process starts from a compiled entry that only a name gives.

**Repair:** Name the actor in the call, as `spawn(worker, arg)`. To choose one at run time, spawn each by name where it is chosen.

**Example**
```mesh
actor worker() do
  receive do
    msg -> msg + 1
  end
end

fn pick() = worker

fn main() = spawn(pick())
```

**Fixed**
```mesh
actor worker() do
  receive do
    msg -> msg + 1
  end
end

fn pick() = worker

fn main() = spawn(worker)
```

Docs: https://meshlang.dev/docs/errors/#e0089

## E0090

**spawn-not-actor** · error · semantic · guide: `meshc guide actors`

`spawn` names a plain `fn`, and only an `actor` runs as a process of its own.

**Repair:** Define what runs in the new process as `actor name(...) do ... end` and spawn that, or call the function directly.

**Example**
```mesh
fn hello() do
  println("hi")
end

fn main() do
  let _ = spawn(hello)
end
```

**Fixed**
```mesh
actor hello() do
  println("hi")
end

fn main() do
  let _ = spawn(hello)
end
```

Docs: https://meshlang.dev/docs/errors/#e0090

## E0091

**decode-type-unknown** · error · semantic · guide: `meshc guide strings`

A generic type's `from_json` is called where nothing fixes its type arguments.

**Repair:** Annotate the result with the full type, as `let r :: Result<Box<Int>, String> = Box.from_json(text)`.

**Example**
```mesh
struct Box<T> do
  value :: T
end deriving(Json)

fn main() do
  let _r = Box.from_json("{}")
end
```

**Fixed**
```mesh
struct Box<T> do
  value :: T
end deriving(Json)

fn main() do
  let _r :: Result<Box<Int>, String> = Box.from_json("{}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0091

## E0092

**underivable-field-type** · error · semantic · guide: `meshc guide traits`

A derived interface needs every field to implement it, and one field's type does not.

**Repair:** Derive or implement the interface for the field's type, or remove the interface from the `deriving` list.

**Example**
```mesh
struct Inner do
  x :: Int
end deriving(Display)

struct Outer do
  inner :: Inner
end deriving(Eq)

fn main() do
end
```

**Fixed**
```mesh
struct Inner do
  x :: Int
end deriving(Display, Eq)

struct Outer do
  inner :: Inner
end deriving(Eq)

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0092

## E0093

**plaintext-violation** · error · semantic · guide: `meshc guide types`

A `Plaintext` value, or one holding it, reaches a way out of the program other than a seal, an `@display` export, or `declassify`.

**Repair:** Seal the value with `Crypto.aead_seal_plaintext`, `Crypto.hpke_seal_plaintext`, or `Plaintext.seal_for_storage`, export it through `@display`, or release it with `declassify(value, "reason")`. Transform it in place with `Plaintext.map`.

**Example**
```mesh
fn show(text :: Plaintext<String>) = declassify(text, "")
```

**Fixed**
```mesh
fn show(text :: Plaintext<String>) = declassify(text, "explicit display")
```

Docs: https://meshlang.dev/docs/errors/#e0093

## E0094

**invalid-jobs-start** · error · semantic · guide: `meshc guide database`

`Jobs.start` is used indirectly or given a handler the compiler cannot check.

**Repair:** Call `Jobs.start` directly, and pass a named function of type `fn(String) -> String` as its handler.

**Example**
```mesh
fn start(pool :: PoolHandle) do
  Jobs.start(pool, "{}", fn job -> job end)
end
```

**Fixed**
```mesh
fn handle(job :: String) -> String = job

fn start(pool :: PoolHandle) do
  Jobs.start(pool, "{}", handle)
end
```

Docs: https://meshlang.dev/docs/errors/#e0094

## E0095

**invalid-nat-index** · error · semantic · guide: `meshc guide types`

A static `Nat` index is malformed: an index parameter is not of kind `Nat` or is not on a sum type, variant, or function; a literal does not fit in 64 bits; a type and a Nat index stand in each other's place; or an index escapes the case arm that binds it.

**Repair:** Declare each index parameter as `n :: Nat` on a sum type, variant, or function, and write each index from `Nat` literals, those parameters, and `Succ<...>`, with types only in type positions. Return a value whose type names no index bound inside a case arm.

**Example**
```mesh
fn count<n :: Int>() -> Int = 0

fn main() do
end
```

**Fixed**
```mesh
fn count<n :: Nat>() -> Int = 0

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0095

## E0096

**invalid-indexed-type** · error · semantic · guide: `meshc guide types`

An indexed sum type, one of its constructors, or a function with index parameters breaks a rule of indexed types, or an indexed type is used without its arguments or matched by a constructor it cannot have.

**Repair:** Give each constructor of an indexed type a result `-> Family<...>` of its own family that keeps the type parameters and binds every constructor index; write the family with all its arguments; declare index parameters once, without trait bounds, on a function of one clause; derive nothing; and match its constructors in a direct `case` arm.

**Example**
```mesh
type Vec<T, n :: Nat> do
  Empty
  Push<k :: Nat>(T, Vec<T, k>) -> Vec<T, Succ<k>>
end

fn main() do
end
```

**Fixed**
```mesh
type Vec<T, n :: Nat> do
  Empty -> Vec<T, 0>
  Push<k :: Nat>(T, Vec<T, k>) -> Vec<T, Succ<k>>
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0096

## E0097

**reserved-proof-name** · error · semantic · guide: `meshc guide verification`

A struct, sum type, or type alias is named `Proof`, the name of the type of proof values.

**Repair:** Rename the type, for example to `Evidence`, and update its uses.

**Example**
```mesh
struct Proof do
  value :: Int
end

fn main() do
end
```

**Fixed**
```mesh
struct Evidence do
  value :: Int
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0097

## E0098

**missing-embedded-proof** · error · semantic · guide: `meshc guide verification`

A `pub theorem` in a `logic do` block has no `proof` of the same name in the module, so there is no proof value to export.

**Repair:** Write `proof name do ... end` for the theorem in a `logic do` block of the same module, or remove `pub` to keep the theorem out of Mesh code.

**Example**
```mesh
logic do
  pub theorem two() do 2 == 2 end
end

fn main() do
end
```

**Fixed**
```mesh
logic do
  pub theorem two() do 2 == 2 end
  proof two do refl end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0098

## E0099

**duplicate-proof-value** · error · semantic · guide: `meshc guide verification`

Two `pub theorem`s in one module's `logic do` blocks have the same name, so the name would stand for two proof values.

**Repair:** Rename one of the theorems and its `proof`, and update the calls that use it.

**Example**
```mesh
logic do
  pub theorem two() do 2 == 2 end
  proof two do refl end
  pub theorem two() do 1 + 1 == 2 end
end

fn main() do
end
```

**Fixed**
```mesh
logic do
  pub theorem two() do 2 == 2 end
  proof two do refl end
  pub theorem sum() do 1 + 1 == 2 end
  proof sum do refl end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0099

## E0100

**invalid-proposition** · error · semantic · guide: `meshc guide verification`

A proposition has no Mesh proof type: an exported theorem or a `Proof<...>` annotation states something other than a known `Prop` predicate of Nat arguments or a `NatEq` of two Nat terms, or a theorem that takes parameters is used without them.

**Repair:** State the proposition as a predicate declared `-> Prop` in a `logic do` block, applied to Nat literals, `Nat.succ`, or index parameters, or as an equality of Nat terms (`Proof<NatEq<a, b>>` in a type). Call a theorem with parameters as `name(indices..., proofs...)`.

**Example**
```mesh
fn accept(evidence :: Proof<NatEq<2>>) -> Int = 2

fn main() do
end
```

**Fixed**
```mesh
fn accept(evidence :: Proof<NatEq<2, 2>>) -> Int = 2

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0100

## E0101

**resource-crosses-boundary** · error · semantic · guide: `meshc guide types`

A resource reaches an operation that cannot preserve its single owner, such as an actor message, job, collection, or serialization.

**Repair:** Keep the resource in an ownership-aware scope and pass an unrestricted value instead, such as an ID or the data the resource represents.

**Example**
```mesh
resource struct Handle do
  id :: Int
end

fn forward(pid :: Pid, handle :: Handle) = send(pid, handle)
```

**Fixed**
```mesh
resource struct Handle do
  id :: Int
end

fn forward(pid :: Pid, handle :: Handle) = send(pid, handle.id)
```

Docs: https://meshlang.dev/docs/errors/#e0101

## E0102

**duplicate-type-across-modules** · error · semantic · guide: `meshc guide projects`

Two modules define the same struct, sum type, interface, actor, service, or supervisor name. These declarations share a project-wide namespace, even when private.

**Repair:** Rename all but one definition and update its uses, or keep one shared definition and import it where needed.

**Example**
```mesh
module A do
  struct State do
    count :: Int
  end
end

module B do
  struct State do
    name :: String
  end
end
```

**Fixed**
```mesh
module A do
  struct State do
    count :: Int
  end
end

module B do
  struct NamedState do
    name :: String
  end
end
```

Docs: https://meshlang.dev/docs/errors/#e0102

## E0103

**duplicate-pub-fn-across-modules** · error · semantic · guide: `meshc guide projects`

Two modules export a public function with the same compiler symbol. Calls could run only one of the definitions even when the import names the other module.

**Repair:** Rename all but one public function and update its callers, or make the other functions private if their callers are in the same module.

**Example**
```mesh
module A do
  pub fn count() do
    1
  end
end

module B do
  pub fn count() do
    2
  end
end
```

**Fixed**
```mesh
module A do
  pub fn count() do
    1
  end
end

module B do
  pub fn total() do
    2
  end
end
```

Docs: https://meshlang.dev/docs/errors/#e0103

## E0104

**recursive-by-value-type** · error · semantic · guide: `meshc guide types`

A struct holds itself by value, directly or through other structs, so a value of it would contain itself and could never be built.

**Repair:** Hold the recursive field through a type that ends the recursion: an `Option<...>` or a `List<...>` of the struct, or a sum type with a variant that does not hold it.

**Example**
```mesh
struct Node do
  next :: Node
end

fn main() do
end
```

**Fixed**
```mesh
struct Node do
  next :: Option<Node>
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0104

## E0105

**runtime-symbol-collision** · error · semantic · guide: `meshc guide projects`

A public function or a native binding takes a symbol that the Mesh runtime, the compiler, or another native binding already uses, so calls could reach the wrong code.

**Repair:** Rename the function, leaving names that start with `mesh_` to the runtime, and bind each native symbol once with one signature.

**Example**
```mesh
pub fn mesh_string_length(text :: String) -> Int = 0

fn main() do
  println("${mesh_string_length("abc")}")
end
```

**Fixed**
```mesh
pub fn text_length(text :: String) -> Int = 0

fn main() do
  println("${text_length("abc")}")
end
```

Docs: https://meshlang.dev/docs/errors/#e0105

## E0106

**spawn-grouped-unsafe** · error · semantic · guide: `meshc guide actors`

An actor started with `spawn grouped`, or its `terminate` callback, can reach code the compiler cannot prove yields or returns without blocking the shared worker (a native or unaudited runtime call such as I/O, an indirect call, recursion that grows the stack, or a loop in a terminate callback), or `spawn grouped` starts a supervisor child.

**Repair:** Start the actor with plain `spawn`, or move the call the notes trace into an actor started with `spawn` and message it. Write recursion as a tail call, and keep loops out of terminate callbacks.

**Example**
```mesh
actor worker() do
  IO.eprintln("working")
end

fn main() do
  spawn grouped(worker)
end
```

**Fixed**
```mesh
actor worker() do
  IO.eprintln("working")
end

fn main() do
  spawn(worker)
end
```

Docs: https://meshlang.dev/docs/errors/#e0106

## E0107

**node-spawn-argument** · error · semantic · guide: `meshc guide clustering`

`Node.spawn` or `Node.spawn_link` is given a closure instead of a function defined at the top level, or an argument of a type that cannot cross the connection.

**Repair:** Name a top-level actor as the second argument, and pass it only `Int`, `Float`, `Bool`, `String`, `Pid`, or `()` arguments, encoding other data as a `String`.

**Example**
```mesh
actor spawner(host :: String) do
  let _ = Node.spawn(host, fn () -> self() end)
end

fn main() do
  let _ = spawn(spawner, "absent@127.0.0.1:1")
end
```

**Fixed**
```mesh
actor remote() do
  println("started")
end

actor spawner(host :: String) do
  let _ = Node.spawn(host, remote)
end

fn main() do
  let _ = spawn(spawner, "absent@127.0.0.1:1")
end
```

Docs: https://meshlang.dev/docs/errors/#e0107

## E0108

**export-symbol-collision** · error · semantic · guide: `meshc guide projects`

An `@export` C symbol is also another export's, a public Mesh function's, or the Mesh runtime's, so a host could not tell which one it calls.

**Repair:** Give each `@export` its own C symbol, prefixed with the library's name as in `@export("greeter_echo")`, that no public function or `mesh_` runtime symbol uses.

**Example**
```mesh
@export("mesh_println")
pub fn echo(request :: Bytes) -> Bytes ! String do
  Ok(request)
end

fn main() do
end
```

**Fixed**
```mesh
@export("greeter_echo")
pub fn echo(request :: Bytes) -> Bytes ! String do
  Ok(request)
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#e0108

## I0001

**internal-compiler-error** · error · internal

The compiler reached a state it should never reach; the program may well be valid.

**Repair:** Report it with the notes and the smallest program that reproduces it. Until it is fixed, rewrite the construct the notes name another way.

Docs: https://meshlang.dev/docs/errors/#i0001

## I0002

**internal-consistency-warning** · warning · internal

During lowering, the compiler could not confirm the target trait method for a call that passed type checking.

**Repair:** Report the method name, receiver type, and a small program that reproduces the warning. Use an explicit function call if it resolves the intended method.

Docs: https://meshlang.dev/docs/errors/#i0002

## L0001

**deep-nesting** · warning · lint · guide: `meshc guide syntax`

An `if`, `case`, loop, `receive`, or closure is nested more than four levels deep in one function, counted from the function, handler, actor body, or top-level test block that holds it.

**Repair:** Move the construct the label points at into a function of its own and call it, return early from the enclosing function, or use `?` in place of a `case` that only passes an error on.

**Example**
```mesh
fn main() do
  let n = 5
  if n > 0 do
    if n > 1 do
      if n > 2 do
        if n > 3 do
          if n > 4 do
            println("deep")
          end
        end
      end
    end
  end
end
```

**Fixed**
```mesh
fn report(n :: Int) do
  if n > 4 do
    println("deep")
  end
end

fn main() do
  let n = 5
  if n > 0 do
    if n > 1 do
      if n > 2 do
        if n > 3 do
          report(n)
        end
      end
    end
  end
end
```

Docs: https://meshlang.dev/docs/errors/#l0001

## L0002

**collapsible-else-if** · warning · lint · guide: `meshc guide syntax`

The whole body of an `else` is one `if` expression, which `else if` says one level shallower.

**Repair:** Join `else` and the `if` it holds into `else if ... do`, outdent that `if`'s branches one level, and delete the `end` that closed the `else`, so the inner `if`'s `end` closes the whole chain.

**Example**
```mesh
fn sign(n :: Int) -> String do
  if n > 0 do
    "positive"
  else
    if n < 0 do
      "negative"
    else
      "zero"
    end
  end
end

fn main() do
  println(sign(-1))
end
```

**Fixed**
```mesh
fn sign(n :: Int) -> String do
  if n > 0 do
    "positive"
  else if n < 0 do
    "negative"
  else
    "zero"
  end
end

fn main() do
  println(sign(-1))
end
```

Docs: https://meshlang.dev/docs/errors/#l0002

## L0003

**pass-through-arm** · warning · lint · guide: `meshc guide pattern-matching`

A `case` arm's value is its own pattern written again, as in `Ok(n) -> Ok(n)`, which the pattern alone already means.

**Repair:** Delete the `->` and the value, and leave the pattern on its own as the arm: `Ok(n)`.

**Example**
```mesh
fn check(r) do
  case r do
    Ok(n) -> Ok(n)
    Err(e) -> Err("check failed: #{e}")
  end
end

fn main() do
  let _r = check(Err("no input"))
end
```

**Fixed**
```mesh
fn check(r) do
  case r do
    Ok(n)
    Err(e) -> Err("check failed: #{e}")
  end
end

fn main() do
  let _r = check(Err("no input"))
end
```

Docs: https://meshlang.dev/docs/errors/#l0003

## L0004

**bool-comparison** · warning · lint · guide: `meshc guide syntax`

A value is compared with `true` or `false` using `==` or `!=`, which is the value itself or its negation.

**Repair:** Write the value alone in place of `== true` or `!= false`, and `not` the value in place of `== false` or `!= true`.

**Example**
```mesh
fn main() do
  let ready = true
  if ready == true do
    println("ready")
  end
end
```

**Fixed**
```mesh
fn main() do
  let ready = true
  if ready do
    println("ready")
  end
end
```

Docs: https://meshlang.dev/docs/errors/#l0004

## M0001

**manifest-unreadable** · error · project · guide: `meshc guide projects`

The project manifest could not be opened or read as UTF-8.

**Repair:** Create mesh.toml in the project directory or restore read access to it.

Docs: https://meshlang.dev/docs/errors/#m0001

## M0002

**manifest-invalid** · error · project · guide: `meshc guide projects`

The manifest contains invalid TOML or violates the package schema.

**Repair:** Correct the indicated field in mesh.toml. Include a [package] table with string name and version fields.

Docs: https://meshlang.dev/docs/errors/#m0002

## M0003

**entrypoint-not-found** · error · project · guide: `meshc guide projects`

The project does not contain the expected Mesh entrypoint file.

**Repair:** Set [package].entrypoint to an existing project-relative .mpl file or remove it and create main.mpl.

Docs: https://meshlang.dev/docs/errors/#m0003

## M0101

**lockfile-invalid** · error · project · guide: `meshc guide projects`

The mesh.lock file could not be read, parsed, or written.

**Repair:** Fix the indicated TOML or restore access to mesh.lock, then rerun the dependency command.

Docs: https://meshlang.dev/docs/errors/#m0101

## M0102

**dependency-unresolvable** · error · project · guide: `meshc guide projects`

A dependency could not be located, fetched, or checked out.

**Repair:** Correct the dependency path or Git source in mesh.toml and run meshc deps, or run meshpkg install for a registry dependency.

Docs: https://meshlang.dev/docs/errors/#m0102

## M0103

**dependency-cycle** · error · project · guide: `meshc guide projects`

The package dependency chain returns to a package it already visits.

**Repair:** Remove a dependency from the cycle or move shared code into a package both sides can depend on.

Docs: https://meshlang.dev/docs/errors/#m0103

## M0104

**dependency-conflict** · error · project · guide: `meshc guide projects`

The same dependency name is required from different sources.

**Repair:** Update the dependency declarations to use a single source for that name.

Docs: https://meshlang.dev/docs/errors/#m0104

## M0201

**module-cycle** · error · project · guide: `meshc guide projects`

The import chain returns to a module it already visits.

**Repair:** Remove an import from the cycle or move shared definitions to a separate module.

Docs: https://meshlang.dev/docs/errors/#m0201

## M0202

**module-unreadable** · error · project · guide: `meshc guide projects`

A module could not be read or assigned an unambiguous name inside the project.

**Repair:** Check the module path and its read permissions. Use a uniquely named .mpl file inside the project with no symbolic links.

Docs: https://meshlang.dev/docs/errors/#m0202

## M0301

**native-binding-invalid** · error · project · guide: `meshc guide projects`

A manifest-declared native binding could not be loaded as a package file.

**Repair:** Set [native].bindings to package-relative paths to regular .mpl files outside the test tree.

Docs: https://meshlang.dev/docs/errors/#m0301

## M0302

**native-declaration-outside-binding** · error · project · guide: `meshc guide projects`

A native declaration occurs in a module that is not listed as a native binding.

**Repair:** Move the declaration into a file listed in [native].bindings or add its file to that list and provide the native archive.

Docs: https://meshlang.dev/docs/errors/#m0302

## M0303

**native-archive-missing** · error · project · guide: `meshc guide projects`

The requested target has no readable, checksum-verified static archive in the native package.

**Repair:** Provide the static archive for the requested target and record its package-relative path and SHA-256 in [[native.libraries]].

Docs: https://meshlang.dev/docs/errors/#m0303

## P0001

**unexpected-token** · error · syntax · guide: `meshc guide syntax`

The parser reached a token the grammar does not allow at this point.

**Repair:** Remove the token, or replace it with what the label says the grammar expects here.

**Example**
```mesh
fn main() do
  let = 1
end
```

**Fixed**
```mesh
fn main() do
  let _x = 1
end
```

Docs: https://meshlang.dev/docs/errors/#p0001

## P0002

**expected-token** · error · syntax · guide: `meshc guide syntax`

The grammar requires one particular token here, such as `do`, `->`, `in`, or a name, and the source has another one.

**Repair:** Write the token the message expects where the label points, or replace the token found there with it.

**Example**
```mesh
fn main() do
  for x [1, 2] do
    println("#{x}")
  end
end
```

**Fixed**
```mesh
fn main() do
  for x in [1, 2] do
    println("#{x}")
  end
end
```

Docs: https://meshlang.dev/docs/errors/#p0002

## P0003

**unclosed-delimiter** · error · syntax · guide: `meshc guide syntax`

A `(`, `[`, `{`, or string interpolation is still open where its contents end.

**Repair:** Add the matching `)`, `]`, or `}` where the contents end; the related span shows where the delimiter was opened. If the contents are a list, separate its items with `,`.

**Example**
```mesh
fn main() do
  println("hi"
end
```

**Fixed**
```mesh
fn main() do
  println("hi")
end
```

Docs: https://meshlang.dev/docs/errors/#p0003

## P0004

**missing-end** · error · syntax · guide: `meshc guide syntax`

A block opened with `do`, such as a function, `if`, `case`, loop, closure, or type body, has no `end` that closes it.

**Repair:** Add `end` where the block's body ends; the related span shows the `do` it closes. Each `do` takes its own `end`, so count them in nested blocks.

**Example**
```mesh
fn main() do
  println("hi")
```

**Fixed**
```mesh
fn main() do
  println("hi")
end
```

Docs: https://meshlang.dev/docs/errors/#p0004

## P0101

**unknown-character** · error · syntax · guide: `meshc guide syntax`

The source has a character that starts no Mesh token, such as `$`, `^`, or a backtick, outside a string or comment.

**Repair:** Delete the character, or move it into a string or a comment. For an operator, write one Mesh has, such as `*`, `&&`, or `<>`.

**Example**
```mesh
fn main() do
  let n = 2 ^ 3
  println("#{n}")
end
```

**Fixed**
```mesh
fn main() do
  let n = 2 * 3
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#p0101

## P0102

**unterminated-string** · error · syntax · guide: `meshc guide syntax`

A string literal has no closing quote, so it runs to the end of the file.

**Repair:** Add the closing `"` (`"""` for a triple-quoted string) where the text ends. Inside the string, write a quote as `\"`.

**Example**
```mesh
fn main() do
  println("hello)
end
```

**Fixed**
```mesh
fn main() do
  println("hello")
end
```

Docs: https://meshlang.dev/docs/errors/#p0102

## P0103

**unterminated-interpolation** · error · syntax · guide: `meshc guide syntax`

A string interpolation opened with `#{` or `${` has no closing `}`, so it runs to the end of the file.

**Repair:** Close the interpolation with `}` after its expression, then close the string with `"`.

**Example**
```mesh
fn main() do
  let x = 1
  println("x is #{x
```

**Fixed**
```mesh
fn main() do
  let x = 1
  println("x is #{x}")
end
```

Docs: https://meshlang.dev/docs/errors/#p0103

## P0104

**unterminated-block-comment** · error · syntax · guide: `meshc guide syntax`

A block comment opened with `#=` has no closing `=#`, so it runs to the end of the file. Block comments nest: each `#=` inside one needs its own `=#`.

**Repair:** Add `=#` where the comment ends, one for each `#=` it contains.

**Example**
```mesh
#= Prints a greeting.
fn main() do
  println("hi")
end
```

**Fixed**
```mesh
#= Prints a greeting. =#
fn main() do
  println("hi")
end
```

Docs: https://meshlang.dev/docs/errors/#p0104

## P0105

**invalid-slot-pipe** · error · syntax · guide: `meshc guide syntax`

A pipe is written `|0>`, `|1>`, or `|N` without `>`. Slot pipes are numbered from 2, because `|>` already pipes into the first argument.

**Repair:** Write `|>` to pipe into the first argument, or `|N>` with N from 2 up to pipe into argument N.

**Example**
```mesh
fn add(a :: Int, b :: Int) -> Int = a + b

fn main() do
  let n = 1 |1> add(2)
  println("#{n}")
end
```

**Fixed**
```mesh
fn add(a :: Int, b :: Int) -> Int = a + b

fn main() do
  let n = 1 |> add(2)
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#p0105

## P0106

**invalid-regex-literal** · error · syntax · guide: `meshc guide syntax`

A `~` does not start a well-formed regex literal `~r/pattern/flags`: the `r/` is missing, the pattern has no closing `/`, or a flag is not `i`, `m`, or `s`.

**Repair:** Write the literal as `~r/pattern/flags`: escape a `/` in the pattern as `\/`, end the pattern with `/`, and use only the flags `i`, `m`, and `s`.

**Example**
```mesh
fn main() do
  let rx = ~r/[a-z]+/x
  println("#{Regex.is_match(rx, "abc")}")
end
```

**Fixed**
```mesh
fn main() do
  let rx = ~r/[a-z]+/i
  println("#{Regex.is_match(rx, "abc")}")
end
```

Docs: https://meshlang.dev/docs/errors/#p0106

## P0107

**single-ampersand** · error · syntax · guide: `meshc guide syntax`

A lone `&` stands where an operator is expected. Mesh has no `&` operator: logical and is `&&` or `and`.

**Repair:** Write `&&` or `and` for a logical and.

**Example**
```mesh
fn main() do
  let ok = true & false
  println("#{ok}")
end
```

**Fixed**
```mesh
fn main() do
  let ok = true && false
  println("#{ok}")
end
```

Docs: https://meshlang.dev/docs/errors/#p0107

## P0201

**law-syntax** · error · syntax · guide: `meshc guide verification`

A `.law` file does not follow the law grammar: `from Module import name` lines, then `law` declarations written like functions, then `proof` blocks.

**Repair:** Write target imports first, then `law name(param :: Type, ...) -> Type do ... end` for each imported function, then `proof name do ... end` with `refl`, `exact`, `congr`, `induction`, or `cases`, one entry per line.

Docs: https://meshlang.dev/docs/errors/#p0201

## P0202

**law-too-large** · error · syntax · guide: `meshc guide verification`

A law body or proof in a `.law` file nests deeper than the law parser accepts.

**Repair:** Flatten the deepest part: move a nested expression into a helper function of the module under test, or split a deep proof into lemmas used with `exact`.

Docs: https://meshlang.dev/docs/errors/#p0202

## P0301

**logic-syntax** · error · syntax · guide: `meshc guide verification`

A `logic do ... end` block or a `.law` file starting with `logic` does not follow the logic grammar of `from` imports, then `inductive`, `fn`, `theorem`, and `proof` declarations.

**Repair:** Write the token the message expects where the label points: separate entries with new lines or `;`, close each `do` with `end`, write equality as `==`, and annotate binders as `name :: Type`.

**Example**
```mesh
logic do
  theorem two() do 2 = 2 end
end

fn main() do
end
```

**Fixed**
```mesh
logic do
  theorem two() do 2 == 2 end
end

fn main() do
end
```

Docs: https://meshlang.dev/docs/errors/#p0301

## P0302

**logic-too-large** · error · syntax · guide: `meshc guide verification`

A logic block or file nests terms or proofs deeper than the logic parser accepts, or is larger than 4 GiB.

**Repair:** Name the deepest subterm with `let` or a separate `fn` declaration, split a deep proof into lemmas used with `exact`, or split the file.

Docs: https://meshlang.dev/docs/errors/#p0302

## P0401

**test-file-syntax** · error · syntax · guide: `meshc guide testing`

A `describe` block in a `.test.mpl` file has its `setup` after one of its tests, or more than one `teardown`.

**Repair:** Put a describe's `setup` before its first `test` or nested `describe`, and merge its teardowns into one `teardown do ... end`.

Docs: https://meshlang.dev/docs/errors/#p0401

## W0001

**redundant-arm** · warning · semantic · guide: `meshc guide pattern-matching`

A match arm can never run, because earlier arms already match every value it would.

**Repair:** Remove the arm, or move it before the arm that covers it.

**Example**
```mesh
fn main() do
  let n = case 1 do
    _ -> 0
    1 -> 2
  end
  println("#{n}")
end
```

**Fixed**
```mesh
fn main() do
  let n = case 1 do
    1 -> 2
    _ -> 0
  end
  println("#{n}")
end
```

Docs: https://meshlang.dev/docs/errors/#w0001

## W0002

**non-first-clause-annotation** · warning · semantic · guide: `meshc guide syntax`

Visibility, type parameters, or a return type are written on a clause other than the first of a multi-clause function, where they have no effect.

**Repair:** Write the annotation on the first clause only, and remove it from the later ones.

**Example**
```mesh
fn label(0) -> String = "zero"
fn label(n) -> String = "other"

fn main() do
  println(label(0))
end
```

**Fixed**
```mesh
fn label(0) -> String = "zero"
fn label(n) = "other"

fn main() do
  println(label(0))
end
```

Docs: https://meshlang.dev/docs/errors/#w0002

## W0003

**non-exhaustive-clauses** · warning · semantic · guide: `meshc guide pattern-matching`

A multi-clause function or closure has no clause for some arguments, and a call with one of them panics at run time.

**Repair:** Add a clause for each missing pattern, or a final catch-all clause.

**Example**
```mesh
fn label(0) = "zero"
fn label(1) = "one"

fn main() do
  println(label(0))
end
```

**Fixed**
```mesh
fn label(0) = "zero"
fn label(n) = "other"

fn main() do
  println(label(0))
end
```

Docs: https://meshlang.dev/docs/errors/#w0003

## Retired codes

Mesh never reuses retired IDs. Retired IDs are `E0013`, `E0024`, `E0025`, `E0043`.
