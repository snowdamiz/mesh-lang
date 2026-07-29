---
title: Syntax Cheatsheet
description: Quick Mesh syntax reference for variables, types, functions, actors, HTTP, modules, and testing.
---

# Syntax Cheatsheet

A quick reference for Mesh syntax. For details, see the full guides linked in each section.

## Basics

| Syntax | Example |
|--------|---------|
| Variable binding | `let x = 42` |
| Type annotation | `x :: Int` |
| String interpolation | `"Hello, #{name}!"` or `"Hello, ${name}!"` |
| Heredoc string | `"""multiline #{expr} content"""` |
| Comment | `# this is a comment` |
| Doc comment | `## documents the next declaration` |
| Module doc | `##! documents this module` |
| Nested block comment | `#= outer #= inner =# =#` |
| Tuple binding | `let (name, age) = ("Ada", 36)` |
| Ignore a value | `let _ = do_work()` |
| Multiple statements | `let x = 1; let y = 2` |
| Print | `println("hello")` |

## Types

| Type | Example |
|------|---------|
| `Int` | `42`, `0`, `-5` |
| `Float` | `3.14`, `0.5` |
| `String` | `"hello"`, `"#{x}"` |
| `Bool` | `true`, `false` |
| `Bytes` | `Bytes.from_utf8("hello")` |
| `U64`, `U128`, `I128` | `U64.parse("18446744073709551615")` |
| `Json` | `json { status: "ok" }` |
| `Atom` | `:ok`, `:not_found` |
| `Regex` | `~r/[a-z]+/i` |
| Unit | `()`, `nil` |
| Tuple | `(1, "one")`; type `(Int, String)` |
| `List<T>` | `[1, 2, 3]` |
| `Map<K, V>` | `%{"key" => "value"}` |
| `Set` | `Set.new()` (integer values) |
| `Range` | `Range.new(0, 10)`; use `0..10` directly in `for` |
| `Queue` | `Queue.new()` (integer values) |
| `Pid<M>` | returned by `spawn(...)` |
| `Option<T>` | `Some(42)`, `None` (shorthand: `Int?`) |
| `Result<T, E>` | `Ok(42)`, `Err("fail")` (shorthand: `Int!String`) |
| `Fun(A) -> B` | `Fun(Int) -> String` |

Integer literals also support separators and radices: `1_000_000`, `0xff`, `0b1010`, and `0o755`. Floats support exponent notation such as `1.25e3`.

## String Features

```mesh
# Hash-brace interpolation (v12.0, preferred)
let name = "World"
println("Hello, #{name}!")
println("Expr: #{count * 2 + 1}")

# Dollar-brace interpolation (also valid)
println("Hello, ${name}!")

# Heredoc strings (multiline; ordinary quotes and newlines can appear directly)
let body = """
  SELECT * FROM events WHERE id = #{id}
  """

# JSON object literals (prefer over heredoc JSON templates)
let resp = json { status: "ok", count: n }          # {"status":"ok","count":42}
let err  = json { error: reason }                    # {"error":"not found"}
let nest = json { result: json { code: 200 } }       # {"result":{"code":200}}
HTTP.response(200, json { status: "ok", id: id })   # Json is String-compatible at call sites

# Regex literals
let rx = ~r/\d+/
let rx_flags = ~r/[a-z]+/i     # i, m, s flags
let matched = Regex.is_match(rx, "hello123")

# Environment variables
let host = Env.get("HOST", "localhost")
let port = Env.get_int("PORT", 8080)
```

See [Language Basics](/docs/language-basics/) for details.

## Functions

```mesh
# Named function with types
fn add(a :: Int, b :: Int) -> Int do
  a + b
end

# def is a synonym for fn on named functions
def double(n :: Int) -> Int = n * 2

# Generic function and trait bound
fn identity<T>(value :: T) -> T = value
fn render<T>(value :: T) -> String where T: Display = value.to_string()

# Multi-clause (pattern matching)
fn fib(0) = 0
fn fib(1) = 1
fn fib(n) = fib(n - 1) + fib(n - 2)

# Guards
fn abs(n) when n < 0 = -n
fn abs(n) = n

# Direct self-calls in tail position are lowered to a loop
fn sum_to(n, total) do
  if n <= 0 do
    total
  else
    sum_to(n - 1, total + n)
  end
end

# Anonymous function (closure)
let double = fn(x :: Int) -> x * 2 end
let thunk = fn -> 42 end
let classify = fn 0 -> "zero" | _ -> "other" end

# Multi-line and trailing closures
let worker = fn value do
  transform(value)
end
fn with_value(value :: Int, block :: Fun(Int) -> Int) -> Int = block(value)
let result = with_value(10) do |value|
  value * 2
end

# Keyword arguments become one final Map argument
request("/events", method: "POST", content_type: "application/json")

# Pipe operator
let result = 5 |> double |> add_one

# Slot pipe: route value to argument position N
let result = 10 |2> add(1)   # = add(1, 10) = 11

# Multi-line pipe: trailing form (|> at end of line)
let result = 5 |>
  double |>
  add_one

# Multi-line pipe: leading form (|> at start of next line)
let result = value
  |> transform
  |> process

# Useful for long chains (e.g. HTTP router setup)
let router = HTTP.router()
  |> HTTP.on_post("/events", handle_event)
  |> HTTP.on_get("/issues", handle_issues)
```

Both trailing and leading forms produce identical output to their single-line equivalents -- only formatting differs. See [Language Basics](/docs/language-basics/) for details.

## Patterns

```mesh
# Wildcard and name binding
_
value

# Literal patterns
0
-1
"ok"
true
nil

# Tuple and constructors
(left, right)
Some(value)
Result.Err(reason)

# List head/tail, or-pattern, and as-pattern
head :: tail
(0, value) | (1, value)  # alternatives must bind the same names
(x, y) as point

# Guarded match arm
case value do
  n when n > 0 -> "positive"
  _ -> "other"
end
```

`case`/`match` must be exhaustive; redundant arms are warned about. Current patterns do not include struct patterns or list-literal patterns such as `[a, b]`.

## Control Flow

```mesh
# If/else
if x > 0 do
  "positive"
else
  "non-positive"
end

# Case (pattern matching)
case x do
  0 -> "zero"
  1 -> "one"
  _ -> "other"
end

# match is a synonym for case; arms can have guards
match x do
  n when n < 0 -> "negative"
  0 -> "zero"
  _ -> "positive"
end

# For loop (list comprehension)
let doubled = for x in [1, 2, 3] do
  x * 2
end

# Filtered comprehension
let evens = for x in 0..10 when x % 2 == 0 do
  x
end

# Map iteration uses {key, value} binding syntax
for {key, value} in map do
  println("#{key}: #{value}")
end

# For with range
for i in 0..5 do
  println("#{i}")
end

# While loop
while condition do
  # body
  break
end

# continue skips to the next iteration
for x in values do
  if x < 0 do
    continue
  end
  x
end
```

`for` always produces a list. Integer ranges are end-exclusive: `0..5` yields `0` through `4`. `while` produces Unit.

## Structs & Types

```mesh
# Struct definition
struct Point do
  x :: Int
  y :: Int
end deriving(Eq, Display)

# Struct creation
let p = Point { x: 1, y: 2 }

# Immutable struct update
let moved = %{p | x: p.x + 10}

# Generic struct
struct Box<T> do
  value :: T
end

# Sum type
type Color do
  Red
  Green
  Blue
end deriving(Eq, Display)

# Generic sum type with stored data
type Outcome<T> do
  Pending
  Complete(value :: T)
  Failed(reason :: String)
end

# Type alias (transparent -- alias and aliased type are interchangeable)
type Url = String
type Count = Int

# Generic type alias
type Pair<A, B> = (A, B)
type StringResult<T> = Result<T, String>

# Exported type alias (importable by other modules)
pub type UserId = Int

# Cross-module import of a type alias
from Types.User import UserId
fn get_user(id :: UserId) -> String do
  String.from(id)
end
```

Type aliases, including generic aliases, are transparent: a `UserId` value works wherever `Int` is valid with no conversion. See [Type System](/docs/type-system/) for details.

## Interfaces & Traits

```mesh
# Define a custom interface
interface Greeter do
  fn greet(self) -> String

  # Default method
  fn label(self) -> String do
    "greeting=" <> self.greet()
  end
end

# Implement for a type
impl Greeter for Person do
  fn greet(self) -> String do
    "Hello"
  end
end

# Associated types
interface Container do
  type Item
  fn first(self) -> Self.Item
end

impl Container for MyBox do
  type Item = Int
  fn first(self) -> Int do self.value end
end

# Static method: omit self
interface Versioned do
  fn version() -> Int
end

# Generic function bound
fn show<T>(value :: T) -> String where T: Display = value.to_string()

# Deriving built-in traits
struct Tag do
  id :: Int
end deriving(Eq, Hash)

# Struct derives: Eq, Ord, Display, Debug, Hash, Json, Row, Schema
# Sum-type derives: Eq, Ord, Display, Debug, Hash, Json
# Explicit Ord must include Eq.
```

With no deriving clause, structs get `Debug`, `Eq`, `Ord`, and `Hash`; sum types get `Debug`, `Eq`, and `Ord`. An explicit clause is selective. `Json`, `Row`, and `Schema` must be requested explicitly; `Row` and `Schema` are struct-only.

See [Type System -- Traits](/docs/type-system/#traits) for details.

## Numeric Traits

```mesh
# Operator overloading via traits
impl Add for Vec2 do
  type Output = Vec2
  fn add(self, other :: Vec2) -> Vec2 do
    Vec2 { x: self.x + other.x, y: self.y + other.y }
  end
end

# Available: Add (+), Sub (-), Mul (*), Div (/), Mod (%), Neg (unary -)
let sum = v1 + v2     # calls Add.add
let neg = -v1         # calls Neg.neg
```

See [Type System -- Numeric Traits](/docs/type-system/#numeric-traits) for details.

## From/Into Conversion

```mesh
# User-defined conversion
impl From<Int> for Wrapper do
  fn from(n :: Int) -> Wrapper do
    Wrapper { value: n }
  end
end
let w = Wrapper.from(42)
let w2 :: Wrapper = 42.into()  # synthesized from From<Int> for Wrapper

# Built-in conversions
let f = Float.from(42)       # Int -> Float
let s = String.from(42)      # Int -> String

# ? operator auto-converts error types via From
fn process() -> Int!AppError do
  let n = risky()?  # String error auto-converts to AppError
  Ok(n)
end
```

See [Type System -- From/Into](/docs/type-system/#from-into-conversion) for details.

## Iterators

```mesh
# Create a lazy iterator from a List<T>
let iter = Iter.from([1, 2, 3, 4, 5])

# Lazy combinators (chained with pipe operator)
Iter.from(list) |> Iter.map(fn x -> x * 2 end)
Iter.from(list) |> Iter.filter(fn x -> x > 3 end)
Iter.from(list) |> Iter.take(3)
Iter.from(list) |> Iter.skip(2)
Iter.from(list) |> Iter.enumerate()
Iter.from(a) |> Iter.zip(Iter.from(b))

# Terminal operations
Iter.from(list) |> Iter.count()
Iter.from(list) |> Iter.sum()
Iter.from(list) |> Iter.any(fn x -> x > 3 end)
Iter.from(list) |> Iter.all(fn x -> x > 0 end)
Iter.from(list) |> Iter.reduce(0, fn acc, x -> acc + x end)

# Collect into collections
Iter.from(list) |> Iter.map(fn x -> x * 2 end) |> List.collect()
Iter.from(list) |> Iter.enumerate() |> Map.collect()
Iter.from(list) |> Set.collect()
Iter.from(strings) |> String.collect()
```

`Iter.from` currently accepts lists only. `for...in` separately supports lists, maps, sets, ranges, and user-defined `Iterable`/`Iterator` values. Use typed `List.find(list, predicate) -> Option<T>` for search; the current `Iter.find` result is not yet exposed as a typed `Option<T>`.

See [Iterators](/docs/iterators/) for details.

## Error Handling

```mesh
# Option type (T?)
fn find(x :: Int) -> Int? do
  if x > 0 do
    return Some(x)
  end
  None
end

# Result type (T!E)
fn divide(a :: Int, b :: Int) -> Int!String do
  if b == 0 do
    return Err("division by zero")
  end
  Ok(a / b)
end

# Early return with ?
fn compute(x :: Int) -> Int!String do
  let result = divide(x, 2)?
  Ok(result + 10)
end

# ? also propagates None from an Option-returning function
fn first_positive(values :: List<Int>) -> Int? do
  let value = List.find(values, fn n -> n > 0 end)?
  Some(value)
end
```

## Concurrency

```mesh
# Actor definition
actor worker() do
  receive do
    msg -> println("got: #{msg}")
  after 1000 -> println("idle")
  end

  terminate do
    println("stopping")
  end
end

# Spawn returns Pid<MessageType>; send is checked for typed Pids
let pid = spawn(worker)
send(pid, "hello")

# Actor identity and failure links (inside actor/service execution)
let me = self()
link(pid)

# Supervisor
supervisor MySup do
  strategy: one_for_one
  max_restarts: 3
  max_seconds: 5

  child w do
    start: fn -> spawn(worker) end
    restart: permanent
    shutdown: 5000
  end
end

# Service (GenServer)
service Counter do
  fn init(n :: Int) -> Int do n end
  call Get() :: Int do |s| (s, s) end
  cast Reset() do |_s| 0 end
end

let pid = Counter.start(0)
Counter.get(pid)
Counter.reset(pid)
```

See [Concurrency](/docs/concurrency/) for details.

## Modules

```mesh
# Import a module
import String

# Use qualified access
let n = String.length("test")

# Import specific functions
from String import length
let n = length("test")

# Parenthesized multiline selective import
from Geometry import (
  Point,
  distance,
)

# Explicit module and public declarations
pub module Geometry do
  pub struct Point do
    x :: Float
    y :: Float
  end

  pub fn origin() -> Point do
    Point { x: 0.0, y: 0.0 }
  end
end
```

Glob imports are not supported, and private declarations cannot be imported.

## Special Function Declarations

```mesh
# Runtime-owned clustered work; default total copy count is 2
@cluster
pub fn refresh_cache() -> Int do
  1
end

# Explicit total copy count
@cluster(3)
pub fn rebuild_index() -> Int do
  3
end

# Bodyless native ABI binding
@native("mesh_math_add")
pub fn native_add(left :: Int, right :: Int) -> Int
```

`@cluster` applies only to a unique public `fn`/`def`; the old `clustered(work)` spelling is rejected. `@native` declarations require explicit parameter and result types, cannot use generics/bounds/guards, and support `Int`, `Float`, `Bool`, `String`, `Bytes`, `U64`, `U128`, and `I128` values plus `Option`/`Result` returns over supported values.

## Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+`, `-`, `*`, `/`, `%` |
| Comparison | `==`, `!=`, `<`, `>`, `<=`, `>=` |
| Logical | `and` / `&&`, `or` / `\|\|`, `not` / `!` |
| Pipe | `\|>` |
| Slot pipe | `\|N>` (e.g. `\|2>`) |
| String concat | `<>` |
| List concat | `++` |
| Error propagation | `?` |
| Range | `..` |

Precedence from low to high is: pipes; or; and; equality; ordering; range; concatenation; addition; multiplication; prefix; postfix call/field/`?`.

## Testing

```mesh
# File: my_module.test.mpl
# Run with: meshc test my_app

test("basic assertions") do
  assert(1 + 1 == 2)
  assert_eq(10, 5 + 5)
  assert_ne(3, 4)
  assert_raises(fn() do
    assert(false)
  end)
end

describe("grouped tests") do
  setup do
    assert(true)   # runs before each test
  end

  teardown do
    assert(true)   # runs after each test
  end

  test("inner test") do
    assert(true)
  end
end

test("actor messaging") do
  let me = self()
  send(me, 42)
  assert_receive 42, 500   # pattern, timeout_ms
end

# Mock actor for concurrency tests
let mock = Test.mock_actor(fn msg do
  # handle msg
  println("mock received: #{msg}")
  "ignored"   # callback is currently typed String -> String
end)
```

`self()` is available only in actor-context code; test bodies run as test
actors. The identity/link lines above are context-only fragments.
`Test.mock_actor` is a test-owned cleanup helper whose callback return value is
ignored. Use a normal actor when callback behavior itself must be observed.

| Assertion | Description |
|-----------|-------------|
| `assert expr` | Fail if expr is false |
| `assert_eq a, b` | Fail if a != b |
| `assert_ne a, b` | Fail if a == b |
| `assert_raises fn` | Fail if fn does not raise |
| `assert_receive pat, ms` | Fail if not received within timeout |

See [Testing](/docs/testing/) for full guide.

## Standard Library

```mesh
# Crypto
let h256 = Crypto.sha256("hello")
let h512 = Crypto.sha512("hello")
let mac = Crypto.hmac_sha256("key", "msg")
let ok = Crypto.secure_compare("a", "a")   # Bool, constant-time
let id = Crypto.uuid4()                    # UUID v4 string

# Encoding
let raw = Bytes.from_utf8("hello")
let size = Bytes.length(raw)
let b64_bytes = Bytes.to_base64(raw)
case Bytes.to_utf8(raw) do
  Ok(s) -> println(s)
  Err(e) -> println(e)
end

# Checked wide integers
case U64.parse("18446744073709551615") do
  Ok(value) -> value |> U64.to_string() |> println()
  Err(e) -> println(e)
end

let b64 = Base64.encode("hello")
case Base64.decode(b64) do
  Ok(s) -> println(s)
  Err(e) -> println(e)
end
let url = Base64.encode_url("hello")

let hex = Hex.encode("hi")   # "6869"
case Hex.decode(hex) do
  Ok(s) -> println(s)
  Err(e) -> println(e)
end

# DateTime
let dt = DateTime.utc_now()
let iso = DateTime.to_iso8601(dt)
let ms = DateTime.to_unix_ms(dt)
case DateTime.from_iso8601("2024-01-15T10:30:00Z") do
  Ok(dt2) ->
    let next = DateTime.add(dt2, 7, :day)
    let diff = DateTime.diff(next, dt2, :day)   # Float
    let before = DateTime.is_before(dt2, next)  # Bool
    let after = DateTime.is_after(next, dt2)    # Bool
  Err(e) -> println(e)
end
```

See [Standard Library](/docs/stdlib/) for full reference.
