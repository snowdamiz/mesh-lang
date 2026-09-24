---
title: Standard Library
description: Strings, collections, files, regex, checked arithmetic, bytes, cryptography, host capabilities, encoding, and time in Mesh
---

# Standard Library

Mesh's standard library is available without package installation. Module-qualified functions can be used directly; `import Module` is optional. Concurrency, web, database, iterator, and distributed modules have dedicated guides, while this page covers the general-purpose modules.

A function can also be called as a method on its first argument: `"mesh".length()` is `String.length("mesh")`, `xs.map(f)` is `List.map(xs, f)`, and `m.get(key)` is `Map.get(m, key)`. [Language Basics](/docs/language-basics/) explains how the value's type selects the module.

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
| `String.from(value)` | `String` | Show any value with `Display`, as `"${value}"` would |
| `String.collect(iterator)` | `String` | Consume a string-producing iterator |

The `<>` operator concatenates two strings. `println(text)` writes a `String` to standard output followed by a newline, and `print(text)` writes it without one. Both accept only `String`; format other values first with interpolation, `println("#{count}")`, or `println(String.from(count))`.

`panic(message)` stops with a runtime error: it ends the current actor (a supervisor can restart it), or the program with exit status 101 when called from `main`, printing `Mesh panic: message`. It never returns, so it fits any branch:

```mesh
fn parse_port(text :: String) -> Int do
  case String.to_int(text) do
    Some(port) -> port
    None -> panic("not a port: #{text}")
  end
end

fn main() do
  println("${parse_port("8080")}")
end
```

## Input, Environment, and Files

| Function | Returns | Description |
|----------|---------|-------------|
| `IO.read_line()` | `Result<String, String>` | Read one line from standard input |
| `IO.eprintln(text)` | `Unit` | Write a line to standard error |
| `Env.get(name, default)` | `String` | Read an environment variable or use a default |
| `Env.get_int(name, default)` | `Int` | Read a decimal environment variable or use a default |
| `Env.get_secret_hex(name)` | `Result<SecretBytes, CryptoError>` | Decode a required hex value directly into actor-owned secret storage |
| `Env.args()` | `List<String>` | Return native command-line arguments |
| `File.read(path)` | `Result<String, String>` | Read a UTF-8 text file |
| `File.write(path, text)` | `Result<Unit, String>` | Create or replace a text file |
| `File.append(path, text)` | `Result<Unit, String>` | Append text, creating the file when needed |
| `File.exists(path)` | `Bool` | Test whether a path exists |
| `File.delete(path)` | `Result<Unit, String>` | Delete a file |
| `File.read_bytes(path, offset, length)` | `Result<Bytes, String>` | Read up to `length` bytes starting at `offset` |
| `File.write_bytes(path, offset, bytes, truncate)` | `Result<Unit, String>` | Write bytes at `offset`, creating the file when needed |
| `File.size(path)` | `Result<Int, String>` | Byte length of a regular file |

File operations return error text instead of terminating the program:

```mesh
case File.read("settings.txt") do
  Ok(contents) -> println(contents)
  Err(error) -> IO.eprintln("settings: #{error}")
end
```

The byte functions work on bounded ranges without decoding UTF-8. Each call reads or writes 1 byte to 64 KiB, the offset must not be negative, and the range must end within the first 16 MiB of the file; any other range returns `Err("invalid binary file range")`. `File.read_bytes` returns fewer bytes near the end of the file and empty `Bytes` at or past it. `File.write_bytes` overwrites in place and keeps later bytes; writing past the end fills the gap with zero bytes. `truncate = true` empties the file first and is accepted only at offset `0`. Because a write needs at least one byte, use `File.write(path, "")` to empty a file. `File.size` returns an error for directories and other non-regular files.

```mesh
fn copy_from(source :: String, target :: String, offset :: Int, size :: Int) -> Int!String do
  if offset >= size do
    Ok(size)
  else
    let chunk = File.read_bytes(source, offset, 65_536)?
    File.write_bytes(target, offset, chunk, offset == 0)?
    copy_from(source, target, offset + Bytes.length(chunk), size)
  end
end

fn copy_file(source :: String, target :: String) -> Int!String do
  copy_from(source, target, 0, File.size(source)?)
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

`map`, `filter`, `reduce`, `head`, and `tail` are also available without the module name: `head(xs)` is `List.head(xs)`, and `reduce(xs, 0, fn acc, x -> acc + x end)` is `List.reduce(xs, 0, fn acc, x -> acc + x end)`.

### Maps and Sets

| Functions | Purpose |
|-----------|---------|
| `Map.new`, `Map.put`, `Map.get`, `Map.delete`, `Map.has_key`, `Map.size` | Core map operations |
| `Map.keys`, `Map.values`, `Map.merge` | Inspect or combine maps |
| `Map.to_list`, `Map.from_list`, `Map.collect` | Convert `(key, value)` tuples |
| `Set.new`, `Set.add`, `Set.remove`, `Set.contains`, `Set.size` | Core integer-set operations |
| `Set.union`, `Set.intersection`, `Set.difference` | Set algebra |
| `Set.to_list`, `Set.from_list`, `Set.collect` | Convert integer sets |

`Map.get` requires an existing key: a missing one is a runtime error, as `List.get` past the end is. Call `Map.has_key` first when absence is normal.

Lists, maps and sets are immutable: `List.append`, `List.concat` (`++`), `Map.put`, `Map.delete`, `Set.add` and `Set.remove` return a new collection, and the one they were given keeps its elements. Building a collection one element at a time is still cheap: the newest version of a collection grows in place, in amortized constant time, and maps and sets find keys through a hash index. Changing an older version (a value some later change was already made to) copies it. `Queue.push` and `Queue.pop` take amortized constant time too.

### Tuples, Ranges, and Queues

| Function | Returns | Description |
|----------|---------|-------------|
| `Tuple.first(tuple)` | element's own type | First element |
| `Tuple.second(tuple)` | element's own type | Second element |
| `Tuple.nth(tuple, index)` | element's own type | Element at a zero-based index |
| `Tuple.size(tuple)` | `Int` | Tuple arity |
| `Range.new(start, end)` | `Range` | Create the half-open range `[start, end)` |
| `Range.length(range)` | `Int` | Number of integers in the range; `0` when `end <= start`, and the largest `Int` when the true count is larger |
| `Range.to_list(range)` | `List<Int>` | Materialize a range |
| `Range.map(range, fn)` | `List<Int>` | Map its integers |
| `Range.filter(range, predicate)` | `List<Int>` | Retain matching integers |
| `Queue.new()` | `Queue` | Create an empty integer FIFO |
| `Queue.push(queue, value)` | `Queue` | Return a queue with a value appended |
| `Queue.pop(queue)` | `(Int, Queue)` | Return `(front_value, remaining_queue)` |
| `Queue.peek(queue)` | `Int` | Read the front value |
| `Queue.size(queue)` | `Int` | Count queued values |
| `Queue.is_empty(queue)` | `Bool` | Test for an empty queue |

A tuple accessor returns the element's own type, taken from the tuple's type, so `Tuple.first(("a", 1))` is a `String`, and a helper with an unannotated parameter — `fn head(p) do Tuple.first(p) end` — works on any tuple long enough. A *computed* index needs every element to share one type, since any of them could be the one it selects; with a literal index the elements may differ. Where the tuple's type is not known at the accessor, such as an unannotated parameter indexed by a variable, the result is the declared `Int`, so annotate the parameter when the elements are not integers. `Queue.pop` returns a typed `(Int, Queue)`, so `let (front, rest) = Queue.pop(queue)` binds both. An index past the end panics at run time.

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

### Building binary values

`BytesBuilder` appends fields into a buffer with a fixed capacity, then
returns them as `Bytes`:

```mesh
fn encode_frame(kind :: Int, payload :: Bytes) -> Bytes!BinaryError do
  let builder = BytesBuilder.new(7 + Bytes.length(payload))?
  BytesBuilder.write_u8(builder, 1)?
  BytesBuilder.write_u16_be(builder, kind)?
  BytesBuilder.write_u32_be(builder, Bytes.length(payload))?
  BytesBuilder.write_bytes(builder, payload)?
  BytesBuilder.finish(builder)
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `BytesBuilder.new(capacity)` | `Result<BytesBuilder, BinaryError>` | Start an empty builder that holds at most `capacity` bytes (0 through 65,536) |
| `BytesBuilder.write_u8(builder, value)` | `Result<Unit, BinaryError>` | Append one byte |
| `BytesBuilder.write_u16_be(builder, value)` | `Result<Unit, BinaryError>` | Append a big-endian 16-bit integer |
| `BytesBuilder.write_u32_be(builder, value)` | `Result<Unit, BinaryError>` | Append a big-endian 32-bit integer |
| `BytesBuilder.write_bytes(builder, bytes)` | `Result<Unit, BinaryError>` | Append bytes |
| `BytesBuilder.finish(builder)` | `Result<Bytes, BinaryError>` | Consume the builder and return what it holds |

`BytesBuilder` is move-only: the write functions borrow it and `finish`
consumes it. A failed write appends nothing. The runtime returns these
`BinaryError` variants: `InvalidLimit` for a capacity outside 0 through
65,536, `InvalidValue` for an integer that does not fit the field width,
`OutputTooLarge` for a write past the capacity, and `InvalidLength` for a
builder that is no longer usable.

The variants of `BinaryError` are declared by the `mesh-binary` source
package; import them with `from Binary.Reader import BinaryError` to match
on them, or match `Err(_)` without the package. The package also adds a
bounded immutable `BinaryReader`. Its vectors use a canonical unsigned 32-bit
big-endian length prefix, and `finish` rejects trailing bytes.

## Wide integers

`U64`, `U128`, and `I128` are opaque integer values for protocol fields that
do not fit Mesh `Int`. Construction and arithmetic are checked. Convert to
`Int` only when the value is known to fit.

```mesh
fn parse_count(text :: String) -> Int!String do
  let value = U64.parse(text)?
  println(U64.to_string(value))
  U64.to_int(value)
end

fn main() do
  # Prints the value, then "u64 does not fit Int"
  case parse_count("18446744073709551615") do
    Ok(count) -> println("#{count}")
    Err(error) -> println(error)
  end
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
| `Float.to_string(value)` | `String` | Shortest text that reads back as the same value (`1.5`, `2.0`, `1.0e20`) |
| `Float.from(value)` | `Float` | Convert an integer to a float |

`Float.to_string` and string interpolation always show a finite `Float` with a decimal point. Magnitudes of at least `1e16` or below `1e-4` use exponent form, such as `1.0e20` and `1.5e-7`; `1e15` prints as `1000000000000000.0`. The special values print as `inf`, `-inf`, and `NaN`.

## Crypto

The `Crypto` module is binary-first. Public data uses `Bytes`; private keys and
derived key material are move-only resources that cannot be printed, serialized,
or sent through actor mailboxes. Fallible operations return `CryptoError`.

The runtime keeps secrets, private keys, AEAD keys, secret maps, and storage
keys in a table, each owned by the actor that created it, and zeroizes their
memory when they are destroyed; an exiting actor's resources are destroyed
with it. One resource holds at most 64 KiB. An actor may own 4,096 resources
totalling 4 MiB, and the process 65,536 totalling 64 MiB; past those limits
operations return `ResourceLimitExceeded`.

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
fn authenticate() -> Int!CryptoError do
  let key = Secret.random(32)?
  let tag = Crypto.hmac_sha256(key, Bytes.from_utf8("message"))?
  Secret.destroy(tag)
  Secret.destroy(key)
  Ok(0)
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Crypto.random_bytes(length)` | `Result<Bytes, CryptoError>` | OS-backed random public bytes |
| `Secret.random(length)` | `Result<SecretBytes, CryptoError>` | OS-backed move-only secret bytes |
| `Secret.concat(first, second)` | `Result<SecretBytes, CryptoError>` | Consume two secrets and join them, up to 64 KiB |
| `Crypto.hmac_sha256(key, message)` | `Result<SecretBytes, CryptoError>` | HMAC with a borrowed secret key |
| `Crypto.hkdf_sha256(key, salt, info, length)` | `Result<SecretBytes, CryptoError>` | Bounded HKDF output |
| `Crypto.argon2id(password, salt, memory_kib, iterations, parallelism, length)` | `Result<SecretBytes, CryptoError>` | Argon2id v1.3 password KDF with a borrowed secret |
| `Crypto.x25519_generate()` | `Result<X25519KeyPair, CryptoError>` | Generate an X25519 key pair |
| `Crypto.x25519_from_seed(seed)` | `Result<X25519KeyPair, CryptoError>` | Legacy 32-byte `Bytes` private-key constructor |
| `Crypto.x25519_from_secret(material)` | `Result<X25519KeyPair, CryptoError>` | Consume 32 secret bytes as an X25519 private key |
| `Crypto.x25519_public(key)` | `Result<X25519PublicKey, CryptoError>` | Derive the public key again |
| `Crypto.x25519_shared(key, peer)` | `Result<SecretBytes, CryptoError>` | Derive a shared secret |
| `Crypto.signing_generate()` | `Result<SigningKeyPair, CryptoError>` | Generate an Ed25519 key pair |
| `Crypto.signing_from_seed(seed)` | `Result<SigningKeyPair, CryptoError>` | Legacy 32-byte `Bytes` seed constructor |
| `Crypto.signing_from_secret(material)` | `Result<SigningKeyPair, CryptoError>` | Consume 32 secret bytes as an Ed25519 private key |
| `Crypto.mlkem_from_seed(seed)` | `Result<MlKemKeyPair, CryptoError>` | Legacy 64-byte `Bytes` seed constructor |
| `Crypto.mlkem_from_secret(material)` | `Result<MlKemKeyPair, CryptoError>` | Consume 64 secret bytes as an ML-KEM-768 private key |
| `Crypto.sign(key, message)` | `Result<Signature, CryptoError>` | Sign with a borrowed private key |
| `Crypto.verify(key, message, signature)` | `Result<Bool, CryptoError>` | Strict signature verification |
| `Crypto.aead_key(material)` | `Result<AeadKey, CryptoError>` | Consume 32 secret bytes as an AEAD key |
| `Crypto.aead_seal(key, nonce, aad, plaintext)` | `Result<Bytes, CryptoError>` | ChaCha20-Poly1305 encryption |
| `Crypto.aead_open(key, nonce, aad, ciphertext)` | `Result<Bytes, CryptoError>` | Authenticate before returning plaintext |

Borrowed keys remain owned by the caller. `Crypto.aead_key`, `Secret.concat`,
and the `*_from_secret` constructors consume their input, including on error.
Use `Secret.destroy` for early destruction; otherwise the compiler inserts
destruction on every scope exit.

`Crypto.hmac_sha512(key, message)` is a legacy helper kept from the earlier
string API: it takes two `String` values, returns the HMAC-SHA-512 as
lowercase hex, and treats the key as ordinary data. Use `Crypto.hmac_sha256`
with a `SecretBytes` key in new code.

`Crypto.argon2id` accepts salts from 8 through 64 bytes, memory from
`8 * parallelism` through 65,536 KiB, 1 through 10 iterations, 1 through 8
lanes, outputs from 16 through 64 bytes, and passwords up to 65,536 bytes. The
low end exists for published vectors, compatibility tests, and explicitly
versioned application profiles; these bounds are resource-safety limits, not a
password policy. Applications must pin a reviewed profile instead of exposing
the parameters to users. Messenger recovery pins its values in the versioned
backup profile and stores the salt and profile version with the ciphertext.

### Public-key encryption (HPKE)

`Crypto.hpke_seal` encrypts one message to an X25519 public key with RFC 9180
HPKE in base mode, using DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, and
ChaCha20-Poly1305. Every call uses a fresh ephemeral key. The result is the
32-byte encapsulated key followed by the ciphertext and its 16-byte tag, so it
is 48 bytes longer than the plaintext.

```mesh
fn send_invite() -> Bool!CryptoError do
  let recipient = Crypto.x25519_generate()?
  let info = Bytes.from_utf8("example-app/v1/invite")
  let aad = Bytes.from_utf8("room 42")
  let plaintext = Bytes.from_utf8("welcome")
  let sealed = Crypto.hpke_seal(recipient.public_key, info, aad, plaintext)?
  let opened = Crypto.hpke_open(recipient.private_key, info, aad, sealed)?
  Ok(Bytes.secure_equals(opened, plaintext))
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Crypto.hpke_seal(public_key, info, aad, plaintext)` | `Result<Bytes, CryptoError>` | Encrypt `Bytes` to a recipient |
| `Crypto.hpke_open(private_key, info, aad, sealed)` | `Result<Bytes, CryptoError>` | Decrypt with a borrowed private key |
| `Crypto.hpke_seal_secret(public_key, info, aad, secret)` | `Result<Bytes, CryptoError>` | Encrypt a borrowed `SecretBytes` without copying it into `Bytes` |
| `Crypto.hpke_open_secret(private_key, info, aad, sealed)` | `Result<SecretBytes, CryptoError>` | Decrypt directly into a new secret |

`info` is application context bound into the key schedule, up to 65,472
bytes. `aad` is authenticated but not encrypted, up to 64 KiB, and the
plaintext is also limited to 64 KiB. Opening with a different key, `info`, or
`aad`, or opening a modified message, returns `AuthenticationFailed`. A sealed
value shorter than 48 bytes returns `InvalidLength`, and a malformed or
low-order recipient key returns `InvalidPublicKey`.

### ML-KEM-768

ML-KEM is a post-quantum key encapsulation mechanism. The sender encapsulates
to the receiver's public key and gets a ciphertext plus a 32-byte shared
secret; the receiver decapsulates the ciphertext to get the same secret.

```mesh
fn agree() -> Bool!CryptoError do
  let receiver = Crypto.mlkem_generate()?
  let (ciphertext, sender_secret) = Crypto.mlkem_encapsulate(receiver.public_key)?
  let receiver_secret = Crypto.mlkem_decapsulate(receiver.private_key, ciphertext)?
  let sender_key = Crypto.aead_key(sender_secret)?
  let receiver_key = Crypto.aead_key(receiver_secret)?
  let nonce = Bytes.from_utf8("unique nonce")
  let sealed = Crypto.aead_seal(sender_key, nonce, Bytes.empty(), Bytes.from_utf8("hi"))?
  let opened = Crypto.aead_open(receiver_key, nonce, Bytes.empty(), sealed)?
  Ok(Bytes.length(opened) == 2)
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Crypto.mlkem_generate()` | `Result<MlKemKeyPair, CryptoError>` | Generate an ML-KEM-768 key pair |
| `Crypto.mlkem_encapsulate(public_key)` | `Result<(MlKemCiphertext, SecretBytes), CryptoError>` | Return a ciphertext for the receiver and the shared secret |
| `Crypto.mlkem_decapsulate(private_key, ciphertext)` | `Result<SecretBytes, CryptoError>` | Recover the shared secret with a borrowed private key |

`Crypto.mlkem_from_seed` and `Crypto.mlkem_from_secret` above build a key pair
from a 64-byte seed. Public keys are 1,184 bytes and ciphertexts 1,088 bytes;
other lengths return `InvalidPublicKey` and `InvalidLength(1088, actual)`.
Decapsulation uses the implicit rejection of FIPS 203: a modified ciphertext
still returns `Ok`, with a different secret. Authenticate the result before
trusting it, for example by opening an AEAD message with it as above.

### Key types and errors

`X25519KeyPair`, `SigningKeyPair` (Ed25519), and `MlKemKeyPair` have a
move-only `private_key` field and a public `public_key` field.
`X25519PublicKey`, `SigningPublicKey`, `MlKemPublicKey`, `MlKemCiphertext`,
and `Signature` each hold one `bytes :: Bytes` field. Build one from received
bytes with, for example, `X25519PublicKey { bytes: received }`; the operation
that uses it checks the length.

`CryptoError` has these variants:

| Variant | Returned when |
|---------|---------------|
| `InvalidLength(expected, actual)` | An input or requested output is outside its bound; `expected` is the bound or exact size |
| `InvalidKey` | Key material has the wrong size or kind, or a `SecretMap` key is invalid, duplicated, or missing |
| `InvalidPublicKey` | A public key has the wrong length, or an X25519 key is a low-order point |
| `InvalidSignature` | A signature is malformed; a well-formed signature that does not verify returns `Ok(false)` |
| `AuthenticationFailed` | AEAD, HPKE, or storage authentication failed; no plaintext is returned |
| `EntropyUnavailable` | The operating system's random source failed |
| `SecretDestroyed` | A resource was already destroyed or belongs to another actor |
| `ResourceLimitExceeded` | A resource quota, a `SecretMap` capacity, or a storage-key counter is exhausted |
| `UnsupportedOperation` | A storage blob or context has an unknown version, algorithm, or purpose, or the purpose does not match the sealed value |
| `InternalFailure` | An unexpected runtime failure, including a platform storage key without host callbacks |

```mesh
fn describe(error :: CryptoError) -> String do
  case error do
    InvalidLength(expected, actual) -> "expected #{expected} bytes, got #{actual}"
    AuthenticationFailed -> "authentication failed"
    _ -> "crypto failure"
  end
end
```

### Secret maps

`SecretMap` stores up to 64 secrets under public `Bytes` keys in one
actor-owned, zeroizing resource, for sets of keys that change together such as
skipped message keys. It follows the rules of `SecretBytes`: it cannot be
printed, compared, sent to another actor, or serialized except by sealing.

```mesh
fn open_skipped(
  skipped :: borrow SecretMap,
  id :: Bytes,
  nonce :: Bytes,
  aad :: Bytes,
  ciphertext :: Bytes
) -> Bytes!CryptoError do
  let message_key = SecretMap.copy(skipped, id)?
  SecretMap.delete(skipped, id)?
  let key = Crypto.aead_key(message_key)?
  Crypto.aead_open(key, nonce, aad, ciphertext)
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `SecretMap.new(capacity)` | `Result<SecretMap, CryptoError>` | Empty map holding 1 to 64 entries |
| `SecretMap.insert(map, key, secret)` | `Result<Unit, CryptoError>` | Consume `secret` and store it under a new key |
| `SecretMap.contains(map, key)` | `Bool` | Test for a key |
| `SecretMap.copy(map, key)` | `Result<SecretBytes, CryptoError>` | Return a new secret holding the stored value; the entry stays |
| `SecretMap.delete(map, key)` | `Result<Unit, CryptoError>` | Remove and zeroize an entry; a missing key is not an error |
| `SecretMap.fork(map)` | `Result<SecretMap, CryptoError>` | Independent map with the same capacity and entries |
| `SecretMap.merge(target, source)` | `Result<Unit, CryptoError>` | Consume `source` and add its entries to `target` |
| `SecretMap.seal_for_storage(map, storage_key, context)` | `Result<Bytes, CryptoError>` | Seal a borrowed map; see below |
| `SecretMap.unseal_from_storage(blob, storage_key, context)` | `Result<SecretMap, CryptoError>` | Restore a sealed map |

Every function borrows its map except `merge`, which consumes `source`. Keys
must be 1 to 128 bytes. `insert` returns `InvalidKey` for an invalid or
existing key and `ResourceLimitExceeded` when the map is full, and destroys the
secret on any failure. `copy` of a missing key returns `InvalidKey`, while
`contains` returns `false` for an invalid key. The encoded map, including six
bytes of framing per entry, must fit in 64 KiB.

`merge` rejects a key present in both maps with `InvalidKey` and destroys
`source` on any failure. When the combined entries exceed the capacity of
`target`, it drops the oldest entries, in insertion order, until they fit.
`fork` copies every stored secret into a second resource: change the fork while
preparing an update, keep it once the update is verified, and let the unused
map be destroyed at the end of its scope.

### Sealing secrets for storage

A resource is never written out directly. Seal it with a `StorageKey` into an
authenticated blob, store the blob as ordinary `Bytes`, and unseal it into a
new resource later:

```mesh
fn storage_context(
  account :: Bytes,
  device :: Bytes,
  session :: Bytes,
  object :: Bytes,
  purpose :: Int,
  snapshot :: Int
) -> Bytes!BinaryError do
  let builder = BytesBuilder.new(123)?
  BytesBuilder.write_u8(builder, 1)?
  BytesBuilder.write_bytes(builder, account)?
  BytesBuilder.write_bytes(builder, device)?
  BytesBuilder.write_bytes(builder, session)?
  BytesBuilder.write_bytes(builder, object)?
  BytesBuilder.write_u16_be(builder, purpose)?
  BytesBuilder.write_u32_be(builder, snapshot / 4_294_967_296)?
  BytesBuilder.write_u32_be(builder, snapshot % 4_294_967_296)?
  BytesBuilder.finish(builder)
end

fn seal_attachment_key(context :: Bytes) -> Bytes!CryptoError do
  let storage_key = StorageKey.ephemeral()?
  let attachment_key = Secret.random(32)?
  let blob = Secret.seal_for_storage(attachment_key, storage_key, context)?
  let restored = Secret.unseal_from_storage(blob, storage_key, context)?
  Secret.destroy(restored)
  Ok(blob)
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `StorageKey.ephemeral()` | `Result<StorageKey, CryptoError>` | Random key that exists only in this process |
| `StorageKey.platform()` | `Result<StorageKey, CryptoError>` | Load or create the application's durable key through the host secure store |
| `StorageKey.seal_bytes(value, storage_key, context)` | `Result<Bytes, CryptoError>` | Seal public `Bytes` of up to 64 KiB |
| `StorageKey.unseal_bytes(blob, storage_key, context)` | `Result<Bytes, CryptoError>` | Authenticate a blob and return its `Bytes` |

`Secret`, `SecretMap`, `X25519PrivateKey`, `SigningPrivateKey`, and
`MlKemPrivateKey` each provide `seal_for_storage(value, storage_key,
context) -> Result<Bytes, CryptoError>`, which borrows the value and the key,
and `unseal_from_storage(blob, storage_key, context)`, which returns a new
resource of that type. `Secret.seal_for_storage` accepts exactly 32 bytes and
`MlKemPrivateKey` seals its 64-byte seed.

The context is exactly 123 bytes and names what the blob holds:

| Bytes | Field | Rule |
|-------|-------|------|
| 0 | Version | `1` |
| 1–32 | Account ID | 32 bytes |
| 33–48 | Device ID | 16 bytes |
| 49–80 | Session ID | 32 bytes; all zero for purposes 5 through 10 and 15 |
| 81–112 | Object ID | 32 bytes |
| 113–114 | Purpose | Big-endian 16-bit identifier |
| 115–122 | Snapshot version | Big-endian 64-bit integer, not zero |

The purpose must match what is sealed: `Secret` takes 1 (root key), 2
(sending chain key), 3 (receiving chain key), 4 (header key), 5 (attachment
key), 11 (skipped message key), or 16 (group epoch secret); `SecretMap` takes
12 (skipped-key map); `SigningPrivateKey` takes 6 (account authorization key)
or 7 (device signing key); `X25519PrivateKey` takes 8 (device DH key), 9
(signed prekey), 10 (one-time prekey), 13 (ratchet DH key), or 17 (group
TreeKEM key); `MlKemPrivateKey` takes 15 (ML-KEM prekey seed); and
`StorageKey.seal_bytes` takes 14 (local data). The runtime checks the version,
purpose, session rule, and snapshot; the IDs are opaque bytes that you choose.

The blob records a SHA-256 binding of the whole context, so unsealing needs
the same 123 bytes and the same key. A different key or context, or any change
to the blob, returns `AuthenticationFailed` without plaintext. A context that
is not 123 bytes returns `InvalidLength(123, actual)`, and a zero snapshot or a
non-zero session ID where it must be zero also returns `InvalidLength`. An
unknown version or purpose, or a purpose for a different value type, returns
`UnsupportedOperation`.

A blob is the plaintext encrypted with ChaCha20-Poly1305 plus 67 bytes of
header and tag, so a 32-byte key seals to 99 bytes. Each nonce is the key's
4-byte prefix followed by a 64-bit counter that the key reserves before every
seal and never reuses, including after a failed seal.

`StorageKey.ephemeral()` keeps its key and counter in memory, so its blobs
cannot be opened after the process exits; use it for tests and short-lived
tools. `StorageKey.platform()` stores the key, nonce prefix, and counter as
one secure-store record named `mesh/storage-key/v2`, creating it on first use
and rewriting it to reserve each counter. It needs the host's secure-store
callbacks (see [Host Capabilities](#host-capabilities)) and returns
`Err(InternalFailure)` without them. In `meshc test`,
`Test.install_in_memory_secure_store()` provides them.

### UUID

```mesh
fn main() do
  let id = Crypto.uuid4()
  println(id)   # e.g. "550e8400-e29b-41d4-a716-446655440000"
end
```

`Crypto.uuid4()` generates a cryptographically random UUID v4 in the standard `8-4-4-4-12` format.

## Host Capabilities

The `Host` module calls back into the application that embeds a Mesh
library, such as a mobile app. The host registers its callbacks with
`mesh_library_register_host_callbacks` after `mesh_library_init`; see
[Library Builds](/docs/library-builds/). Every function takes request `Bytes`
and returns the callback's response `Bytes`:

| Function | Returns | Host callback |
|----------|---------|---------------|
| `Host.secure_store_put(request)` | `Result<Bytes, String>` | `secure_store_put` |
| `Host.secure_store_get(request)` | `Result<Bytes, String>` | `secure_store_get` |
| `Host.secure_store_delete(request)` | `Result<Bytes, String>` | `secure_store_delete` |
| `Host.push_get_token(request)` | `Result<Bytes, String>` | `push_get_token` |
| `Host.background_schedule(request)` | `Result<Bytes, String>` | `background_schedule` |
| `Host.network_state(request)` | `Result<Bytes, String>` | `network_state` |
| `Host.monotonic_clock(request)` | `Result<Bytes, String>` | `monotonic_clock` |
| `Host.wall_clock(request)` | `Result<Bytes, String>` | `wall_clock` |
| `Host.log_redacted(request)` | `Result<Bytes, String>` | `log_redacted` |

The runtime copies the request and response without interpreting them, so
their formats are an agreement between your Mesh code and the host. The one
format the runtime relies on is the secure store's, because
`StorageKey.platform()` uses it: a put request is a 4-byte big-endian key
length, the key, then the value; a get or delete request is the key alone; and
a get callback reports a missing key with status `2`.

A callback runs synchronously on the calling actor's thread. Requests and
responses are limited to 1 MiB each, and the response buffer is zeroized after
it is copied. Failures are returned as `Err` text:

| Error | Meaning |
|-------|---------|
| `host_callback_not_registered` | No callbacks are registered, as in an ordinary executable |
| `host_callback_missing` | Callbacks are registered, but not this one |
| `host_callback_failed:<capability>:<status>` | The callback returned a non-zero status; capabilities are numbered 1 through 9 in table order |
| `host_callback_input_too_large` | The request is larger than 1 MiB |
| `host_callback_output_too_large` | The callback reported a response larger than 1 MiB |

In `meshc test`, `Test.install_in_memory_secure_store()` and
`Test.set_push_token(selector, token)` register test callbacks for the secure
store and push token; see [Testing](/docs/testing/).

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
| `Random.next_int(state, min, max)` | `(Int, Int)` | Return `(next_state, value)` over the inclusive range |
| `Random.next_unit_ppm(state)` | `(Int, Int)` | Return `(next_state, value)` from `0` through `999_999` |

This generator is not suitable for secrets. Use `Crypto.uuid4` for cryptographically random identifiers.

## What's Next?

- [Concurrency](/docs/concurrency/) — actors, jobs, timers, and bounded channels
- [Iterators](/docs/iterators/) — lazy pipelines and collection terminals
- [Testing](/docs/testing/) — write and run tests with `meshc test`
- [Developer Tools](/docs/tooling/) — meshc, meshpkg, formatter, REPL, LSP
- [Web](/docs/web/) — HTTP server, client, and WebSocket
