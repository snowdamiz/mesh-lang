# Native packages (ABI 1)

Native packages are the escape hatch for reusable binary, cryptographic, and
vendor-SDK code that should not become part of `mesh-rt`. Mesh only links
checksummed static archives selected for the exact compilation target.

## Manifest

```toml
[native]
abi = 1
bindings = ["bindings/math.mpl"]

[[native.libraries]]
target = "aarch64-apple-darwin"
path = "native/aarch64-apple-darwin/libmath.a"
sha256 = "64-lowercase-hex-characters"
```

Each target may name one `.a` archive (`.lib` for Windows MSVC). Paths must be
package-relative and cannot be linker flags. `meshc` verifies the artifact
hash before every link. Registry tarballs remain pinned by `mesh.lock`; git
dependencies must match the locked commit; path packages rely on the
per-archive hash.

Bindings are generated or hand-written Mesh source:

```mesh
@native("mesh_math_add")
pub fn add(left :: Int, right :: Int) -> Int
```

Native declarations are valid only in files listed by `bindings`. Symbols must
be C identifiers and signatures must be public, concrete, fully annotated, and
bodyless. Mesh never runs package build scripts. Generate bindings and static
archives before publishing the package.

## ABI and ownership

ABI 1 uses the target C ABI:

| Mesh | C ABI |
| --- | --- |
| `Int` | `int64_t` |
| `Float` | `double` |
| `Bool` | `uint8_t` (`0` or `1`) |
| `String`, `Bytes`, `U64`, `U128`, `I128` | opaque pointer |
| `Result<T, E>`, `Option<T>` return | `{ uint8_t tag; void *value; }` |

Pointer parameters are borrowed for the call and must not be retained or
freed. Pointer returns become Mesh-owned and must use a Mesh constructor or
`mesh_gc_alloc_actor`. Scalar `Result`/`Option` payloads are boxed with
`mesh_gc_alloc_actor`; `Ok`/`Some` use tag `0`, `Err`/`None` use tag `1`.
Native-owned handles travel as `Int`, require an explicit close function, and
must never be treated as pointers or serialized.

A native function reports recoverable failures through `Result`; it must not
panic, unwind, or long-jump across the ABI. `Err` strings are Mesh strings
created with `mesh_string_new`. Crashes and memory violations remain process
failures.

User structs, collections, tuples, closures, actors, and generic native
functions are rejected because their layouts are not part of ABI 1.

See [`examples/native-math`](../examples/native-math/README.md) for a complete
non-trading package.
