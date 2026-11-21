# Mesh traits

## Interfaces and implementations

`interface Name do ... end` declares a trait. Interfaces may take type parameters, declare required methods, and provide methods with default bodies. `impl Interface for Type do ... end` supplies required methods with matching signatures. A method with `self` can use receiver syntax such as `box.first()`. A static method omits `self` and uses the implementing type, as in `Box.from(value)`.

An interface can declare an associated type with `type Item` and reference it as `Self.Item`. Each implementation binds it once. A generic function can require an interface using `where T: Display`. See [types](types.md) and the [interface reference](https://meshlang.dev/docs/reference/#interfaces-and-implementations).

```mesh
interface Container do
  type Item
  fn first(self) -> Self.Item
  fn label(self) -> String do
    "container"
  end
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

fn render<T>(value :: T) -> String where T: Display do
  value.to_string()
end

fn main() do
  let box = IntBox { value: 42 }
  println(box.label() <> ": " <> render(box.first()))  # container: 42
end
```

## Built-in interfaces and conversion

- `Eq` drives `==` and `!=`. `Ord` drives ordering and `compare(a, b)`, which returns `Less`, `Equal`, or `Greater`. An `Ord` implementation defines `lt(self, other) -> Bool`.
- `Display` provides `to_string()` and interpolation. `Debug` provides `inspect()`, which quotes and escapes strings. `Hash` provides `hash()`, and `Default` provides `default()` for the type expected by the caller.
- `Add`, `Sub`, `Mul`, `Div`, `Mod`, and unary `Neg` have an associated `Output` type. `Iterable` and `Iterator` support `for ... in`. See [numeric traits](https://meshlang.dev/docs/type-system/#numeric-traits) and [iterators](https://meshlang.dev/docs/iterators/).
- `impl From<Source> for Target` provides `Target.from(value)` and generates `value.into()`. Give `into()` an expected type, such as `let ratio :: Float = 7.into()`. `Float.from(7)` names the target explicitly. [From and Into](https://meshlang.dev/docs/type-system/#from-into-conversion)
- `impl TryFrom<Source> for Target` provides `Target.try_from(value)` returning `Target!E` and generates `value.try_into()`. An expected result type selects the target for `try_into()`. Postfix `?` can propagate that result's error. [TryFrom and TryInto](https://meshlang.dev/docs/type-system/#tryfrom-tryinto-conversion)

See [error handling](error-handling.md) for a `From` implementation used during `?` propagation.

## Deriving

`deriving(...)` after `end` selects the generated implementations. `deriving()` selects none. Without a clause, ordinary structs derive `Eq`, `Ord`, `Debug`, and `Hash`, and ordinary sum types derive `Eq`, `Ord`, and `Debug`. For explicit `Ord`, include `Eq` in the same clause. Resource types derive nothing.

| Derive | Generated behavior |
| --- | --- |
| `Eq`, `Ord` | Structural equality and ordering. Struct ordering compares fields in declaration order. |
| `Display`, `Debug`, `Hash` | Display text, debug text, and a hash consistent with equality. |
| `Json` | Encoding via `value.to_json()` or `Json.encode(value)`, and decoding via `Type.from_json(text) -> Type!String`. Sum types encode a `"tag"` and a `"fields"` array. Fields must support JSON, including nested types. |
| `Row` | Struct-only `Type.from_row(row) -> Type!String`, where `row` is a `Map<String, String>`. Column names match fields. Supported fields are `String`, `Int`, `Float`, `Bool`, and their optional forms. |
| `Schema` | Struct-only metadata for the ORM and query APIs. |

Generic derives use the actual field types at each instantiation. A generic decoder can use an annotation such as `let parsed :: Box<Int>!String = Box.from_json(text)` to select the instantiation.

```mesh
struct User do
  name :: String
  age :: Int
end deriving(Json, Row)

fn main() do
  let row = %{"name" => "Ada", "age" => "37"}
  case User.from_row(row) do
    Ok(user) ->
      let text = Json.encode(user)
      println(text)  # {"name":"Ada","age":37}
      case User.from_json(text) do
        Ok(decoded) -> println("#{decoded.name}: #{decoded.age}")  # Ada: 37
        Err(e) -> println(e)
      end
    Err(e) -> println(e)
  end
end
```

See [strings and JSON](strings.md), [database row decoding](database.md), and the [deriving reference](https://meshlang.dev/docs/type-system/#deriving).
