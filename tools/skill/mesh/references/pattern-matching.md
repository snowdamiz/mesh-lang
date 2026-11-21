# Mesh pattern matching

`case` and `match` evaluate the first arm whose pattern matches and whose optional `when` guard is true. Arms share a result type. The compiler rejects incomplete coverage (E0012) and warns about redundant arms. Guards can use pattern bindings, but guarded arms do not establish coverage.

## Pattern forms

| Form | Meaning |
| --- | --- |
| `_`, `name` | Match any value. A lowercase name binds it, and `_` discards it. |
| `0`, `-1`, `3.5`, `"ok"`, `:ok`, `true`, `nil` | Match a literal value. Heredoc strings can also be literal patterns. |
| `(left, right)` | Match tuple elements. |
| `Point { x: 0, y }`, `Geo.Point { x }` | Match struct fields. `field: pattern` tests or destructures a field, a bare field name binds it, and omitted fields are unrestricted. |
| `None`, `Some(Ok(x))`, `Result.Err(e)` | Match variants with or without payloads. Constructors may be nested or qualified. |
| `[]`, `[a, b]`, `head :: tail` | Match a list of exactly the written length, or split a non-empty list into head and tail. `[]` and `head :: tail` cover all lists. |
| `Ok(n) \| Err(n)` | Match either alternative. Both sides must bind the same names at compatible types. |
| `pattern as whole` | Bind the complete matched value as well as any names inside the pattern. |

```mesh
fn describe(items :: List<Int>) -> String do
  match items do
    [] -> "empty"
    [0 | 1] -> "one zero or one"
    first :: _ as all when first < 0 -> do
      let count = List.length(all)
      "negative first, #{count} items"
    end
    first :: rest -> "#{first}, #{List.length(rest)} more"
  end
end

fn main() do
  println(describe([]))       # empty
  println(describe([1]))      # one zero or one
  println(describe([-2, 4]))  # negative first, 2 items
  println(describe([3, 4]))   # 3, 1 more
end
```

## Arm bodies and other binding sites

An arm can use one expression after `->`, a `-> do ... end` block, or an indented body on the next line. A `return` in the body exits the enclosing function. An arm with just a pattern can rebuild a complete matched value. `Ok(value)` on its own means `Ok(value) -> Ok(value)`, useful when another arm changes the error type. Forms such as `_` or a bare payload-bearing `Ok` require an explicit body.

`let` and `for` accept patterns that match every value of the input type, including tuple and struct destructuring with names and wildcards. Examples include `let (name, _) = pair` and `let Point { x, y } = point`. Use `case` for patterns that can fail. Function clauses, multi-clause closures, and actor `receive` arms also support patterns and guards. See [syntax](syntax.md), [actors](actors.md), and the [pattern matching guide](https://meshlang.dev/docs/language-basics/#pattern-matching).
