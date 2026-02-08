# Feature Research: Trait System & Stdlib Protocols

**Domain:** Trait/type class system and standard library protocols for a statically-typed functional language
**Researched:** 2026-02-07
**Confidence:** HIGH (extremely well-established domain: Rust, Haskell, Scala 3, Swift, Elixir all provide extensive prior art)

---

## Current State in Snow

Before defining features, here is what already exists in Snow:

**Working:**
- `interface` / `impl` parsing (parser emits `InterfaceDef` and `ImplDef` AST nodes)
- `TraitRegistry` validates trait definitions and impl method signatures (param count, return type matching)
- Where clauses: `fn show<T>(x :: T) where T: Printable` checked at call sites
- Compiler-known traits: Add, Sub, Mul, Div, Mod, Eq, Ord, Not with primitive impls
- Operator overloading dispatches through trait lookup
- MIR lowering: impl methods are lowered as standalone functions; interface defs are erased

**Not yet working:**
- Codegen for trait method dispatch (currently skips InterfaceDef/ImplDef in LLVM codegen)
- No monomorphization of generic trait-bounded functions
- No default method implementations
- No associated types
- No supertraits (trait inheritance)
- No deriving mechanism
- No stdlib protocols (Display, Hash, Default, Iterator, From/Into, etc.)
- No method call syntax (`value.method()` -- only `method(value)`)

---

## Feature Landscape

### Table Stakes (Users Expect These)

These features are required for the trait system to be usable. Without them, users cannot write polymorphic code that compiles and runs.

| Feature | Why Expected | Complexity | Dependencies | Notes |
|---------|--------------|------------|--------------|-------|
| User-defined traits with codegen | Without codegen, traits are purely decorative. Users define interfaces and expect code using them to compile to native binaries. | High | MIR lowering, LLVM codegen, monomorphization | Currently MIR lowering extracts impl methods as standalone functions but does not wire up trait-bounded dispatch. Needs monomorphization pass that creates specialized copies. |
| Trait method dispatch (static) | When calling `to_string(42)` where `to_string` comes from a trait impl, the compiler must resolve which impl's method body to call at compile time. | High | TraitRegistry lookup, MIR, codegen | Monomorphization: at each call site, resolve the concrete type, look up the impl, and emit a direct call to the specialized function. No vtables needed for v1.3. |
| Where clause enforcement in codegen | Where clauses type-check but must also affect monomorphization -- the compiler needs to select the right impl body based on the concrete type substituted for T. | Medium | Existing where-clause checking, monomorphization | The type checker already validates constraints. Codegen needs to thread concrete types through to monomorphized function names. |
| Display / to_string protocol | Every language needs a way to convert values to string representations. Currently `println` only works with String. Users need `"${my_struct}"` to work for user types. | Medium | Trait system codegen, string interpolation integration | Rust: `Display`, Haskell: `Show`, Elixir: `String.Chars`, Swift: `CustomStringConvertible`. Snow should call it `Display` with a `to_string(self) -> String` method. Auto-impl for primitives (Int, Float, Bool already have runtime support). |
| Debug / inspect protocol | Programmers need to see the structure of values during development. Distinct from Display (user-facing). | Medium | Trait system codegen | Rust: `Debug`, Haskell: `Show` (serves both purposes), Elixir: `Inspect`. Snow should call it `Debug` with an `inspect(self) -> String` method. Should show structure: `Point { x: 1, y: 2 }`. Consider auto-derive for structs and sum types. |
| Eq protocol (user types) | Currently Eq only works for primitives. Users need `==` on structs and sum types. | Medium | Existing Eq trait, operator dispatch, struct/sum type codegen | Structural equality for structs (all fields equal). Tag + field equality for sum types. This is the single most requested feature once people start defining types. |
| Ord protocol (user types) | Users need `<`, `>`, `<=`, `>=` on custom types for sorting, ranges, etc. | Medium | Existing Ord trait, Eq dependency | Requires Eq as semantic prerequisite (same values must not be ordered differently). Lexicographic ordering for structs (field by field). |
| Default method implementations | Traits should be able to provide default method bodies that implementors can override. Reduces boilerplate massively. | Medium | Trait definition parsing, impl validation | Rust: `fn method(&self) { default_body }`, Haskell: default in class definition, Swift: protocol extensions. Critical for keeping stdlib protocols ergonomic -- e.g., Display could provide a default `inspect` that wraps `to_string`. |
| Multiple trait bounds | `where T: Display, T: Eq` already parses. Need `where T: Display + Eq` shorthand syntax. | Low | Parser, where-clause checking | Already works with comma-separated constraints. The `+` syntax is syntactic sugar. Low priority but expected. |
| Impl for user-defined structs | Users need to write `impl Display for Point do ... end` for their own struct types and have it work end-to-end. | High | Full trait dispatch pipeline | This is the core use case. If this doesn't work, nothing else matters. |
| Impl for user-defined sum types | Same as structs but for sum types (ADTs). `impl Display for Color do ... end`. | High | Full trait dispatch pipeline, pattern matching in impl methods | Sum type impls often need to pattern match on variants inside the method body. |

### Differentiators (Competitive Advantage)

Features that would make Snow's trait system stand out. Not expected, but highly valued.

| Feature | Value Proposition | Complexity | Dependencies | Notes |
|---------|-------------------|------------|--------------|-------|
| Auto-derive for common traits | `type Point do x :: Int, y :: Int end deriving(Eq, Ord, Display, Debug)` -- compiler generates implementations automatically. Massive ergonomic win. | High | Compiler codegen for each derivable trait, struct/sum type metadata | Rust: `#[derive(...)]`, Haskell: `deriving (Eq, Show, Ord)`, Swift: auto-synthesis. The single biggest ergonomic improvement for a trait system. Without it, every struct requires ~10 lines of boilerplate per trait. |
| Coherent auto-impls for primitives | All primitives automatically implement Display, Debug, Eq, Ord, Hash, Default without user action. Users can immediately use `to_string(42)` or `42 == 42`. | Medium | Builtin trait registration (extend existing register_compiler_known_traits) | Already partially done for arithmetic/comparison traits. Extending to Display/Debug/Hash/Default is incremental. |
| From/Into conversion protocol | Type-safe conversions: `impl From<Int> for MyId`. Implementing `From` auto-provides `Into`. Enables `let id = from(42)` or pipeline `42 |> into`. | Medium | Trait system, blanket impl mechanism | Rust: `From`/`Into`, Haskell: explicit conversion functions. The blanket impl (`From` implies `Into`) requires the concept of blanket/generic impls -- may be complex. Could start with manual both-ways. |
| Iterator/Iterable protocol | Unified iteration: any type implementing `Iterator` works with `map`, `filter`, `reduce`. Currently these only work with `List`. | Very High | Associated types (for Item type), lazy evaluation infrastructure, collection integration | Rust: `Iterator` with `type Item` + `next()`, Haskell: `Foldable`/`Traversable`, Elixir: `Enumerable`. This is a massive feature. Associated types are a prerequisite. Consider deferring to v1.4+. |
| Hash protocol | Required for user types as Map keys or Set elements. `impl Hash for Point`. | Medium | Hash function codegen, Map/Set integration | Rust: `Hash`, Haskell: `Hashable`. Structural hashing for structs (hash all fields). Must maintain invariant: `a == b` implies `hash(a) == hash(b)`. |
| Default protocol | Construct default values: `Default.default()` gives a zero-initialized or sensible default instance. | Low | Trait system codegen | Rust: `Default`, Haskell: `Def`/`Monoid mempty`. Useful for builder patterns, container defaults. Low complexity but moderate value. |
| Supertraits (trait inheritance) | `interface Ord requires Eq` -- Ord requires Eq to be implemented first. Enables type system to infer that `T: Ord` implies `T: Eq`. | Medium | Trait definition parsing, constraint propagation in type checker | Rust: `trait Ord: Eq`, Haskell: `class Eq a => Ord a`. Essential for a principled trait hierarchy but can be added incrementally. |
| Trait method dot-syntax | `my_point.to_string()` instead of `to_string(my_point)`. Uniform function call syntax. | High | Parser changes, method resolution, UFCS infrastructure | Elixir uses pipe `value |> to_string`, Rust uses dot syntax. Snow already has pipes, so dot syntax is less critical. But combined with traits, it makes code read naturally: `point.display()`. Could defer to v1.4+. |
| Negative trait bounds | `where T: not Clone` -- specify that a type must NOT implement a trait. | High | Type checker, constraint resolution | Rust doesn't have stable negative bounds. Haskell doesn't have them. Very niche. Probably never needed. |

### Anti-Features (Commonly Requested, Often Problematic)

Features to deliberately NOT build, or to build with extreme caution.

| Feature | Why Requested | Why Problematic | Alternative |
|---------|---------------|-----------------|-------------|
| Dynamic dispatch (vtables, trait objects) | "I want heterogeneous collections: `List<dyn Display>`" | Adds runtime overhead, complicates the type system massively, requires heap allocation for trait objects, and conflicts with HM inference. Rust's `dyn Trait` is widely considered the most confusing part of the language. | Use sum types (ADTs) for heterogeneous collections. `type Printable do StringVal(String), IntVal(Int) end`. This is the functional approach and works perfectly with exhaustive pattern matching. |
| Orphan impls (impl foreign trait for foreign type) | "I want to impl Display for List" when both Display and List are in stdlib | Breaks coherence: two libraries could both impl the same trait for the same type, causing ambiguity. The orphan rule exists to prevent this. | Use newtype wrappers: `type MyList do items :: List end` then `impl Display for MyList`. Or allow orphan impls only within the same compilation unit (single-file programs). |
| Implicit trait resolution (Scala 2 implicits) | "Type class instances should be automatically resolved from scope" | Scala 2's implicits were universally criticized as confusing, hard to debug, and a source of mysterious compilation errors. Scala 3 replaced them with explicit `given`/`using`. | Explicit impl blocks. `impl Trait for Type do ... end`. No magic resolution. If a trait is not implemented, the compiler says so clearly. |
| Operator overloading beyond existing traits | "I want to define `+` for my custom Point type" | Already possible via `impl Add for Point`. The anti-feature is allowing arbitrary operator invention (`+++`, `<=>`, etc.) which makes code unreadable. | Restrict operator overloading to the existing compiler-known trait set (Add, Sub, Mul, Div, Mod, Eq, Ord, Not). Users can add impls for these but cannot invent new operators. |
| Higher-kinded types (HKT) | "I want Functor, Applicative, Monad as traits" | Massively increases type system complexity. Haskell has them; most other languages deliberately avoid them. HKT requires kind checking, higher-kinded inference, and makes error messages incomprehensible. | Provide specific patterns (map/filter/reduce) as module functions or dedicated protocols without requiring the full Functor/Applicative/Monad hierarchy. Snow's pipe operator handles most composition needs. |
| Multi-parameter type classes | "I want `interface Convert<A, B> do fn convert(a :: A) -> B end`" | Complex interaction with type inference. Requires functional dependencies or associated types to be usable. Haskell's MPTC + FunDeps is powerful but a well-known source of confusion. | Use single-parameter traits with associated types for conversions: `interface From<T> do fn from(value :: T) -> Self end`. The "Self" type is the implementing type, "T" is the source type. |
| Specialization | "A more specific impl should override a more general one" | Unsound in general (Rust's specialization has been unstable for years due to soundness issues). Allows overlapping impls which break coherence guarantees. | No overlapping impls. Each (trait, type) pair has exactly one impl. Use newtype wrappers for specialization needs. |
| Automatic Serialize/Deserialize | "Every type should automatically be serializable to JSON/MessagePack" | Serialization is inherently lossy and format-dependent. Auto-deriving works for simple cases but creates brittle APIs where internal type changes break serialization contracts. | Provide `Serialize` and `Deserialize` as derivable traits with explicit opt-in: `type Config do ... end deriving(Serialize)`. Not automatic, not universal. Defer to v1.4+ or later. |

---

## Stdlib Protocol Specifications

Detailed behavioral specifications for each stdlib protocol, with examples in Snow syntax.

### 1. Display (String Representation)

**Purpose:** Convert a value to a human-readable string. Used by string interpolation (`"${value}"`), `println`, and explicit `to_string()` calls.

**Trait Definition:**
```snow
interface Display do
  fn to_string(self) -> String
end
```

**Built-in Impls:**
```snow
impl Display for Int do
  fn to_string(self) -> String do
    # compiler intrinsic: int_to_string
  end
end

impl Display for Float do ... end
impl Display for Bool do ... end    # "true" / "false"
impl Display for String do ... end  # identity
```

**User Impl Example:**
```snow
type Point do
  x :: Int
  y :: Int
end

impl Display for Point do
  fn to_string(self) -> String do
    "(${self.x}, ${self.y})"
  end
end

# Usage:
let p = Point { x: 1, y: 2 }
println("Point: ${p}")  # "Point: (1, 2)"
```

**Design Decisions:**
- String interpolation `"${expr}"` should call `to_string` on any type implementing Display
- If a type does not implement Display, string interpolation is a compile error
- Display for collections (List, Map) should be provided as stdlib impls

**Precedent:** Rust `Display`, Haskell `Show`, Elixir `String.Chars`, Swift `CustomStringConvertible`

**Confidence:** HIGH

### 2. Debug (Developer Inspection)

**Purpose:** Show the internal structure of a value for debugging. More verbose than Display.

**Trait Definition:**
```snow
interface Debug do
  fn inspect(self) -> String
end
```

**Built-in Impls:**
```snow
impl Debug for Int do
  fn inspect(self) -> String do to_string(self) end
end

impl Debug for Point do
  fn inspect(self) -> String do
    "Point { x: ${self.x}, y: ${self.y} }"
  end
end
```

**Design Decisions:**
- Debug should be auto-derivable for structs and sum types
- Debug output shows type name, field names, and field values
- For sum types: `"Color::Red"` or `"Shape::Circle(5.0)"`
- Consider a `dbg()` built-in that prints and returns the value (like Rust's `dbg!`)

**Precedent:** Rust `Debug`, Elixir `Inspect`

**Confidence:** HIGH

### 3. Eq (Equality)

**Purpose:** Test two values of the same type for equality. Backs the `==` and `!=` operators.

**Trait Definition (already exists, extend to user types):**
```snow
interface Eq do
  fn eq(self, other :: Self) -> Bool
end
```

**Current State:** Already registered as compiler-known trait with impls for Int, Float, String, Bool.

**Extension Needed:**
```snow
# Auto-derive for structs: all fields must be Eq
type Point do
  x :: Int
  y :: Int
end deriving(Eq)

# Generated impl:
impl Eq for Point do
  fn eq(self, other :: Point) -> Bool do
    self.x == other.x and self.y == other.y
  end
end

# For sum types:
type Color do Red, Green, Blue end deriving(Eq)

# Generated impl:
impl Eq for Color do
  fn eq(self, other :: Color) -> Bool do
    case (self, other) do
      (Red, Red) -> true
      (Green, Green) -> true
      (Blue, Blue) -> true
      _ -> false
    end
  end
end
```

**Design Decisions:**
- Structural equality: two values are equal if all their fields are equal
- Eq is the foundation -- Ord, Hash depend on it
- Float equality should use IEEE comparison (NaN != NaN) -- same as Rust's PartialEq, not Eq. Consider whether Snow needs PartialEq vs Eq distinction. **Recommendation: Keep it simple with just `Eq` and document that Float equality follows IEEE rules.** The PartialEq/Eq distinction in Rust confuses beginners and mostly matters for HashMap keys.
- For sum types with data: tags must match AND fields must be equal

**Precedent:** Rust `PartialEq`/`Eq`, Haskell `Eq`, Swift `Equatable`, Elixir `==` (structural by default)

**Confidence:** HIGH

### 4. Ord (Ordering)

**Purpose:** Define a total ordering on values. Backs `<`, `>`, `<=`, `>=` operators and enables sorting.

**Trait Definition (already exists, extend):**
```snow
interface Ord do
  fn cmp(self, other :: Self) -> Ordering
end
```

**Design Note:** This requires an `Ordering` sum type:
```snow
type Ordering do
  Less
  Equal
  Greater
end
```

**Current State:** Ord exists with impls for Int and Float, but returns Bool (not Ordering). This should be refactored to return an Ordering type, with `<`, `>`, etc. derived from `cmp`.

**Auto-derive:**
```snow
type Point do x :: Int, y :: Int end deriving(Eq, Ord)

# Generated: lexicographic comparison (first field, then second, etc.)
impl Ord for Point do
  fn cmp(self, other :: Point) -> Ordering do
    case cmp(self.x, other.x) do
      Equal -> cmp(self.y, other.y)
      result -> result
    end
  end
end
```

**Design Decisions:**
- Ord should semantically require Eq (supertrait relationship, even if not enforced initially)
- Lexicographic ordering for derived impls (compare fields in declaration order)
- For sum types: compare by variant tag first, then by variant fields
- Operators `<`, `>`, `<=`, `>=` should be defined in terms of `cmp` returning Ordering

**Precedent:** Rust `PartialOrd`/`Ord`, Haskell `Ord`, Swift `Comparable`

**Confidence:** HIGH

### 5. Hash (Hashing)

**Purpose:** Produce a stable hash value for use in Map keys and Set elements.

**Trait Definition:**
```snow
interface Hash do
  fn hash(self) -> Int
end
```

**Built-in Impls:**
```snow
impl Hash for Int do
  fn hash(self) -> Int do self end  # identity hash for integers
end

impl Hash for String do
  fn hash(self) -> Int do
    # compiler intrinsic: FNV-1a or similar
  end
end

impl Hash for Bool do
  fn hash(self) -> Int do
    if self do 1 else 0 end
  end
end
```

**Auto-derive:**
```snow
type Point do x :: Int, y :: Int end deriving(Eq, Hash)

# Generated: combine field hashes
impl Hash for Point do
  fn hash(self) -> Int do
    hash(self.x) * 31 + hash(self.y)
  end
end
```

**Critical Invariant:** If `a == b` then `hash(a) == hash(b)`. The compiler should warn (or error) if Hash is derived without Eq, or if Eq is manually implemented but Hash is derived (risking inconsistency).

**Design Decisions:**
- Return type is Int (64-bit) for simplicity
- Hash combining uses a simple polynomial hash (multiply by prime, add next field)
- Float hashing: hash the bit representation, but document that NaN hashes are unpredictable
- Hash is needed for Map<K, V> keys and Set<T> elements -- without it, only primitive keys work

**Precedent:** Rust `Hash`, Haskell `Hashable`, Swift `Hashable`

**Confidence:** HIGH

### 6. Default (Default Values)

**Purpose:** Construct a "zero" or default instance of a type.

**Trait Definition:**
```snow
interface Default do
  fn default() -> Self
end
```

**Note:** This is a static method (no `self` parameter). This has implications for the trait system -- currently all trait methods take `self`. Default is the first trait that requires a static/associated function. This is a design decision point.

**Built-in Impls:**
```snow
impl Default for Int do fn default() -> Int do 0 end end
impl Default for Float do fn default() -> Float do 0.0 end end
impl Default for String do fn default() -> String do "" end end
impl Default for Bool do fn default() -> Bool do false end end
```

**Auto-derive:**
```snow
type Config do
  host :: String
  port :: Int
  debug :: Bool
end deriving(Default)

# Generated:
impl Default for Config do
  fn default() -> Config do
    Config { host: default(), port: default(), debug: default() }
  end
end
```

**Design Decisions:**
- Requires static method support in traits (no `self` parameter)
- All fields must implement Default for auto-derive to work
- Useful for builder patterns and zero-initialization
- Lower priority than Display/Eq/Ord -- can defer if static methods are complex

**Precedent:** Rust `Default`, Haskell (various, `mempty`), Swift (no direct equivalent)

**Confidence:** MEDIUM (static method support is a design question)

### 7. From / Into (Type Conversions)

**Purpose:** Infallible type conversion. `From<A>` on type B means "B can be constructed from an A."

**Trait Definition:**
```snow
interface From<T> do
  fn from(value :: T) -> Self
end
```

**Examples:**
```snow
type UserId do value :: Int end

impl From<Int> for UserId do
  fn from(value :: Int) -> UserId do
    UserId { value: value }
  end
end

# Usage:
let id = from(42)   # UserId { value: 42 }
let id2 = 42 |> from  # works with pipes
```

**Design Decisions:**
- `From` is a parameterized trait -- requires single-parameter type class support
- In Rust, implementing `From<A> for B` auto-provides `Into<B> for A`. This requires blanket impl support.
- **Recommendation for v1.3:** Implement `From` without the automatic `Into` blanket. Users implement `From` and call `from()` explicitly. Add `Into` blanket in v1.4.
- `TryFrom` / `TryInto` (fallible conversions returning Result) should be deferred to v1.4+
- From<String> for Int (parsing) is inherently fallible -- use a separate `parse` function, not From

**Precedent:** Rust `From`/`Into`, Haskell (explicit conversion functions), Scala `Conversion`

**Confidence:** MEDIUM (parameterized traits and blanket impls add complexity)

### 8. Iterator / Iterable (Deferred to v1.4+)

**Purpose:** Unified lazy iteration protocol. Any type implementing Iterator works with map/filter/reduce.

**Why Defer:**
- Requires associated types (`type Item` in the trait)
- Requires lazy evaluation infrastructure (iterators are lazy in Rust, strict in Elixir)
- Snow's current List operations (map, filter, reduce) are eager and work fine
- The monomorphization pass needs significant extension for associated types

**Sketch for future:**
```snow
interface Iterator do
  type Item
  fn next(self) -> Option<Item>
end

# Then map, filter, reduce work on any Iterator
fn map<I, T, U>(iter :: I, f :: Fun(T) -> U) -> List<U>
  where I: Iterator, I.Item = T
do
  # implementation
end
```

**Precedent:** Rust `Iterator` with `type Item`, Haskell `Foldable`/`Traversable`, Elixir `Enumerable`

**Confidence:** HIGH (well-understood, but complex to implement; correct to defer)

---

## Feature Dependencies

```
Existing Features (already built)
    |
    v
[1] Trait Method Dispatch (codegen)
    |   Requires: MIR lowering changes, monomorphization for trait-bounded generics
    |
    v
[2] Display + Debug Protocols
    |   Requires: [1], built-in impls for primitives
    |   Enables: string interpolation for user types, debugging
    |
    v
[3] Eq + Ord for User Types
    |   Requires: [1], struct field comparison codegen
    |   Enables: == and < on user-defined types
    |
    v
[4] Default Method Implementations
    |   Requires: [1], trait def parsing changes
    |   Enables: ergonomic stdlib protocols
    |
    v
[5] Hash Protocol
    |   Requires: [1], [3] (Eq prerequisite for correctness)
    |   Enables: user types as Map keys and Set elements
    |
    v
[6] Auto-derive Mechanism
    |   Requires: [1], [2], [3], struct/sum type metadata
    |   Enables: `deriving(Eq, Display, Debug, Hash)` syntax
    |
    v
[7] Default Protocol
    |   Requires: [1], static method support in traits
    |
    v
[8] From/Into Conversions
    |   Requires: [1], parameterized traits (already parsed)
    |
    v
[9] Supertraits
    |   Requires: [1], constraint propagation
    |
    v
[10] Associated Types (deferred)
    |   Enables: Iterator protocol
    |
    v
[11] Iterator Protocol (deferred)
```

**Critical path:** [1] -> [2] + [3] (parallel) -> [5] -> [6]

**The single most important feature is [1]: getting trait method dispatch working in codegen.** Everything else depends on it. If trait methods don't generate code, no protocol works.

---

## MVP Definition

### Launch With (v1.3 -- Trait System Milestone)

**Phase 1: Core Trait Dispatch**
- Trait method dispatch via monomorphization (static dispatch)
- User-defined traits compile and run end-to-end
- Where clause constraints enforced and specialized in codegen
- Impl methods for structs and sum types generate correct LLVM IR

**Phase 2: Essential Stdlib Protocols**
- Display trait with built-in impls for Int, Float, Bool, String
- String interpolation integration (`"${value}"` calls `to_string`)
- Debug trait with built-in impls for primitives
- Eq trait extended to structs and sum types
- Ord trait extended to structs and sum types (with Ordering type)

**Phase 3: Ergonomics**
- Default method implementations in traits
- Hash trait with built-in impls for primitives
- Default trait with built-in impls for primitives
- Display/Debug impls for stdlib collection types (List, Map, Set)

**Phase 4: Auto-derive (stretch goal for v1.3)**
- `deriving(Eq, Ord, Display, Debug, Hash)` syntax
- Compiler-generated impl bodies for structs (field-by-field)
- Compiler-generated impl bodies for sum types (tag + field comparison)

### Add After Validation (v1.4+)

- From/Into conversion protocol (needs parameterized trait maturity)
- Supertraits (trait inheritance hierarchy)
- Method dot-syntax (`value.method()` as alternative to `method(value)`)
- Iterator/Iterable protocol (needs associated types)
- TryFrom/TryInto (fallible conversions)
- Blanket impls (`impl<T: Display> Debug for T`)
- Serialize/Deserialize protocols

### Future Consideration (v2+)

- Associated types in traits
- Dynamic dispatch (trait objects) -- if ever
- Higher-kinded types -- probably never
- Multi-parameter type classes -- probably never
- Specialization -- probably never

---

## Competitor Feature Analysis

### Trait System Mechanics

| Feature | Rust | Haskell | Elixir | Swift | Scala 3 | Snow Recommendation |
|---------|------|---------|--------|-------|---------|---------------------|
| Term | `trait` | `class` (type class) | `defprotocol` | `protocol` | `trait` | `interface` (already chosen) |
| Impl syntax | `impl Trait for Type` | `instance Class Type` | `defimpl Protocol, for: Type` | `extension Type: Protocol` | `given Instance: Trait` | `impl Trait for Type do ... end` (already chosen) |
| Static dispatch | Yes (monomorphization) | Yes (dictionary passing) | N/A (dynamic) | Yes (witness tables) | Yes (monomorphization + erasure) | Yes (monomorphization) |
| Dynamic dispatch | `dyn Trait` (vtable) | Always (dictionary passing) | Always (dispatch table) | Protocol existentials | Subtyping | Not in v1.3. Sum types instead. |
| Default methods | Yes | Yes | No (protocols are pure interfaces) | Yes (protocol extensions) | Yes | Yes -- v1.3 Phase 3 |
| Associated types | Yes | Yes (type families) | No | Yes | Yes | Deferred to v1.4+ |
| Supertraits | Yes (`trait A: B`) | Yes (`class B a => A a`) | No | Yes (`: B`) | Yes (`extends B`) | v1.4+ |
| Deriving | `#[derive(...)]` | `deriving (...)` | `@derive [...]` | Auto-synthesis | `derives` | `deriving(...)` -- v1.3 Phase 4 |
| Orphan rule | Yes (strict) | Yes (relaxed with extensions) | No (last impl wins) | No | No | Yes (simplified: impl requires local trait or local type) |
| Coherence | Yes | Yes | No | No | No | Yes (one impl per trait-type pair) |

### Stdlib Protocols

| Protocol | Rust | Haskell | Elixir | Swift | Snow Recommendation |
|----------|------|---------|--------|-------|---------------------|
| String display | `Display` | `Show` | `String.Chars` | `CustomStringConvertible` | `Display` with `to_string(self) -> String` |
| Debug display | `Debug` | `Show` (same) | `Inspect` | `CustomDebugStringConvertible` | `Debug` with `inspect(self) -> String` |
| Equality | `PartialEq` + `Eq` | `Eq` | `==` (structural) | `Equatable` | `Eq` (single trait, no Partial/Total split) |
| Ordering | `PartialOrd` + `Ord` | `Ord` | `Comparable` (not a protocol) | `Comparable` | `Ord` (single trait, returns Ordering) |
| Hashing | `Hash` | `Hashable` | N/A (Erlang terms) | `Hashable` (inherits Equatable) | `Hash` with `hash(self) -> Int` |
| Default values | `Default` | `def`/`Monoid` | N/A | N/A | `Default` with `default() -> Self` |
| Conversion | `From`/`Into` | Explicit functions | N/A | N/A | `From<T>` with `from(T) -> Self` |
| Iteration | `Iterator` | `Foldable`/`Traversable` | `Enumerable` | `Sequence`/`IteratorProtocol` | Deferred to v1.4+ |
| Serialization | `Serialize`/`Deserialize` (serde) | `aeson`/`binary` | N/A (Erlang terms) | `Codable` | Deferred to v1.4+ |
| Cloning | `Clone` | N/A (immutable by default) | N/A (immutable) | N/A (value types copy) | Not needed (Snow is functional/immutable) |
| Copying | `Copy` | N/A | N/A | N/A | Not needed (all values are semantically copies in Snow) |

### Key Insight from Competitor Analysis

**Elixir's protocol system is the closest spiritual ancestor** to what Snow needs, despite Elixir being dynamically typed. Elixir protocols:
- Are defined separately from types (just like Snow's `interface`)
- Implementations are separate (`defimpl` = Snow's `impl ... for ... do`)
- Are consolidated at compile time (Snow does this via monomorphization)
- Have a small, focused set: `String.Chars`, `Inspect`, `Enumerable`, `Collectable`

**Rust's trait system is the closest mechanical model** for Snow's implementation:
- Monomorphization for static dispatch
- Coherence (one impl per type)
- Derive macros for boilerplate reduction

**Snow should combine Elixir's philosophy (small, focused protocols with clean syntax) with Rust's mechanics (monomorphization, coherence, derive).**

---

## Actor-Specific Protocol Considerations

Snow's actor system creates unique requirements for traits:

### Message Display
Actors receive messages as sum types. Being able to `inspect` a message for logging/debugging is critical:
```snow
type CounterMsg do
  Increment
  Decrement
  GetCount
end deriving(Debug)

# In actor receive:
receive do
  msg -> println("Received: ${inspect(msg)}")
end
```

### Message Serialization (v1.4+)
For distributed actors (if ever), messages need serialization. This is NOT needed for v1.3 (single-process actors).

### Pid Display
`Pid` should implement Display to show actor identity:
```snow
impl Display for Pid do
  fn to_string(self) -> String do
    # compiler intrinsic: "#PID<0.42.0>" format
  end
end
```

---

## Implementation Complexity Assessment

| Feature | Estimated Effort | Risk Level | Notes |
|---------|-----------------|------------|-------|
| Trait method dispatch (codegen) | 3-5 days | HIGH | Core pipeline change across MIR, mono, codegen. Most complex single feature. |
| Display for primitives | 1 day | LOW | Runtime functions already exist (int_to_string, etc.) |
| Display integration with interpolation | 1-2 days | MEDIUM | Need to hook into string interpolation compilation |
| Eq for structs/sum types | 2-3 days | MEDIUM | Field-by-field comparison codegen |
| Ord with Ordering type | 2-3 days | MEDIUM | Need new sum type, refactor existing Ord |
| Default method impls | 2-3 days | MEDIUM | Trait definition changes, impl resolution changes |
| Hash for primitives | 1 day | LOW | Simple runtime functions |
| Auto-derive mechanism | 3-5 days | HIGH | Compiler needs struct metadata, generate impl AST/MIR |
| Default protocol (static methods) | 2-3 days | MEDIUM | Requires trait method without `self` |
| From/Into protocol | 2-3 days | MEDIUM | Parameterized traits already parsed |
| Supertraits | 2-3 days | MEDIUM | Constraint propagation in type checker |

**Total estimated effort for v1.3 MVP:** 15-25 days (Phases 1-3)
**With auto-derive (Phase 4):** 20-30 days

---

## Sources

### Rust Traits
- [Effective Rust: Standard Traits](https://effective-rust.com/std-traits.html)
- [Tour of Rust's Standard Library Traits](https://github.com/pretzelhammer/rust-blog/blob/master/posts/tour-of-rusts-standard-library-traits.md)
- [Rust By Example: Derive](https://doc.rust-lang.org/rust-by-example/trait/derive.html)
- [Rust Advanced Traits](https://doc.rust-lang.org/book/ch20-02-advanced-traits.html)
- [Rust Iterator Trait](https://doc.rust-lang.org/std/iter/trait.Iterator.html)
- [Rust From/Into and Newtypes](https://www.lurklurk.org/effective-rust/newtype.html)
- [Rust Coherence and Orphan Rules](https://rust-lang.github.io/rfcs/2451-re-rebalancing-coherence.html)

### Haskell Type Classes
- [Haskell Report: Predefined Types and Classes](https://www.haskell.org/onlinereport/haskell2010/haskellch6.html)
- [Typeclassopedia](https://wiki.haskell.org/Typeclassopedia)

### Elixir Protocols
- [Elixir Protocols Documentation](https://hexdocs.pm/elixir/protocols.html)
- [Elixir Protocol Module](https://hexdocs.pm/elixir/Protocol.html)

### Swift Protocols
- [Swift Standard Library Protocols](https://bugfender.com/blog/swift-standard-library-protocols/)
- [Swift Equatable](https://developer.apple.com/documentation/Swift/Equatable)

### Scala 3
- [Scala 3 Type Classes](https://docs.scala-lang.org/scala3/book/ca-type-classes.html)
- [Scala 3 Extension Methods](https://docs.scala-lang.org/scala3/reference/contextual/extension-methods.html)

### Static Dispatch / Monomorphization
- [Rust Polymorphism Guide](https://www.somethingsblog.com/2024/11/03/rust-polymorphism-a-comprehensive-guide-to-static-dynamic-dispatch-enums/)
- [Rust Dispatch: When Enums Beat dyn Trait](https://www.somethingsblog.com/2025/04/20/rust-dispatch-explained-when-enums-beat-dyn-trait/)

---
*Feature research for: Snow Language Trait System & Stdlib Protocols*
*Researched: 2026-02-07*
