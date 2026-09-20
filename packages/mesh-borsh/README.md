# mesh-borsh

Bounded Borsh readers and writers for Mesh native packages.

The package is source-only so it never checks in host-specific binaries. Build
the archive for the current host, calculate its SHA-256, then declare that exact
target and checksum in `mesh.toml`:

```sh
rustc --crate-name mesh_borsh_native --crate-type staticlib --edition 2021 -O \
  native/lib.rs -o native/libmesh_borsh.a
rustc -vV
shasum -a 256 native/libmesh_borsh.a
```

```toml
[package]
name = "mesh-borsh"
version = "0.1.0"

[native]
abi = 1
bindings = ["bindings/borsh.mpl"]

[[native.libraries]]
target = "<host triple from rustc -vV>"
path = "native/libmesh_borsh.a"
sha256 = "<archive SHA-256>"
```

Readers copy their input, enforce a caller-selected collection limit, and reject
EOF, invalid booleans, invalid UTF-8, and trailing data. Writers enforce a
caller-selected output limit. Call `finish_reader`/`finish_writer`; use the
idempotent `close_reader`/`close_writer` only when abandoning a handle early.
