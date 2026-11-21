# Mesh types

## Core type forms

| Type | Meaning |
| --- | --- |
| `Int`, `Float` | Signed 64-bit integer and double-precision float. Integer literals support decimal, `0x`, `0b`, and `0o` forms and digit separators. An exponent makes a literal a `Float`. |
| `Bool`, `String` | `true` or `false`, and immutable UTF-8 text. |
| `()` | Unit. Both `()` and `nil` denote the unit value. |
| `Atom`, `Regex` | Named constant such as `:ok`, and regular expression such as `~r/^[a-z]+$/i`. |
| `Bytes`, `Json` | Binary data and structured JSON. See [strings](strings.md). |
| `(A, B)`, `Fun(A, B) -> R` | Tuple and function types. |
| `T?`, `T!E` | `Option<T>` (`Some(value)` or `None`) and `Result<T, E>` (`Ok(value)` or `Err(error)`). The shorthand also accepts tuple payloads such as `(Int, String)?`. See [error handling](error-handling.md). |
| `Ordering` | A sum type with `Less`, `Equal`, and `Greater`, returned by `compare(a, b)`. |
| `List<T>`, `Map<K, V>`, `Set<T>`, `Queue<T>`, `Range`, `Iter<T>` | Collections, range values, and lazy iterators. See [collections](collections.md). |
| `Pid<M>` | Actor identity with mailbox message type `M`. See [actors](actors.md). |
| `U64`, `U128`, `I128` | Checked wide integers constructed and operated on through their modules. See [wide integers](https://meshlang.dev/docs/stdlib/#wide-integers). |

Numeric conversions include `Float.from(42)` and `Float.to_int(3.5)`. Text parsing is fallible. `String.to_int(text)` returns `Int?` and `String.to_float(text)` returns `Float?`. See [numeric conversion](https://meshlang.dev/docs/stdlib/#math-and-numeric-conversion).

## Inference, generics, and aliases

Parameter types, return types, and local types can be inferred. An annotation such as `let port :: Int = 8080` documents or constrains the inferred type. The compiler can generalize functions and closure literals, so `let id = fn x -> x end` can be called at both `Int` and `String`. A binding that holds a call result, such as `let f = make_identity()`, has one type shared by all its uses.

Generic parameters go after the name, as in `fn identity<T>(x :: T) -> T = x`. `where T: Display` constrains a type parameter to values implementing the interface. See [traits](traits.md).

`type UserId = Int` and `type Pair<A, B> = (A, B)` are transparent aliases. An alias and its underlying type are interchangeable. Export an alias with `pub type` for use in other modules.

## Structs and sum types

A `struct` has named fields. A literal supplies every field, and `value.field` reads one. `%{value | field: replacement}` returns an updated copy of an ordinary struct. A `type ... do ... end` sum type has nullary, positional, or named-payload variants. Constructors and struct names can be qualified when needed. See [pattern matching](pattern-matching.md) for constructor and struct patterns.

```mesh
struct Box<T> do
  value :: T
end

type Lookup<T> do
  Missing
  Found(T)
  Rejected(reason :: String)
end

type Count = Int

fn describe(result :: Lookup<Int>) -> String do
  case result do
    Missing -> "missing"
    Found(n) -> "found #{n}"
    Rejected(reason) -> reason
  end
end

fn main() do
  let initial :: Count = 7
  let box = Box { value: initial }
  let updated = %{box | value: box.value + 1}
  println(describe(Found(updated.value)))  # found 8
end
```

## Ownership and labeled values

`resource struct Name do ... end` declares an affine resource, and `resource Name` declares an opaque resource. Passing a resource to an ordinary or `consume` parameter moves it. A `borrow` parameter lends it for the call. A resource held inside a tuple, struct, or sum type makes that value a resource too. E0053 covers move and borrow violations such as use after move. E0101 covers boundaries that cannot preserve the resource’s ownership, such as an actor mailbox or JSON. See the [resource ownership rules](https://meshlang.dev/docs/type-system/#resource-types).

`Plaintext<T>` carries a compiler-tracked label for content with restricted exits. See [plaintext rules](https://meshlang.dev/docs/type-system/#plaintext).

See [traits](traits.md) for derivation and conversion and the [type system guide](https://meshlang.dev/docs/type-system/) for more examples.

## Nat-indexed sums and proof types

Declare a Nat index in a sum type or function with `n :: Nat`. An indexed
constructor's result states the index it produces. Mesh checks the index
when constructing and matching values.

```mesh
type Vec<T, n :: Nat> do
  Empty -> Vec<T, 0>
  Push<k :: Nat>(T, Vec<T, k>) -> Vec<T, Succ<k>>
end

fn first<T, n :: Nat>(values :: Vec<T, Succ<n>>) -> T do
  case values do
    Push(head, _) -> head
  end
end

fn main() do
  let single :: Vec<Int, 1> = Push(42, Empty)
  println("#{first(single)}")
end
```

`first` receives a successor index, so `Push` is its only possible arm.
The native program prints `42`. Giving `single` a length of `2` fails type
checking. For `Proof<Predicate<Index>>`, `Proof<NatEq<A, B>>`, and proofs
over compiled indexed values, read [verification](verification.md).
