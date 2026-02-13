---
title: Syntax Cheatsheet
---

# Syntax Cheatsheet

A quick reference for Mesh syntax. For details, see the full guides linked in each section.

## Basics

| Syntax | Example |
|--------|---------|
| Variable binding | `let x = 42` |
| Type annotation | `x :: Int` |
| String interpolation | `"Hello, ${name}!"` |
| Comment | `# this is a comment` |
| Print | `println("hello")` |

## Types

| Type | Example |
|------|---------|
| `Int` | `42`, `0`, `-5` |
| `Float` | `3.14`, `0.5` |
| `String` | `"hello"`, `"${x}"` |
| `Bool` | `true`, `false` |
| `List<T>` | `[1, 2, 3]` |
| `Map<K, V>` | `%{"key" => "value"}` |
| `Option<T>` | `Some(42)`, `None` (shorthand: `Int?`) |
| `Result<T, E>` | `Ok(42)`, `Err("fail")` (shorthand: `Int!String`) |
| `Fun(A) -> B` | `Fun(Int) -> String` |

## Functions

```mesh
# Named function with types
fn add(a :: Int, b :: Int) -> Int do
  a + b
end

# Multi-clause (pattern matching)
fn fib(0) = 0
fn fib(1) = 1
fn fib(n) = fib(n - 1) + fib(n - 2)

# Guards
fn abs(n) when n < 0 = -n
fn abs(n) = n

# Anonymous function (closure)
let double = fn(x :: Int) -> x * 2 end

# Pipe operator
let result = 5 |> double |> add_one
```

See [Language Basics](/docs/language-basics/) for details.

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

# For loop (list comprehension)
let doubled = for x in [1, 2, 3] do
  x * 2
end

# For with range
for i in 0..5 do
  println("${i}")
end

# While loop
while condition do
  # body
  break
end
```

## Structs & Types

```mesh
# Struct definition
struct Point do
  x :: Int
  y :: Int
end deriving(Eq, Display)

# Struct creation
let p = Point { x: 1, y: 2 }

# Sum type
type Color do
  Red
  Green
  Blue
end deriving(Eq, Display)

# Type alias
type Mapper = Fun(Int) -> String
```

See [Type System](/docs/type-system/) for details.

## Traits

```mesh
# Available derives
# Eq, Ord, Display, Debug, Hash, Json

# Deriving on structs
struct Tag do
  id :: Int
end deriving(Eq, Hash)

# Deriving on sum types
type Status do
  Active
  Inactive
end deriving(Eq, Display)
```

See [Type System -- Traits](/docs/type-system/#traits) for details.

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
```

## Concurrency

```mesh
# Actor definition
actor worker() do
  receive do
    msg -> println("got: ${msg}")
  end
end

# Spawn and send
let pid = spawn(worker)
send(pid, "hello")

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
```

## Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+`, `-`, `*`, `/`, `%` |
| Comparison | `==`, `!=`, `<`, `>`, `<=`, `>=` |
| Logical | `and`, `or`, `not` |
| Pipe | `|>` |
| String concat | `++` |
| Error propagation | `?` |
| Range | `..` |
