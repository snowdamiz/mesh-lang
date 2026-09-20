# mesh.nvim

Repo-owned Neovim support pack for Mesh.

Together with VS Code, Neovim is a **first-class** editor host in the public Mesh tooling contract: <https://meshlang.dev/docs/tooling/>.

This pack is installed through Neovim's native package runtime under `pack/*/start/mesh-nvim`, stays intentionally bounded to the audited classic syntax plus native `meshc lsp` path proven in this repository, and requires **Neovim 0.11+**. The audited syntax now covers the current language surface described below.

## What this pack does

- Detects `*.mpl` as `filetype=mesh` through native runtime package loading.
- Applies the separate classic Vim syntax grammar for the current audited Mesh language surface.
- Auto-enables a native `vim.lsp` config named `mesh` on Neovim 0.11+ without `nvim-lspconfig`.
- Starts `meshc lsp` with repo-local discovery that favors local dogfooding builds before falling back to well-known install paths or `PATH`.

## Syntax highlighting

The classic Vim grammar recognizes:

- every compiler keyword and every visible operator, delimiter, and punctuation token, checked by probes derived from the compiler token definitions
- declarations, imports and multi-segment module paths, bare calls, Unicode identifiers, built-in types, and constructors
- `@cluster`, `@cluster(N)`, and `@native(...)`
- ORM schema forms and relationships, including `table`, `primary_key`, `timestamps`, `belongs_to`, `has_one`, `has_many`, and `deriving(...)`
- supervisor clauses and values, maps and struct updates, result and optional types, patterns and wildcards, pipes and slot pipes, atoms, single- and physical-multiline regular expressions, string interpolation, and nested block comments

Module-qualified calls are matched structurally instead of from an exhaustive method list. That covers calls through ORM modules such as `Query` and `Repo`, the database, web, concurrency, distributed-runtime, numeric, and binary namespaces, and multi-segment official or user package namespaces as those APIs evolve.

This is a separate classic Vim grammar in `syntax/mesh.vim`; Neovim does not consume the TextMate grammar shared by VS Code and the documentation site. The two implementations are tested against the same current-language fixture and compiler-derived token vocabulary.

Classic Vim regular expressions do not expose Rust's exact Unicode
`is_alphabetic` / `is_alphanumeric` character properties. The grammar uses
exact ASCII rules plus a permissive non-ASCII branch so every compiler-valid
Unicode identifier is highlighted; malformed non-ASCII starts may still look
like identifiers until `meshc lsp` reports them.

An unterminated multiline token such as `~r/...` remains open to the end of the
buffer in the lexical grammar; `meshc lsp` supplies the corresponding compiler
diagnostic.

Declaration-name highlighting is deliberately conservative where item and expression contexts have the same token shape. Private parameterless forms beginning `fn name do`, `fn name when ...`, or `fn name -> ...` keep the name's ordinary identifier highlight, while uppercase `fn Name(...)` forms that overlap constructor-pattern closures retain non-declaration constructor/type highlighting. Their keywords, guards, annotations, types, operators, and bodies are still highlighted, and the compiler/LSP parses them normally. Public functions, `def` declarations, interface methods, conventional lowercase private `fn name(...)` declarations, generic declarations, and direct `fn name = ...` declarations receive declaration-name highlighting.

## What this pack does **not** claim

- No Tree-sitter grammar.
- No semantic highlighting. `meshc lsp` does not currently advertise an LSP semantic-tokens provider, and classic Vim syntax rules do not resolve symbol identity.
- No plugin-manager-specific setup.
- No claims beyond the classic syntax plus native `meshc lsp` path proven in `scripts/verify-m036-s02.sh`.
- No syntax-tree or type-aware distinction between compiler built-ins, official packages, user modules, and similarly shaped names.

## Install

Neovim only needs this directory to appear somewhere on `packpath` as `pack/*/start/mesh-nvim`.
A direct repo-local install looks like this:

```bash
mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/mesh/start"
ln -s \
  "/absolute/path/to/mesh-lang/tools/editors/neovim-mesh" \
  "${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/mesh/start/mesh-nvim"
```

Equivalent locations under any active `packpath` also work, as long as the final path shape is `pack/<group>/start/mesh-nvim`.

After installation, opening any `*.mpl` file should load:

- `ftdetect/mesh.vim` for `filetype=mesh`
- `syntax/mesh.vim` for classic syntax groups
- `plugin/mesh.lua` to auto-enable the native LSP config on supported Neovim versions

## `meshc` resolution and overrides

The LSP transport starts `meshc lsp` and resolves the binary in this order:

1. explicit override via `vim.g.mesh_lsp_path`
2. explicit override via `require('mesh').setup({ lsp_path = '/absolute/path/to/meshc' })`
3. repo/workspace-local `target/debug/meshc`
4. repo/workspace-local `target/release/meshc`
5. well-known install locations:
   - `~/.mesh/bin/meshc`
   - `/usr/local/bin/meshc`
   - `/opt/homebrew/bin/meshc`
6. `PATH`

Root detection is separate from binary discovery:

- workspace root prefers `mesh.toml`
- then falls back to root `main.mpl`
- then falls back to `.git`
- otherwise Mesh attaches in honest single-file mode (`root_dir = nil`)

If discovery fails, the pack reports the searched candidates and tells you to set one of the explicit overrides above.

## Verification

For the full repo-root public tooling/editor proof chain, run this from the repository root:

```bash
bash scripts/verify-m036-s03.sh
```

Use the Neovim-specific verifier below when you only need to replay this pack's bounded proof surface:

```bash
NEOVIM_BIN="${NEOVIM_BIN:-nvim}" bash scripts/verify-m036-s02.sh
```

The command exits `0` only after these named phases pass:

1. `corpus` — materialize the shared interpolation corpus, including markdown-backed docs snippets, into temporary `.mpl` files under `.tmp/m036-s02/`
2. `shared-grammar` — replay `bash scripts/verify-m036-s01.sh`
3. `upstream-lsp` — replay `cargo test -q -p meshc --test e2e_lsp -- --nocapture`
4. `neovim` — install this pack through a real `pack/*/start/mesh-nvim` path and run the headless Neovim smoke covering current syntax, compiler-derived token probes, and LSP attach/root-resolution assertions for a backend-shaped manifest-rooted fixture, a manifest-first override-entry fixture, and honest single-file mode

The syntax proof replays the shared interpolation corpus, checks `@cluster`, `@cluster(3)`, and bare `cluster` boundaries in `scripts/fixtures/cluster-decorators.mpl`, exercises the current core/ORM/supervisor/native/package surface fixture, and verifies the compiler-derived keyword, operator, delimiter, and visible punctuation probes with explicit line/column syntax-stack output. The significant `Newline` punctuation token is tracked as compiler vocabulary but has no visible character to probe.

The verifier emits phase-local logs and leaves artifacts under `.tmp/m036-s02/` so failures stay attributable by phase and case.

Optional narrow runs are available when you only need one side while iterating:

```bash
NEOVIM_BIN="${NEOVIM_BIN:-nvim}" bash scripts/verify-m036-s02.sh syntax
NEOVIM_BIN="${NEOVIM_BIN:-nvim}" bash scripts/verify-m036-s02.sh lsp
NEOVIM_BIN="${NEOVIM_BIN:-nvim}" bash scripts/verify-m036-s02.sh neovim
```
