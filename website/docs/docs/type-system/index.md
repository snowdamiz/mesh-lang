---
title: Type System
---

# Type System

Mesh has a powerful static type system with full type inference. You rarely need to write type annotations -- the compiler figures out types from context. When you do annotate, you get compile-time safety guarantees.

## Type Inference

The Mesh compiler infers types from how values are used. You can declare variables without annotations and the compiler determines the correct type:

```mesh
fn main() do
  let x = 42           # inferred as Int
  let name = "hello"   # inferred as String
  let pi = 3.14        # inferred as Float
  let active = true    # inferred as Bool
  println("${x} ${name} ${pi} ${active}")
end
```

Function return types can also be inferred:

```mesh
fn double(x :: Int) do
  x * 2
end
```

The compiler infers that `double` returns `Int` because `x * 2` produces an `Int`. You can always add explicit annotations for clarity:

```mesh
fn double(x :: Int) -> Int do
  x * 2
end
```

### When to Annotate

Type annotations are optional in many places, but recommended for:

- **Function signatures** -- makes the API clear to readers
- **Complex generic functions** -- helps the compiler and your teammates
- **Public interfaces** -- documents the contract

## Generics

Generic functions and types let you write code that works with any type. Use angle brackets to declare type parameters:

```mesh
struct Box<T> do
  value :: T
end deriving(Display, Eq)

fn main() do
  let b1 = Box { value: 42 }
  let b2 = Box { value: 42 }
  let bs = Box { value: "hello" }
  println("${b1}")
  println("${bs}")
  println("${b1 == b2}")
end
```

Generic functions use the same angle bracket syntax:

```mesh
fn apply(f :: Fun(Int) -> String, x :: Int) -> String do
  f(x)
end

fn apply2(f :: Fun(Int, Int) -> Int, a :: Int, b :: Int) -> Int do
  f(a, b)
end

fn main() do
  let result = apply(fn x -> "${x}" end, 42)
  println(result)

  let sum = apply2(fn a, b -> a + b end, 10, 20)
  println("${sum}")
end
```

Function types are written as `Fun(ParamTypes) -> ReturnType`. A zero-argument function type is `Fun() -> ReturnType`.

## Structs

Structs are product types -- they group multiple fields together. Define them with the `struct` keyword:

```mesh
struct Point do
  x :: Int
  y :: Int
end deriving(Eq, Ord, Display, Debug, Hash)
```

Create instances with curly brace syntax and access fields with dot notation:

```mesh
fn main() do
  let p = Point { x: 1, y: 2 }
  let q = Point { x: 1, y: 2 }
  let r = Point { x: 3, y: 4 }
  println("${p}")
  println("${p == q}")
  println("${p == r}")
end
```

Structs can be generic:

```mesh
struct Box<T> do
  value :: T
end deriving(Display, Eq)

fn main() do
  let b = Box { value: 42 }
  println("${b}")
end
```

## Sum Types

Sum types (also called algebraic data types or tagged unions) define a type that can be one of several variants. Use the `type` keyword:

```mesh
type Color do
  Red
  Green
  Blue
end
```

Variants are used directly by name. Pattern match on them with `case`:

```mesh
fn describe(c :: Color) -> Int do
  case c do
    Red -> 1
    Green -> 2
    Blue -> 3
  end
end

fn main() do
  let r = Red
  println("${describe(r)}")
  println("${describe(Green)}")
  println("${describe(Blue)}")
end
```

### Variants with Data

Variants can carry data. Mesh has built-in `Option` and `Result` types that follow this pattern:

```mesh
fn find_positive(a :: Int, b :: Int) -> Int? do
  if a > 0 do
    return Some(a)
  end
  if b > 0 do
    return Some(b)
  end
  None
end

fn main() do
  let r = find_positive(5, 10)
  case r do
    Some(val) -> println("${val}")
    None -> println("none")
  end
end
```

The `Int?` syntax is shorthand for `Option<Int>`. For error handling, use `Result` with the `!` shorthand:

```mesh
fn safe_divide(a :: Int, b :: Int) -> Int!String do
  if b == 0 do
    return Err("division by zero")
  end
  Ok(a / b)
end

fn compute(x :: Int) -> Int!String do
  let result = safe_divide(x, 2)?
  Ok(result + 10)
end

fn main() do
  let r = compute(20)
  case r do
    Ok(val) -> println("${val}")
    Err(msg) -> println(msg)
  end
end
```

The `?` operator propagates errors early -- if the expression evaluates to `Err` or `None`, the function returns immediately with that error.

## Traits

Traits define shared behavior that types can implement. Define a trait with the `trait` keyword and implement it with `impl`:

```mesh
struct Pair do
  a :: Int
  b :: Int
end

fn main() do
  let p = Pair { a: 10, b: 20 }
  let q = Pair { a: 10, b: 20 }
  println("${p == q}")
end
```

### Built-in Traits

Mesh provides several built-in traits:

| Trait | Purpose |
|-------|---------|
| `Eq` | Equality comparison (`==`, `!=`) |
| `Ord` | Ordering comparison (`<`, `>`, `<=`, `>=`) |
| `Display` | String representation for `println` and `"${...}"` |
| `Debug` | Detailed debug output |
| `Hash` | Hashing for use in maps and sets |
| `Json` | JSON serialization and deserialization |

### Trait Bounds on Functions

Function type annotations can use the `Fun()` type to accept functions as arguments, enabling higher-order programming:

```mesh
fn apply(f :: Fun(Int) -> String, x :: Int) -> String do
  f(x)
end

fn run_thunk(thunk :: Fun() -> Int) -> Int do
  thunk()
end

fn main() do
  let result = apply(fn x -> "${x}" end, 42)
  println(result)
  let val = run_thunk(fn -> 99 end)
  println("${val}")
end
```

## Deriving

Instead of manually implementing traits, you can derive them automatically. Add `deriving(...)` at the end of a struct or sum type definition:

```mesh
struct Point do
  x :: Int
  y :: Int
end deriving(Eq, Ord, Display, Debug, Hash)

fn main() do
  let p = Point { x: 1, y: 2 }
  let q = Point { x: 1, y: 2 }
  println("${p}")
  println("${p == q}")
end
```

### Deriving on Sum Types

Sum types support deriving the same traits:

```mesh
type Color do
  Red
  Green
  Blue
end deriving(Eq, Ord, Display, Debug, Hash)

fn main() do
  let r = Red
  let g = Green
  println("${r}")
  println("${g}")
  println("${r == r}")
  println("${r == g}")
end
```

### Selective Deriving

You can derive only the traits you need:

```mesh
struct Tag do
  id :: Int
end deriving(Eq)

fn main() do
  let a = Tag { id: 1 }
  let b = Tag { id: 1 }
  println("${a == b}")
end
```

### Deriving on Generic Types

Generic types can also derive traits:

```mesh
struct Box<T> do
  value :: T
end deriving(Display, Eq)

fn main() do
  let b1 = Box { value: 42 }
  let b2 = Box { value: 42 }
  let b3 = Box { value: 99 }
  println("${b1}")
  println("${b1 == b2}")
  println("${b1 == b3}")
end
```

### Available Derives

| Derive | What it generates |
|--------|-------------------|
| `Eq` | Structural equality comparison |
| `Ord` | Structural ordering (field-by-field or variant order) |
| `Display` | Human-readable string representation |
| `Debug` | Detailed debug string representation |
| `Hash` | Hash value computation |
| `Json` | JSON serialization and deserialization |

## Next Steps

- [Concurrency](/docs/concurrency/) -- actors, message passing, and supervision
- [Syntax Cheatsheet](/docs/cheatsheet/) -- quick reference for all Mesh syntax
