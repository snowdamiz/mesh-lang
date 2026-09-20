# Mesh Language

[![VS Code Marketplace Version](https://img.shields.io/visual-studio-marketplace/v/OpenWorthTechnologies.mesh-lang)](https://marketplace.visualstudio.com/items?itemName=OpenWorthTechnologies.mesh-lang)
[![VS Code Marketplace Installs](https://img.shields.io/visual-studio-marketplace/i/OpenWorthTechnologies.mesh-lang)](https://marketplace.visualstudio.com/items?itemName=OpenWorthTechnologies.mesh-lang)

Language support for [Mesh](https://meshlang.dev) -- an expressive, readable programming language with built-in concurrency via actors and supervision trees.

VS Code is a **first-class** editor host in the public Mesh tooling contract. The contract lives at [meshlang.dev/docs/tooling](https://meshlang.dev/docs/tooling/) and keeps this README scoped to the VS Code install, packaging, and run path.

## Features

- **Syntax Highlighting** -- the shared Mesh TextMate grammar used by both VS Code and the documentation site, with compiler-derived coverage for every keyword and every visible operator, delimiter, and punctuation token plus the current built-in type and language-DSL surface
- **Language Configuration** -- line and nested-block comment commands, bracket matching, auto-closing pairs, and Mesh-specific indentation and folding for multiline `do`/`end` blocks
- **Verified LSP Diagnostics** -- real-time parse and type errors from the Mesh compiler
- **Verified Hover** -- inferred type information on hover
- **Verified Go to Definition** -- same-file go-to-definition inside backend-shaped project code
- **Verified Document Formatting** -- format the current Mesh document through `meshc lsp`
- **Verified Signature Help** -- parameter hints with active-parameter tracking for function calls

### Highlighting coverage

The bundled grammar recognizes:

- declarations and names for functions, handlers, modules, structs, sum types, type aliases, actors, services, supervisors, and interfaces
- imports, multi-segment module paths, bare function calls, Unicode identifiers, built-in types, and constructors
- `@cluster`, `@cluster(N)`, and `@native(...)`
- ORM schema forms including `table`, `primary_key`, `timestamps`, `belongs_to`, `has_one`, `has_many`, and `deriving(...)`
- supervisor clauses and values, including child specifications, restart strategies, and shutdown behavior
- maps and struct updates, result and optional types, patterns and wildcards, pipes and slot pipes, ranges, annotations, and the rest of the compiler's operator and punctuation vocabulary
- numbers, atoms, single- and physical-multiline regular expressions, line comments, nested block comments, escaped strings, and both `#{...}` plus `${...}` interpolation in double- and triple-quoted strings

Qualified calls are recognized structurally as module paths followed by a call, rather than from a hard-coded method-name list. Calls such as `Query.where(...)` and `Repo.one(...)`, as well as database, web, concurrency, distributed-runtime, numeric, binary, and multi-segment package namespaces, therefore receive module/function scopes without requiring the grammar to enumerate every ORM or package API.

The syntax layer is lexical TextMate highlighting. `meshc lsp` does not currently advertise an LSP semantic-tokens provider, so highlighting does not resolve whether a name refers to a compiler built-in, an official package, a user module, or another symbol with the same spelling. This repository also does not ship a Tree-sitter grammar. Neovim uses its own classic Vim grammar rather than this TextMate file.

An unterminated multiline token such as `~r/...` remains open to the end of the
document in the lexical grammar; `meshc lsp` supplies the corresponding
compiler diagnostic.

Declaration-name scoping is deliberately conservative where item and expression contexts have the same token shape. Private parameterless forms beginning `fn name do`, `fn name when ...`, or `fn name -> ...` keep the name's ordinary identifier scope, while uppercase `fn Name(...)` forms that overlap constructor-pattern closures retain non-declaration constructor/type scopes. Their keywords, guards, annotations, types, operators, and bodies are still highlighted, and the compiler/LSP parses them normally. Public functions, `def` declarations, interface methods, conventional lowercase private `fn name(...)` declarations, generic declarations, and direct `fn name = ...` declarations receive declaration-name scopes.

The shared grammar regression suite tokenizes the same fixtures through TextMate and the documentation site's Shiki integration. It derives the canonical keyword, operator, delimiter, and punctuation vocabulary from `compiler/mesh-common/src/token.rs`; the significant `Newline` punctuation token is tracked structurally but has no visible glyph to scope. The suite separately checks the current built-in types and representative core, ORM, supervisor, native-package, and namespaced-module forms. The transport-level regression suite exercises the LSP path over real stdio JSON-RPC against a small backend-shaped Mesh project, so the documented editor experience stays tied to the same bounded tooling surface as the CLI. The editor-host smoke remains intentionally bounded to same-file go-to-definition inside backend-shaped project code plus clean diagnostics and hover for a manifest-first override-entry fixture rooted by `mesh.toml` + `lib/start.mpl`.

## Installation

Install Mesh first with the verified public installer pair `https://meshlang.dev/install.sh` and `https://meshlang.dev/install.ps1`. The public installers place both `meshc` and `meshpkg` on your PATH; the extension itself uses `meshc lsp`.

**macOS and Linux:**

```sh
curl -sSf https://meshlang.dev/install.sh | sh
```

**Windows x86_64 (PowerShell):**

```powershell
irm https://meshlang.dev/install.ps1 | iex
```

Verify the installed binaries:

```sh
meshc --version
meshpkg --version
```

For the clustered runtime proof behind this public install contract, use [Distributed Proof](https://meshlang.dev/docs/distributed-proof/) and the public [Developer Tools](https://meshlang.dev/docs/tooling/) guide.

Then build the current packaged extension from source:

```sh
npm install
npm run compile
npm run package
```

The package step writes the current versioned artifact to `dist/mesh-lang-<version>.vsix`. To install that freshly built artifact into VS Code, run:

```sh
npm run install-local
```

## Verification

When you need the full repo-root public tooling/editor proof chain instead of only the extension-local package/install loop, run this from the repository root:

```bash
bash scripts/verify-m036-s03.sh
```

That verifier replays the docs contract, VitePress build, existing VSIX/public README proof, this real Extension Development Host smoke, and the repo-owned Neovim replay from one named-phase command.

## Requirements

The Mesh compiler (`meshc`) must be installed and available in your PATH. The verified public installers at `https://meshlang.dev/install.sh` and `https://meshlang.dev/install.ps1` install both `meshc` and `meshpkg`; this extension connects to the built-in language server provided by `meshc`.

## Extension Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `mesh.lsp.path` | `meshc` | Path to the meshc binary. Must be in PATH, or provide an absolute path. |

## Release Notes

See [CHANGELOG.md](CHANGELOG.md) for a detailed list of changes in each release.
