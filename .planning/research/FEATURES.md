# Feature Landscape

**Domain:** Developer tooling for the Mesh programming language -- install script, prebuilt binary distribution, LSP enhancements (completion, document symbols, signature help), VS Code TextMate grammar update, extension marketplace publishing, REPL/formatter audit, docs fixes.
**Researched:** 2026-02-13
**Confidence:** HIGH for install scripts, grammar, marketplace. MEDIUM for LSP completion (depends on available type info in existing analysis pipeline).

## Existing System Baseline

Before defining features, here is what Mesh already has (verified from codebase):

- **CLI binary:** `meshc` -- single binary with subcommands: `build`, `init`, `deps`, `fmt`, `repl`, `lsp`. Built with Rust + clap. Bundles LLVM-compiled runtime. Current install: `git clone` + `cargo build`.
- **LSP server:** `mesh-lsp` crate using `tower-lsp` v0.20. Advertises: `text_document_sync` (FULL), `hover_provider`, `definition_provider`. Does NOT advertise: completion, document symbols, signature help, formatting, rename, references.
- **Analysis pipeline:** `analysis.rs` calls `mesh_parser::parse(source)` then `mesh_typeck::check(&parse)`. Result contains: parse tree (rowan CST), typeck result with type map (`BTreeMap<TextRange, Ty>`), error list, warning list. Single-file analysis only (no multi-module context in LSP).
- **VS Code extension:** `editors/vscode-mesh/` at v0.1.0, publisher `mesh-lang`. Not published to any marketplace. Uses `vscode-languageclient` v9. Finds `meshc` via: settings > workspace target/ > `~/.mesh/bin/meshc` > `/usr/local/bin/meshc` > PATH.
- **TextMate grammar:** `mesh.tmLanguage.json` with 48 keywords in the lexer but only a subset in the grammar. Website imports grammar directly via `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json` for Shiki highlighting.
- **Language configuration:** Comment toggling (`#`), brackets, auto-closing pairs, indent/dedent on `do`/`end` blocks, folding.
- **Target platforms:** macOS (arm64, x86_64) and Linux (x86_64).

### Grammar Gap Analysis

The TextMate grammar is missing keywords and operators added in phases 1.7-7.0:

**Keywords in lexer but missing from grammar `keyword.control`:**
- `for`, `while`, `cond`, `break`, `continue` (control flow added in phases 1.5-6.3)

**Keywords in lexer but missing from grammar `keyword.declaration`:**
- `trait`, `alias` (added in phases 3.x-6.x)

**Keywords in lexer but missing from grammar `keyword.operator`:**
- `send`, `receive`, `monitor`, `terminate`, `trap`, `after` (actor/supervision keywords, phases 6-7)

**Operators in lexer but missing from grammar:**
- `..` (range), `<>` (diamond/string concat), `++` (list concat), `=>` (fat arrow), `?` (try operator), `|` (or-pattern bar), `&&`, `||`

**Types in lexer/typeck but missing from grammar `support.type`:**
- `Tuple`, `Range`, `Char`, `Ref`, `Atom`, `Duration`, `Instant`, `Regex`, `Bytes` (if any of these exist as builtins)

**Literal patterns missing from grammar:**
- Hex literals (`0xFF`), binary literals (`0b1010`), scientific notation (`1.0e10`)
- Triple-quoted strings (`"""..."""`)
- Doc comments (`## ...`, `##! ...`)

---

## Table Stakes

Features users expect from a programming language's developer tooling. Missing = language feels amateur or unusable in practice.

### 1. Install Script with Platform Detection

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `curl -sSf https://mesh-lang.org/install.sh \| sh` one-liner | Every modern language (Rust, Zig, Deno, Gleam, Bun) uses this pattern. Users expect a single command to install. | **Med** | Shell script: detect OS (`uname -s`) and arch (`uname -m`), download correct binary, place in `~/.mesh/bin/`, update PATH. |
| Platform detection (macOS arm64, macOS x86_64, Linux x86_64) | Must auto-detect the 3 target platforms and fail clearly on unsupported ones. | **Low** | `uname -s` for OS, `uname -m` for arch. Map to download URLs. |
| `~/.mesh/bin/meshc` install location | Consistent well-known location. VS Code extension already checks `~/.mesh/bin/meshc`. | **Low** | `mkdir -p ~/.mesh/bin && cp meshc ~/.mesh/bin/`. |
| PATH configuration with shell detection | Must add `~/.mesh/bin` to PATH in the correct shell config (`.bashrc`, `.zshrc`, `.profile`). | **Med** | Detect shell via `$SHELL`, append `export PATH="$HOME/.mesh/bin:$PATH"` if not already present. Handle bash, zsh, fish. |
| SHA-256 checksum verification | Security table stakes. Every serious install script verifies downloads. | **Low** | Host `.sha256` files alongside binaries. Verify with `shasum -a 256 -c`. |
| `set -euo pipefail` error safety | Script must fail on any error, not silently corrupt. | **Low** | First line of script. |
| TLS-only download (HTTPS enforcement) | Must not allow HTTP fallback. | **Low** | `curl --proto '=https' --tlsv1.2`. |
| Idempotent re-runs | Running the installer twice must not break anything. Must detect existing install and update. | **Low** | Check if binary exists, compare versions, replace if newer. |
| `--yes` / non-interactive mode | CI/CD environments need unattended install. | **Low** | Skip confirmation prompt when flag present. |
| Uninstall instructions | Users must know how to remove it. | **Low** | Print `rm -rf ~/.mesh` at end, or provide `meshc self uninstall`. |

**Confidence: HIGH** -- The rustup install pattern is the gold standard and is well-documented.

### 2. Prebuilt Binary Distribution via GitHub Releases

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Prebuilt tarballs for 3 target triples | Users cannot be expected to have Rust + LLVM toolchain installed. Prebuilt binaries are mandatory. | **High** | CI pipeline: build `meshc` on each platform, create tarballs, upload to GitHub Release. LLVM linkage complicates cross-compilation. |
| GitHub Actions CI for automated releases | Manual release process does not scale and will be forgotten. | **Med** | Use `cargo-dist` or custom workflow. Trigger on git tag push. Build matrix: `macos-14` (arm64), `macos-13` (x86_64), `ubuntu-latest` (x86_64). |
| SHA-256 checksum files per artifact | Install script needs these. Also manual verification. | **Low** | `shasum -a 256 binary.tar.gz > binary.tar.gz.sha256`. |
| Versioned release naming | `meshc-v0.8.0-aarch64-apple-darwin.tar.gz` pattern. | **Low** | Standard naming convention. |
| GitHub Release with changelog | Users discovering the project need release notes. | **Low** | Can auto-generate from git log or maintain CHANGELOG.md. |

**Key challenge:** Mesh bundles LLVM (via `inkwell` with `llvm21-1` feature). This means the binary is large (likely 50-200MB depending on LLVM linkage) and cross-compilation requires LLVM headers for the target platform. Each platform must be built ON that platform (native compilation in CI), not cross-compiled.

**Confidence: HIGH** -- GitHub Actions build matrices for Rust projects are well-established.

### 3. TextMate Grammar Completeness

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| All 48 keywords properly categorized and highlighted | Syntax highlighting is the most basic feature. Incomplete highlighting makes the language look broken. | **Low** | Update regex patterns in `mesh.tmLanguage.json`. Pure data change, no logic. |
| All operators highlighted | Missing `..`, `<>`, `++`, `=>`, `?`, `\|`, `&&`, `\|\|` cause visual inconsistency. | **Low** | Add patterns to operators section. |
| Doc comments (`##`, `##!`) with distinct scope | Doc comments should render differently from regular comments. Table stakes for any documented language. | **Low** | Add patterns: `comment.block.documentation.mesh` for `##` lines. |
| Hex/binary/scientific number literals | `0xFF`, `0b1010`, `1.0e10` must highlight as numbers, not identifiers/errors. | **Low** | Extend number regex patterns. |
| Triple-quoted strings | `"""..."""` must highlight as strings. | **Low** | Add begin/end pattern with `"""` delimiters. |
| String interpolation in triple-quoted strings | `${expr}` inside `"""..."""` must highlight consistently. | **Low** | Reuse existing interpolation pattern within triple-quoted string scope. |
| Module-qualified function calls | `List.map(fn)`, `Map.get(key)` -- the module part should highlight as a type/module, not a variable. | **Med** | Pattern: `\b([A-Z][a-zA-Z0-9_]*)\s*\.` with capture for module name. |
| Pipe operator `\|>` chain highlighting | Already present. Verify it works correctly. | **Low** | Already in grammar as `keyword.operator.pipe.mesh`. |

**Critical dependency:** The website's Shiki syntax highlighting imports directly from `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json`. Any grammar update automatically fixes website code highlighting too. This is a single change with double impact.

**Confidence: HIGH** -- TextMate grammar patterns are well-documented and testable via VS Code's "Developer: Inspect Editor Tokens and Scopes" command.

### 4. VS Code Extension Published to Marketplace

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Published to VS Code Marketplace | Users must be able to `ext install mesh-lang.mesh-lang` to get syntax highlighting. If they can't find it in marketplace, the language does not exist to most developers. | **Med** | Requires: Azure DevOps PAT, publisher ID registration, `vsce publish`. One-time setup + CI automation. |
| Published to Open VSX | Cursor, VSCodium, Windsurf users rely on Open VSX. 10M+ developers use VS Code forks that only access Open VSX. | **Med** | Requires: Eclipse Foundation account, OVSX token, `npx ovsx publish`. |
| Extension icon | Marketplace listings without icons look abandoned. | **Low** | Design a simple Mesh logo icon (128x128 PNG). |
| Extension README with screenshots | Marketplace README is the extension's landing page. Must show syntax highlighting, hover, diagnostics. | **Low** | Screenshots of VS Code with Mesh code, hover showing types, red squiggly diagnostics. |
| CHANGELOG.md | Marketplace renders this as the changelog tab. | **Low** | Document what's in each version. |
| Version bump to 0.2.0+ | v0.1.0 signals "not really released." 0.2.0+ signals intentional first public release. | **Low** | Update `package.json` version. |
| CI/CD publish on tag | Manual publishing will fall behind. Automate publish to both marketplaces on release. | **Med** | GitHub Action: `vsce publish` + `npx ovsx publish` using secrets `VSCE_PAT` and `OVSX_PAT`. |

**Confidence: HIGH** -- VS Code extension publishing is extremely well-documented.

### 5. LSP: Document Symbols (Outline + Breadcrumbs)

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `textDocument/documentSymbol` handler | Powers VS Code's Outline panel, breadcrumb bar, and "Go to Symbol" (`Cmd+Shift+O`). Without it, navigating large Mesh files requires manual scrolling. | **Med** | Walk the CST (rowan parse tree), emit `DocumentSymbol` for each fn, struct, module, actor, service, supervisor, interface, impl, let binding. |
| Hierarchical symbols | Symbols must nest: functions inside modules, fields inside structs, methods inside impl blocks. | **Med** | Use `DocumentSymbol.children` for nested constructs. Walk tree recursively. |
| Correct `SymbolKind` mapping | Mesh constructs map to: `fn` -> Function (12), `struct` -> Struct (23), `module` -> Module (2), `actor` -> Class (5), `interface` -> Interface (11), `let` -> Variable (13), `type` -> TypeParameter (26). | **Low** | Static mapping from CST node kind to SymbolKind. |
| Selection range vs full range | `range` = entire definition (including body), `selection_range` = the name identifier only. Critical for breadcrumbs to highlight the right text. | **Low** | Already have CST ranges. Extract name token range vs full node range. |

**Value multiplier:** One implementation powers three VS Code features simultaneously (Outline, Breadcrumbs, Go to Symbol).

**Confidence: HIGH** -- CST walking for document symbols is straightforward. The existing analysis pipeline already has the parse tree.

### 6. LSP: Code Completion

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Keyword completion | Typing `re` suggests `return`, `receive`. Most basic form of completion. | **Low** | Static list of 48 keywords. Filter by prefix. Emit as `CompletionItemKind::Keyword` (14). |
| Built-in type completion | Typing `Li` suggests `List`. | **Low** | Static list of built-in types. Emit as `CompletionItemKind::Struct` (22) or `Class` (7). |
| In-scope variable completion | Typing `co` suggests `count` if `let count = 10` is in scope. | **High** | Requires scope analysis: walk CST upward from cursor position, collect all let bindings, fn parameters, and function names visible at that point. |
| Function signature in detail | Completion for `add` shows `fn add(a, b) -> Int` in the detail field. | **High** | Requires type information from typeck result. Must map completion position to available typed symbols. |
| Trigger characters: `.` | Typing `list.` should trigger completion with available methods/fields. | **High** | Requires resolving the type of the expression before the dot, then looking up methods/fields on that type. This is significantly more complex than keyword completion. |
| Module-qualified completion | Typing `List.` suggests `map`, `filter`, `reduce`, `new`. | **Med** | Requires knowing which module functions exist. Could start with hardcoded built-in module members. |
| Snippet completions for common patterns | `fn` -> expands to `fn name(params) do\n  \nend`. | **Low** | Static snippet definitions. Emit as `CompletionItemKind::Snippet` (15) with `InsertTextFormat::Snippet`. |

**Recommended phasing:** Start with keyword + type + snippet completion (LOW effort, immediate value). Defer dot-triggered and scope-aware completion to a follow-up. The keyword/type/snippet layer requires zero changes to the analysis pipeline.

**Confidence: MEDIUM** -- Keyword completion is trivial. Scope-aware completion requires significant analysis work that may not fit in this milestone.

### 7. LSP: Signature Help

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `textDocument/signatureHelp` for function calls | When typing `add(`, show parameter info: `fn add(a :: Int, b :: Int) -> Int`. Highlights which parameter the cursor is on. | **High** | Requires: 1) detecting cursor is inside a call expression, 2) resolving the called function, 3) retrieving its parameter names and types, 4) determining which parameter position the cursor is at by counting commas. |
| Trigger characters: `(` and `,` | `(` triggers initial display, `,` advances to next parameter. | **Low** | Declared in `ServerCapabilities.signature_help_provider.trigger_characters`. |
| Active parameter highlighting | Parameter 0 highlighted when after `(`, parameter 1 after first `,`, etc. | **Med** | Count commas between `(` and cursor position in the CST. |

**Dependency:** Useful signature help requires type annotations on functions to be available. For functions with explicit annotations, this works well. For inferred-only functions, parameter types may show as type variables.

**Confidence: MEDIUM** -- The CST contains the information needed, but extracting function signatures at the cursor position requires careful tree traversal.

---

## Differentiators

Features that set Mesh apart from other young languages' tooling. Not expected, but signal quality.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Website syntax highlighting auto-updates | Grammar fix is a single change that updates both VS Code AND website. No other young language has this shared-grammar architecture. | **Zero** | Already built -- Shiki imports from `mesh.tmLanguage.json`. |
| Install script with version management | `mesh install v0.8.0` or `mesh self update`. Most young languages only offer "install latest." | **Med** | Extend install script to accept version arg, download specific release. |
| LSP semantic tokens | Semantic highlighting on top of TextMate grammar. Allows the LSP to override token colors based on type information (e.g., distinguish mutable vs immutable, local vs module-level). | **High** | Requires implementing `textDocument/semanticTokens/full`. Significant LSP work. |
| LSP workspace symbols | `Cmd+T` to search across all files in workspace. | **High** | Requires multi-file indexing, which the LSP currently does not support (single-file analysis only). |
| Formatter integration in LSP | `textDocument/formatting` handler that runs `mesh_fmt::format_source`. | **Low** | Wire existing formatter through LSP. Single function call. |
| Error lens / inline diagnostics | Already works via existing `publishDiagnostics`. Extensions like Error Lens pick this up automatically. | **Zero** | Already functional with current LSP. |

---

## Anti-Features

Features to explicitly NOT build in this developer tooling milestone.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| **Tree-sitter grammar** | Duplicates the TextMate grammar effort. Tree-sitter is used by Neovim/Helix but not VS Code. Build it when there's Neovim user demand. | Keep TextMate grammar as single source of truth. |
| **DAP (Debug Adapter Protocol)** | Mesh compiles to native binaries. DAP requires debug info in DWARF format, stepping support, etc. Enormous scope. | Users use `gdb`/`lldb` on compiled binaries. |
| **Homebrew/APT/Pacman packaging** | Each package manager has its own review process, maintenance burden, and submission timeline. Not worth it pre-1.0. | Direct binary distribution via install script. Revisit post-1.0. |
| **Windows support** | Not a target platform. Binary distribution, install script, and CI would need significant additional work. | Document "WSL recommended" for Windows users. |
| **Full rename/refactoring support** | Requires complete scope analysis across files, which the single-file LSP does not support. | Defer until multi-file LSP analysis exists. |
| **Language-specific formatter for extension** | The LSP could provide formatting, but the extension should not bundle a separate formatter. | Use `textDocument/formatting` through the LSP, which calls the existing `meshc fmt` pipeline. |
| **Extension settings UI** | Complex webview-based settings panels. The single `mesh.lsp.path` setting is sufficient. | Keep settings minimal. |
| **Auto-update mechanism** | Self-updating binaries add complexity and security concerns. | Users re-run install script or download new release. |
| **Code actions / quick fixes** | Requires deep understanding of error recovery and suggested fixes. Premature for current tooling maturity. | Focus on accurate diagnostics first. |
| **Inlay hints** | Type annotations shown inline. Useful but requires complete type information at every expression, which the LSP already stores in typeck. | Could be a fast follow-up, but not table stakes. Defer. |

---

## Feature Dependencies

```
TextMate Grammar Update (no dependencies, standalone)
  |
  +-> Website syntax highlighting (automatic, shared file)
  |
  +-> Extension marketplace publishing (needs updated grammar first)
       |
       +-> Icon + README + screenshots (needs working grammar to screenshot)
       |
       +-> CI/CD publish pipeline (needs publisher accounts set up)

Prebuilt Binary Distribution
  |
  +-> GitHub Actions CI pipeline (build matrix for 3 targets)
  |     |
  |     +-> SHA-256 checksums (generated in CI)
  |     |
  |     +-> GitHub Release with artifacts
  |
  +-> Install script (downloads from GitHub Release)
       |
       +-> Platform detection (OS + arch)
       |
       +-> PATH configuration (shell detection)
       |
       +-> Checksum verification (uses SHA-256 files from CI)

LSP: Document Symbols (depends only on existing CST)
  |
  +-> Outline panel, Breadcrumbs, Go to Symbol in VS Code

LSP: Keyword/Type Completion (depends only on static keyword list)
  |
  +-> Scope-aware completion (depends on CST scope analysis)
  |     |
  |     +-> Dot-triggered completion (depends on type resolution)

LSP: Signature Help (depends on CST + typeck result)

LSP: Formatting via LSP (depends on existing mesh_fmt crate)
```

**Key ordering insight:** Grammar update and LSP document symbols have zero dependencies on each other and can be done in parallel. Install script depends on prebuilt binaries existing, so CI pipeline must come first. Marketplace publishing should happen after grammar is updated (so the published extension has correct highlighting).

---

## MVP Recommendation

Prioritize by impact-to-effort ratio and dependency order:

1. **TextMate grammar update** -- Fix all missing keywords, operators, types, literals, doc comments. Single file change with highest immediate visual impact. Also fixes website highlighting for free.

2. **LSP document symbols** -- Add `textDocument/documentSymbol` handler. Walks existing CST. Enables Outline, Breadcrumbs, Go to Symbol. Medium effort, triple feature payoff.

3. **LSP keyword/type/snippet completion** -- Static completion for keywords, built-in types, and common patterns. No analysis pipeline changes needed. Immediately makes the editor feel alive.

4. **LSP signature help** -- Add `textDocument/signatureHelp` for function calls. Requires CST traversal to find call site and resolve function. Medium-high effort.

5. **GitHub Actions CI pipeline** -- Build matrix for 3 targets, produce tarballs + checksums, create GitHub Release on tag push. Unblocks install script.

6. **Install script** -- Shell script with platform detection, download, checksum verification, PATH setup. Depends on #5.

7. **VS Code extension marketplace publishing** -- Register publisher, set up PAT/OVSX tokens, publish to both marketplaces. Depends on #1 (updated grammar).

8. **LSP formatting** -- Wire `mesh_fmt` through `textDocument/formatting`. Very low effort if done after the above.

**Defer to follow-up:**
- Scope-aware completion (requires analysis pipeline work)
- Dot-triggered completion (requires type resolution at cursor)
- Semantic tokens (high effort, incremental value over TextMate)
- Workspace symbols (requires multi-file indexing)
- Version management in install script (nice-to-have)
- Inlay hints (fast follow-up once type map is exposed)

---

## Sources

- [VS Code Syntax Highlight Guide](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide) -- HIGH confidence
- [VS Code Language Extensions Overview](https://code.visualstudio.com/api/language-extensions/overview) -- HIGH confidence
- [VS Code Publishing Extensions](https://code.visualstudio.com/api/working-with-extensions/publishing-extension) -- HIGH confidence
- [VS Code Semantic Highlight Guide](https://code.visualstudio.com/api/language-extensions/semantic-highlight-guide) -- HIGH confidence
- [TextMate Language Grammars Manual](https://macromates.com/manual/en/language_grammars) -- HIGH confidence
- [LSP 3.17 Specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/) -- HIGH confidence
- [tower-lsp LanguageServer trait](https://docs.rs/tower-lsp/latest/tower_lsp/trait.LanguageServer.html) -- HIGH confidence
- [lsp-types CompletionItemKind](https://docs.rs/lsp-types/latest/lsp_types/struct.CompletionItemKind.html) -- HIGH confidence
- [lsp-types DocumentSymbol](https://docs.rs/lsp-types/latest/lsp_types/struct.DocumentSymbol.html) -- HIGH confidence
- [lsp-types SignatureHelp](https://docs.rs/lsp-types/latest/lsp_types/struct.SignatureHelp.html) -- HIGH confidence
- [cargo-dist documentation](https://axodotdev.github.io/cargo-dist/) -- HIGH confidence
- [rustup-init installer architecture](https://deepwiki.com/rust-lang/rustup/5.1-rustup-init-installer) -- MEDIUM confidence
- [Open VSX Registry](https://open-vsx.org/) -- HIGH confidence
- [Building VS Code Extensions in 2026](https://abdulkadersafi.com/blog/building-vs-code-extensions-in-2026-the-complete-modern-guide) -- MEDIUM confidence
- [My Experience Publishing an Extension for All VS Code IDEs](https://davidgomes.com/my-experience-with-publishing-an-extension-for-all-vs-code-ides/) -- MEDIUM confidence
- [Sorbet Document Outline (LSP documentSymbol example)](https://sorbet.org/docs/outline) -- HIGH confidence
- Mesh codebase: `mesh.tmLanguage.json`, `token.rs`, `server.rs`, `analysis.rs`, `extension.ts`, `package.json` -- verified directly
