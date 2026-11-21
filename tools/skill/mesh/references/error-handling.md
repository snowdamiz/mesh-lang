# Mesh error handling

`Result<T, E>` (`T!E`) carries `Ok(value)` or `Err(error)`. The error can be a string, a struct, or a sum type. `Option<T>` (`T?`) carries `Some(value)` or `None` and represents absence. See [types](types.md) and [pattern matching](pattern-matching.md).

## Handling and propagation

A `case` handles the variants explicitly. Postfix `?` unwraps `Ok` or `Some`, or exits the enclosing function or closure immediately with `Err` or `None`. The enclosing return type must be the corresponding `Result` or `Option`, and can be inferred. Handle failures explicitly in `main`, which returns unit.

Multiple `?` operations stop at the first failure or absence. After a pipe, `x |> parse()?` means `(x |> parse())?`. If the error type changes during propagation, the compiler uses a matching `From<SourceError>` implementation for the destination error type. `Result.map_err` can supply a conversion or add context before propagation.

```mesh
struct InputError do
  message :: String
end

impl From<String> for InputError do
  fn from(message :: String) -> InputError do
    InputError { message: message }
  end
end

fn check_positive(n :: Int) -> Int!String do
  if n <= 0 do
    return Err("expected a positive number")
  end
  Ok(n)
end

fn parse_positive(text :: String) -> Int!InputError do
  let n = text |> String.to_int() |> Option.ok_or("expected an integer")?
  let value = check_positive(n) |> Result.map_err(fn e -> "#{text}: #{e}" end)?
  Ok(value)
end

fn first_label(values :: List<Int>) -> String? do
  let n = List.find(values, fn n -> n > 0 end)?
  Some("first positive: #{n}")
end

fn main() do
  for text in ["42", "oops", "-5"] do
    case parse_positive(text) do
      Ok(n) -> println("#{n}")
      Err(e) -> println(e.message)
    end
  end

  for values in [[-3, 2], [-1, 0]] do
    println(first_label(values) |> Option.unwrap_or("no positive values"))
  end
end
```

The program prints `42`, `expected an integer`, `-5: expected a positive number`, `first positive: 2`, and `no positive values` on separate lines.

## Option and Result helpers

Each helper takes the option or result first and can be called by module, through a pipe, or as a method.

| Helper | Behavior |
| --- | --- |
| `Option.map`, `Result.map` | Transform the value inside `Some` or `Ok`. |
| `Option.and_then`, `Result.and_then` | Run a function that itself returns an `Option` or `Result`. The Result error type stays the same. |
| `Result.map_err` | Transform the value inside `Err`, possibly changing its type. |
| `Option.unwrap_or`, `Result.unwrap_or` | Return the value or a default. The default is evaluated before the call. |
| `Option.ok_or`, `Result.ok` | Supply an error for `None`, or turn an `Err` into `None`. |
| `Option.is_some`, `Option.is_none`, `Result.is_ok`, `Result.is_err` | Test the variant. |

## Panics

`panic(message)` has return type `Never`, so it fits any branch. It reports the message as an `R0001` diagnostic on standard error (a JSON line when `MESH_DIAGNOSTICS=json`) and ends the current actor, which a supervisor can restart. In `main` it ends the program with status 101. A missing `Map.get` key, an out-of-range `List.get`, and integer division by zero also panic.

See [traits](traits.md) for `From` and `TryFrom`, the [error handling walkthrough](https://meshlang.dev/docs/language-basics/#error-handling), and the [standard-library helper signatures](https://meshlang.dev/docs/stdlib/#option-and-result).
