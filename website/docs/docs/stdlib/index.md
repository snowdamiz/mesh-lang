---
title: Standard Library
description: Strings, collections, files, regex, checked arithmetic, bytes, cryptography, encoding, and time in Mesh
---

# Standard Library

Mesh's standard library is available without package installation. Module-qualified functions can be used directly; `import Module` is optional. Concurrency, web, database, iterator, and distributed modules have dedicated guides, while this page covers the general-purpose modules.

## Strings

String indexing is by Unicode code point rather than byte. `String.slice(text, start, end)` uses a zero-based, exclusive end and clamps both positions to the string's bounds.

| Function | Returns | Description |
|----------|---------|-------------|
| `String.length(text)` | `Int` | Count Unicode code points |
| `String.slice(text, start, end)` | `String` | Return a clamped code-point slice |
| `String.contains(text, needle)` | `Bool` | Test for a substring |
| `String.starts_with(text, prefix)` | `Bool` | Test the beginning |
| `String.ends_with(text, suffix)` | `Bool` | Test the ending |
| `String.trim(text)` | `String` | Remove surrounding whitespace |
| `String.to_upper(text)` | `String` | Unicode uppercase conversion |
| `String.to_lower(text)` | `String` | Unicode lowercase conversion |
| `String.replace(text, from, to)` | `String` | Replace every occurrence |
| `String.split(text, delimiter)` | `List<String>` | Split on a literal delimiter |
| `String.join(parts, separator)` | `String` | Join a list of strings |
| `String.to_int(text)` | `Option<Int>` | Parse a signed integer after trimming |
| `String.to_float(text)` | `Option<Float>` | Parse a float after trimming |
| `String.from(value)` | `String` | Convert an `Int`, `Float`, or `Bool` |
| `String.collect(iterator)` | `String` | Consume a string-producing iterator |

The `<>` operator concatenates two strings. `println(value)` writes to standard output with a newline, and `print(value)` writes without one.

## Input, Environment, and Files

| Function | Returns | Description |
|----------|---------|-------------|
| `IO.read_line()` | `Result<String, String>` | Read one line from standard input |
| `IO.eprintln(text)` | `Unit` | Write a line to standard error |
| `Env.get(name, default)` | `String` | Read an environment variable or use a default |
| `Env.get_int(name, default)` | `Int` | Read a decimal environment variable or use a default |
| `Env.args()` | `List<String>` | Return native command-line arguments |
| `File.read(path)` | `Result<String, String>` | Read a UTF-8 text file |
| `File.write(path, text)` | `Result<Unit, String>` | Create or replace a text file |
| `File.append(path, text)` | `Result<Unit, String>` | Append text, creating the file when needed |
| `File.exists(path)` | `Bool` | Test whether a path exists |
| `File.delete(path)` | `Result<Unit, String>` | Delete a file |

File operations return error text instead of terminating the program:

```mesh
case File.read("settings.txt") do
  Ok(contents) -> println(contents)
  Err(error) -> IO.eprintln("settings: #{error}")
end
```

## Regular Expressions

Use `~r/pattern/` for a literal pattern. Literal flags are `i` (case-insensitive), `m` (multi-line), and `s` (dot matches newlines). Use `Regex.compile` for a pattern known only at runtime.

```mesh
fn main() do
  let identifier = ~r/^[a-z][a-z0-9_]*$/i
  if Regex.is_match(identifier, "mesh_14") do
    println("valid")
  end
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Regex.compile(pattern)` | `Result<Regex, String>` | Compile a dynamic pattern |
| `Regex.is_match(regex, text)` | `Bool` | Test whether the pattern matches |
| `Regex.captures(regex, text)` | `Option<List<String>>` | Return the whole match followed by capture groups |
| `Regex.replace(regex, text, replacement)` | `String` | Replace every non-overlapping match |
| `Regex.split(regex, text)` | `List<String>` | Split text at matches |

## Eager Collections

Lists and maps are polymorphic. Sets and queues currently store `Int` values. Collection updates are immutable: keep the returned collection.

### Lists

| Functions | Purpose |
|-----------|---------|
| `List.new`, `List.length`, `List.append` | Create, count, and append |
| `List.head`, `List.tail`, `List.get`, `List.last`, `List.nth` | Positional access |
| `List.concat`, `List.reverse`, `List.take`, `List.drop` | Reshape a list |
| `List.map`, `List.filter`, `List.reduce`, `List.flat_map`, `List.flatten` | Transform and fold |
| `List.find`, `List.any`, `List.all`, `List.contains` | Search and predicates |
| `List.sort` | Sort with a comparator returning a negative, zero, or positive `Int` |
| `List.zip`, `List.enumerate` | Pair lists or attach zero-based indices |
| `List.collect` | Consume an iterator into a list |

`List.head`, `List.tail`, `List.get`, `List.last`, and `List.nth` require an existing element. Check the length or use `List.find`, which returns `Option<T>`, when absence is expected.

### Maps and Sets

| Functions | Purpose |
|-----------|---------|
| `Map.new`, `Map.put`, `Map.get`, `Map.delete`, `Map.has_key`, `Map.size` | Core map operations |
| `Map.keys`, `Map.values`, `Map.merge` | Inspect or combine maps |
| `Map.to_list`, `Map.from_list`, `Map.collect` | Convert `(key, value)` tuples |
| `Set.new`, `Set.add`, `Set.remove`, `Set.contains`, `Set.size` | Core integer-set operations |
| `Set.union`, `Set.intersection`, `Set.difference` | Set algebra |
| `Set.to_list`, `Set.from_list`, `Set.collect` | Convert integer sets |

`Map.get` requires an existing key; call `Map.has_key` first when absence is normal.

### Tuples, Ranges, and Queues

| Function | Returns | Description |
|----------|---------|-------------|
| `Tuple.first(tuple)` | `Int` | First integer element |
| `Tuple.second(tuple)` | `Int` | Second integer element |
| `Tuple.nth(tuple, index)` | `Int` | Integer element at a zero-based index |
| `Tuple.size(tuple)` | `Int` | Tuple arity |
| `Range.new(start, end)` | `Range` | Create the half-open range `[start, end)` |
| `Range.length(range)` | `Int` | Number of integers in the range |
| `Range.to_list(range)` | `List<Int>` | Materialize a range |
| `Range.map(range, fn)` | `List<Int>` | Map its integers |
| `Range.filter(range, predicate)` | `List<Int>` | Retain matching integers |
| `Queue.new()` | `Queue` | Create an empty integer FIFO |
| `Queue.push(queue, value)` | `Queue` | Return a queue with a value appended |
| `Queue.pop(queue)` | `Tuple` | Return `(front_value, remaining_queue)` |
| `Queue.peek(queue)` | `Int` | Read the front value |
| `Queue.size(queue)` | `Int` | Count queued values |
| `Queue.is_empty(queue)` | `Bool` | Test for an empty queue |

`Queue.pop` and `Queue.peek` require a non-empty queue.

## Bytes

`Bytes` stores arbitrary binary data without treating it as UTF-8. It does not
implicitly convert to `String`; use `Bytes.to_utf8` when text is expected and
handle its `Result`.

```mesh
case "ff0041" |> Bytes.from_hex() do
  Ok(raw) ->
    println("#{Bytes.length(raw)}")
    raw |> Bytes.to_base64() |> println()
  Err(error) -> println(error)
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Bytes.empty()` | `Bytes` | Empty byte sequence |
| `Bytes.from_list(values)` | `Result<Bytes, BytesError>` | Copy checked integer byte values (0 through 255) |
| `Bytes.to_list(bytes)` | `List<Int>` | Copy bytes to integer values |
| `Bytes.repeat(byte, count)` | `Result<Bytes, BytesError>` | Construct a checked repeated byte sequence |
| `Bytes.length(bytes)` | `Int` | Byte length |
| `Bytes.get(bytes, index)` | `Result<Int, String>` | Byte value at a checked index |
| `Bytes.slice(bytes, start, length)` | `Result<Bytes, String>` | Checked subrange |
| `Bytes.concat(left, right)` | `Result<Bytes, String>` | Concatenate two byte sequences |
| `Bytes.secure_equals(left, right)` | `Bool` | Constant-time equality |
| `Bytes.from_utf8(text)` | `Bytes` | Copy UTF-8 string bytes |
| `Bytes.to_utf8(bytes)` | `Result<String, String>` | Validate and decode UTF-8 |
| `Bytes.to_base64(bytes)` | `String` | Standard padded Base64 |
| `Bytes.from_base64(text)` | `Result<Bytes, String>` | Decode padded or unpadded Base64 |
| `Bytes.to_base58(bytes)` | `String` | Base58 encode |
| `Bytes.from_base58(text)` | `Result<Bytes, String>` | Base58 decode |
| `Bytes.to_hex(bytes)` | `String` | Lowercase hexadecimal |
| `Bytes.from_hex(text)` | `Result<Bytes, String>` | Decode case-insensitive hexadecimal |
| `Bytes.read_u16_be(bytes, offset)` | `Result<Int, BytesError>` | Read a checked big-endian 16-bit integer |
| `Bytes.read_u32_be(bytes, offset)` | `Result<U64, BytesError>` | Read a checked big-endian 32-bit integer |
| `Bytes.read_u64_be(bytes, offset)` | `Result<U64, BytesError>` | Read a checked big-endian 64-bit integer |
| `Bytes.read_u16_le(bytes, offset)` | `Result<Int, BytesError>` | Read a checked little-endian 16-bit integer |
| `Bytes.read_u32_le(bytes, offset)` | `Result<U64, BytesError>` | Read a checked little-endian 32-bit integer |
| `Bytes.read_u64_le(bytes, offset)` | `Result<U64, BytesError>` | Read a checked little-endian 64-bit integer |
| `Bytes.write_u16_be(value)` | `Result<Bytes, BytesError>` | Write a checked big-endian 16-bit integer |
| `Bytes.write_u32_be(value)` | `Result<Bytes, BytesError>` | Write a checked big-endian 32-bit integer |
| `Bytes.write_u64_be(value)` | `Result<Bytes, BytesError>` | Write a big-endian 64-bit integer |
| `Bytes.read_uint_le(bytes, offset, width)` | `Result<String, String>` | Read a 1, 2, 4, or 8-byte unsigned integer as a full-range decimal string |
| `Bytes.write_uint_le(value, width)` | `Result<Bytes, String>` | Write a decimal unsigned integer at width 1, 2, 4, or 8 |

Checked construction and fixed-width APIs use the nominal `BytesError` type;
handle failures with `Err(_)` without depending on runtime error text.

The `mesh-binary` source package adds a bounded immutable `BinaryReader`.
Its vectors use a canonical unsigned 32-bit big-endian length prefix, and
`finish` rejects trailing bytes.

## Wide integers

`U64`, `U128`, and `I128` are opaque integer values for protocol fields that
do not fit Mesh `Int`. Construction and arithmetic are checked. Convert to
`Int` only when the value is known to fit.

```mesh
case U64.parse("18446744073709551615") do
  Ok(value) -> do
    value |> U64.to_string() |> println()
    case value |> U64.to_int() do
      Ok(number) -> println("#{number}")
      Err(error) -> println(error)
    end
  end
  Err(error) -> println(error)
end
```

Each module exposes the same surface:

| Function | Returns | Description |
|----------|---------|-------------|
| `U64.parse(text)` | `Result<U64, String>` | Checked decimal parse |
| `U64.compare(left, right)` | `Int` | `-1`, `0`, or `1` |
| `U64.add(left, right)` | `Result<U64, String>` | Checked addition |
| `U64.subtract(left, right)` | `Result<U64, String>` | Checked subtraction |
| `U64.multiply(left, right)` | `Result<U64, String>` | Checked multiplication |
| `U64.divide(left, right)` | `Result<U64, String>` | Checked integer division; division by zero is an error |
| `U64.to_int(value)` | `Result<Int, String>` | Bounded conversion |
| `U64.to_string(value)` | `String` | Canonical decimal string |

Replace `U64` with `U128` or `I128` for the corresponding width and
signedness: for example, `U128.multiply(left, right)` performs checked
128-bit unsigned multiplication. `Bytes.read_uint_le` decimal output can be
passed to `U64.parse`.

## Checked Integer Arithmetic

Normal `Int` operators are convenient for ordinary arithmetic. Use `Checked` at financial, protocol, and resource-accounting boundaries where overflow or invalid division must be returned as data.

| Function | Returns | Description |
|----------|---------|-------------|
| `Checked.add(left, right)` | `Result<Int, String>` | Checked addition |
| `Checked.sub(left, right)` | `Result<Int, String>` | Checked subtraction |
| `Checked.mul(left, right)` | `Result<Int, String>` | Checked multiplication |
| `Checked.div(left, right)` | `Result<Int, String>` | Checked division, including zero and minimum-value overflow |
| `Checked.abs(value)` | `Result<Int, String>` | Checked absolute value |
| `Checked.mul_div(a, b, denominator, rounding)` | `Result<Int, String>` | Multiply through a wide intermediate, divide, and round |
| `Checked.rescale(raw, from_scale, to_scale, rounding)` | `Result<Int, String>` | Convert a fixed-point integer between decimal scales |

Rounding is explicit: `:toward_zero`, `:floor`, `:ceil`, `:half_away_from_zero`, or `:half_even`.

```mesh
case Checked.mul_div(1_005, 1, 100, :half_even) do
  Ok(value) -> println("#{value}")
  Err(error) -> println(error)
end
```

## Math and Numeric Conversion

| Function | Returns | Description |
|----------|---------|-------------|
| `Math.abs(value)` | Same numeric type | Absolute value |
| `Math.min(left, right)` | Same numeric type | Smaller value |
| `Math.max(left, right)` | Same numeric type | Larger value |
| `Math.pi` | `Float` | π constant |
| `Math.pow(base, exponent)` | `Float` | Floating-point power |
| `Math.sqrt(value)` | `Float` | Square root |
| `Math.floor(value)` | `Int` | Round down |
| `Math.ceil(value)` | `Int` | Round up |
| `Math.round(value)` | `Int` | Round to the nearest integer |
| `Int.to_float(value)` | `Float` | Convert an integer |
| `Int.to_string(value)` | `String` | Decimal formatting |
| `Float.to_int(value)` | `Int` | Convert a float to an integer |
| `Float.from(value)` | `Float` | Convert an integer to a float |

## Crypto

The `Crypto` module is binary-first. Public data uses `Bytes`; private keys and
derived key material are move-only resources that cannot be printed, serialized,
or sent through actor mailboxes. Fallible operations return `CryptoError`.

### Hashing

```mesh
fn main() do
  let input = Bytes.from_utf8("hello")
  let hash = Crypto.sha256(input)
  println(Bytes.to_hex(hash))
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Crypto.sha256(input)` | `Bytes` | Binary SHA-256 digest |
| `Crypto.sha512(input)` | `Bytes` | Binary SHA-512 digest |
| `Crypto.sha256_hex(input)` | `String` | Lowercase presentation form |
| `Crypto.sha512_hex(input)` | `String` | Lowercase presentation form |

### Secrets and authenticated cryptography

```mesh
fn authenticate() -> Int ! CryptoError do
  let key = Secret.random(32) ?
  let tag = Crypto.hmac_sha256(key, Bytes.from_utf8("message")) ?
  Secret.destroy(tag)
  Secret.destroy(key)
  Ok(0)
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Crypto.random_bytes(length)` | `Result<Bytes, CryptoError>` | OS-backed random public bytes |
| `Secret.random(length)` | `Result<SecretBytes, CryptoError>` | OS-backed move-only secret bytes |
| `Crypto.hmac_sha256(key, message)` | `Result<SecretBytes, CryptoError>` | HMAC with a borrowed secret key |
| `Crypto.hkdf_sha256(key, salt, info, length)` | `Result<SecretBytes, CryptoError>` | Bounded HKDF output |
| `Crypto.argon2id(password, salt, memory_kib, iterations, parallelism, length)` | `Result<SecretBytes, CryptoError>` | Argon2id v1.3 password KDF with a borrowed secret |
| `Crypto.x25519_generate()` | `Result<X25519KeyPair, CryptoError>` | Generate an X25519 key pair |
| `Crypto.x25519_public(key)` | `Result<X25519PublicKey, CryptoError>` | Derive the public key again |
| `Crypto.x25519_shared(key, peer)` | `Result<SecretBytes, CryptoError>` | Derive a shared secret |
| `Crypto.signing_generate()` | `Result<SigningKeyPair, CryptoError>` | Generate an Ed25519 key pair |
| `Crypto.sign(key, message)` | `Result<Signature, CryptoError>` | Sign with a borrowed private key |
| `Crypto.verify(key, message, signature)` | `Result<Bool, CryptoError>` | Strict signature verification |
| `Crypto.aead_key(material)` | `Result<AeadKey, CryptoError>` | Consume 32 secret bytes as an AEAD key |
| `Crypto.aead_seal(key, nonce, aad, plaintext)` | `Result<Bytes, CryptoError>` | ChaCha20-Poly1305 encryption |
| `Crypto.aead_open(key, nonce, aad, ciphertext)` | `Result<Bytes, CryptoError>` | Authenticate before returning plaintext |

Borrowed keys remain owned by the caller. `Crypto.aead_key` consumes its input,
including on error. Use `Secret.destroy` for early destruction; otherwise the
compiler inserts destruction on every scope exit.

`Crypto.argon2id` accepts salts from 8 through 64 bytes, memory from
`8 * parallelism` through 65,536 KiB, 1 through 10 iterations, 1 through 8
lanes, outputs from 16 through 64 bytes, and passwords up to 65,536 bytes. The
low end exists for published vectors, compatibility tests, and explicitly
versioned application profiles; these bounds are resource-safety limits, not a
password policy. Applications must pin a reviewed profile instead of exposing
the parameters to users. Messenger recovery pins its values in the versioned
backup profile and stores the salt and profile version with the ciphertext.

### UUID

```mesh
fn main() do
  let id = Crypto.uuid4()
  println(id)   # e.g. "550e8400-e29b-41d4-a716-446655440000"
end
```

`Crypto.uuid4()` generates a cryptographically random UUID v4 in the standard `8-4-4-4-12` format.

## Encoding

### Base64

The `Base64` module encodes and decodes the UTF-8 bytes of `String` values.
Decoding returns `Result<String, String>` because the input may be malformed or
decode to invalid UTF-8. Use `Bytes.to_base64` and `Bytes.from_base64` for
arbitrary binary values.

```mesh
fn main() do
  let encoded = Base64.encode("hello world")
  println(encoded)   # aGVsbG8gd29ybGQ=

  case Base64.decode(encoded) do
    Ok(s) -> println(s)   # hello world
    Err(e) -> println("decode error: #{e}")
  end

  # URL-safe variant (replaces + with - and / with _)
  let url_enc = Base64.encode_url("hello world")
  case Base64.decode_url(url_enc) do
    Ok(s) -> println(s)
    Err(e) -> println(e)
  end
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Base64.encode(s)` | `String` | Encode to standard Base64 (padded) |
| `Base64.decode(s)` | `Result<String, String>` | Decode standard Base64 |
| `Base64.encode_url(s)` | `String` | Encode to URL-safe Base64 |
| `Base64.decode_url(s)` | `Result<String, String>` | Decode URL-safe Base64 |

### Hex

The `Hex` module encodes and decodes the UTF-8 bytes of `String` values.
Decoding is case-insensitive and returns `Result<String, String>`. Use
`Bytes.to_hex` and `Bytes.from_hex` for arbitrary binary values.

```mesh
fn main() do
  let h = Hex.encode("hi")
  println(h)   # 6869

  case Hex.decode(h) do
    Ok(s) -> println(s)   # hi
    Err(e) -> println("decode error: #{e}")
  end
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Hex.encode(s)` | `String` | Encode bytes as lowercase hex |
| `Hex.decode(s)` | `Result<String, String>` | Decode hex string (case-insensitive) |

## DateTime

The `DateTime` module provides UTC timestamps, ISO 8601 parsing and formatting, Unix timestamp interop, arithmetic, and comparison. Internally, `DateTime` values are backed by a 64-bit Unix millisecond timestamp.

### Current Time

```mesh
fn main() do
  let dt = DateTime.utc_now()
  let ms = DateTime.to_unix_ms(dt)
  let iso = DateTime.to_iso8601(dt)
  println(iso)   # e.g. "2024-01-15T10:30:00.000Z"
end
```

### Parsing and Formatting

```mesh
fn main() do
  case DateTime.from_iso8601("2024-01-15T10:30:00Z") do
    Ok(dt) ->
      let formatted = DateTime.to_iso8601(dt)
      println(formatted)   # "2024-01-15T10:30:00.000Z"
    Err(e) -> println("parse error: #{e}")
  end
end
```

### Unix Timestamp Interop

```mesh
fn main() do
  case DateTime.from_unix_ms(1705316200000) do
    Ok(dt) -> println("#{DateTime.to_unix_ms(dt)}")
    Err(error) -> println(error)
  end

  case DateTime.from_unix_secs(1705316200) do
    Ok(dt) -> println("#{DateTime.to_unix_secs(dt)}")
    Err(error) -> println(error)
  end
end
```

### Arithmetic

```mesh
fn main() do
  case DateTime.from_iso8601("2024-01-15T10:30:00Z") do
    Ok(dt) ->
      let next_week = DateTime.add(dt, 7, :day)
      let tomorrow = DateTime.add(dt, 1, :day)
      let later = DateTime.add(dt, 2, :hour)
      let diff = DateTime.diff(next_week, dt, :day)
      println("#{diff}")   # 7.0
    Err(_) -> println("error")
  end
end
```

`DateTime.add(dt, n, unit)` supports `:ms`, `:second`, `:minute`, `:hour`,
`:day`, and `:week`. Negative `n` subtracts.

`DateTime.diff(dt1, dt2, unit)` accepts the same units and returns a `Float`
representing how much later `dt1` is than `dt2`. It is negative if `dt1` is
earlier.

### Comparison

```mesh
fn main() do
  case DateTime.from_iso8601("2024-01-15T10:30:00Z") do
    Ok(dt) ->
      let future = DateTime.add(dt, 1, :day)
      let is_before = DateTime.is_before(dt, future)   # true
      let is_after = DateTime.is_after(future, dt)     # true
      println("#{is_before}")
    Err(_) -> println("error")
  end
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `DateTime.utc_now()` | `DateTime` | Current UTC time |
| `DateTime.from_iso8601(s)` | `Result<DateTime, String>` | Parse ISO 8601 string |
| `DateTime.to_iso8601(dt)` | `String` | Format as ISO 8601 (`"...Z"`) |
| `DateTime.from_unix_ms(n)` | `Result<DateTime, String>` | Validate Unix milliseconds |
| `DateTime.from_unix_secs(n)` | `Result<DateTime, String>` | Validate Unix seconds |
| `DateTime.to_unix_ms(dt)` | `Int` | To Unix milliseconds |
| `DateTime.to_unix_secs(dt)` | `Int` | To Unix seconds |
| `DateTime.add(dt, n, unit)` | `DateTime` | Add duration (`:ms`, `:second`, `:minute`, `:hour`, `:day`, `:week`) |
| `DateTime.diff(dt1, dt2, unit)` | `Float` | Signed difference in given unit |
| `DateTime.is_before(dt1, dt2)` | `Bool` | True if dt1 is before dt2 |
| `DateTime.is_after(dt1, dt2)` | `Bool` | True if dt1 is after dt2 |

## Monotonic Time and Durations

Use `DateTime` for timestamps that people or external systems need to read. Use `Monotonic` for elapsed time and deadlines; it cannot jump when the wall clock changes.

| Function | Returns | Description |
|----------|---------|-------------|
| `Monotonic.now_nanos()` | `Int` | Nanoseconds since a process-local monotonic origin |
| `Monotonic.elapsed(start, finish)` | `Result<Int, String>` | Checked non-negative difference |
| `Duration.millis(value)` | `Result<Int, String>` | Convert non-negative milliseconds to nanoseconds |
| `Duration.seconds(value)` | `Result<Int, String>` | Convert non-negative seconds to nanoseconds |

Both duration conversions detect negative inputs and integer overflow. Their nanosecond results can be passed to APIs such as `Channel.recv`.

## Deterministic Randomness

`Random` threads generator state explicitly, making runs reproducible:

| Function | Returns | Description |
|----------|---------|-------------|
| `Random.seed(seed)` | `Int` | Create a deterministic state |
| `Random.next_int(state, min, max)` | `Tuple` | Return `(next_state, value)` over the inclusive range |
| `Random.next_unit_ppm(state)` | `Tuple` | Return `(next_state, value)` from `0` through `999_999` |

This generator is not suitable for secrets. Use `Crypto.uuid4` for cryptographically random identifiers.

## What's Next?

- [Concurrency](/docs/concurrency/) — actors, jobs, timers, and bounded channels
- [Iterators](/docs/iterators/) — lazy pipelines and collection terminals
- [Testing](/docs/testing/) — write and run tests with `meshc test`
- [Developer Tools](/docs/tooling/) — meshc, meshpkg, formatter, REPL, LSP
- [Web](/docs/web/) — HTTP server, client, and WebSocket
