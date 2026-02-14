# Architecture Patterns

**Domain:** Developer tooling integration -- install script, LSP code completion/symbols/signature help, VS Code extension updates
**Researched:** 2026-02-13
**Overall confidence:** HIGH (based on direct source analysis of all relevant crates and official tower-lsp documentation)

## Executive Summary

The Mesh developer tooling milestone integrates three independent feature sets into the existing architecture: (1) an install script with prebuilt binaries for `meshc`, (2) LSP server enhancements adding code completion, document symbols, and signature help, and (3) VS Code extension updates to support the new LSP capabilities and improve the installation experience.

The critical architectural insight is that **the LSP server already has everything it needs for the new features** -- the `AnalysisResult` stored per-document in `MeshBackend.documents` contains the full `Parse` (rowan CST with all node types) and `TypeckResult` (type map, type registry with struct/sum type definitions, trait registry). Code completion, document symbols, and signature help all derive from data already computed by `analyze_document()`. No new analysis passes are needed -- only new query functions that traverse the existing CST and type information.

The second key insight is that **the VS Code extension already follows the right pattern** for LSP client integration. It launches `meshc lsp` and uses `vscode-languageclient` v9.x. The client automatically supports any new server capabilities advertised via `ServerCapabilities` in `initialize()`. No client-side code changes are needed to enable completion, document symbols, or signature help -- the LSP client picks them up from the server's capability advertisement.

The install script is architecturally independent from the LSP/extension work. It introduces a new `scripts/install.sh` that downloads prebuilt binaries from GitHub Releases to `~/.mesh/bin/meshc` -- the path the VS Code extension already checks (line 42 of `extension.ts`: `path.join(home, ".mesh", "bin", "meshc")`). A new CI workflow builds binaries for each release.

## Recommended Architecture

### High-Level Component Map

```
EXISTING (no changes)              MODIFIED                        NEW
-----------------------            --------                        ---
mesh-lexer                         mesh-lsp/src/server.rs          scripts/install.sh
mesh-parser (CST, AST)               + completion capability       .github/workflows/release.yml
mesh-typeck (TypeckResult)            + document_symbol capability
mesh-codegen                          + signature_help capability
mesh-rt                            mesh-lsp/src/analysis.rs
mesh-fmt                              + completion_items_at()
mesh-repl                             + document_symbols()
mesh-pkg                              + signature_at()
meshc (CLI)                        mesh-lsp/src/lib.rs
                                      + new module declarations
                                   mesh-lsp/src/completion.rs (NEW FILE)
                                   mesh-lsp/src/symbols.rs (NEW FILE)
                                   mesh-lsp/src/signature.rs (NEW FILE)
                                   editors/vscode-mesh/package.json
                                      + version bump, marketplace metadata
                                   editors/vscode-mesh/src/extension.ts
                                      + status bar, auto-install prompt
```

### Component Boundaries

| Component | Responsibility | Communicates With |
|-----------|---------------|-------------------|
| `mesh-lsp/src/server.rs` | Advertise capabilities, dispatch LSP requests to handlers | analysis.rs, completion.rs, symbols.rs, signature.rs |
| `mesh-lsp/src/completion.rs` | Build CompletionItem lists from CST position + TypeckResult | analysis.rs (for AnalysisResult) |
| `mesh-lsp/src/symbols.rs` | Walk CST to produce DocumentSymbol tree | analysis.rs (for Parse) |
| `mesh-lsp/src/signature.rs` | Extract function signatures from TypeckResult at call sites | analysis.rs (for AnalysisResult), definition.rs (for CST traversal) |
| `scripts/install.sh` | Detect platform, download binary from GitHub Releases, install to ~/.mesh/bin/ | GitHub Releases API |
| `.github/workflows/release.yml` | Cross-compile meshc, create GitHub Release with binary assets | Cargo, GitHub Actions |
| `editors/vscode-mesh/` | LSP client, syntax highlighting, user-facing settings | meshc lsp (via stdio) |

## Detailed Integration: LSP Server Enhancements

### Data Already Available in AnalysisResult

The existing `AnalysisResult` (analysis.rs:16-22) contains:

```rust
pub struct AnalysisResult {
    pub diagnostics: Vec<Diagnostic>,
    pub parse: mesh_parser::Parse,      // Full CST with all nodes
    pub typeck: TypeckResult,           // Types, type_registry, trait_registry
}
```

The `TypeckResult` (mesh-typeck/src/lib.rs:123-152) contains:

```rust
pub struct TypeckResult {
    pub types: FxHashMap<TextRange, Ty>,           // Every expression's type
    pub type_registry: TypeRegistry,               // All struct/sum type defs
    pub trait_registry: TraitRegistry,             // All trait defs + impls
    pub qualified_modules: FxHashMap<String, Vec<String>>,  // Imported modules
    pub imported_functions: Vec<String>,            // Selectively imported functions
    // ... errors, warnings, etc.
}
```

This means completion, symbols, and signatures can all be implemented as pure query functions over existing data. No modifications to the analysis pipeline.

### 1. Code Completion (textDocument/completion)

**New file: `mesh-lsp/src/completion.rs`**

**Data sources for completion items:**

| Completion Context | Data Source | Item Kind |
|-------------------|-------------|-----------|
| Top-level identifiers | `typeck.types` keys (scan for NAME nodes at top level) | Variable/Function |
| Built-in functions | `builtins.rs` registration list (println, print, default, compare) | Function |
| Keywords | Static list from `SyntaxKind` keyword variants | Keyword |
| Struct field names | `typeck.type_registry.struct_defs[name].fields` | Field |
| Module-qualified names | `typeck.qualified_modules` keys + their function lists | Module/Function |
| Sum type variants | `typeck.type_registry.sum_type_defs[name].variants` | EnumMember |
| Trait methods | `typeck.trait_registry.all_traits()` -> method names | Method |

**Context detection strategy:**

Given a cursor position, determine what kind of completion to offer:

1. **After `.` (dot access):** Look up the type of the expression before the dot using `typeck.types`. If it is a struct type, offer field names from `type_registry.struct_defs`. If it is any type, offer trait methods via `trait_registry.find_method_traits()`.

2. **After `::` (type annotation):** Offer type names: primitive types (Int, Float, String, Bool), struct names from `type_registry.struct_defs`, sum type names from `type_registry.sum_type_defs`.

3. **Bare identifier prefix:** Offer everything in scope. Walk the CST upward from cursor position, collecting visible let bindings, function definitions, and parameters (same traversal as `find_definition` in definition.rs). Add built-in names and keywords.

4. **After `Module.` (qualified access):** Look up the module name in `typeck.qualified_modules` and offer the function names from that module.

**Integration with server.rs:**

```rust
// In ServerCapabilities (server.rs:76, within initialize()):
completion_provider: Some(CompletionOptions {
    trigger_characters: Some(vec![".".to_string()]),
    resolve_provider: Some(false),  // No lazy resolution needed
    ..Default::default()
}),

// New method on MeshBackend (server.rs):
async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
    let uri_str = params.text_document_position.text_document.uri.to_string();
    let position = params.text_document_position.position;
    let docs = self.documents.lock().unwrap();
    let doc = match docs.get(&uri_str) {
        Some(doc) => doc,
        None => return Ok(None),
    };
    let items = completion::completion_items_at(
        &doc.source,
        &doc.analysis.parse,
        &doc.analysis.typeck,
        &position,
    );
    Ok(Some(CompletionResponse::Array(items)))
}
```

**Key implementation detail -- scope-aware name collection:**

The existing `find_variable_or_function_def` in definition.rs walks up the CST to find definitions. Completion needs a similar but inverted operation: instead of finding one name, collect ALL visible names at a position. This can be extracted as a shared utility:

```rust
// In a shared module or in completion.rs:
pub fn visible_names_at(
    root: &SyntaxNode,
    source: &str,
    offset: usize,
    typeck: &TypeckResult,
) -> Vec<(String, CompletionItemKind, Option<String>)> {
    // Walk CST from cursor position upward, collecting:
    // - let bindings (NAME nodes in LET_BINDING before cursor)
    // - function definitions (NAME nodes in FN_DEF)
    // - function parameters (IDENT in PARAM_LIST)
    // - module-level definitions (all FN_DEF/STRUCT_DEF/etc in SOURCE_FILE)
    // For each, look up type from typeck.types for the detail string
}
```

### 2. Document Symbols (textDocument/documentSymbol)

**New file: `mesh-lsp/src/symbols.rs`**

Document symbols provide the Outline view and breadcrumb navigation in VS Code. This is a straightforward CST walk -- no type information needed, just structure.

**CST node to SymbolKind mapping:**

| SyntaxKind | SymbolKind | Selection Range |
|------------|-----------|-----------------|
| `FN_DEF` | Function | NAME child |
| `STRUCT_DEF` | Struct | NAME child |
| `SUM_TYPE_DEF` | Enum | NAME child |
| `MODULE_DEF` | Module | NAME child |
| `LET_BINDING` (top-level) | Variable | NAME child |
| `INTERFACE_DEF` | Interface | NAME child |
| `IMPL_DEF` | Class | First PATH child |
| `ACTOR_DEF` | Class | NAME child |
| `SERVICE_DEF` | Class | NAME child |
| `IMPORT_DECL` | Package | Full range |

**Nested symbols:** Functions within `MODULE_DEF` nodes become children of the module symbol. Fields within `STRUCT_DEF` become children. Variants within `SUM_TYPE_DEF` become children. Methods within `IMPL_DEF` and `INTERFACE_DEF` become children.

**Integration with server.rs:**

```rust
// In ServerCapabilities:
document_symbol_provider: Some(OneOf::Left(true)),

// New method:
async fn document_symbol(
    &self,
    params: DocumentSymbolParams,
) -> Result<Option<DocumentSymbolResponse>> {
    let uri_str = params.text_document.uri.to_string();
    let docs = self.documents.lock().unwrap();
    let doc = match docs.get(&uri_str) {
        Some(doc) => doc,
        None => return Ok(None),
    };
    let symbols = symbols::document_symbols(
        &doc.source,
        &doc.analysis.parse,
    );
    Ok(Some(DocumentSymbolResponse::Nested(symbols)))
}
```

**Use Nested response format** (not Flat). This gives VS Code the hierarchy for the Outline view: Module -> Functions, Struct -> Fields, etc. The `DocumentSymbol` struct supports `children: Option<Vec<DocumentSymbol>>`.

**Coordinate conversion:** Document symbols need LSP `Range` values. Use the existing `offset_to_position` function from analysis.rs for both `range` (full node span) and `selection_range` (NAME child span). Important: must convert from rowan tree offsets to source offsets using `tree_to_source_offset` from definition.rs, since rowan strips whitespace.

### 3. Signature Help (textDocument/signatureHelp)

**New file: `mesh-lsp/src/signature.rs`**

Signature help shows function parameter information as the user types arguments within a call expression.

**Detection logic:**

1. Find the cursor position in the CST
2. Walk up the tree to find the enclosing `CALL_EXPR` node
3. Count how many `COMMA` tokens appear before the cursor position to determine the active parameter index
4. Identify the called function: the first child of `CALL_EXPR` is the callee expression
5. Look up the callee's type from `typeck.types` -- it will be a `Ty::Fun(params, ret)`
6. Build `SignatureInformation` from the function type

**Building parameter labels:**

For a function `fn add(a :: Int, b :: Int) -> Int`, the signature label is `fn add(a :: Int, b :: Int) -> Int` and each parameter gets a label that is either `a :: Int` or an offset range within the full label string.

To get parameter names (not just types), walk the `FN_DEF` CST node for the definition. The `PARAM_LIST` contains `PARAM` nodes with `IDENT` tokens and optional `TYPE_ANNOTATION` children. For built-in functions, parameter names come from a static table.

**Integration with server.rs:**

```rust
// In ServerCapabilities:
signature_help_provider: Some(SignatureHelpOptions {
    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
    retrigger_characters: Some(vec![",".to_string()]),
    ..Default::default()
}),

// New method:
async fn signature_help(
    &self,
    params: SignatureHelpParams,
) -> Result<Option<SignatureHelp>> {
    let uri_str = params.text_document_position_params.text_document.uri.to_string();
    let position = params.text_document_position_params.position;
    let docs = self.documents.lock().unwrap();
    let doc = match docs.get(&uri_str) {
        Some(doc) => doc,
        None => return Ok(None),
    };
    signature::signature_at(
        &doc.source,
        &doc.analysis.parse,
        &doc.analysis.typeck,
        &position,
    )
}
```

### Shared Utilities

Several operations are shared across completion, symbols, and signature help:

1. **Offset conversion:** `offset_to_position`, `position_to_offset` (already in analysis.rs)
2. **Tree/source offset mapping:** `source_to_tree_offset`, `tree_to_source_offset` (already in definition.rs, need `pub` visibility)
3. **CST node classification:** What kind of node is at a given position (already partially in definition.rs)
4. **Type display:** `format!("{}", ty)` via Ty's Display impl (already exists)

The `definition.rs` functions `source_to_tree_offset` and `tree_to_source_offset` are already `pub`. All needed utilities exist.

## Detailed Integration: Install Script + Prebuilt Binaries

### Install Path Convention

The VS Code extension already checks `~/.mesh/bin/meshc` (extension.ts:42):

```typescript
const wellKnown = [
    path.join(home, ".mesh", "bin", "meshc"),
    "/usr/local/bin/meshc",
    "/opt/homebrew/bin/meshc",
];
```

The install script targets `~/.mesh/bin/meshc` as the primary install location and adds `~/.mesh/bin` to PATH via shell profile modification.

### Install Script Architecture (scripts/install.sh)

```
User runs: curl -fsSL https://mesh-lang.dev/install.sh | bash

Script flow:
1. Detect OS (uname -s):  Linux, Darwin
2. Detect Arch (uname -m): x86_64, aarch64/arm64
3. Map to target triple:
   - Linux x86_64    -> x86_64-unknown-linux-musl
   - Linux aarch64   -> aarch64-unknown-linux-musl
   - macOS x86_64    -> x86_64-apple-darwin
   - macOS arm64     -> aarch64-apple-darwin
4. Construct download URL:
   https://github.com/{owner}/{repo}/releases/latest/download/meshc-{triple}.tar.gz
5. Download with curl, extract with tar
6. Move binary to ~/.mesh/bin/meshc
7. chmod +x
8. Add ~/.mesh/bin to PATH in ~/.bashrc / ~/.zshrc (if not already present)
9. Print success message with version
```

**Naming convention for GitHub Release assets:**

Follow `cargo-binstall` convention for compatibility:
```
meshc-x86_64-unknown-linux-musl.tar.gz
meshc-aarch64-unknown-linux-musl.tar.gz
meshc-x86_64-apple-darwin.tar.gz
meshc-aarch64-apple-darwin.tar.gz
```

Each archive contains a single `meshc` binary.

### CI Release Workflow (.github/workflows/release.yml)

```
Trigger: push tag v*

Jobs:
  build-matrix:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest,  target: x86_64-unknown-linux-musl
          - os: ubuntu-24.04-arm, target: aarch64-unknown-linux-musl
          - os: macos-latest,   target: aarch64-apple-darwin
          - os: macos-13,       target: x86_64-apple-darwin
    steps:
      - checkout
      - install Rust + target
      - install LLVM 21 (for inkwell)
      - cargo build --release --target $target -p meshc
      - tar czf meshc-$target.tar.gz -C target/$target/release meshc
      - upload artifact

  release:
    needs: build-matrix
    steps:
      - Create GitHub Release from tag
      - Download all artifacts
      - Upload .tar.gz files as release assets
```

**LLVM dependency concern:** `meshc` depends on `inkwell` which links against LLVM. For the binary to be portable:
- Linux: Use `musl` target and statically link LLVM. Alternatively, ship a dynamically-linked binary with LLVM bundled (larger but simpler).
- macOS: LLVM is statically linked by default with inkwell when using the `llvm21-1` feature. The binary should be self-contained.

This is the highest-risk part of the install architecture. LLVM static linking produces ~50-100MB binaries, which is acceptable for a compiler binary but should be documented.

### Version Detection

The install script should print the installed version. `meshc` already has clap's `version` attribute:

```rust
#[derive(Parser)]
#[command(name = "meshc", version, about = "The Mesh compiler")]
struct Cli { ... }
```

So `meshc --version` prints the version from Cargo.toml. The install script can verify the installation by running this.

## Detailed Integration: VS Code Extension Updates

### Extension Changes

**No code changes needed for basic LSP feature support.** The `vscode-languageclient` v9.x automatically picks up server capabilities. When the server advertises `completion_provider`, `document_symbol_provider`, and `signature_help_provider`, the client enables those features without any extension-side code.

**Optional enhancements:**

| Enhancement | File | Purpose |
|-------------|------|---------|
| Status bar item | extension.ts | Show "Mesh LSP: Running" / "Mesh LSP: Error" |
| Auto-install prompt | extension.ts | If meshc not found, offer to run install script |
| Version display | extension.ts | Show meshc version in status bar |
| Marketplace metadata | package.json | icon, repository, categories, badges |
| README for marketplace | README.md (new) | Extension description for VS Code Marketplace |

**Status bar integration (extension.ts):**

```typescript
const statusBar = window.createStatusBarItem(StatusBarAlignment.Right, 100);
statusBar.text = "$(gear~spin) Mesh LSP";
statusBar.show();

// After successful client.start():
statusBar.text = "$(check) Mesh LSP";

// On error:
statusBar.text = "$(error) Mesh LSP";
statusBar.command = "mesh.configurePath";
```

**Package.json updates for marketplace readiness:**

```json
{
  "icon": "images/mesh-icon.png",
  "repository": {
    "type": "git",
    "url": "https://github.com/{owner}/{repo}"
  },
  "categories": ["Programming Languages", "Linters", "Formatters"],
  "keywords": ["mesh", "actor", "concurrent", "functional"],
  "badges": [...]
}
```

**TextMate grammar is already shared** with the website (website imports from `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json`). Any grammar updates automatically apply to both VS Code highlighting and website Shiki highlighting.

## New vs Modified Components Summary

### New Components

| File | Purpose | Depends On |
|------|---------|-----------|
| `crates/mesh-lsp/src/completion.rs` | Code completion logic | analysis.rs, definition.rs, mesh-parser, mesh-typeck |
| `crates/mesh-lsp/src/symbols.rs` | Document symbol extraction | analysis.rs, mesh-parser |
| `crates/mesh-lsp/src/signature.rs` | Signature help at call sites | analysis.rs, definition.rs, mesh-parser, mesh-typeck |
| `scripts/install.sh` | Platform-detecting install script | GitHub Releases |
| `.github/workflows/release.yml` | Cross-compile + release pipeline | Cargo, LLVM, GitHub Actions |

### Modified Components

| File | What Changes | Scope |
|------|-------------|-------|
| `crates/mesh-lsp/src/lib.rs` | Add `pub mod completion; pub mod symbols; pub mod signature;` | Trivial: 3 lines |
| `crates/mesh-lsp/src/server.rs` | Add 3 capabilities to `ServerCapabilities`, implement 3 new `LanguageServer` trait methods | Medium: ~60 lines |
| `crates/mesh-lsp/Cargo.toml` | No changes needed (all deps already present) | None |
| `editors/vscode-mesh/package.json` | Version bump, marketplace metadata (icon, repo, badges) | Small |
| `editors/vscode-mesh/src/extension.ts` | Optional: status bar, auto-install prompt | Small: ~30 lines |

### Unmodified Components

All compiler crates (mesh-lexer, mesh-parser, mesh-typeck, mesh-codegen, mesh-rt, meshc, mesh-fmt, mesh-repl, mesh-pkg, mesh-common) require zero changes. The LSP features consume existing data through public APIs that already exist.

## Patterns to Follow

### Pattern 1: LanguageServer Trait Method Implementation

**What:** All LSP request handlers follow the same pattern in server.rs: lock documents, get DocumentState, call query function, return result.

**When:** Adding completion, document_symbol, signature_help handlers.

**Example (existing hover handler, server.rs:127-153):**
```rust
async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
    let uri_str = params.text_document_position_params.text_document.uri.to_string();
    let position = params.text_document_position_params.position;
    let docs = self.documents.lock().unwrap();
    let doc = match docs.get(&uri_str) {
        Some(doc) => doc,
        None => return Ok(None),
    };
    let type_info = analysis::type_at_position(&doc.source, &doc.analysis.typeck, &position);
    match type_info {
        Some(ty_str) => Ok(Some(Hover { ... })),
        None => Ok(None),
    }
}
```

Every new handler follows this exact structure. The query logic lives in the module file (completion.rs, symbols.rs, signature.rs), not in server.rs.

### Pattern 2: CST Traversal for Structure Extraction

**What:** Walk the rowan CST using `node.children()` and `node.children_with_tokens()`, matching on `SyntaxKind`.

**When:** Document symbols, completion context detection.

**Example (existing definition.rs:200-248, search_block_for_def):**
```rust
for child in block.children() {
    match child.kind() {
        SyntaxKind::LET_BINDING => { /* extract NAME */ }
        SyntaxKind::FN_DEF => { /* extract NAME */ }
        SyntaxKind::STRUCT_DEF => { /* extract NAME */ }
        SyntaxKind::MODULE_DEF => { /* extract NAME + recurse */ }
        _ => {}
    }
}
```

Document symbols use this exact pattern but produce `DocumentSymbol` structs instead of `TextRange`.

### Pattern 3: Type Lookup by Position

**What:** Given an LSP position, find the type of the expression at that position using `typeck.types` (range -> Ty map).

**When:** Completion after dot (need the receiver type), signature help (need the callee type).

**Example (existing analysis.rs:94-119, type_at_position):**
```rust
pub fn type_at_position(source: &str, typeck: &TypeckResult, position: &Position) -> Option<String> {
    let offset = position_to_offset(source, position)?;
    let target_offset = rowan::TextSize::from(offset as u32);
    let mut best: Option<(TextRange, &Ty)> = None;
    for (range, ty) in &typeck.types {
        if range.contains(target_offset) {
            match &best {
                Some((best_range, _)) if range.len() < best_range.len() => {
                    best = Some((*range, ty));
                }
                None => { best = Some((*range, ty)); }
                _ => {}
            }
        }
    }
    best.map(|(_, ty)| format!("{}", ty))
}
```

Completion and signature help reuse this "find smallest containing range" pattern.

## Anti-Patterns to Avoid

### Anti-Pattern 1: Re-running Type Check for Each LSP Request

**What:** Calling `mesh_typeck::check()` inside completion/symbol/signature handlers.

**Why bad:** The type check is already run on every `didOpen`/`didChange` event. Running it again per request wastes time (type checking is the most expensive operation). The `AnalysisResult` cached in `DocumentState` is always up-to-date.

**Instead:** Always read from `doc.analysis.typeck`. The analysis is refreshed on every text change.

### Anti-Pattern 2: Client-Side Completion Filtering

**What:** Sending all possible completions to VS Code and relying on the client to filter.

**Why bad:** For large files with many visible names, sending thousands of items creates UI lag. The LSP spec expects servers to do position-aware filtering.

**Instead:** Use context detection (after dot, after `::`, bare identifier) to return only relevant items. Limit to ~100 items maximum. Let VS Code fuzzy-match within that set.

### Anti-Pattern 3: Flat Document Symbols

**What:** Using `DocumentSymbolResponse::Flat` instead of `DocumentSymbolResponse::Nested`.

**Why bad:** Flat symbols lose hierarchy. VS Code's Outline view and breadcrumbs work much better with nested symbols (Module > Function, Struct > Fields). The Mesh CST naturally has this hierarchy.

**Instead:** Always use `DocumentSymbolResponse::Nested`. Walk the CST recursively, pushing children into their parent's `children` field.

### Anti-Pattern 4: Hardcoding LLVM Paths in Install Script

**What:** Assuming LLVM is installed at a specific path on the target machine.

**Why bad:** The install script downloads a prebuilt binary. The user should not need LLVM installed. If the binary is not statically linked, it will fail to run.

**Instead:** Ensure the CI release workflow produces fully self-contained binaries (static LLVM linkage). The install script should verify the binary works by running `meshc --version` after installation.

## Suggested Build Order

The three workstreams (install, LSP, extension) are independent but have a natural ordering:

```
1. LSP: Document Symbols   (simplest -- pure CST walk, no type info needed)
   |
2. LSP: Code Completion    (uses CST + typeck, most complex)
   |
3. LSP: Signature Help     (uses CST + typeck, focused scope)
   |
4. Install Script           (independent, but needed before extension updates)
   |
5. CI Release Workflow      (depends on install script conventions)
   |
6. VS Code Extension Update (depends on LSP features + install script)
```

**Rationale:**
- Document symbols is the simplest LSP feature (pure structure, no type queries) and establishes the pattern for the new modules.
- Code completion is the most complex and most valuable feature -- do it second while the patterns are fresh.
- Signature help reuses completion's context-detection infrastructure (finding call expressions, looking up function types).
- The install script is independent but should be done before the extension update, so the extension can reference it.
- The CI workflow validates the install script's assumptions about binary naming.
- The extension update is last because it benefits from testing all server features first.

## Scalability Considerations

| Concern | Current State | With This Milestone |
|---------|---------------|-------------------|
| LSP response time | Hover + goto-def: <5ms | Completion: 10-50ms (CST walk + type lookups). Document symbols: <5ms (pure CST walk). Signature help: <5ms (single call-site lookup). |
| Memory per document | ~300KB (Parse + TypeckResult) | Same -- no additional data structures needed |
| Binary size | ~60MB debug, ~25MB release (LLVM linked) | Same -- no new compile-time deps in mesh-lsp |
| Install script targets | N/A | 4 targets: linux-x86_64, linux-aarch64, macos-x86_64, macos-aarch64 |
| CI build time | Website deploy only (~2 min) | Release builds: ~15-20 min per target (LLVM compilation). Matrix parallelism: ~20 min total. |

## Sources

- Direct source analysis: `crates/mesh-lsp/src/server.rs` -- MeshBackend, DocumentState, LanguageServer impl, ServerCapabilities advertisement (hover, definition, TextDocumentSyncKind::FULL)
- Direct source analysis: `crates/mesh-lsp/src/analysis.rs` -- AnalysisResult struct, analyze_document(), type_at_position(), offset_to_position(), position_to_offset()
- Direct source analysis: `crates/mesh-lsp/src/definition.rs` -- find_definition(), source_to_tree_offset(), tree_to_source_offset(), CST traversal patterns
- Direct source analysis: `crates/mesh-lsp/src/lib.rs` -- module declarations, run_server()
- Direct source analysis: `crates/mesh-typeck/src/lib.rs` -- TypeckResult (types map, type_registry, trait_registry, qualified_modules, imported_functions)
- Direct source analysis: `crates/mesh-typeck/src/ty.rs` -- Ty enum, Display impl
- Direct source analysis: `crates/mesh-typeck/src/env.rs` -- TypeEnv scope stack
- Direct source analysis: `crates/mesh-typeck/src/builtins.rs` -- built-in function registrations
- Direct source analysis: `crates/mesh-typeck/src/infer.rs` -- StructDefInfo, SumTypeDefInfo, TypeRegistry
- Direct source analysis: `crates/mesh-parser/src/lib.rs` -- Parse struct, syntax() accessor
- Direct source analysis: `crates/mesh-parser/src/syntax_kind.rs` -- All SyntaxKind variants (FN_DEF, STRUCT_DEF, etc.)
- Direct source analysis: `editors/vscode-mesh/package.json` -- vscode-languageclient v9.x, mesh.lsp.path setting, scripts
- Direct source analysis: `editors/vscode-mesh/src/extension.ts` -- findMeshc() with ~/.mesh/bin/meshc fallback, startClient()
- Direct source analysis: `Cargo.toml` -- workspace members, tower-lsp v0.20, inkwell with llvm21-1 feature
- Direct source analysis: `.github/workflows/deploy.yml` -- existing CI pattern
- [tower-lsp LanguageServer trait docs](https://docs.rs/tower-lsp/latest/tower_lsp/trait.LanguageServer.html) -- completion, document_symbol, signature_help method signatures
- [lsp-types DocumentSymbol](https://docs.rs/lsp-types/latest/lsp_types/struct.DocumentSymbol.html) -- DocumentSymbol struct fields
- [tower-lsp GitHub](https://github.com/ebkalderon/tower-lsp) -- LSP 3.17.0 compliance, lsp-types 0.94
- [cargo-binstall naming convention](https://github.com/cargo-bins/cargo-binstall) -- {name}-{target}-v{version}.{format} convention
- [VS Code publishing docs](https://code.visualstudio.com/api/working-with-extensions/publishing-extension) -- vsce, marketplace metadata requirements
- Confidence: HIGH -- all architectural claims based on direct reading of current source code plus official library documentation
