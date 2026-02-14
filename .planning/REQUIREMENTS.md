# Requirements: Mesh v8.0 Developer Tooling

**Defined:** 2026-02-13
**Core Value:** Make Mesh installable, editable, and navigable -- developers should be able to install with one command, get complete syntax highlighting and code intelligence, and find accurate documentation.

## Install Infrastructure

- [ ] **INST-01**: User can install meshc via `curl -sSf https://mesh-lang.org/install.sh | sh` one-liner
- [ ] **INST-02**: Install script auto-detects platform (macOS arm64, macOS x86_64, Linux x86_64, Linux arm64) and downloads correct binary
- [ ] **INST-03**: Install script verifies downloaded binary with SHA-256 checksum before extraction
- [ ] **INST-04**: Install script places binary at `~/.mesh/bin/meshc` and configures PATH in user's shell profile (bash, zsh, fish)
- [ ] **INST-05**: Install script is idempotent -- re-running updates to newer version without breaking existing install
- [ ] **INST-06**: Install script supports `--yes` flag for non-interactive CI/CD usage

## Binary Distribution

- [ ] **DIST-01**: GitHub Actions CI pipeline builds meshc for macOS arm64, macOS x86_64, Linux x86_64, and Linux arm64 on tag push
- [ ] **DIST-02**: CI pipeline produces versioned tarballs (`meshc-v{version}-{arch}-{os}.tar.gz`) with SHA-256 checksum files
- [ ] **DIST-03**: CI pipeline creates GitHub Release with all artifacts and changelog

## TextMate Grammar

- [ ] **GRAM-01**: All control flow keywords highlighted (`for`, `while`, `cond`, `break`, `continue`)
- [ ] **GRAM-02**: All declaration keywords highlighted (`trait`, `alias`)
- [ ] **GRAM-03**: All actor/supervision keywords highlighted (`send`, `receive`, `monitor`, `terminate`, `trap`, `after`)
- [ ] **GRAM-04**: All operators highlighted (`..`, `<>`, `++`, `=>`, `?`, `|`, `&&`, `||`)
- [ ] **GRAM-05**: Doc comments (`##`, `##!`) highlighted with distinct documentation scope
- [ ] **GRAM-06**: Hex (`0xFF`), binary (`0b1010`), and scientific (`1.0e10`) number literals highlighted
- [ ] **GRAM-07**: Triple-quoted strings (`"""..."""`) with interpolation highlighted
- [ ] **GRAM-08**: Module-qualified calls (`List.map`, `Map.get`) highlight module name as type/namespace
- [ ] **GRAM-09**: Remove `nil` from constants (Mesh uses `None`, not `nil`)
- [ ] **GRAM-10**: Website Shiki highlighting automatically updated (shared grammar file, no separate work)

## LSP: Document Symbols

- [ ] **SYM-01**: `textDocument/documentSymbol` handler returns hierarchical symbols for fn, struct, module, actor, service, supervisor, interface, impl, let
- [ ] **SYM-02**: Symbols use correct `SymbolKind` mapping (fn->Function, struct->Struct, module->Module, actor->Class, interface->Interface)
- [ ] **SYM-03**: `range` covers entire definition, `selection_range` covers name identifier only

## LSP: Code Completion

- [ ] **COMP-01**: Keyword completion -- typing prefix suggests matching Mesh keywords
- [ ] **COMP-02**: Built-in type completion -- typing prefix suggests built-in types (Int, Float, String, List, Map, Set, Option, Result, etc.)
- [ ] **COMP-03**: Snippet completion for common patterns (fn, let, struct, match, for, while, actor, interface, impl)
- [ ] **COMP-04**: In-scope variable and function name completion from CST walk

## LSP: Signature Help

- [ ] **SIG-01**: `textDocument/signatureHelp` shows parameter names and types when typing inside function call parentheses
- [ ] **SIG-02**: Active parameter highlighting advances with each comma
- [ ] **SIG-03**: Triggered by `(` and `,` characters

## LSP: Formatting

- [ ] **FMT-01**: `textDocument/formatting` handler wires existing `mesh_fmt::format_source` through LSP

## VS Code Extension

- [ ] **EXT-01**: Extension published to VS Code Marketplace (`mesh-lang.mesh-lang`)
- [ ] **EXT-02**: Extension published to Open VSX Registry (Cursor, VSCodium, Windsurf users)
- [ ] **EXT-03**: Extension has icon, README with screenshots, CHANGELOG.md
- [ ] **EXT-04**: Extension version bumped to 0.2.0
- [ ] **EXT-05**: `.vsixignore` excludes dev files and prevents secret leakage

## REPL/Formatter Audit

- [ ] **AUDIT-01**: Formatter correctly handles all v1.7-v7.0 syntax (for, while, trait, impl, associated types, iterator pipelines)
- [ ] **AUDIT-02**: REPL correctly evaluates all v7.0 features (iterators, From/Into, numeric traits, collect)

## Documentation Corrections

- [ ] **DOCS-01**: Getting-started guide uses correct binary name `meshc` (not `mesh`)
- [ ] **DOCS-02**: Landing page install command replaced with working `curl | sh` install script
- [ ] **DOCS-03**: Hero section version badge updated from `v0.1.0` to current version
- [ ] **DOCS-04**: Getting-started description corrected (compiler written in Rust, compiles via LLVM -- not "compiles through Rust")
- [ ] **DOCS-05**: All code examples in getting-started verified to work with `meshc`

## Future Requirements (Deferred)

- Dot-triggered completion (`list.` suggests methods) -- requires type resolution at cursor position
- Semantic tokens (LSP semantic highlighting on top of TextMate grammar)
- Workspace symbols (`Cmd+T` across all files) -- requires multi-file LSP indexing
- Install script version management (`meshc self update`, install specific version)
- Inlay hints (inline type annotations) -- requires exposing type map
- Tree-sitter grammar for Neovim/Helix -- build when there's user demand
- Homebrew/APT/Pacman packaging -- revisit post-1.0
- Extension settings UI -- single `mesh.lsp.path` setting is sufficient

## Out of Scope

| Feature | Reason |
|---------|--------|
| DAP (Debug Adapter Protocol) | Requires DWARF debug info, stepping support -- enormous scope. Users use gdb/lldb. |
| Windows support | Not a target platform. WSL recommended for Windows users. |
| Full rename/refactoring | Requires complete cross-file scope analysis. Defer until multi-file LSP. |
| Code actions / quick fixes | Requires deep error recovery understanding. Focus on diagnostics first. |
| Auto-update mechanism | Security complexity. Users re-run install script. |
| Extension webview settings | Over-engineered for current needs. |
| Cross-compile in CI | LLVM linkage prevents cross-compilation. Build natively per platform. |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| GRAM-01 | TBD | Pending |
| GRAM-02 | TBD | Pending |
| GRAM-03 | TBD | Pending |
| GRAM-04 | TBD | Pending |
| GRAM-05 | TBD | Pending |
| GRAM-06 | TBD | Pending |
| GRAM-07 | TBD | Pending |
| GRAM-08 | TBD | Pending |
| GRAM-09 | TBD | Pending |
| GRAM-10 | TBD | Pending |
| SYM-01 | TBD | Pending |
| SYM-02 | TBD | Pending |
| SYM-03 | TBD | Pending |
| COMP-01 | TBD | Pending |
| COMP-02 | TBD | Pending |
| COMP-03 | TBD | Pending |
| COMP-04 | TBD | Pending |
| SIG-01 | TBD | Pending |
| SIG-02 | TBD | Pending |
| SIG-03 | TBD | Pending |
| FMT-01 | TBD | Pending |
| DIST-01 | TBD | Pending |
| DIST-02 | TBD | Pending |
| DIST-03 | TBD | Pending |
| INST-01 | TBD | Pending |
| INST-02 | TBD | Pending |
| INST-03 | TBD | Pending |
| INST-04 | TBD | Pending |
| INST-05 | TBD | Pending |
| INST-06 | TBD | Pending |
| EXT-01 | TBD | Pending |
| EXT-02 | TBD | Pending |
| EXT-03 | TBD | Pending |
| EXT-04 | TBD | Pending |
| EXT-05 | TBD | Pending |
| AUDIT-01 | TBD | Pending |
| AUDIT-02 | TBD | Pending |
| DOCS-01 | TBD | Pending |
| DOCS-02 | TBD | Pending |
| DOCS-03 | TBD | Pending |
| DOCS-04 | TBD | Pending |
| DOCS-05 | TBD | Pending |
