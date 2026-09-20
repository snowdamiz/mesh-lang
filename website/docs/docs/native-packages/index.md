---
title: Native Packages
description: Call checksummed static native libraries from Mesh through the manifest-gated ABI 1 boundary.
---

# Native Packages

Native packages are Mesh's narrow escape hatch for cryptography, binary codecs,
and vendor SDKs that cannot be implemented reasonably in Mesh alone. A native
package combines ordinary Mesh modules with:

- one or more manifest-declared binding modules;
- a checksummed static library for every supported compilation target; and
- public, fully typed `@native` declarations that name C ABI symbols.

`meshc` never runs package build scripts. Package authors build every archive
before publishing it, and the compiler verifies the selected archive before
every link.

Use a native package only when a source-only Mesh package is not sufficient.
Native code can crash the process or violate memory safety, so the package's
ownership contract is part of its public API.

## Complete minimal package

The manifest opts into ABI 1 and lists every file allowed to contain native
declarations:

```toml
[package]
name = "native-math"
version = "0.1.0"

[native]
abi = 1
bindings = ["bindings/math.mpl"]

[[native.libraries]]
target = "aarch64-apple-darwin"
path = "native/aarch64-apple-darwin/libmath.a"
sha256 = "64-lowercase-hex-characters"
```

The binding module declares, but does not implement, the foreign function:

```mesh
@native("mesh_math_add")
pub fn add(left :: Int, right :: Int) -> Int
```

Application code imports it like any other public Mesh function:

```mesh
from Bindings.Math import add

fn main() do
  add(20, 22) |> String.from() |> println()
end
```

Build the static library, put it at the declared path, replace the placeholder
checksum, and compile:

```bash
mkdir -p native/aarch64-apple-darwin
cc -c native/math.c -o native/math.o
ar rcs native/aarch64-apple-darwin/libmath.a native/math.o
shasum -a 256 native/aarch64-apple-darwin/libmath.a

meshc build .
./output
```

Use the exact target reported by your Rust toolchain:

```bash
rustc -vV | sed -n 's/^host: //p'
```

Use `sha256sum` instead of `shasum -a 256` on systems where that is the
available SHA-256 tool. The
[native-math example](https://github.com/hyperpush-org/mesh-lang/tree/main/examples/native-math)
shows the complete C and Mesh sides.

## Manifest rules

### `abi`

Only `abi = 1` is supported. A different value is rejected while reading the
manifest.

### `bindings`

Each binding path must:

- be relative to the package root;
- end in `.mpl`;
- resolve to a file inside the package; and
- be listed explicitly before it may contain `@native`.

An `@native` declaration in an ordinary, undeclared source module is a compile
error. Binding modules still participate in normal module imports and public
symbol visibility.

### `libraries`

Each `[[native.libraries]]` entry describes one prebuilt library:

| Field | Contract |
| --- | --- |
| `target` | Exact target triple used by `meshc build --target`; no wildcard or fallback matching |
| `path` | Package-relative `.a` file, or `.lib` for a `windows-msvc` target |
| `sha256` | Lowercase, 64-character SHA-256 digest of the archive |

A package may declare several targets, but only one library for each exact
target. The build fails if the current target is missing, the file escapes the
package root, or its digest differs.

For example:

```toml
[[native.libraries]]
target = "x86_64-unknown-linux-gnu"
path = "native/x86_64-unknown-linux-gnu/libcodec.a"
sha256 = "..."

[[native.libraries]]
target = "aarch64-apple-darwin"
path = "native/aarch64-apple-darwin/libcodec.a"
sha256 = "..."

[[native.libraries]]
target = "x86_64-pc-windows-msvc"
path = "native/x86_64-pc-windows-msvc/codec.lib"
sha256 = "..."
```

Do not put linker flags, system-library names, or build commands in `path`.
ABI 1 accepts a package-owned static archive, not an arbitrary linker command.

## Declaration rules

A native declaration has this shape:

```mesh
@native("vendor_symbol")
pub fn operation(input :: Bytes, limit :: Int) -> Bytes!String
```

Every declaration must be:

- `pub`;
- bodyless;
- annotated with one non-empty C identifier;
- fully annotated for every parameter and the return type;
- concrete, with no generic parameters or `where` clause; and
- free of function guards.

Only the following value types cross ABI 1:

| Mesh type | C ABI representation |
| --- | --- |
| `Int` | `int64_t` |
| `Float` | `double` |
| `Bool` | `uint8_t`, where `0` is false and `1` is true |
| `String` | Opaque pointer |
| `Bytes` | Opaque pointer |
| `U64` | Opaque pointer |
| `U128` | Opaque pointer |
| `I128` | Opaque pointer |

Parameters must be one of those value types. A return may be a value type or an
`Option<T>` / `Result<T, E>` whose payload types are also ABI-safe values.

User structs, tuples, collections, closures, actors, interfaces, traits, and
generic values do not have a stable ABI 1 layout and are rejected. Pass a
bounded `Bytes` or `String` representation when a richer protocol must cross
the boundary.

## Tagged returns

`Option` and `Result` returns use a tagged pair:

| Return | Tag |
| --- | --- |
| `Some(value)` | `0` |
| `None` | `1` |
| `Ok(value)` | `0` |
| `Err(error)` | `1` |

The C ABI shape is `{ uint8_t tag; void *value; }`. Scalar payloads must be
boxed in Mesh-managed memory. An error string must be a Mesh string, not a
borrowed C string.

## Ownership and failure rules

The native side must follow these rules:

- Pointer parameters are borrowed for the duration of the call. Do not retain
  or free them.
- Pointer returns become Mesh-owned. Allocate them with a Mesh runtime
  constructor or `mesh_gc_alloc_actor`.
- Create returned error strings with `mesh_string_new`.
- Box scalar `Option` and `Result` payloads with `mesh_gc_alloc_actor`.
- Represent a native-owned resource as an `Int` handle and provide an explicit,
  idempotent close operation. Never serialize or reinterpret a handle as a
  pointer.
- Report recoverable failures with `Result`. Never panic, unwind, or long-jump
  across the ABI.

An `Err` is a normal Mesh value. A native crash, invalid pointer, buffer
overrun, or ownership violation is a process failure and cannot be converted
into `Err` by the language runtime.

## Resolving a native dependency

The compiler does not fetch dependencies during a build:

- Run `meshpkg install` for exact-version registry dependencies.
- Run `meshc deps` for git dependencies and to refresh their lock entries.
- Path dependencies resolve from their declared local paths.
- Commit `mesh.lock` so registry checksums and git revisions remain exact.

At build time, Mesh walks transitive dependencies, loads every manifest-gated
binding module, selects the archive matching the effective target, verifies its
SHA-256, and links it into the final binary.

```bash
meshc deps
meshpkg install
meshc build . --target x86_64-unknown-linux-gnu
```

Run `meshc deps` before `meshpkg install` when a project mixes dependency
sources. The registry installer preserves existing git/path lock entries while
adding its checksum-pinned entries.

Cross-compilation also requires a working target linker and system toolchain.
Adding a Rust target alone does not provide a foreign C linker or system
libraries.

## Publishing

`meshpkg publish` includes every declared binding and native library in the
package archive. It verifies each declared native digest before uploading, so a
stale manifest cannot publish silently.

Before publishing:

1. Build every target archive you intend to support.
2. Compute and record every SHA-256.
3. Test each target independently.
4. Confirm that unsupported targets fail with a clear missing-target error.
5. Publish a new immutable package version whenever an archive changes.

Registry package names are owner-scoped. See [Packages and the Registry](/docs/packages/)
for authentication, exact versions, lockfiles, and publishing rules.

## Security checklist

- Keep the ABI small and concrete.
- Bound all input lengths and output allocation.
- Validate enum tags, UTF-8, numeric conversions, and trailing bytes.
- Make handle close operations safe to call during error cleanup.
- Never retain a borrowed Mesh pointer.
- Never accept secrets implicitly through package build scripts; Mesh does not
  execute them.
- Treat an archive checksum as integrity, not as a security review of its code.
