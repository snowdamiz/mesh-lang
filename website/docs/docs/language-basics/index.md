---
title: Language Basics
description: "Core Mesh syntax and semantics: literals, bindings, functions, patterns, control flow, collections, modules, results, options, and pipes."
---

# Language Basics

This guide covers the core Mesh language. After reading it, you can write modules that use literals, immutable bindings, functions and closures, pattern matching, control flow, collections, pipes, and typed error propagation.

## Variables

Variables in Mesh are created with `let` bindings and are immutable by default:

```mesh
fn main() do
  let name = "Mesh"
  let age = 30
  let pi = 3.14
  let active = true
  println("Hello, ${name}!")
end
```

You can add type annotations with `::` to be explicit about a variable's type:

```mesh
fn main() do
  let x :: Int = 42
  let greeting :: String = "hello"
  println("${x}: ${greeting}")
end
```

Type annotations are optional -- the compiler infers types from context. Use annotations when you want to be explicit or when the compiler needs a hint.

Since variables are immutable, you cannot reassign them. Instead, you create a new binding with the same name (shadowing):

```mesh
fn main() do
  let x = 1
  let x = x + 1
  println("${x}")
end
```

Tuple destructuring is supported in bindings:

```mesh
fn main() do
  let (name, age) = ("Ada", 36)
  println("#{name} is #{age}")
end
```

Bindings can use `_` when a value is intentionally unused:

```mesh
let _ = do_work()
```

## Comments and Statement Boundaries

Mesh supports line comments, documentation comments, module documentation, and nested block comments:

```mesh
# A regular line comment
## Documentation for the declaration that follows
##! Documentation for this module

#=
  A block comment.
  #= Block comments can nest. =#
=#
```

Newlines normally end statements. A semicolon can separate statements on the same line:

```mesh
let x = 1; let y = 2
```

Newlines are not statement boundaries inside parentheses, brackets, or braces. Pipe expressions also support explicit leading and trailing continuation forms, described in [Multi-Line Pipes](#multi-line-pipes).

## Basic Types

Mesh has the following core types:

| Type | Description | Example |
|------|-------------|---------|
| `Int` | Signed machine integer | `42`, `-5`, `0xff` |
| `Float` | Floating-point number | `3.14`, `1.0e6` |
| `String` | UTF-8 text | `"hello"` |
| `Bool` | Boolean value | `true`, `false` |
| `Bytes` | Opaque binary data | `Bytes.from_utf8("hello")` |
| `U64`, `U128`, `I128` | Checked wide protocol integers | `U64.parse("18446744073709551615")` |
| `Json` | A typed JSON value | `json { status: "ok" }` |
| `Atom` | A symbolic value | `:ok`, `:not_found` |
| `Regex` | A compiled regular expression | `~r/[a-z]+/i` |
| `()` | Unit, the absence of a useful value | `()`, `nil` |
| `(A, B)` | Tuple | `(1, "one")` |
| `List<T>` | Immutable sequence | `[1, 2, 3]` |
| `Map<K, V>` | Immutable key-value collection | `%{"a" => 1}` |
| `Set` | Immutable unique collection of `Int` values | `Set.new()` |
| `Range` | Integer range value | `Range.new(0, 10)` |
| `Queue` | Immutable queue of `Int` values | `Queue.new()` |
| `Pid<M>` | Actor identity accepting messages of type `M` | returned by `spawn(...)` |
| `Option<T>` | Optional value | `Some(42)`, `None`; shorthand `Int?` |
| `Result<T, E>` | Success or failure | `Ok(42)`, `Err("failed")`; shorthand `Int!String` |
| `Fun(A) -> B` | Function value | `fn x -> x + 1 end` |

`Int`, `Float`, `String`, and `Bool` have literal syntax. `U64`, `U128`, and `I128` are opaque checked values intended for full-width protocol fields; construct and operate on them with their modules rather than `Int` literals or arithmetic operators.

### Numeric Literals

Integer literals can use decimal, hexadecimal, binary, or octal notation. Underscores are ignored:

```mesh
let decimal = 1_000_000
let hex = 0xff_ff
let binary = 0b1111_0000
let octal = 0o777
let scientific = 1.25e3
```

### Unit and `nil`

`()` is the Unit value. `nil` is an equivalent spelling and is useful when a value is required syntactically but carries no information:

```mesh
fn log_done() do
  println("done")
  nil
end
```

### Atoms and Regular Expressions

Atoms are lightweight symbolic values. An atom begins with `:` followed by a lowercase letter or underscore:

```mesh
let status = :ready
let unit = :millisecond
```

Regex literals use `~r/.../` and accept `i` (case-insensitive), `m` (multiline), and `s` (dot matches newline) flags:

```mesh
let digits = ~r/\d+/
let name = ~r/^[a-z]+$/im
let matched = Regex.is_match(digits, "item-42")
```

### String Interpolation

Strings support two interpolation syntaxes -- `#{}` (preferred, v12.0) and `${}` (also valid). Expressions inside the braces are evaluated and rendered through their `Display` implementations:

```mesh
fn main() do
  let name = "Mesh"
  let val = 42
  println("Hello, #{name}!")
  println("The answer is #{val}")
  println("Double: #{val * 2}")
end
```

### Heredoc Strings

Use triple-quote `"""..."""` for multiline strings. Heredocs support
interpolation, and ordinary quote characters and newlines can appear directly
until the closing triple quote:

```mesh
fn main() do
  let id = 42
  let name = "Alice"
  let body = """
    {"id": #{id}, "name": "#{name}"}
    """
  println(body)
end
```

Heredocs are useful for SQL queries and any multiline string content where backslash escaping would be cumbersome. For JSON objects, prefer `json { }` literals instead (see [JSON Literals](#json-literals)).

### Type Inference

The Mesh compiler infers types from how values are used. You rarely need to write type annotations for local variables:

```mesh
fn main() do
  let x = 42          # inferred as Int
  let name = "Mesh"   # inferred as String
  let flag = true     # inferred as Bool
  println("${x} ${name} ${flag}")
end
```

### Boolean Logic

Boolean values support `and`, `or`, and `not`. Symbolic `&&`, `||`, and `!` spellings are also available:

```mesh
fn main() do
  let t = true
  let f = false
  if t and not f do
    println("logic works")
  end
end
```

### Operators and Precedence

From lowest to highest precedence, Mesh groups operators as follows:

| Group | Operators |
|-------|-----------|
| Pipes | `|>`, `|N>` |
| Boolean or | `or`, `||` |
| Boolean and | `and`, `&&` |
| Equality | `==`, `!=` |
| Ordering | `<`, `>`, `<=`, `>=` |
| Range in a `for` source | `..` |
| Concatenation | `<>`, `++` |
| Addition | `+`, `-` |
| Multiplication | `*`, `/`, `%` |
| Prefix | `-`, `not`, `!` |
| Postfix | calls, field access, `?` |

`<>` concatenates strings and `++` concatenates lists. Parentheses can make any grouping explicit.

## Functions

Functions are declared with the `fn` keyword, followed by the name, parameters, and a `do...end` body:

```mesh
fn add(a :: Int, b :: Int) -> Int do
  a + b
end

fn greet(name :: String) -> String do
  "Hello, ${name}!"
end

fn main() do
  println("${add(10, 20)}")
  println(greet("Mesh"))
end
```

The last expression in a function body is the return value -- there is no need for an explicit `return` keyword (though `return` is available for early exits).

`def` is an exact synonym for `fn` on named functions:

```mesh
def greet(name :: String) -> String do
  "Hello, #{name}!"
end
```

### One-Line Functions

For simple functions, you can use the concise `=` syntax:

```mesh
fn double(x) = x * 2
fn square(x :: Int) -> Int = x * x

fn main() do
  println("${double(21)}")
  println("${square(6)}")
end
```

### Generic Functions and Bounds

Declare type parameters after the function name. Mesh generalizes inferred local bindings and functions, so reusable code can remain polymorphic:

```mesh
fn identity<T>(value :: T) -> T do
  value
end

fn main() do
  println("#{identity(42)}")
  println(identity("mesh"))
end
```

Use a `where` clause when an operation requires a trait:

```mesh
fn render<T>(value :: T) -> String where T: Display do
  value.to_string()
end
```

Multiple bounds are comma-separated: `where T: Display, U: Eq`. See [Type System](/docs/type-system/#generics-and-trait-bounds) for inference and trait details.

### Keyword Arguments

A run of `name: value` arguments at the end of a call is collected into one final `Map` argument:

```mesh
fn request(path :: String, options :: Map<String, String>) -> String do
  path
end

let result = request("/events", method: "POST", content_type: "application/json")
```

Positional arguments must come before keyword arguments. This is syntax sugar for passing a map; it does not add default or reordered named parameters to a function declaration.

### Multi-Clause Functions

Functions can have multiple clauses that pattern match on their arguments, similar to Elixir:

```mesh
fn fib(0) = 0
fn fib(1) = 1
fn fib(n) = fib(n - 1) + fib(n - 2)

fn to_string(true) = "yes"
fn to_string(false) = "no"

fn main() do
  println("${fib(10)}")
  println(to_string(true))
  println(to_string(false))
end
```

The compiler tries each clause in order and uses the first one that matches. Clauses for the same function and arity must be consecutive, and a catch-all clause must be last. Functions can reuse a name at different arities.

### Guard Clauses

Multi-clause functions can include `when` guards for additional conditions:

```mesh
fn abs(n) when n < 0 = -n
fn abs(n) = n

fn classify(n) when n > 0 = "positive"
fn classify(n) when n < 0 = "negative"
fn classify(n) = "zero"

fn main() do
  println("${abs(-5)}")
  println(classify(10))
  println(classify(-3))
  println(classify(0))
end
```

### Direct Tail Recursion

A direct call to the current function in tail position is lowered to a loop, so
this accumulator-style recursion does not grow the call stack:

```mesh
fn sum_to(n :: Int, total :: Int) -> Int do
  if n <= 0 do
    total
  else
    sum_to(n - 1, total + n)
  end
end
```

Tail positions include the final expression of blocks, `if` branches,
`case`/`match` arms, `let` continuations, explicit `return`, and actor receive
arms or timeouts. Mutual recursion and self-calls followed by more work are
ordinary calls and are not eliminated.

### Closures

Anonymous functions (closures) are created with `fn...end`:

```mesh
fn main() do
  let factor = 3
  let triple = fn(x :: Int) -> x * factor end
  println("${triple(7)}")
  println("${triple(10)}")
end
```

Closures capture variables from their surrounding scope. There are two syntax forms:

- **Arrow syntax** for one-line closures: `fn x -> x * 2 end`
- **Do-end syntax** for multi-line closures: `fn x do ... end`
- **Zero-argument syntax**: `fn -> 42 end` or `fn do ... end`
- **Multi-clause syntax**: `fn 0 -> "zero" | n -> "non-zero" end`

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]

  # Arrow syntax
  let doubled = list |> map(fn x -> x * 2 end)

  # Do-end syntax for multi-line bodies
  let processed = map(list, fn x do
    let doubled = x * 2
    let incremented = doubled + 1
    incremented
  end)

  println("${doubled}")
  println("${processed}")
end
```

A call can also take a trailing closure:

```mesh
fn with_value(value :: Int, block :: Fun(Int) -> Int) -> Int do
  block(value)
end

let result = with_value(10) do |value|
  value * 2
end
```

## Pattern Matching

The `case` expression matches a value against patterns and executes the first matching branch. `match` is an equivalent spelling:

```mesh
fn describe(x :: Int) -> String do
  case x do
    0 -> "zero"
    1 -> "one"
    _ -> "other"
  end
end

fn main() do
  println(describe(0))
  println(describe(1))
  println(describe(42))
end
```

The `_` pattern is a wildcard that matches anything.

Every unguarded `case` or `match` must cover all possible values. The compiler reports a non-exhaustive match as an error and warns about redundant arms. An arm with a `when` guard does not count as exhaustive because the guard may be false.

### Pattern Forms

Patterns can bind names and decompose tuples and constructors:

| Pattern | Meaning |
|---------|---------|
| `_` | Match anything without binding it |
| `name` | Match anything and bind it |
| `42`, `-1`, `"ok"`, `true`, `nil` | Literal pattern |
| `(left, right)` | Tuple pattern |
| `Some(value)`, `Result.Ok(value)` | Constructor pattern |
| `head :: tail` | Match a non-empty list as its head and tail |
| `left | right` | Or-pattern; both sides must bind the same names |
| `pattern as whole` | Match a pattern and also bind the complete value |

```mesh
fn describe_pair(value) -> String do
  case value do
    (0, y) -> "on y axis at #{y}"
    (x, 0) -> "on x axis at #{x}"
    (x, y) as _point when x == y -> "diagonal at #{x}"
    _ -> "other"
  end
end
```

List literal patterns such as `[first, second]` and struct patterns are not part of the current grammar. Use `head :: tail`, tuples, or sum-type constructors instead.

### Matching on Constructors

You can match on sum type constructors and destructure their contents:

```mesh
type Color do
  Red
  Green
  Blue
end

fn color_name(c :: Color) -> String do
  case c do
    Red -> "red"
    Green -> "green"
    Blue -> "blue"
  end
end

fn main() do
  let c = Red
  println(color_name(c))
end
```

Variants can be qualified with their type or module when that makes the source clearer:

```mesh
case result do
  Result.Ok(value) -> value
  Result.Err(_) -> 0
end
```

### Guards

`case`, `match`, function clauses, and multi-clause closures can use a `when` guard. Guards must evaluate to `Bool` and are syntactically limited to literals and names, comparisons, boolean operators, grouping, and named function calls:

```mesh
case score do
  n when n >= 90 -> "excellent"
  n when n >= 60 -> "passing"
  _ -> "retry"
end
```

### Matching on Results

Pattern matching works naturally with `Ok` and `Err` result types:

```mesh
fn safe_divide(a :: Int, b :: Int) -> Int!String do
  if b == 0 do
    return Err("division by zero")
  end
  Ok(a / b)
end

fn main() do
  let r = safe_divide(10, 2)
  case r do
    Ok(val) -> println("Result: ${val}")
    Err(msg) -> println("Error: ${msg}")
  end
end
```

See the [Error Handling](#error-handling) section below for more on result types.

## Control Flow

### If/Else

The `if/else` expression evaluates a condition and runs the corresponding branch:

```mesh
fn max(a :: Int, b :: Int) -> Int do
  if a > b do
    a
  else
    b
  end
end

fn main() do
  println("${max(10, 20)}")
end
```

`if` is an expression in Mesh, so it returns a value. The `else` branch is optional when the result is not used.

### For Loops

The `for...in` expression iterates over ranges and collections:

```mesh
fn main() do
  # Iterate over a range (0 through 4)
  for i in 0..5 do
    println("${i}")
  end
end
```

`start..end` is end-exclusive, so `0..5` yields `0`, `1`, `2`, `3`, and `4`.

For loops can also iterate over lists:

```mesh
fn main() do
  let names = ["Alice", "Bob", "Charlie"]
  for name in names do
    println("Hello, ${name}!")
  end
end
```

#### Filter Clauses

Add a `when` clause to filter elements during iteration:

```mesh
fn main() do
  let evens = for i in 0..10 when i % 2 == 0 do
    i
  end
  for e in evens do
    println("${e}")
  end
end
```

Every `for` expression returns a list containing one body result per accepted element, making it a list comprehension even when the body is used primarily for side effects.

#### Map Iteration

Iterate over map entries with destructuring:

```mesh
fn main() do
  let m = Map.new()
  let m = Map.put(m, 1, 10)
  let m = Map.put(m, 2, 20)
  let m = Map.put(m, 3, 30)

  let vals = for {k, v} in m do
    v
  end

  let total = List.length(vals)
  println("${total}")
end
```

### While Loops

The `while` loop repeats its body as long as the condition is true:

```mesh
fn main() do
  while true do
    println("loop ran")
    break
  end
  println("after loop")
end
```

### Break and Continue

Use `break` to exit a loop early and `continue` to skip to the next iteration:

```mesh
fn main() do
  # break exits the loop
  while true do
    println("before break")
    break
  end
  println("after loop")

  # continue skips the rest of the current iteration
  let result = for x in [1, 2, 3, 4, 5] when x > 1 do
    if x == 3 do
      continue
    end
    x
  end
  for r in result do
    println("${r}")
  end
end
```

## Pipe Operator

The pipe operator `|>` passes the result of the left-hand expression as the first argument to the right-hand function. It turns nested calls into readable left-to-right chains:

```mesh
fn double(x :: Int) -> Int do
  x * 2
end

fn add_one(x :: Int) -> Int do
  x + 1
end

fn main() do
  # Without pipes (nested, reads inside-out)
  let a = add_one(double(5))

  # With pipes (chained, reads left-to-right)
  let b = 5 |> double |> add_one

  println("${a}")
  println("${b}")
end
```

Both `a` and `b` equal `11`. The pipe version reads naturally: "take 5, double it, add one."

### Pipes with Closures

Pipes work well with higher-order functions like `map`, `filter`, and `reduce`:

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]

  let doubled = list |> map(fn x -> x * 2 end)
  let filtered = doubled |> filter(fn x -> x > 4 end)
  let sum = reduce(filtered, 0, fn acc, x -> acc + x end)

  println("${sum}")
end
```

### Slot Pipe Operator

The slot pipe `|N>` routes the left-hand value to a specific argument position (N) instead of the first position:

```mesh
fn add(a :: Int, b :: Int) -> Int do
  a + b
end

fn main() do
  # Slot pipe: 10 |2> add(1) = add(1, 10) = 11
  let result = 10 |2> add(1)
  println("#{result}")

  # Chain slot pipe and regular pipe
  let chained = 5 |2> add(10) |> add(1)
  println("#{chained}")
end
```

Use `|2>` to insert the piped value as the second argument, `|3>` for the third, and so on. Slot pipes can be chained with regular pipes.

### Multi-Line Pipes

Long pipe chains can be split across lines using either the **trailing form** (`|>` at the end of a line) or the **leading form** (`|>` at the start of the next line):

```mesh
fn double(x :: Int) -> Int do
  x * 2
end

fn add_one(x :: Int) -> Int do
  x + 1
end

fn negate(x :: Int) -> Int do
  -x
end

fn main() do
  # Trailing form: |> at the end of each line
  let result = 5 |>
    double |>
    add_one |>
    negate

  # Leading form: |> at the start of continuation lines
  let result2 = 5
    |> double
    |> add_one
    |> negate

  println("#{result}")
  println("#{result2}")
end
```

Both forms produce identical compiled output to their single-line equivalents -- only formatting differs. Choose whichever reads more clearly for your use case.

Multi-line pipes are especially useful for long chains where all steps would not fit on a single line, such as building an HTTP router:

```mesh
fn main() do
  let router = HTTP.router()
    |> HTTP.on_post("/api/events", handle_event)
    |> HTTP.on_get("/api/issues", handle_issues)
    |> HTTP.on_get("/api/dashboard", handle_dashboard)
end
```

## Error Handling

Mesh uses result types for error handling. A function that can fail returns `T!E`, where `T` is the success type and `E` is the error type:

```mesh
fn safe_divide(a :: Int, b :: Int) -> Int!String do
  if b == 0 do
    return Err("division by zero")
  end
  Ok(a / b)
end
```

- `Ok(value)` wraps a successful result
- `Err(error)` wraps an error
- The return type `Int!String` means "returns an `Int` on success or a `String` error on failure"

### The Try Operator

The postfix `?` operator works with both `Result` and `Option`. It unwraps `Ok(value)` or `Some(value)`; `Err(error)` or `None` returns immediately from the enclosing function:

```mesh
fn step1(x :: Int) -> Int!String do
  if x < 0 do
    return Err("negative input")
  end
  Ok(x * 2)
end

fn step2(x :: Int) -> Int!String do
  if x > 100 do
    return Err("too large")
  end
  Ok(x + 1)
end

fn pipeline(x :: Int) -> Int!String do
  let a = step1(x)?
  let b = step2(a)?
  Ok(b)
end

fn main() do
  let r = pipeline(10)
  case r do
    Ok(val) -> println("${val}")
    Err(msg) -> println(msg)
  end
end
```

The `?` after `step1(x)` means: if `step1` returns `Ok(value)`, bind `value` to `a` and continue; if it returns `Err(e)`, immediately return `Err(e)` from the current function. This keeps error handling concise without deeply nested pattern matches.

When the enclosing `Result` uses a different error type, Mesh looks for a matching `From<SourceError>` implementation and converts the error during propagation. See [From/Into Conversion](/docs/type-system/#from-into-conversion).

For `Option`, the enclosing function must return `Option`:

```mesh
fn first_positive(values :: List<Int>) -> Int? do
  let value = List.find(values, fn n -> n > 0 end)?
  Some(value)
end
```

### Handling Results with Pattern Matching

Use `case` to handle both success and error cases:

```mesh
fn safe_divide(a :: Int, b :: Int) -> Int!String do
  if b == 0 do
    return Err("division by zero")
  end
  Ok(a / b)
end

fn main() do
  let r = safe_divide(10, 0)
  case r do
    Ok(val) -> println("Result: ${val}")
    Err(msg) -> println("Error: ${msg}")
  end
end
```

## Modules

Mesh organizes code into modules. The standard library provides built-in modules like `String`, `List`, and `Map`, accessed with dot notation:

```mesh
import String

fn main() do
  let n = String.length("test")
  println("${n}")
end
```

The `import` statement makes a module available. You can also import specific public names directly:

```mesh
from String import length

fn main() do
  let n = length("test")
  println("${n}")
end
```

Selective imports can be comma-separated or parenthesized across lines:

```mesh
from Geometry import (
  Point,
  distance,
  translate,
)
```

Glob imports are not supported. Private names cannot be imported.

You can define a module explicitly with `module ... do ... end` and export declarations with `pub`:

```mesh
pub module Geometry do
  pub struct Point do
    x :: Float
    y :: Float
  end

  pub fn origin() -> Point do
    Point { x: 0.0, y: 0.0 }
  end

  fn internal_helper() -> Int do
    0
  end
end
```

`pub` is available on functions, modules, structs, interfaces, supervisors, sum types, and type aliases. Actors, services, impl blocks, imports, and local bindings are not declared `pub`.

### Standard Library Modules

Mesh includes several built-in modules:

| Module   | Purpose                     | Example                          |
|----------|-----------------------------|----------------------------------|
| `List`   | List operations             | `List.length(xs)`, `List.get(xs, 0)` |
| `Map`    | Key-value maps              | `Map.new()`, `Map.put(m, k, v)` |
| `Set`    | Unique value sets           | `Set.new()`, `Set.add(s, v)`    |
| `String` | String manipulation         | `String.length(s)`              |

## Clustered and Native Function Declarations

Mesh has two source decorators for function boundaries. They are declarations with compiler-defined behavior, not general-purpose annotations.

### `@cluster`

`@cluster` marks a public function as runtime-owned clustered work. The uncounted form uses the default total copy count of two; `@cluster(N)` requests an explicit total copy count:

```mesh
@cluster
pub fn refresh_cache() -> Int do
  1
end

@cluster(3)
pub fn rebuild_index() -> Int do
  3
end
```

The decorated target must resolve to one public, non-overloaded function. The removed `clustered(work)` spelling is not supported. See [Autonomous Clusters](/docs/autonomous-clusters/) for deployment and runtime policy.

### `@native`

`@native("symbol")` declares a Mesh signature implemented by a symbol in a checksum-verified static library:

```mesh
@native("mesh_math_add")
pub fn add(left :: Int, right :: Int) -> Int

@native("mesh_decode")
pub fn decode(input :: Bytes) -> Bytes!String
```

A native declaration:

- must be `pub` and have no Mesh body;
- must give every parameter and the return value an explicit type;
- cannot have generic parameters, a `where` clause, or a guard;
- can pass `Int`, `Float`, `Bool`, `String`, `Bytes`, `U64`, `U128`, and `I128`;
- can additionally return `Option` or `Result` containing supported ABI values.

The package's `[native]` manifest entry selects ABI version 1 bindings and a SHA-256-pinned archive for the exact target. The package manager never executes a native build script.

### Working with Lists

Lists are a core data structure. You can create them with literal syntax or the `List` module:

```mesh
fn main() do
  # List literal
  let xs = [1, 2, 3]
  let len = List.length(xs)
  println("${len}")

  # Access by index
  let first = List.get(xs, 0)
  println("${first}")
end
```

### Working with Maps

Maps are key-value collections:

```mesh
fn main() do
  let m = Map.new()
  let m = Map.put(m, 1, 10)
  let m = Map.put(m, 2, 20)
  let m = Map.put(m, 3, 30)

  for {k, v} in m do
    println("${k}: ${v}")
  end
end
```

Note that `Map.put` returns a new map -- all collections in Mesh are immutable.

Map literals use `%{key => value}`:

```mesh
fn main() do
  let scores = %{"Ada" => 10, "Lin" => 9}
  println("#{Map.size(scores)}")
end
```

## JSON Literals

Use `json { }` to construct JSON objects without manual string escaping or interpolation:

```mesh
# Simple object literal
let response = json { status: "ok", count: 42 }
# response has type Json and encodes as {"status":"ok","count":42}

# Multi-line (same result)
let event = json {
  issue_id: issue_id,
  severity: "high"
}
```

Keys are bare identifiers (no quotes needed). Values are any Mesh expression — the type determines how they are serialized:

| Mesh type | JSON output |
|-----------|-------------|
| `String`  | `"quoted string"` |
| `Int`     | `42` (unquoted number) |
| `Float`   | `3.14` (unquoted) |
| `Bool`    | `true` / `false` |
| `nil`     | `null` |
| `Option<T>` | `null` (None) or the value (Some) |
| `List<T>` | JSON array |
| Struct with `deriving(Json)` | nested JSON object |

Nested `json { }` values embed raw — no double-encoding:

```mesh
let inner = json { code: 200 }
let outer = json { result: inner, ok: true }
# outer is: {"result":{"code":200},"ok":true}
```

The result of `json { }` has type `Json`. `Json` is implicitly compatible with `String` at call sites, so it can be passed directly to APIs such as `HTTP.response` or `Ws.broadcast` without manual encoding:

```mesh
HTTP.response(200, json { status: "ok", affected: n })
HTTP.response(401, json { error: "unauthorized" })
Ws.broadcast(room, json { id: record_id })
```

This replaces heredoc JSON templates (`"""{"key":"#{val}"}"""`) and manual string concatenation (`"{\"key\":\"" <> val <> "\"}"`) with readable, type-safe object literals.

> **Note:** Keys must be bare identifiers. Reserved keywords (`type`, `fn`, `let`, etc.) cannot be used as keys directly — use heredoc strings for JSON objects with keyword-named fields.

## Type Aliases

A type alias creates a new name for an existing type. The alias is **transparent** -- the compiler treats the alias and the original type as identical, so no conversion is needed:

```mesh
type Url = String
type Count = Int

fn fetch(url :: Url) -> String do
  # url is transparently a String -- no conversion needed
  url
end

fn main() do
  let u :: Url = "https://example.com"
  println(fetch(u))
end
```

Type aliases improve code readability by giving domain-meaningful names to primitive types without introducing any runtime overhead.

### Exported Type Aliases

Use `pub type` to export a type alias so other modules can import and use it:

```mesh
# types/user.mpl
pub type UserId = Int
pub type Email = String
```

```mesh
# main.mpl
from Types.User import UserId, Email

fn create_user(id :: UserId, email :: Email) -> String do
  "user-#{id}: #{email}"
end

fn main() do
  println(create_user(1, "alice@example.com"))
end
```

Because aliases are transparent, a `UserId` value satisfies any `Int` constraint and an `Email` value satisfies any `String` constraint.

Aliases can be generic:

```mesh
type Pair<A, B> = (A, B)
type StringResult<T> = Result<T, String>

let pair :: Pair<Int, String> = (1, "one")
let result :: StringResult<Int> = Ok(42)
```

Type arguments are substituted into the aliased type, and the result remains transparent at runtime.

See [Type System](/docs/type-system/) for full trait and type documentation.

## What's Next?

You now have a solid foundation in the Mesh language. Continue with:

- [Type System](/docs/type-system/) -- structs, sum types, traits, and advanced type features
- [Iterators](/docs/iterators/) -- lazy iterator pipelines, combinators, and collection materialization
- [Concurrency](/docs/concurrency/) -- actors, message passing, supervision trees, and services
- [Syntax Cheatsheet](/docs/cheatsheet/) -- quick reference for all Mesh syntax
