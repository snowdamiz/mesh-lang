# Mesh Pattern Matching

## Case Expressions
1. `case expr do pattern -> result ... end` — matches expr against patterns in order.
2. The `_` wildcard matches anything and binds nothing.
3. Every `case` must be exhaustive — compiler rejects non-exhaustive patterns.
4. `case` is an expression: all arms must return the same type.
5. Arms evaluate to the expression after `->` (no explicit `return` needed).

```mesh
fn describe_number(n :: Int) -> String do
  case n do
    0 -> "zero"
    1 -> "one"
    _ -> "other"
  end
end
```

## ADT Pattern Matching
1. Match on ADT variants by constructor name: `Red`, `Some(v)`, `Ok(x)`, `Err(e)`.
2. Payload variables in patterns bind the carried value: `Ok(value)` binds the inner value to `value`.
3. Nested patterns: `Some(Ok(x))` matches `Some` containing `Ok` containing `x`.
4. Use explicit arms to make the compiler require a decision when a variant is added. A wildcard covers future variants as well as today's remaining cases.

```mesh
type Color do
  Red
  Green
  Blue
end

case c do
  Red -> println("red")
  Green -> println("green")
  Blue -> println("blue")
end

# Result matching:
case process(42) do
  Ok(s) -> println(s)
  Err(e) -> println("error: #{e}")
end

# Option matching:
case maybe_value do
  Some(v) -> println("got: #{v}")
  None -> println("nothing")
end
```

## Struct Pattern Destructuring
1. Struct fields can be destructured in case arms: `User { name: n, age: a }`.
2. Only the fields you need must be named; all named fields bind.
3. Destructured bindings are immutable.

## Let Pattern Binding
Destructure tuples in a `let` binding. Use `case` to bind the payload of a
sum-type variant and handle the other variants.

```mesh
let (name, age) = ("Ada", 37)
println("#{name}: #{age}")
```

## Wildcards and Literal Patterns
1. `_` — matches any value, discards it (no binding).
2. Integer literals: `0`, `1`, `42` — match exact values.
3. String literals: `"hello"` — match exact string values.
4. Bool literals: `true`, `false`.
5. Join alternatives in one arm with `|`. Each alternative must bind the same names at the same types, as in `Ok(n) | Err(n)` for a `Result<Int, Int>`.
