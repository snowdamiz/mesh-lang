---
title: Type System
description: "Static typing and inference in Mesh: generics, enums, traits, aliases, annotations, and compile-time safety."
---

# Type System

Mesh has a static Hindley-Milner-style type system with local and function inference, unification, let polymorphism, algebraic data types, and trait constraints. You rarely need to annotate local values, but annotations make public contracts and native boundaries explicit.

## Core Type Forms

| Type form | Meaning |
|-----------|---------|
| `Int`, `Float`, `Bool`, `String` | Literal scalar types |
| `Bytes` | Opaque binary data |
| `U64`, `U128`, `I128` | Checked wide protocol integers |
| `Json` | Typed JSON value, implicitly compatible with `String` at call sites |
| `Atom`, `Regex` | Symbol and compiled-regex values |
| `()` | Unit; `nil` is the equivalent value spelling |
| `(A, B)` | Tuple |
| `Fun(A, B) -> R` | Function |
| `List<T>`, `Map<K, V>` | Parametric collections |
| `Set` | Immutable set of `Int` values |
| `Range` | Integer range value |
| `Queue` | Immutable queue of `Int` values |
| `Option<T>` / `T?` | Optional value |
| `Result<T, E>` / `T!E` | Success or failure |
| `Pid<M>` | Actor identity whose mailbox accepts `M` |

`U64`, `U128`, and `I128` are not alternate literal types. They are opaque checked values constructed through their modules, and their arithmetic functions return `Result` on overflow or underflow.

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

The compiler also generalizes reusable bindings. The same identity function can be called at unrelated types:

```mesh
fn identity(value) do
  value
end

let number = identity(42)
let text = identity("mesh")
```

Recursive functions and functions declared later in the same module are registered before their bodies are checked, so mutually recursive definitions can refer to one another.

### When to Annotate

Type annotations are optional in many places, but recommended for:

- **Function signatures** -- makes the API clear to readers
- **Complex generic functions** -- helps the compiler and your teammates
- **Public interfaces** -- documents the contract

## Generics and Trait Bounds

Generic functions and types let you write code that works with any type. Declare type parameters in angle brackets:

```mesh
struct Box<T> do
  value :: T
end deriving(Display, Eq)

fn wrap<T>(value :: T) -> Box<T> do
  Box { value: value }
end

fn choose<A>(condition :: Bool, left :: A, right :: A) -> A do
  if condition do
    left
  else
    right
  end
end

fn main() do
  let b1 = wrap(42)
  let b2 = wrap(42)
  let bs = wrap("hello")
  println("${b1}")
  println("${bs}")
  println("${b1 == b2}")
end
```

Add a `where` clause when the function body needs a trait operation:

```mesh
fn render<T>(value :: T) -> String where T: Display do
  value.to_string()
end

fn same<T>(left :: T, right :: T) -> Bool where T: Eq do
  left == right
end

fn combine<A, B>(left :: A, right :: B) -> String where A: Display, B: Display do
  left.to_string() <> right.to_string()
end
```

Bounds are comma-separated and trait names may be qualified. They are checked at each call site.

### Function Types

Use `Fun(ParamTypes) -> ReturnType` when a value is itself callable:

```mesh
fn apply<A, B>(f :: Fun(A) -> B, value :: A) -> B do
  f(value)
end

fn run_thunk(thunk :: Fun() -> Int) -> Int do
  thunk()
end

let text = apply(fn n -> "value=#{n}" end, 42)
let answer = run_thunk(fn -> 42 end)
```

Closures infer captured environment types separately from their callable parameter and result types.

## Type Aliases

Type aliases give descriptive names to existing types without creating a new type. The `type Alias = ExistingType` syntax declares an alias that is completely transparent -- the compiler treats the alias and the aliased type as identical, so no conversion is ever needed.

```mesh
# Type aliases give descriptive names to existing types
type Url = String
type Count = Int

fn fetch_page(url :: Url) -> Int do
  String.length(url)   # Url is String -- no conversion needed
end

fn main() do
  let u :: Url = "https://example.com"
  println("${fetch_page(u)}")
end
```

### Pub Aliases for Cross-Module Use

Mark an alias `pub type` to export it so other modules can import it by name with `from Module import AliasName`. This lets you define a canonical name in one place and share it across the codebase without repeating the definition:

```mesh
# types/ids.mpl
pub type UserId = Int
pub type Email = String

# main.mpl
from Types.Ids import UserId, Email

fn create_profile(id :: UserId, email :: Email) -> String do
  "user-#{id}: #{email}"
end

fn main() do
  let result = create_profile(42, "alice@example.com")
  println(result)
end
```

### When to Use Type Aliases

Type aliases are useful when you want to:

- Add semantic meaning to primitive types (e.g., `UserId` vs `Int`, `Fingerprint` vs `String`)
- Document the intended use of a parameter without creating a new distinct type
- Share a type name across modules without repeating the underlying type definition

Aliases can be generic:

```mesh
type Pair<A, B> = (A, B)
type StringResult<T> = Result<T, String>
type Handler<Input, Output> = Fun(Input) -> Output

let pair :: Pair<Int, String> = (1, "one")
let result :: StringResult<Int> = Ok(42)
```

Generic arguments are substituted into the target type, and the alias remains transparent.

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

Every declared field is required, and unknown fields are rejected. Struct values are immutable; create a copy with selected fields replaced using `%{base | field: value}`:

```mesh
let moved = %{p | x: p.x + 10}
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

Sum types can be generic, and variants can carry positional or named fields:

```mesh
type Outcome<T> do
  Pending
  Complete(value :: T)
  Failed(reason :: String)
end

fn is_complete<T>(outcome :: Outcome<T>) -> Bool do
  case outcome do
    Pending -> false
    Complete(_) -> true
    Failed(_) -> false
  end
end
```

Variant constructors are available unqualified (`Complete(value)`) and qualified (`Outcome.Complete(value)`). Patterns destructure both positional and named variant fields by position.

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

Traits define shared behavior that types can implement. Define a trait with the `interface` keyword and implement it with `impl`:

```mesh
interface Greeter do
  fn greet(self) -> String
end

struct Person do
  name :: String
end

impl Greeter for Person do
  fn greet(self) -> String do
    "Hello, I'm ${self.name}"
  end
end

fn main() do
  let p = Person { name: "Alice" }
  println(p.greet())
end
```

Interfaces can be generic and can declare required associated types. An interface method with a body is a default method; an implementation may omit it:

```mesh
interface Named do
  fn name(self) -> String

  fn label(self) -> String do
    "name=" <> self.name()
  end
end
```

A method without a `self` parameter is static. User-defined conversion methods can be called through the destination type, as in `Wrapper.from(value)`. The compiler-provided `default()` function is resolved from context:

```mesh
let value :: Int = default()
```

An `impl` must provide every required method and associated type with a matching signature. Overlapping implementations for the same trait and type are rejected. If multiple in-scope interfaces provide an equally valid method name, the compiler reports the candidates instead of choosing one arbitrarily.

### Built-in Traits

Mesh provides these compiler-known traits:

| Trait | Contract |
|-------|----------|
| `Add`, `Sub`, `Mul`, `Div`, `Mod` | Binary numeric operators with associated `Output` |
| `Neg` | Unary `-` with associated `Output` |
| `Eq` | `==` and `!=` |
| `Ord` | `<`, `>`, `<=`, `>=`, plus `compare` returning `Ordering` |
| `Not` | Boolean negation |
| `Display` | `to_string()` and interpolation |
| `Debug` | `inspect()` |
| `Hash` | `hash()` returning an `Int` hash value |
| `Default` | Static `default()` constructor |
| `Iterator` | `next()` with associated `Item` |
| `Iterable` | `iter()` with associated `Item` and `Iter` |
| `From<S>`, `Into<T>` | Infallible conversion |
| `TryFrom<S>`, `TryInto<T>` | Fallible conversion |

`Option`, `Result`, and `Ordering` are built-in sum types. `Ordering` has `Less`, `Equal`, and `Greater` constructors.

`deriving(Json)` is convenient syntax that generates `ToJson` and `FromJson` implementations; the derived capability is not a single interface literally named `Json`. Likewise, `deriving(Row)` generates row decoding support and `deriving(Schema)` generates schema metadata.

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

An explicit deriving clause is selective: only the listed capabilities are generated. `deriving()` explicitly generates none. For backward compatibility, omitting the clause has defaults:

| Definition | No `deriving` clause |
|------------|----------------------|
| Struct | `Debug`, `Eq`, `Ord`, `Hash` |
| Sum type | `Debug`, `Eq`, `Ord` |

`Display`, `Json`, `Row`, and `Schema` are never enabled by omission; list them explicitly.

### Deriving on Sum Types

Sum types support `Eq`, `Ord`, `Display`, `Debug`, `Hash`, and `Json`:

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

| Derive | Struct | Sum type | What it generates |
|--------|:------:|:--------:|-------------------|
| `Eq` | Yes | Yes | Structural equality |
| `Ord` | Yes | Yes | Structural ordering |
| `Display` | Yes | Yes | Human-readable `to_string()` |
| `Debug` | Yes | Yes | Detailed `inspect()` |
| `Hash` | Yes | Yes | Hash value |
| `Json` | Yes | Yes | `to_json()` and static `from_json(...)` |
| `Row` | Yes | No | Static row decoding |
| `Schema` | Yes | No | Static database-schema metadata |

An explicit `Ord` derive requires `Eq` in the same list:

```mesh
struct Coordinate do
  x :: Int
  y :: Int
end deriving(Eq, Ord)
```

`deriving(Json)` validates every stored field. Directly supported values include `Int`, `Float`, `Bool`, `String`, generic parameters, `Option`, `List`, `Map<String, V>`, and nested values that implement `ToJson`.

`deriving(Row)` accepts `Int`, `Float`, `Bool`, `String`, and `Option` of those types. `deriving(Schema)` is for structs and emits metadata used by the database/query APIs, including table, fields, primary key, relationships, field types, and column accessors.

## Associated Types

Interfaces can declare associated types -- type members that implementing types must define. This enables generic protocols where the concrete types are determined by the implementation:

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
  let b = IntBox { value: 42 }
  println("${b.first()}")
end
```

Use `Self.Item` in method signatures to reference the associated type. The compiler resolves it to the concrete type from each implementation.

Every implementation must bind each required associated type exactly once. Missing and undeclared bindings are compile errors.

Interfaces can have multiple associated types:

```mesh
interface Mapper do
  type Input
  type Output
  fn apply(self) -> Self.Output
end
```

## Numeric Traits

Mesh provides built-in traits for arithmetic operators. Implement them to use `+`, `-`, `*`, `/`, and `%` with your custom types:

| Trait | Operator | Method |
|-------|----------|--------|
| `Add` | `+` | `add(self, other)` |
| `Sub` | `-` | `sub(self, other)` |
| `Mul` | `*` | `mul(self, other)` |
| `Div` | `/` | `div(self, other)` |
| `Mod` | `%` | `mod(self, other)` |
| `Neg` | `-` (unary) | `neg(self)` |

Each numeric trait has an associated `type Output` that determines the result type:

```mesh
struct Vec2 do
  x :: Float
  y :: Float
end

impl Add for Vec2 do
  type Output = Vec2
  fn add(self, other :: Vec2) -> Vec2 do
    Vec2 { x: self.x + other.x, y: self.y + other.y }
  end
end

impl Neg for Vec2 do
  type Output = Vec2
  fn neg(self) -> Vec2 do
    Vec2 { x: 0.0 - self.x, y: 0.0 - self.y }
  end
end

fn main() do
  let a = Vec2 { x: 1.0, y: 2.0 }
  let b = Vec2 { x: 3.0, y: 4.0 }
  let sum = a + b
  let neg = -a
  println("${sum.x}, ${sum.y}")
  println("${neg.x}, ${neg.y}")
end
```

## From/Into Conversion

The `From` trait defines how to convert one type into another. Implement `From<SourceType> for TargetType` with a `from` function:

```mesh
struct Wrapper do
  value :: Int
end

impl From<Int> for Wrapper do
  fn from(n :: Int) -> Wrapper do
    Wrapper { value: n * 2 }
  end
end

fn main() do
  let w = Wrapper.from(21)
  println("${w.value}")
end
```

### Automatic Into

Every `impl From<Source> for Target` also makes `Into<Target>` available on the source. Give the result an annotation when the target is otherwise ambiguous:

```mesh
fn main() do
  let w :: Wrapper = 21.into()
  println("#{w.value}")
end
```

You do not write the corresponding `Into` implementation yourself.

### Built-in Conversions

Mesh provides built-in `From` implementations for common type conversions:

| Conversion | Example | Result |
|------------|---------|--------|
| Int to Float | `Float.from(42)` | `42.0` |
| Int to String | `String.from(42)` | `"42"` |
| Float to String | `String.from(3.14)` | `"3.14"` |
| Bool to String | `String.from(true)` | `"true"` |

### Error Type Conversion with ?

When you implement `From<SourceError> for TargetError`, the `?` operator automatically converts error types. This lets you compose functions with different error types:

```mesh
struct AppError do
  message :: String
end

impl From<String> for AppError do
  fn from(msg :: String) -> AppError do
    AppError { message: msg }
  end
end

fn risky() -> Int!String do
  Err("something failed")
end

fn process() -> Int!AppError do
  let n = risky()?    # auto-converts String error to AppError
  Ok(n + 1)
end

fn main() do
  let r = process()
  case r do
    Ok(val) -> println("${val}")
    Err(e) -> println(e.message)
  end
end
```

## TryFrom/TryInto Conversion

`TryFrom` and `TryInto` are for fallible conversions -- conversions that can fail and return a `Result`. Where `From` always succeeds, `TryFrom` returns `Result<TargetType, ErrorType>` so callers can handle the failure case explicitly.

### Implementing TryFrom

Implement `TryFrom<SourceType>` for your type with a `try_from` function that returns `Result<Self, E>`. Call it via `TargetType.try_from(value)`:

```mesh
struct PositiveInt do
  value :: Int
end

impl TryFrom<Int> for PositiveInt do
  fn try_from(n :: Int) -> Result<PositiveInt, String> do
    if n > 0 do
      Ok(PositiveInt { value: n })
    else
      Err("must be positive")
    end
  end
end

fn main() do
  let r = PositiveInt.try_from(42)
  case r do
    Ok(p) -> println("${p.value}")    # prints: 42
    Err(e) -> println("error: ${e}")
  end
  let r2 = PositiveInt.try_from(-1)
  case r2 do
    Ok(p) -> println("${p.value}")
    Err(e) -> println("${e}")         # prints: must be positive
  end
end
```

### Automatic TryInto

When you implement `TryFrom<F>` for a type, `TryInto` is automatically available on the source type -- you never need to write a `TryInto` impl yourself. Call `.try_into()` on the source value with a type annotation so the compiler knows what target type to use:

```mesh
# No TryInto impl needed -- derived automatically from TryFrom<Int> for PositiveInt
fn main() do
  let r :: Result<PositiveInt, String> = 42.try_into()
  case r do
    Ok(p) -> println("${p.value}")    # prints: 42
    Err(e) -> println("error: ${e}")
  end
  let r2 :: Result<PositiveInt, String> = (-5).try_into()
  case r2 do
    Ok(p) -> println("${p.value}")
    Err(e) -> println("${e}")         # prints: must be positive
  end
end
```

### Using ? with TryFrom

The `?` operator works naturally with `try_from` and `try_into` results, just like it does with any `Result`. If the conversion fails, `?` propagates the `Err` immediately -- no manual case matching needed at the call site:

```mesh
fn double_positive(n :: Int) -> Int!String do
  let p = PositiveInt.try_from(n)?   # propagates Err if n <= 0
  Ok(p.value * 2)
end

fn main() do
  case double_positive(21) do
    Ok(v) -> println("${v}")         # prints: 42
    Err(e) -> println("error: ${e}")
  end
  case double_positive(-1) do
    Ok(v) -> println("${v}")
    Err(e) -> println("${e}")        # prints: must be positive
  end
end
```

TryFrom/TryInto is for fallible conversions. For infallible conversions, use [From/Into](#from-into-conversion).

## Next Steps

- [Iterators](/docs/iterators/) -- lazy iterator pipelines, combinators, and collection materialization
- [Concurrency](/docs/concurrency/) -- actors, message passing, and supervision
- [Syntax Cheatsheet](/docs/cheatsheet/) -- quick reference for all Mesh syntax
