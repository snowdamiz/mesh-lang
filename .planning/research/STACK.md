# Stack Research

**Domain:** Developer Tooling for Mesh Programming Language (install scripts, binary distribution, LSP improvements, VS Code extension publishing, REPL/formatter audit)
**Researched:** 2026-02-13
**Confidence:** HIGH

## Recommended Stack

### Core Technologies

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| tower-lsp | 0.20 (existing) | LSP server framework | Already in use; provides `completion`, `signature_help`, `document_symbol` trait methods ready to implement -- no version change needed |
| lsp-types | 0.94 (via tower-lsp 0.20) | LSP protocol types | CompletionItem, DocumentSymbol, SignatureInformation all available in current version |
| @vscode/vsce | ^2.22.0 (existing) | VS Code extension packaging and publishing | Already in devDependencies; handles `vsce package` and `vsce publish` to Marketplace |
| vscode-languageclient | ^9.0.1 (existing) | VS Code LSP client | Already in use; supports all LSP 3.17 features client-side including completion, signature help, document symbols |
| GitHub Actions | N/A | CI/CD for binary releases and extension publishing | Already using for website deploy; extend for release workflow |
| Shell script (POSIX sh) | N/A | Install script for prebuilt binaries | Standard `curl -LsSf | sh` pattern used by Rust ecosystem (rustup, cargo-dist, Gleam) |

### Supporting Libraries (Rust -- No New Dependencies)

No new Rust crate dependencies are needed. All LSP features are implemented by overriding additional trait methods on the existing `tower_lsp::LanguageServer` trait. The data needed for completion, symbols, and signature help is already present in the existing `TypeckResult`, `Parse`, and `TypeEnv` structures.

| Existing Library | Version | New Usage | Why Sufficient |
|-----------------|---------|-----------|----------------|
| tower-lsp | 0.20 | Override `completion()`, `signature_help()`, `document_symbol()` methods | These are provided methods on `LanguageServer` trait; just override them |
| rowan | 0.16 | Walk CST for document symbols extraction | Already used for go-to-definition; same traversal pattern for symbols |
| mesh-typeck | internal | Extract function/struct/type info for completions | `TypeckResult.type_registry` has all struct/sum-type defs; `builtins.rs` has all built-in names |
| mesh-parser | internal | AST item enumeration for document symbols | `SourceFile::items()` already iterates FnDef, StructDef, ModuleDef, etc. with Name/ParamList |
| mesh-fmt | internal | LSP formatting integration via `textDocument/formatting` | `format_source()` already works; just wire to LSP `formatting()` method |

### Supporting Libraries (Node.js/TypeScript -- No New Dependencies)

The VS Code extension needs no new dependencies. `vscode-languageclient` ^9.0.1 already supports all client-side features for completion, signature help, and document symbols. The extension currently declares `documentSelector` and `fileEvents` which is sufficient for all new LSP capabilities.

| Existing Library | Version | Notes |
|-----------------|---------|-------|
| vscode-languageclient | ^9.0.1 | Completion, signature help, symbols all work automatically when server advertises capabilities |
| @vscode/vsce | ^2.22.0 | `vsce publish` with Personal Access Token for Marketplace |
| typescript | ^5.3.0 | Build toolchain, no change needed |

### Development Tools

| Tool | Purpose | Notes |
|------|---------|-------|
| GitHub Actions (release workflow) | Build prebuilt binaries for macOS (x86_64 + aarch64), Linux (x86_64) | Matrix build with `cargo build --release`, then upload to GitHub Releases |
| `vsce` CLI | Package and publish VS Code extension | Run `vsce package --no-dependencies` then `vsce publish` with PAT |
| Azure DevOps PAT | VS Code Marketplace authentication | Required for publishing; max 1-year expiry, store in GitHub Secrets |
| `softprops/action-gh-release` | GitHub Action for creating releases | Standard Action for uploading binary artifacts to GitHub Releases |

## Detailed Technical Decisions

### 1. Install Script: POSIX Shell Script (Not cargo-dist)

**Decision:** Write a custom POSIX shell install script, not use cargo-dist.

**Why:** Mesh depends on LLVM 21 at compile time. cargo-dist is designed for standalone Rust binaries and has significant limitations with LLVM system dependencies:
- cargo-dist's musl static linking explicitly cannot dynamically link against C libraries (which LLVM requires)
- The `llvm-sys` crate needs `llvm-config` available at build time, which binary distributions of LLVM typically don't include
- cargo-dist's cross-compilation support is still evolving for complex system deps

**Install script pattern:** Follow the Gleam/rustup model:
1. Detect OS (macOS/Linux) and architecture (x86_64/aarch64)
2. Construct download URL from GitHub Releases: `https://github.com/{owner}/{repo}/releases/latest/download/meshc-{os}-{arch}.tar.gz`
3. Download, verify checksum, extract to `~/.mesh/bin/`
4. Add to PATH (or print instructions)

**The script itself needs no dependencies** beyond `curl`, `tar`, and `sh` -- all present on macOS and Linux by default.

**Confidence:** HIGH -- this pattern is proven by rustup, Gleam, Deno, Bun, and many other language toolchains.

### 2. Binary Distribution: Native Builds per Platform via GitHub Actions

**Decision:** Build binaries natively on each platform runner (not cross-compile).

**Why:** Mesh's `meshc` binary links against LLVM 21. Cross-compiling LLVM-linked binaries is extremely fragile. Building natively on `macos-latest` (for aarch64) and `ubuntu-latest` (for x86_64 Linux) avoids all linker complexity.

**Build matrix:**

| Target | Runner | Notes |
|--------|--------|-------|
| `x86_64-apple-darwin` | `macos-13` | Intel Mac; install LLVM 21 via Homebrew |
| `aarch64-apple-darwin` | `macos-latest` (ARM) | Apple Silicon; install LLVM 21 via Homebrew |
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | Install LLVM 21 from apt.llvm.org |

**Not targeting Windows initially** because Mesh does not currently have Windows support in its codegen/runtime (the runtime uses POSIX APIs). Adding Windows is a separate future effort.

**Linking strategy:** Dynamic linking to LLVM on macOS (Homebrew manages LLVM), static linking on Linux where possible. The install script should document LLVM as a runtime dependency on macOS or bundle the required LLVM shared libraries.

**Alternative considered:** cargo-dist. Rejected because cargo-dist cannot handle the LLVM system dependency correctly for static binary distribution, and its cross-compilation for C-dependent crates is unreliable.

**Confidence:** HIGH -- native CI builds are the most reliable approach for LLVM-dependent binaries.

### 3. LSP Completion: Use Existing TypeckResult + Builtins Data

**Decision:** Build completion items from three sources, all already available:
1. **Built-in names** from `builtins.rs`: `println`, `print`, `IO.read_line`, `String.length`, etc.
2. **User-defined names** from `TypeckResult.type_registry`: struct defs, sum type defs, function schemes
3. **Keywords**: `fn`, `let`, `if`, `case`, `do`, `end`, `struct`, `module`, `type`, `import`, `from`, etc.

**tower-lsp integration:** Override `completion()` method, declare `completion_provider: Some(CompletionOptions { trigger_characters: Some(vec![".".to_string()]), ..Default::default() })` in ServerCapabilities.

**No new crates needed.** The `AnalysisResult` in `mesh-lsp/analysis.rs` already contains the `TypeckResult` which has `type_registry` (all struct/sum-type defs) and `types` (all inferred types). The `analysis::analyze_document` function runs the full parse + typecheck pipeline on every change.

**Confidence:** HIGH -- all data sources already exist; this is wiring, not research.

### 4. LSP Document Symbols: Walk AST Items

**Decision:** Use the existing `SourceFile::items()` iterator to produce hierarchical `DocumentSymbol` responses.

**Mapping:**

| Mesh Item | LSP SymbolKind | Detail String |
|-----------|---------------|---------------|
| FnDef | Function | Parameter list + return type if annotated |
| StructDef | Struct | Field names |
| SumTypeDef | Enum | Variant names |
| ModuleDef | Module | (children are nested symbols) |
| LetBinding | Variable | Inferred type from TypeckResult |
| InterfaceDef | Interface | Method names |
| ImplDef | Class | "impl Trait for Type" |
| ActorDef | Class | Actor name |
| ServiceDef | Class | Service name |
| TypeAliasDef | TypeParameter | Aliased type |

**Return format:** Use hierarchical `DocumentSymbol` (not flat `SymbolInformation`) because Mesh has nested items (functions inside modules, methods inside actors/services).

**Confidence:** HIGH -- direct mapping from existing AST types.

### 5. LSP Signature Help: Extract From TypeckResult

**Decision:** Implement `signature_help()` by:
1. Finding the enclosing call expression at the cursor position
2. Resolving the callee name through the existing name resolution
3. Looking up the function's type scheme in `TypeckResult` or builtins
4. Formatting parameters with types from the scheme

**Trigger characters:** `(` and `,` -- standard for function call signature help.

**tower-lsp integration:** Declare `signature_help_provider: Some(SignatureHelpOptions { trigger_characters: Some(vec!["(".to_string(), ",".to_string()]), ..Default::default() })` in ServerCapabilities.

**Confidence:** MEDIUM -- requires finding the enclosing call expression, which needs new CST traversal logic (unlike document symbols which just walk top-level items). The type lookup part is straightforward.

### 6. LSP Formatting: Wire mesh-fmt to textDocument/formatting

**Decision:** Implement `formatting()` by calling `mesh_fmt::format_source()` and producing a single full-document `TextEdit`.

**Why full-document TextEdit:** `mesh-fmt` already produces the complete formatted output. Diffing to produce minimal edits is unnecessary complexity -- VS Code handles full-document replacement efficiently.

**Confidence:** HIGH -- trivial integration; `format_source()` is a pure function that takes source and returns formatted source.

### 7. VS Code Extension Publishing: vsce + GitHub Actions

**Decision:** Publish to the VS Code Marketplace using `@vscode/vsce` (already in devDeps) with Azure DevOps Personal Access Token.

**Publishing workflow:**
1. Create publisher on marketplace.visualstudio.com (publisher name: `mesh-lang`, already in package.json)
2. Generate Azure DevOps PAT with Marketplace publish scope
3. Store PAT as GitHub Secret `VSCE_PAT`
4. Add GitHub Actions workflow: on tag push, `vsce publish` with version from tag

**Extension updates needed before publishing:**
- Add `icon` field in package.json (128px squared PNG)
- Add `repository` field pointing to GitHub
- Ensure `README.md` exists in editors/vscode-mesh/ (marketplace description)
- Add `CHANGELOG.md` for marketplace changelog tab
- Consider `galleryBanner.color` for visual branding

**Not publishing to Open VSX initially.** Open VSX has ~6K extensions vs Marketplace's ~80K. Add later if demand exists.

**Confidence:** HIGH -- vsce is the standard tool, already configured in the project.

### 8. REPL Audit: No Stack Changes

**Decision:** The REPL audit is a quality pass, not a feature addition. No new dependencies.

**Areas to audit:**
- Multi-line continuation edge cases (nested do/end with comments)
- String result formatting for complex types (lists, structs, sum types)
- Error recovery (partial input that fails parse/typecheck should not crash the JIT)
- `:load` interaction with session state

**Existing stack is sufficient:** rustyline 15 for line editing, inkwell 0.8 for JIT execution, mesh-rt for runtime support.

**Confidence:** HIGH -- audit scope, not stack scope.

### 9. Formatter Audit: No Stack Changes

**Decision:** The formatter audit is a quality pass. No new dependencies.

**Areas to audit:**
- Pipe operator multiline formatting (documented known issue)
- Interface method body formatting (documented known issue)
- Comment preservation edge cases
- Long parameter list wrapping

**Existing stack is sufficient:** rowan 0.16 CST, Wadler-Lindig IR in mesh-fmt.

**Confidence:** HIGH -- audit scope, not stack scope.

## Installation

No new Rust dependencies to install. The workspace Cargo.toml remains unchanged.

For the VS Code extension publishing setup:
```bash
# One-time: create marketplace publisher
# Visit https://marketplace.visualstudio.com/manage and create publisher "mesh-lang"

# One-time: generate PAT at https://dev.azure.com
# Add to GitHub Secrets as VSCE_PAT

# Build and publish (automated via GitHub Actions, or manually):
cd editors/vscode-mesh
npm run compile
npx vsce publish
```

For the install script (no build step, it's a shell script):
```bash
# The install script itself is a static file served from GitHub
# Users will run:
curl -LsSf https://raw.githubusercontent.com/{owner}/{repo}/main/install.sh | sh
```

## Alternatives Considered

| Recommended | Alternative | When to Use Alternative |
|-------------|-------------|-------------------------|
| Custom shell install script | cargo-dist | If Mesh didn't depend on LLVM; cargo-dist works great for pure-Rust binaries |
| Native CI builds per platform | Cross-compilation from Linux | If binary had no C/LLVM dependencies; cross works for pure Rust or simple C deps |
| tower-lsp 0.20 (stay) | tower-lsp-server (community fork) | If needing LSP 3.18 features or notebook support; not needed for Mesh's use case |
| tower-lsp 0.20 (stay) | Upgrade to tower-lsp-server (community) | When tower-lsp 0.20 becomes unmaintained; community fork has newer lsp-types but requires dropping async_trait macro |
| Full-document TextEdit for formatting | Minimal diff-based edits | If formatter performance on large files becomes an issue (unlikely for Mesh source sizes) |
| VS Code Marketplace only | Marketplace + Open VSX | When significant Codium/Theia/Eclipse user base requests it |
| Dynamic LLVM linking (macOS) | Static LLVM linking | If producing fully self-contained binaries; requires building LLVM from source with static libs |
| GitHub Releases | Homebrew tap | After initial distribution is stable; Homebrew is a secondary channel |

## What NOT to Use

| Avoid | Why | Use Instead |
|-------|-----|-------------|
| cargo-dist for binary releases | Cannot handle LLVM system dependency correctly; musl static linking prohibits C library linkage | Custom GitHub Actions workflow with native builds per platform |
| `cross` (cross-rs) for CI builds | Cannot cross-compile to macOS; Docker-based approach fails with LLVM system deps | Native runner per platform (`macos-latest`, `ubuntu-latest`) |
| tower-lsp-server (community fork) | Breaking change: removes `#[async_trait]` macro, requires lsp-types migration; tower-lsp 0.20 has all needed methods | Stay on tower-lsp 0.20; monitor community fork for future migration |
| esbuild/webpack bundler for VS Code extension | Extension has minimal dependencies (just vscode-languageclient); bundling adds complexity with no benefit | `vsce package --no-dependencies` (already configured) |
| Homebrew formula as primary distribution | Requires maintaining a tap, formula updates, and adds friction for non-macOS users | Shell install script that works on macOS and Linux uniformly |
| `lsp-types` upgrade (0.94 -> 0.97) | Would require tower-lsp fork or migration to community fork; 0.94 has all types needed | Stay on lsp-types 0.94 (via tower-lsp 0.20) |

## Version Compatibility

| Package | Compatible With | Notes |
|---------|-----------------|-------|
| tower-lsp 0.20 | tokio 1.x, lsp-types 0.94 | Stable; last release from original author. All needed LSP 3.17 features present. |
| vscode-languageclient ^9.0.1 | VS Code ^1.75.0 | Supports LSP 3.17; auto-negotiates capabilities with server |
| @vscode/vsce ^2.22.0 | Node.js >= 18 | Used for packaging and publishing |
| LLVM 21 (Inkwell 0.8) | Rust stable (tested 1.92.0) | Binary builds must match LLVM version exactly |
| rustyline 15 | Rust stable | No compatibility concerns |
| rowan 0.16 | Rust stable | No compatibility concerns |

## Integration Points

### LSP Server -> VS Code Extension (Automatic)

The VS Code extension does NOT need code changes for new LSP features. The `vscode-languageclient` library automatically:
- Sends `textDocument/completion` requests when the user types
- Sends `textDocument/signatureHelp` requests on trigger characters
- Sends `textDocument/documentSymbol` requests for outline view and breadcrumbs
- Sends `textDocument/formatting` requests on format command

All of this is driven by the server's `ServerCapabilities` response in `initialize()`. The extension just needs to start the server and declare the document selector -- which it already does.

### Install Script -> Binary Distribution

The install script must match the exact naming convention used by the GitHub Actions release workflow:
```
meshc-x86_64-apple-darwin.tar.gz
meshc-aarch64-apple-darwin.tar.gz
meshc-x86_64-unknown-linux-gnu.tar.gz
```

The install script and release workflow must be developed together to ensure naming alignment.

### meshc Binary -> LSP Server

The install script installs `meshc` to `~/.mesh/bin/meshc`. The VS Code extension already checks this path (line 42 of `extension.ts`):
```typescript
path.join(home, ".mesh", "bin", "meshc"),
```

No changes needed -- the extension's `findMeshc()` function already looks in the correct location.

## Sources

- [tower-lsp 0.20 docs](https://docs.rs/tower-lsp/0.20.0/tower_lsp/trait.LanguageServer.html) -- LanguageServer trait with completion, signature_help, document_symbol methods (HIGH confidence)
- [tower-lsp GitHub](https://github.com/ebkalderon/tower-lsp) -- Repository and release history (HIGH confidence)
- [tower-lsp-community/tower-lsp-server](https://github.com/tower-lsp-community/tower-lsp-server) -- Community fork with updated lsp-types; evaluated and not adopted (HIGH confidence)
- [LSP 3.17 Specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/) -- Protocol capability definitions (HIGH confidence)
- [VS Code Publishing Extensions](https://code.visualstudio.com/api/working-with-extensions/publishing-extension) -- vsce publishing workflow, PAT setup, marketplace requirements (HIGH confidence)
- [Gleam Installation Guide](https://gleam.run/getting-started/installing/) -- Install script and binary distribution pattern for a similar Rust-built language toolchain (HIGH confidence)
- [cargo-dist releases](https://github.com/axodotdev/cargo-dist/releases) -- Evaluated for binary distribution; rejected due to LLVM dependency constraints (HIGH confidence)
- [Cross-platform Rust CI/CD Pipeline](https://ahmedjama.com/blog/2025/12/cross-platform-rust-pipeline-github-actions/) -- GitHub Actions matrix build pattern for Rust binaries (MEDIUM confidence)
- [houseabsolute/actions-rust-cross](https://github.com/houseabsolute/actions-rust-cross) -- Cross-compilation GitHub Action; evaluated, not suitable for LLVM deps (HIGH confidence)
- [cargo-dist LLVM limitations](https://github.com/axodotdev/cargo-dist/releases/tag/v0.4.0) -- System dependency handling and musl static linking constraints (HIGH confidence)

---
*Stack research for: Mesh Developer Tooling Milestone*
*Researched: 2026-02-13*
