# Mesh strings

## Literals and interpolation

Strings are immutable UTF-8 text. `"Hello, #{name}"` interpolates an expression through `Display`. `${expr}` is an alternate spelling. Quotes inside an interpolated expression belong to that expression.

String escapes include `\n`, `\t`, `\r`, `\0`, `\\`, `\"`, and `\u{1F389}`. Escape the first character in `\#{` or `\${` for a literal interpolation marker. A triple-quoted heredoc supports the same escapes and interpolation, as well as ordinary double quotes and newlines. In multiline form, it drops the opening newline and the closing indentation line, and removes the closing delimiter's indentation from content lines.

```mesh
fn main() do
  let port = 8080
  let banner = """
    Mesh listens on "#{port}".
    Ready.
    """
  println(banner)
end
```

See the [string literal and heredoc guide](https://meshlang.dev/docs/language-basics/#string-interpolation).

## Text operations

- `String.length(text)` counts Unicode code points. `String.slice(text, start, end)` uses zero-based code-point offsets with an exclusive end and clamps both offsets to the string bounds.
- `String.contains`, `String.starts_with`, and `String.ends_with` test substrings. `String.trim`, `String.trim_start`, and `String.trim_end` remove whitespace.
- `String.split(text, delimiter)` returns a `List<String>`, and `String.join(parts, separator)` combines one. `String.replace(text, from, to)` replaces literal occurrences. `String.to_upper` and `String.to_lower` change case. `String.repeat(text, count)` returns an empty string when the count is zero or negative.
- `String.to_int(text)` and `String.to_float(text)` trim and parse, returning `Option<Int>` and `Option<Float>`. `String.from(value)` formats a `Display` value, as interpolation does.
- `<>` and `++` concatenate strings. Many String functions also work as methods, such as `text.trim()`. `println(text)` prints a string and a newline, while `print(text)` omits the newline.

Use `Bytes` for binary data and byte offsets. See [String functions](https://meshlang.dev/docs/stdlib/#strings) and [Bytes](https://meshlang.dev/docs/stdlib/#bytes).

## JSON

`json { key: value }` builds a structured `Json` object and escapes string values. Keys are bare identifiers. For arbitrary keys, use `Json.parse` on JSON text, or `Json.encode` on a `Map<String, V>`. Nested `Json` embeds as JSON. Lists and tuples encode as arrays, `None` and `nil` encode as `null`, and `Some(value)` encodes its payload. Structs and sum types can [derive JSON support](traits.md).

Both a literal and the result inside `Ok` from `Json.parse(text)` support checked accessors such as `Json.object_get`, `Json.array_get`, and `Json.as_string`. They return `Result<..., String>`. A `Json` can be interpolated or passed where an API expects a `String`. Use `Json.encode(value)` for a string return value or a `List<String>` element.

```mesh
fn label(value :: Json) -> String!String do
  let nested = Json.object_get(value, "result")?
  let name = Json.object_get(nested, "name")?
  Json.as_string(name)
end

fn main() do
  let nested = json { name: "Ada \"A\"" }
  let payload = json { result: nested, active: true }
  println(Json.encode(payload))  # {"result":{"name":"Ada \"A\""},"active":true}
  case label(payload) do
    Ok(name) -> println(name)  # Ada "A"
    Err(e) -> println(e)
  end
end
```

See the [JSON module reference](https://meshlang.dev/docs/web/#json-module) for typed access and encoding.

## Regular expressions

`~r/pattern/ims` creates a regex literal. Flags `i`, `m`, and `s` enable case-insensitive matching, multiline anchors, and dot matching a newline, respectively. An invalid literal pattern panics when evaluated. `Regex.compile(text)` returns `Result<Regex, String>` for a pattern supplied as text.

- `Regex.is_match(rx, text)` tests for a match.
- `Regex.captures(rx, text)` returns `Option<List<String>>` for the first match. Index zero is the whole match, and capture groups start at one. An unmatched optional group is an empty string.
- `Regex.replace(rx, text, replacement)` replaces non-overlapping matches. `Regex.split(rx, text)` returns the parts between matches.

```mesh
fn main() do
  let words = ~r/(\w+)\s+(\w+)/
  case Regex.captures(words, "hello world") do
    Some(groups) ->
      println(List.get(groups, 0))  # hello world
      println(List.get(groups, 1))  # hello
    None -> println("no match")
  end

  case Regex.compile("\\d+") do
    Ok(digits) -> println("#{Regex.is_match(digits, "abc123")}")  # true
    Err(e) -> println(e)
  end
end
```

See [regular expression APIs](https://meshlang.dev/docs/stdlib/#regular-expressions).

## Environment values

`Env.get(name, default)` returns a string and uses the default when the variable is absent or contains invalid Unicode. A set but empty variable yields `""`. `Env.get_int(name, default)` uses the default when the variable is absent or fails to parse as an `Int`, including empty input. `Env.args()` returns command-line arguments. See [input, environment, and files](https://meshlang.dev/docs/stdlib/#input-environment-and-files).
