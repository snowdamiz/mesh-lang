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

Docs: https://meshlang.dev/docs/errors/#e0034

## E0035

**private-item** · error · semantic · guide: `meshc guide projects`

An import names a definition its module does not mark `pub`.

**Repair:** Add `pub` to the definition in its module, or stop importing it.

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

Docs: https://meshlang.dev/docs/errors/#e0046

## E0047

**clustered-route-invalid-arguments** · error · semantic · guide: `meshc guide clustering`

`HTTP.clustered(...)` received arguments other than an optional replication count and a handler reference.

**Repair:** Write `HTTP.clustered(handler)` or `HTTP.clustered(count, handler)`, where `handler` names a public top-level route handler.

Docs: https://meshlang.dev/docs/errors/#e0047

## E0048

**clustered-route-private-handler** · error · semantic · guide: `meshc guide clustering`

`HTTP.clustered(...)` wraps a handler that is not `pub`, so other nodes cannot run it.

**Repair:** Add `pub` to the handler's definition, or remove the `HTTP.clustered(...)` wrapper.

Docs: https://meshlang.dev/docs/errors/#e0048

## E0049

**clustered-route-misplaced** · error · semantic · guide: `meshc guide clustering`

`HTTP.clustered(...)` appears somewhere other than the handler argument of `HTTP.route(...)` or `HTTP.on_*(...)`.

**Repair:** Pass `HTTP.clustered(handler)` directly as the handler argument of `HTTP.route(...)` or `HTTP.on_*(...)`.

Docs: https://meshlang.dev/docs/errors/#e0049

## E0050

**clustered-route-conflicting-count** · error · semantic · guide: `meshc guide clustering`

One clustered route handler is wrapped with different replication counts in different places.

**Repair:** Use one replication count for the handler everywhere it is wrapped in `HTTP.clustered(...)`.

Docs: https://meshlang.dev/docs/errors/#e0050

## E0051

**clustered-route-origin-missing** · error · semantic · guide: `meshc guide clustering`

A handler imported by bare name into `HTTP.clustered(...)` has lost the module that defines it, so its runtime name is unknown.

**Repair:** Import the handler's module and wrap the handler by its qualified name, as `HTTP.clustered(Module.handler)`.

Docs: https://meshlang.dev/docs/errors/#e0051

## E0052

**invalid-native-declaration** · error · semantic · guide: `meshc guide projects`

A `@native` function declaration is private, generic, not fully annotated, or names a symbol that is not a C identifier.

**Repair:** Declare it as a public, non-generic function with every parameter and the return type annotated, and a C identifier as its symbol.

Docs: https://meshlang.dev/docs/errors/#e0052

## E0053

**resource-ownership-violation** · error · semantic · guide: `meshc guide types`

A resource value, which has exactly one owner, is used after it moved or crosses a boundary it cannot cross.

**Repair:** Move each resource once, or pass it to a direct `borrow` parameter. Across an actor, job, or JSON boundary, pass what the resource stands for instead, such as an id or the data it holds.

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

Docs: https://meshlang.dev/docs/errors/#e0055

## E0056

**invalid-pass-through-arm** · error · semantic · guide: `meshc guide pattern-matching`

A match arm with no `->` gives its pattern as its value, and this pattern cannot stand for one.

**Repair:** Write the arm's value after `->`, as `pattern -> value`.

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

Docs: https://meshlang.dev/docs/errors/#e0065

## E0066

**ambiguous-static-method** · error · semantic · guide: `meshc guide traits`

A static interface method, one without `self`, is called bare while several types provide it.

**Repair:** Call it on the type that should provide it, as `Type.method()`.

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

Docs: https://meshlang.dev/docs/errors/#e0081

## E0082

**untyped-method-parameter** · error · semantic · guide: `meshc guide traits`

A method parameter has no annotation and nothing in the method fixes its type; a method is compiled once, for the type it is implemented for.

**Repair:** Annotate the parameter with its type, as `param :: Type`.

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

Docs: https://meshlang.dev/docs/errors/#e0088

## E0089

**spawn-computed** · error · semantic · guide: `meshc guide actors`

`spawn` is given a function computed at run time, and a process starts from a compiled entry that only a name gives.

**Repair:** Name the actor in the call, as `spawn(worker, arg)`. To choose one at run time, spawn each by name where it is chosen.

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

Docs: https://meshlang.dev/docs/errors/#e0093

## E0094

**invalid-jobs-start** · error · semantic · guide: `meshc guide database`

`Jobs.start` is used indirectly or given a handler the compiler cannot check.

**Repair:** Call `Jobs.start` directly, and pass a named function of type `fn(String) -> String` as its handler.

Docs: https://meshlang.dev/docs/errors/#e0094

## E0095

**invalid-nat-index** · error · semantic · guide: `meshc guide types`

A static `Nat` index or a theorem application is malformed or does not fit what its declaration states.

**Repair:** Write the index from `Nat` literals, index parameters, and `Succ<...>`, and apply each theorem to exactly the arguments its statement declares.

Docs: https://meshlang.dev/docs/errors/#e0095

## I0001

**internal-compiler-error** · error · internal

The compiler reached a state it should never reach; the program may well be valid.

**Repair:** Report it with the notes and the smallest program that reproduces it. Until it is fixed, rewrite the construct the notes name another way.

Docs: https://meshlang.dev/docs/errors/#i0001

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
