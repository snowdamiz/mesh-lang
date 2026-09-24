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

The `?` and `!` suffixes apply to a tuple type too: `(Int, String)?` is `Option<(Int, String)>`, and `(Int, Int)!String` is `Result<(Int, Int), String>`.

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

A `let` is generalized when its value is a literal, a name, a closure (`let id = fn x -> x end`) or a module's function (`let len = String.length`); each use of such a binding gets its own copy. A value that is computed, such as a call's result (`let f = make_identity()`), is computed once, so it has the one type its uses agree on, and using it at two types is a type error.

Recursive functions and functions declared later in the same module are registered before their bodies are checked, so mutually recursive definitions can refer to one another.

### When to Annotate

Type annotations are optional in many places, but recommended for:

- **Function signatures** -- makes the API clear to readers
- **Complex generic functions** -- helps the compiler and your teammates
- **Public interfaces** -- documents the contract

An annotation must name a type that exists: a built-in type, one defined or imported in the module, or a type parameter in scope. A misspelled name such as `Strng` is an error (E0069), not a new type.

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

A type parameter stands for whatever type each caller picks, so the body has to treat its values as values of an unknown type. `fn bad<T>(x :: T) -> T do 5 end` is an error (E0067): it would only work when `T` is `Int`.

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

`<>` and `++` join two `String`s or two `List`s, so applying them to values of a type parameter is always an error (E0073), whatever its bounds. Convert the values first, as `combine` does with `to_string()`.

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

Generic arguments are substituted into the target type, and the alias remains transparent. An alias may name another alias (`type Twin<T> = Pair<T, T>`), and a struct literal may be written through an alias of a struct (`IntBox { value: 5 }` with `type IntBox = Box<Int>`). An alias cannot refer to itself; a recursive type needs a struct or sum type.

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

Variant constructors are available unqualified (`Complete(value)`) and qualified (`Outcome.Complete(value)`). Patterns destructure both positional and named variant fields by position. Within a module, a variant name belongs to one sum type; a module's own variant shadows an imported variant of the same name, which stays reachable qualified (`GroupTreeError.InvalidMember`).

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

An `impl` is for a type without type parameters: one for a generic type (`impl Greeter for Box` with `struct Box<T>`, or `for List<Int>`) is not supported and is reported at its header. Derived traits (`deriving(...)`) do cover generic types.

Interfaces can be generic and can declare required associated types. An interface method with a body is a default method; an implementation may omit it:

```mesh
interface Named do
  fn name(self) -> String

  fn label(self) -> String do
    "name=" <> self.name()
  end
end
```

An `impl` must provide every required method and associated type with a matching signature. Overlapping implementations for the same trait and type are rejected. If multiple in-scope interfaces provide an equally valid method name, the compiler reports the candidates instead of choosing one arbitrarily; name the interface to pick one: `Named.hello(value)`. A method can also be called as a function of its receiver, `hello(value)`, which dispatches by the value's type like `value.hello()`.

The standard library works the other way round too: a `String`, `List`, `Map`, `Set` or `Range` value takes its module's functions as methods, with itself as the first argument. `"mesh".length()` is `String.length("mesh")`, and `list.map(f)`, `m.get(key)` and `(1..4).to_list()` work the same way (see [Method-Call Syntax](/docs/language-basics/#method-call-syntax)).

### Static Methods

A method without a `self` parameter is static. Call it on the implementing type:

```mesh
interface Versioned do
  fn version() -> Int
end

struct Config do
  path :: String
end

struct Schema do
  name :: String
end

impl Versioned for Config do
  fn version() -> Int do
    1
  end
end

impl Versioned for Schema do
  fn version() -> Int do
    3
  end
end

impl Versioned for Int do
  fn version() -> Int do
    0
  end
end

fn version_of<T>(_value :: T) -> Int where T: Versioned do
  T.version()
end

fn main() do
  println("#{Config.version()} #{Schema.version()} #{Int.version()}")  # 1 3 0
  println("#{version_of(Config { path: "app.toml" })}")                # 1
end
```

In a generic function, a type parameter bounded by the interface names the impl: `T.version()` calls the one for the type `T` stands for. A built-in type can be the receiver too, as `Int.version()` shows. A bare `version()` works when exactly one type implements the method; when several do, as here, it is error E0066, and the compiler suggests the qualified call.

Conversions are static methods as well: `Wrapper.from(value)` (see [From/Into Conversion](#from-into-conversion)). The compiler-provided `default()` function is resolved from context:

```mesh
let value :: Int = default()
```

### `Self` in Interface Signatures

In an interface, `Self` stands for the implementing type. A method can take or return it. An impl may write `Self` too, where it means the type the impl is for, or name that type, as `merge` does here:

```mesh
interface Scalable do
  fn scale(self, factor :: Int) -> Self
  fn merge(self, other :: Self) -> Self
end

struct Size do
  width :: Int
  height :: Int
end

impl Scalable for Size do
  fn scale(self, factor :: Int) -> Self do
    Size { width: self.width * factor, height: self.height * factor }
  end

  fn merge(self, other :: Size) -> Size do
    Size { width: self.width + other.width, height: self.height + other.height }
  end
end

fn grow<T>(value :: T) -> T where T: Scalable do
  value.scale(2).merge(value)
end

fn main() do
  let s = grow(Size { width: 2, height: 3 })
  println("#{s.width}x#{s.height}")  # 6x9
end
```

A generic caller gets its own type back: `grow` returns a `T`. `Self.Item` names an associated type of the implementing type; see [Associated Types](#associated-types).

### Choosing an Impl by Result Type

A type can implement one generic interface several times with different type arguments. When the impls differ only in what a method returns, the type the call is expected to have picks one:

```mesh
interface Convert<T> do
  fn convert(self) -> T
end

struct Meters do
  value :: Int
end

impl Convert<String> for Meters do
  fn convert(self) -> String do
    "#{self.value}m"
  end
end

impl Convert<Int> for Meters do
  fn convert(self) -> Int do
    self.value * 100
  end
end

fn centimeters(m :: Meters) -> Int do
  m.convert()
end

fn main() do
  let m = Meters { value: 3 }
  let label :: String = m.convert()
  println("#{label} #{centimeters(m)}")  # 3m 300
end
```

An annotation or a declared return type fixes the expected type. When nothing does, as in `let x = m.convert()`, the call is error E0065, which lists the result types of the candidate impls.

### Built-in Traits

Mesh provides these compiler-known traits:

| Trait | Contract |
|-------|----------|
| `Add`, `Sub`, `Mul`, `Div`, `Mod` | Binary numeric operators with associated `Output` |
| `Neg` | Unary `-` with associated `Output` |
| `Eq` | `==` and `!=` |
| `Ord` | `<`, `>`, `<=`, `>=`, plus `compare` returning `Ordering`; an impl defines `lt(self, other) -> Bool`, and `compare` follows from it |
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

Tuples, unit, `Option`, `Result`, `Ordering`, lists, maps and sets compare with `==` by their contents (a list of tuples of options works too), and tuples, lists, `Option`, `Result` and `Ordering` also order with `<` (element by element) and print with `to_string()`, `inspect()` and interpolation: `(1, "a")` prints as `(1, a)`, `Some(2.5)` as `Some(2.5)`, and `inspect()` quotes and escapes strings at every level (a list holding the string `a"b` inspects as `["a\"b"]`). `Bool` orders `false < true`. `compare(a, b)` works on any ordered value. A `Float` always prints with a decimal point (`42.0`, never `42`; `1.0e20` and `1.5e-7` for very large and very small values), except the non-finite `inf`, `-inf` and `NaN`.

Derived traits treat each field by its own type: a `List<String>` field prints as a list of strings, compares element by element, and hashes consistently with `==` (so equal values always have equal `hash()`), in generic types too (`Box<List<Int>>` and `Box<List<String>>` each get their own).

A struct field of function type is called directly: for `struct Op do run :: Fun(Int) -> Int end`, `op.run(10)` calls the function stored in the field.

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

`Display`, `Json`, `Row`, and `Schema` are never enabled by omission; list them explicitly. A type with a field that holds a function gets none of `Debug`, `Eq`, `Ord` and `Hash` by default (a function cannot be compared, hashed or shown), and listing one of them is an error. A [resource type](#resource-types) derives nothing at all.

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

`deriving(Json)` validates every stored field. Directly supported values include `Int`, `Float`, `Bool`, `String`, generic parameters, tuples (as JSON arrays), `Option` (`None` is `null`), `List`, `Map<String, V>`, and values of types that derive `Json` — including the type itself and types declared later in the module. A generic type decodes at the instantiation the context asks for: `let r :: Result<Box<Int>, String> = Box.from_json(text)`. Decoding an `Int` field accepts only a whole number in `Int` range.

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

Every `impl From<Source> for Target` also makes `Into<Target>` available on the source. `into()` converts to the type the call is expected to have, so it needs an annotation or another context that fixes the target. The same applies to the built-in conversions below and to a source with several `From` impls:

```mesh
fn main() do
  let w :: Wrapper = 21.into()
  let f :: Float = 2.into()
  let s :: String = 7.into()
  println("#{w.value} #{f} #{s}")  # 42 2.0 7
end
```

Without a target, as in `let x = 5.into()`, the call is error E0065. You do not write the corresponding `Into` implementation yourself.

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

## Resource Types

A resource type is affine: each of its values can be moved at most once. Use one for a value that stands for something that must not be duplicated, such as an open connection or key material. The compiler tracks every resource and reports misuse as error E0053 (`resource ownership violation`).

```mesh
resource struct Session do
  id :: Int
  user :: String
end

fn describe(session :: borrow Session) -> String do
  "session #{session.id} for #{session.user}"
end

fn rename(session :: Session, user :: String) -> Session do
  %{session | user: user}
end

fn close(session :: consume Session) -> Int do
  println("closing #{session.id}")
  session.id
end

fn main() do
  let session = Session { id: 1, user: "ada" }
  println(describe(session))
  let session = rename(session, "grace")
  println(describe(session))
  let closed = close(session)
  println("closed #{closed}")
end
```

Using `session` after `close(session)` would be an error: ``resource `session` was used after it moved``.

### Declaring Resources

| Declaration | Meaning |
|-------------|---------|
| `resource struct Name do ... end` | A struct whose values are resources. It is built, read and updated like any struct. |
| `resource Name` | An opaque resource: it has no fields, and its name is not a constructor, so Mesh code cannot create a value of it. |
| `pub resource Name`, `pub resource struct Name do ... end` | Exported forms |

A struct or sum type with a field that holds a resource is a resource too, and so is an `Option`, `Result` or tuple that holds one.

### Moves and Parameter Modes

Binding a resource to another name, passing it to a function, returning it, storing it in a struct, variant or tuple, and updating it with `%{value | field: new}` all move it. After a move the old name cannot be used. If either branch of an `if` or `case` moves a resource, it counts as moved afterwards, and a loop cannot move a resource from outside the loop, because the loop could run more than once. Reading a field that is not itself a resource, such as `session.id`, does not move the value; taking out a field that holds a resource moves the whole value, unless the field goes straight to a `borrow` parameter.

A parameter's mode says what a call does with a resource argument:

| Parameter | Effect on the caller's value |
|-----------|------------------------------|
| `x :: T` | Moved into the call |
| `x :: consume T` | Moved into the call; states the intent explicitly |
| `x :: borrow T` | Lent for the duration of the call; the caller keeps it |

A function cannot move a borrowed parameter (``borrowed resource `x` cannot be moved``), but it can read its fields and pass it on to another `borrow` parameter. A function may return without moving a resource parameter it owns.

### Restrictions

A resource cannot:

- be sent to an actor, passed to `spawn`, or captured by a closure;
- be interpolated, printed, compared with `==`, hashed, or encoded as JSON;
- go into a `List`, `Map` or `Set`, and `List<Session>` is not a valid annotation;
- pass through an indirect call, such as a function held in a variable, or through a parameter of a generic type; call a named function directly;
- be bound by a top-level `let`.

A `case` arm that binds a resource must move it on every path out of the arm, and a pattern cannot discard one with `_`.

A resource type derives nothing: listing any trait in its `deriving(...)` is an error, and the default `Debug`, `Eq`, `Ord` and `Hash` are not generated.

### Standard Library Resources

`PgConn` is a resource. `Pg.connect` returns one, the query functions such as `Pg.execute` and `Pg.query` borrow it, and `Pg.close` consumes it. A `Pg.transaction` or `Repo.transaction` callback must declare its connection parameter as `conn :: borrow PgConn` (see [Databases](/docs/databases/)). `SqliteConn` is an ordinary value.

`SecretBytes`, `AeadKey` and the private keys of the `Crypto` key pairs are resources, which makes `X25519KeyPair`, `SigningKeyPair` and `MlKemKeyPair` resources too; so is `BytesBuilder`, whose writes borrow it and whose `finish` consumes it (see the [Standard Library](/docs/stdlib/)).

## Next Steps

- [Iterators](/docs/iterators/) -- lazy iterator pipelines, combinators, and collection materialization
- [Concurrency](/docs/concurrency/) -- actors, message passing, and supervision
- [Syntax Cheatsheet](/docs/cheatsheet/) -- quick reference for all Mesh syntax
