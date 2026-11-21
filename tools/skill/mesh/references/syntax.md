# Mesh syntax

## Files, modules, and visibility

- Source files use `.mpl`. An executable starts at `fn main()` in `main.mpl` by default. `[package].entrypoint` in `mesh.toml` can select another file.
- A path such as `geo/points.mpl` gives the file module `Geo.Points`. `import Geo.Points` exposes qualified calls such as `Points.distance(p)`. `from Geo.Points import Point, distance` brings selected names into scope. Use parentheses around a selective import list that spans lines.
- `pub` exports a declaration. A named `module Name do ... end` is local to its file unless declared `pub module`. Standard-library modules such as `List` and `File` are in scope without imports.
- `#` starts a line comment, `##` documents a declaration, `##!` documents a module, and `#= ... =#` is a nestable block comment. Newlines usually end statements. Separate statements on one line with `;`. A line continues inside delimiters or after `=` or an infix operator. A leading infix operator also continues the previous line, except `-` and `%`.

See [projects](projects.md) for builds and package layout, the [module and file reference](https://meshlang.dev/docs/reference/#source-files-and-modules) for visibility rules, and the [Hello World walkthrough](https://meshlang.dev/docs/getting-started/#hello-world) for a first executable.

## Bindings, functions, and calls

- `let x = expr` and `let x :: Type = expr` create immutable local bindings. A later `let x = ...` shadows the prior name. Define a zero-argument function for a shared constant. Module-level `let` is error E0080.
- A named function uses `fn` or `def` and has a `do ... end` body or a single expression after `=`. Parameter and return types can be inferred. The final expression supplies the return value. `return value` or bare `return` exits early.
- Consecutive clauses of the same name and arity can dispatch by pattern and `when` guard. Put the catch-all last. The first clause carries the group's visibility, generic parameters, return type, and bounds. See [pattern matching](pattern-matching.md).
- Closures capture lexical bindings. Forms include `fn(x :: Int) -> x + n end`, `fn x -> x + n end`, `fn x do ... end`, and `fn -> 42 end`. `Fun(Int) -> Int` names a function type. `f(args) do |x| ... end` supplies a closure as the last argument.
- Keyword arguments form one final map argument. `request("/events", method: "POST")` passes `%{"method" => "POST"}` as the second argument. On values such as a `String` or `List`, calls like `text.trim()` and `xs.map(f)` can resolve to module functions with the receiver first.

```mesh
fn label(0 :: Int) -> String = "zero"
fn label(n) when n < 0 = "negative"
fn label(n) = "positive: #{n}"

fn main() do
  let offset = 2
  let labels = [-2, -1, 0] |> List.map() do |n|
    label(n + offset)
  end
  println(String.join(labels, ", "))  # zero, positive: 1, positive: 2
end
```

## Control flow

- `if condition do ... else ... end` yields the selected branch value. The condition must be a `Bool` and the branches share a type. An `else if` chain closes with one `end`. An `if` without `else` has type `()`.
- `case` and `match` select the first matching arm. See [pattern matching](pattern-matching.md).
- `for pattern in source when guard do ... end` yields a list of body results. The `when` filter is optional. Sources include lists, maps, sets, ranges, and values implementing `Iterable` or `Iterator`. A map loop can bind `{key, value}` or `(key, value)`.
- `while condition do ... end` returns `()`. `break` exits the enclosing loop and `continue` skips to its next iteration. A binding shadowed inside a loop does not update the condition's binding. For a counter, use an end-exclusive range such as `0..10`. For evolving state, a direct tail call to the current function runs as a loop.

```mesh
fn main() do
  let squares = for n in 0..6 when n % 2 == 0 do
    n * n
  end
  println("#{squares}")  # [0, 4, 16]
end
```

## Pipes and operators

`value |> f(a)` calls `f(value, a)`. `value |N> f(a, ...)` inserts the value into one-based argument position `N`, starting at `2`. Omit the inserted argument from the call. Both pipes can start or end a continued line.

```mesh
fn format_three(left :: String, middle :: String, right :: String) -> String =
  left <> ":" <> middle <> ":" <> right

fn main() do
  "middle"
    |2> format_three("left", "right")
    |> println()  # left:middle:right
end
```

From lowest to highest precedence, the groups are pipes, `or` or `||`, `and` or `&&`, equality (`==`, `!=`), ordering (`<`, `>`, `<=`, `>=`), range (`..`), concatenation (`<>`, `++`), addition and subtraction, multiplication and division (`*`, `/`, `%`), prefix (`-`, `not`, `!`), and postfix (calls, field access, `?`). Both concatenation operators work on strings and lists. Integer division truncates toward zero.

See [error handling](error-handling.md) for postfix `?` and [traits](traits.md) for operator dispatch and conversions.

## Function decorators

`@cluster pub fn ...` uses the default copy count. `@cluster(N) pub fn ...` sets the total copy count. Packages participating in the clustered runtime bootstrap with `Node.start_from_env()`. See [clustering](clustering.md).

`@native` declares a native binding, and `@export("c_symbol")` declares a C ABI export for library builds. See [native packages](https://meshlang.dev/docs/native-packages/) and [library builds](https://meshlang.dev/docs/library-builds/).
