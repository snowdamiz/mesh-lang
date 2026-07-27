# Native math example

This example exercises ABI scalars, generated bindings, `Result` translation,
and Mesh-owned return memory.

```sh
cc -c native/math.c -o native/math.o
ar rcs native/libmath.a native/math.o
```

Copy `mesh.toml.example` to `mesh.toml`. Use
`rustc -vV | sed -n 's/^host: //p'` for the exact target and
`shasum -a 256 native/libmath.a` on macOS
(`sha256sum native/libmath.a` elsewhere) for the checksum, then run:

```sh
meshc build .
./native-math
```

Expected output:

```text
42
negative
```

The package manager deliberately does not execute those build commands.
