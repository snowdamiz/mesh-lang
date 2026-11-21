# Work on a Mesh project

`meshc guide` shows the active compiler version and the digest of its guide.
For installed toolchains, see the [install and update guide](https://meshlang.dev/docs/tooling/#update-an-installed-toolchain).
The [environment-variable reference](https://meshlang.dev/docs/environment-variables/)
covers compiler, runtime, registry, and checker settings.
Run `meshc skills setup --yes --target <agent>` to install the skill for
a coding tool in the current project without prompts.

## Build, run, and diagnose

For a new app, run `meshc init my_app`, then `cd my_app`. Edit `main.mpl`
and run the compiled executable.

```sh
meshc build . --output my_app
./my_app
meshc fmt --check .
meshc lint .
```

Use `[package].entrypoint` in `mesh.toml` to select a different source file
for `fn main()`. A dependency-free executable can build without a manifest.
Add a manifest for package metadata, dependencies, and project test discovery.
Run `meshc check [path]` to parse and type-check without building. Add
`--tests` to include the project's test files. With `--json`, diagnostics
arrive as one JSON v2 object per line on stderr. Use the primary location and
the `fix` field to repair the problem. Run `meshc explain <CODE>` for the
full entry or `meshc explain --json` for the catalog on stdout.
See [Mesh diagnostics](diagnostics.md) for the stream fields.
For editor integration use `meshc lsp`, and for small interactive checks
use `meshc repl`. Use a build or test when behavior depends on a project,
linking, or the native runtime. [Getting started](https://meshlang.dev/docs/getting-started/) · [Developer tools](https://meshlang.dev/docs/tooling/)

## Split code into modules

`policy/retry_rules.mpl` becomes `Policy.RetryRules`. Write
`from Policy.RetryRules import retry` for an unqualified call, or
`import Policy.RetryRules` and call `RetryRules.retry(...)`. Declare the
function `pub` to make it importable. Built-in modules such as `List`
need no import.

`module Name do ... end` defines a file-local module, and `pub module`
allows imports from other files. Use a name distinct from file modules.
Parenthesize a selective import list to span multiple lines.
See [source and module rules](https://meshlang.dev/docs/reference/#source-files-and-modules)
for qualified types, constructors, patterns, and `impl` headers.

## Resolve dependencies and publish packages

Read the exact dependencies in `mesh.toml` and preserve `mesh.lock`.
Use `meshpkg install` for declared registry dependencies and `meshc deps`
for git and path dependencies. The build uses the resolved sources on disk.
Registry dependencies use exact versions. A named `meshpkg install`
downloads the current release and records it in the lockfile. Add its
exact version to the manifest to retain it on a later manifest install.

Import package modules by the package's source-path module name. For
package exports, checksums, publishing, and official packages such as
`mesh-borsh`, `mesh-anchor`, and `mesh-solana`, use the
[package guide](https://meshlang.dev/docs/packages/).

## Choose a native interop boundary

- To call a packaged native archive, use a manifest-listed, bodyless
  `@native("c_symbol")` declaration with a concrete, fully annotated `pub fn`.
  ABI 1 supports `Int`, `Float`, `Bool`, `String`, `Bytes`, `U64`, `U128`, and
  `I128`. `Option` and `Result` returns can carry these ABI values. Follow
  the [native packages guide](https://meshlang.dev/docs/native-packages/)
  for manifests, layout, target selection, and ownership.
- To call Mesh from a host, use `meshc build . --artifact staticlib` or
  `--artifact cdylib` and `@export("c_symbol")` on the public Mesh function.
  Ordinary exports take `Bytes` and return `Bytes!String`. The
  [library builds guide](https://meshlang.dev/docs/library-builds/) covers
  generated headers, lifecycle, host callbacks, and the `@display` boundary.

For tests use `meshc guide testing`. For pure policy laws and proof-bearing
code use `meshc guide verification`. `meshc proof` runs repository-owned
[distributed proof scenarios](https://meshlang.dev/docs/distributed-proof/).
