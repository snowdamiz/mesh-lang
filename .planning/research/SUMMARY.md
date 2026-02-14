# Project Research Summary

**Project:** Mesh Developer Tooling Milestone
**Domain:** Programming language developer tooling (install scripts, binary distribution, LSP enhancements, VS Code extension publishing)
**Researched:** 2026-02-13
**Confidence:** HIGH

## Executive Summary

Mesh requires production-grade developer tooling to move from "build from source" to "install with one command." The research reveals that all the necessary infrastructure is already in place -- the LSP server already has the type information and AST needed for code completion, document symbols, and signature help; the VS Code extension already uses the correct patterns and checks the right install paths; and the existing LLVM-linked binary can be distributed as-is without new dependencies.

The recommended approach is to build in three parallel tracks: (1) implement LSP features by adding query functions over existing data (no new analysis passes needed), (2) create a GitHub Actions release workflow with native builds per platform and a shell install script following the Rust/Gleam pattern, and (3) update the VS Code extension with marketplace metadata and publish to both VS Code Marketplace and Open VSX. These tracks are architecturally independent but must coordinate on naming conventions (binary paths, release asset names).

The key risks are LLVM linkage complexity in CI builds (mitigated by building natively on each platform rather than cross-compiling), LSP mutex deadlocks from concurrent requests (mitigated by never holding locks across async await points), and supply chain security in the install script (mitigated by SHA-256 checksum verification). The existing codebase is well-structured for this work -- the LSP already has hover and go-to-definition working correctly, proving the analysis pipeline is solid.

## Key Findings

### Recommended Stack

**No new dependencies are required.** The existing stack is sufficient for all planned features. The LSP server uses tower-lsp 0.20 which provides completion, document_symbol, and signature_help trait methods ready to override. The VS Code extension uses vscode-languageclient ^9.0.1 which automatically supports all these features client-side when the server advertises the capabilities. Binary distribution requires only shell scripting and GitHub Actions standard actions.

**Core technologies:**
- **tower-lsp 0.20** — LSP server framework with all needed protocol features (completion, document symbols, signature help) already available in current version
- **vscode-languageclient ^9.0.1** — LSP client that automatically picks up new server capabilities without extension code changes
- **@vscode/vsce ^2.22.0** — VS Code extension packaging and publishing to Marketplace (already in devDependencies)
- **GitHub Actions** — CI/CD for native binary builds per platform (macOS x86_64/arm64, Linux x86_64/arm64)
- **POSIX shell script** — Install script following rustup/Gleam pattern (no runtime dependencies beyond curl/tar)

**Critical insight:** The existing mesh-lsp analysis pipeline already computes everything needed. The AnalysisResult contains the full Parse (rowan CST) and TypeckResult (type map, type registry with all struct/sum type definitions, trait registry). Completion, symbols, and signature help are pure query functions over this existing data -- no modifications to the parser or type checker required.

### Expected Features

**Must have (table stakes):**
- **One-line install script** (`curl | sh`) with platform detection and SHA-256 verification -- every modern language has this
- **Prebuilt binaries** for macOS (x86_64, arm64) and Linux (x86_64) -- users cannot be expected to have LLVM toolchain
- **VS Code Marketplace publishing** -- if users can't `ext install`, the language doesn't exist to most developers
- **LSP document symbols** -- powers Outline panel, breadcrumbs, and Go to Symbol (Cmd+Shift+O) which are essential for navigating files
- **LSP completion** for keywords, types, and built-in functions (minimum viable) -- most basic editor intelligence feature
- **TextMate grammar completeness** -- all 48 keywords properly highlighted, missing operators added, doc comments distinguished

**Should have (competitive):**
- **LSP signature help** -- shows function parameter info during typing, standard feature for typed languages
- **LSP formatting integration** -- wire existing mesh_fmt through textDocument/formatting handler
- **Scope-aware completion** -- completions filtered by visibility (local vars, function params, module-level names)
- **Open VSX publishing** -- Cursor, VSCodium, and other VS Code forks (10M+ users) rely on Open VSX exclusively
- **Install script version management** -- ability to install specific versions, not just latest
- **Extension status bar** -- show "Mesh LSP: Running" vs "Error" for visibility

**Defer (v2+):**
- **Dot-triggered completion** (e.g., `list.` suggests methods) -- requires type resolution at cursor position, significantly more complex
- **Semantic tokens** -- semantic highlighting on top of TextMate grammar, requires full textDocument/semanticTokens implementation
- **Workspace symbols** (Cmd+T) -- requires multi-file indexing which LSP currently doesn't support (single-file only)
- **Tree-sitter grammar** -- duplicates TextMate effort, only needed for Neovim/Helix users
- **Debug Adapter Protocol** -- enormous scope, users can use gdb/lldb on compiled binaries

### Architecture Approach

The architecture is additive, not transformative. Three independent workstreams with minimal integration points: (1) LSP enhancements add new modules (completion.rs, symbols.rs, signature.rs) that call existing analysis.rs utilities, (2) install infrastructure adds CI workflow and shell script with no code dependencies, (3) VS Code extension updates are mostly metadata (package.json) with optional status bar additions in extension.ts.

**Major components:**

1. **mesh-lsp/src/completion.rs (NEW)** — Build CompletionItem lists from three data sources already available: built-in names from builtins.rs (static list), user-defined names from TypeckResult.type_registry (all struct/sum type defs), and keywords (static list from SyntaxKind). Context detection determines what to offer: after `.` lookup receiver type for fields/methods, after `::` offer type names, bare identifier offer everything in scope.

2. **mesh-lsp/src/symbols.rs (NEW)** — Walk the CST using existing Parse to produce hierarchical DocumentSymbol responses. Direct mapping from SyntaxKind to SymbolKind: FN_DEF -> Function, STRUCT_DEF -> Struct, MODULE_DEF -> Module with nested children. Powers Outline view, breadcrumbs, and Go to Symbol with a single implementation. Uses existing offset_to_position for coordinate conversion.

3. **mesh-lsp/src/signature.rs (NEW)** — Find enclosing CALL_EXPR at cursor position, count commas for active parameter, resolve callee name through existing definition.rs patterns, look up function type scheme in TypeckResult or builtins, format parameters with types from the scheme. Triggered by `(` and `,` characters.

4. **scripts/install.sh (NEW)** — Detect OS (`uname -s`) and arch (`uname -m`), normalize to release asset naming convention, download tarball + SHA-256 from GitHub Releases, verify checksum, extract to `~/.mesh/bin/meshc` (path VS Code extension already checks), add to PATH via shell profile detection (.bashrc/.zshrc/.profile).

5. **.github/workflows/release.yml (NEW)** — Matrix build on tag push: macos-13 (x86_64), macos-latest (arm64), ubuntu-latest (x86_64), ubuntu-24.04-arm (arm64). Install LLVM 21 per platform (brew on macOS, apt on Linux), cargo build --release natively (no cross-compilation due to LLVM complexity), create tarballs + SHA-256, upload to GitHub Release.

6. **editors/vscode-mesh/package.json (MODIFIED)** — Add marketplace metadata: icon, repository, categories, badges. Version bump to 0.2.0 signals intentional first public release. No code changes needed for LSP features -- vscode-languageclient automatically enables features when server advertises capabilities.

**Pattern to follow:** Every LSP handler follows the MeshBackend pattern from server.rs: lock documents mutex, get DocumentState, call query function in separate module, drop lock before any await. The query functions are pure (no side effects, no async, no locking) and operate only on AnalysisResult. This pattern prevents deadlocks and keeps handlers simple.

**Anti-pattern to avoid:** Never hold std::sync::Mutex across await points. Tokio can schedule multiple handlers on the same thread; if task A holds the mutex and awaits, task B scheduled on the same thread will deadlock trying to acquire the same mutex. The existing hover/definition handlers do this correctly; all new handlers must follow the same pattern.

### Critical Pitfalls

1. **Install script downloads 50MB+ binary without SHA-256 verification** — The meshc binary statically links LLVM 21, producing 50-100MB files. Without checksum verification, interrupted downloads (common at this size) produce corrupted binaries that segfault with confusing errors. Users blame Mesh. Prevention: Generate .sha256 files for every release artifact, download and verify before extraction, fail clearly if mismatch. Also prevents supply chain attacks from compromised CDN/GitHub.

2. **Platform detection fails on non-standard environments** — `uname -m` returns `aarch64` on Linux but `arm64` on macOS for the same ARM64 architecture. Rosetta terminals on Apple Silicon report x86_64 even on ARM hardware. Prevention: Normalize arch names in install script (map both aarch64 and arm64 to single canonical name), use consistent asset naming in GitHub Releases (meshc-{os}-{arch}.tar.gz), detect Rosetta via sysctl on macOS to avoid emulated binaries.

3. **LSP mutex deadlock under concurrent requests** — tower-lsp executes up to 4 async handlers concurrently. If handler A locks std::sync::Mutex and then awaits (e.g., client.log_message().await), Tokio may schedule handler B on the same thread which also tries to lock -- deadlock. Non-deterministic, appears as intermittent "server hangs." Prevention: Never hold mutex guard across await. Lock, extract data to local vars, drop lock, then do async work. Consider switching to tokio::sync::RwLock for document store since most operations are reads.

4. **Forgetting to advertise new LSP capabilities** — Implementing completion() on LanguageServer trait but forgetting to set completion_provider in initialize() ServerCapabilities means the editor never sends requests. Feature silently does nothing. No compile-time check links capability declaration to handler. Prevention: Add capabilities and handlers in same commit, extend server_capabilities test to assert each new capability is present, write integration test that sends request and verifies non-empty response.

5. **VS Code extension published with secrets or wrong PAT scope** — vsce package bundles everything not in .vsixignore. Missing or incomplete .vsixignore leaks .env files, PAT tokens, API keys into published package (Wiz Research found 550+ secrets in published extensions). Also, Azure DevOps PAT must have "Marketplace (Manage)" scope with "All accessible organizations" or vsce publish fails with opaque auth errors. Prevention: Create .vsixignore that excludes everything except required files (out/, syntaxes/, package.json, README.md), run vsce ls before every publish to inspect contents, store PAT as GitHub Secret and automate publishing via CI.

## Implications for Roadmap

Based on research, the work divides into three parallel tracks with minimal dependencies. The recommended phase structure prioritizes high-impact, low-complexity features first (TextMate grammar, LSP document symbols) to deliver immediate value, then tackles the install infrastructure that unblocks wider adoption, followed by more complex LSP features.

### Phase 1: Foundation -- Grammar + Document Symbols
**Rationale:** TextMate grammar update is a single file change with triple impact (VS Code highlighting, website Shiki highlighting via shared import, and correct token scopes for all users). LSP document symbols is the simplest LSP feature (pure CST walk, no type information needed) and powers three VS Code features simultaneously (Outline, Breadcrumbs, Go to Symbol). Both can be done in parallel with zero dependencies on each other.

**Delivers:** Complete syntax highlighting for all 48 keywords plus missing operators (`.., <>, ++, =>, ?, |, &&, ||`), doc comments with distinct scope, hex/binary/scientific number literals, triple-quoted strings. Fully functional Outline panel and breadcrumb navigation for all Mesh files.

**Addresses:** Table stakes features from FEATURES.md -- syntax highlighting completeness and basic navigation features users expect.

**Avoids:** Pitfall 6 (TextMate grammar keyword/type conflicts) by updating grammar in correct pattern order and testing with comprehensive test file.

**Research flags:** SKIP -- both are well-documented patterns (TextMate grammar spec, LSP DocumentSymbol examples in tower-lsp docs).

### Phase 2: Install Infrastructure -- CI + Script
**Rationale:** Binary distribution and install script are the gateway to wider adoption. Users cannot be expected to build from source with LLVM toolchain. Must come before VS Code Marketplace publishing so the extension can reference working install instructions. CI workflow and install script must be developed together to ensure naming alignment.

**Delivers:** GitHub Actions workflow that builds meshc for 4 targets (darwin-x86_64, darwin-arm64, linux-x86_64, linux-arm64) on tag push, creates GitHub Release with tarballs + SHA-256 checksums. POSIX shell install script with platform detection, checksum verification, PATH configuration, idempotent re-runs.

**Uses:** GitHub Actions standard actions (actions/checkout, actions/upload-artifact, softprops/action-gh-release), platform-specific LLVM installation (Homebrew on macOS, apt on Linux), native builds on each runner (no cross-compilation).

**Implements:** scripts/install.sh that downloads from GitHub Releases to ~/.mesh/bin/meshc (path VS Code extension already checks), .github/workflows/release.yml with build matrix.

**Avoids:** Pitfall 1 (corrupted downloads) via SHA-256 verification, Pitfall 2 (platform detection) via arch name normalization, Pitfall 10 (LLVM missing in CI) via per-platform LLVM installation, Pitfall 8 (existing installation handling) via version detection and atomic replacement.

**Research flags:** MEDIUM -- LLVM installation in CI needs validation (apt-get install llvm-21-dev availability, Homebrew llvm@21 formula), binary size with static LLVM linkage needs measurement. Consider one task for "validate LLVM CI setup" before implementing full workflow.

### Phase 3: LSP Completion + Signature Help
**Rationale:** Code completion is the most valuable LSP feature but also the most complex. Start with keyword/type/snippet completion (static lists, no analysis work) to deliver immediate value, then add scope-aware completion (requires CST upward walk), defer dot-triggered completion to v2. Signature help reuses completion's context detection infrastructure (finding call expressions, looking up function types).

**Delivers:** Keyword completion (48 Mesh keywords), built-in type completion (Int, Float, String, List, Map, etc.), snippet completion for common patterns (fn, let, struct), scope-aware variable/function completion (all visible names at cursor position), signature help showing function parameters and types during call expression typing.

**Uses:** Existing AnalysisResult data (Parse CST, TypeckResult with type_registry and types map), existing offset conversion utilities (offset_to_position, position_to_offset), existing builtins.rs for built-in function names.

**Implements:** mesh-lsp/src/completion.rs with completion_items_at() query function, mesh-lsp/src/signature.rs with signature_at() query function, ServerCapabilities advertisement in initialize(), completion() and signature_help() handlers in server.rs.

**Avoids:** Pitfall 3 (mutex deadlock) by following existing handler pattern (lock, extract, drop, async work), Pitfall 4 (missing capability advertisement) by extending server_capabilities test, Pitfall 7 (wrong UTF-16 offsets) by always using offset_to_position, Pitfall 12 (completion trigger conflicts) by only using `.` as trigger character (not `:`).

**Research flags:** LOW for keyword/type completion (static lists), MEDIUM for scope-aware completion (requires CST upward traversal similar to definition.rs but collecting all names instead of finding one). Signature help is MEDIUM (finding call expressions and counting commas for active parameter).

### Phase 4: VS Code Marketplace Publishing
**Rationale:** Publishing to Marketplace makes the extension discoverable to all VS Code users. Must come after grammar update (so published extension has correct highlighting) and ideally after install script exists (so extension error messages can link to install instructions). Publishing to both VS Code Marketplace and Open VSX captures full user base (VS Code + forks).

**Delivers:** Extension published to both VS Code Marketplace and Open VSX with icon, repository link, screenshots, CHANGELOG.md. Version bumped to 0.2.0. CI workflow that automates publishing on release tags. Optional: status bar showing "Mesh LSP: Running/Error", auto-install prompt if meshc not found.

**Uses:** @vscode/vsce for Marketplace publishing, npx ovsx for Open VSX publishing, Azure DevOps PAT (VSCE_PAT) and OVSX token (OVSX_PAT) stored as GitHub Secrets.

**Implements:** package.json marketplace metadata updates, editors/vscode-mesh/README.md for marketplace landing page, .vsixignore for excluding dev files, GitHub Actions workflow for vsce publish + ovsx publish on tag push. Optional: extension.ts status bar additions.

**Avoids:** Pitfall 5 (secrets in .vsix or wrong PAT scope) via .vsixignore and vsce ls review, Pitfall 15 (version collision) via auto-bump in CI, Pitfall 16 (create-publisher removed) by using Marketplace web portal, Pitfall 17 (extension error without install link) by adding install URL to error message.

**Research flags:** SKIP for basic publishing (well-documented in VS Code official docs), LOW for status bar additions (standard VS Code extension API).

### Phase 5: LSP Formatting + Audit
**Rationale:** Wire existing mesh_fmt through LSP textDocument/formatting handler (trivial integration). Audit REPL and formatter for support of new language features added in phases 1.7-7.0 (for, while, trait, impl, From/Into conversion, etc.). This is a quality pass, not a feature addition -- ensures existing tools work correctly with the full current language.

**Delivers:** LSP formatting integration so VS Code "Format Document" runs meshc fmt. REPL audit covering multi-line continuation edge cases, string result formatting for complex types, error recovery, :load interaction. Formatter audit covering pipe operator multiline formatting, interface method bodies, comment preservation, long parameter wrapping. Fixes for any issues found.

**Uses:** Existing mesh_fmt::format_source() function (pure function: source in, formatted source out), existing rowan CST walker in mesh-fmt/src/walker.rs, existing rustyline REPL infrastructure, existing JIT execution via inkwell.

**Implements:** formatting() handler in server.rs that calls mesh_fmt::format_source and produces single full-document TextEdit, potential fixes in mesh-fmt/src/walker.rs for missing SyntaxKind match arms (formatter silently drops unknown nodes), potential fixes in mesh-repl for edge cases.

**Avoids:** Pitfall 13 (formatter data loss for new syntax) by adding catch-all arm in walker.rs that preserves unknown nodes verbatim, adding snapshot tests for every syntax construct.

**Research flags:** SKIP -- no new patterns needed. This is audit/bugfix work, not feature research.

### Phase 6: Documentation Corrections
**Rationale:** Must come after install script exists so docs can reference the real install command. Update getting-started guide, fix binary name (mesh -> meshc), fix compilation command (mesh hello.mpl -> meshc build .), remove fake install command from landing page or replace with real one.

**Delivers:** Accurate getting-started documentation that works when copy-pasted on clean system. All code examples tested and verified. Landing page install command is real and functional.

**Uses:** The working install script from Phase 2, the working meshc binary, the actual CLI interface.

**Implements:** Updates to website/docs/docs/getting-started/index.md and landing page, potential addition of CI job that extracts code blocks and runs them in Docker to verify they work.

**Avoids:** Pitfall 14 (docs show wrong commands) by manually walking through entire getting-started guide on clean macOS and Linux systems after install script is working.

**Research flags:** SKIP -- this is documentation QA, not technical research.

### Phase Ordering Rationale

- **Grammar + Document Symbols first** because they have zero dependencies, deliver immediate visible value, and establish patterns for later LSP work. Both can be done in parallel.
- **Install infrastructure second** because it's a hard dependency for Marketplace publishing (extension needs to reference install instructions) and unblocks wider adoption. CI workflow and install script must be coordinated on naming conventions.
- **Completion + Signature third** because they're the most complex LSP features and benefit from having document symbols working first (proves the CST traversal patterns). Completion and signature can reuse each other's utilities.
- **Marketplace publishing fourth** because it depends on grammar update (correct highlighting in published version) and install script existence (error messages can link to install docs).
- **Formatting + Audit fifth** because it's a quality pass over existing features, not critical path for new functionality. Can be done after core LSP features are working.
- **Documentation corrections last** because they depend on install script being production-ready and all features working so commands can be tested end-to-end.

**Dependency chain:**
```
Grammar (independent) ─────┐
                           ├─> Marketplace Publishing ──> Docs Corrections
Document Symbols ──────────┘

Install Script <── CI Workflow ──> Marketplace Publishing ──> Docs Corrections

Completion ──┐
             ├─> (can be done in parallel with Install track)
Signature ───┘

Formatting + Audit (independent, can be parallel with any phase)
```

### Research Flags

**Phases needing deeper research during planning:**
- **Phase 2 (Install Infrastructure):** LLVM installation in CI (apt package availability for llvm-21-dev on Ubuntu 22.04/24.04, Homebrew formula llvm@21 on macOS 13/14, static vs dynamic linking strategy). Binary size measurement with static LLVM. Asset naming coordination between CI and install script.
- **Phase 3 (LSP Completion):** Scope-aware name collection requires CST upward walk similar to definition.rs but collecting all visible names -- may need to prototype the traversal logic before planning the full implementation.

**Phases with standard patterns (skip research-phase):**
- **Phase 1 (Grammar + Document Symbols):** TextMate grammar is data-only, well-documented. DocumentSymbol LSP pattern is straightforward CST walk with examples in tower-lsp docs.
- **Phase 4 (Marketplace Publishing):** VS Code extension publishing is extremely well-documented in official docs. Standard vsce workflow.
- **Phase 5 (Formatting + Audit):** No new patterns, audit scope only. Existing mesh_fmt and mesh-repl codebases are the specification.
- **Phase 6 (Docs Corrections):** Documentation QA, no technical research needed.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All recommended technologies are already in use and verified working (tower-lsp, vscode-languageclient, vsce, rowan CST). No new dependencies. Direct source code analysis confirms all data structures exist. |
| Features | HIGH | Table stakes features verified via LSP 3.17 spec, VS Code API docs, and analysis of other language tooling (Rust, Gleam, Deno). Feature priorities based on what existing Mesh LSP already supports (hover, definition work correctly). |
| Architecture | HIGH | All architectural recommendations based on direct reading of current codebase (mesh-lsp/src/server.rs, analysis.rs, definition.rs, mesh-typeck TypeckResult). Integration points verified (VS Code extension already checks ~/.mesh/bin/meshc, website already imports mesh.tmLanguage.json). |
| Pitfalls | HIGH | Critical pitfalls sourced from official docs (LSP spec UTF-16 requirement, tower-lsp async handler concurrency, VS Code Marketplace PAT setup, Wiz Research extension secrets finding). Platform detection issues verified from Rust/Gleam install script examples. |

**Overall confidence:** HIGH

The research is grounded in direct codebase analysis (all mesh-lsp files read, all relevant structs examined) combined with official documentation for tower-lsp, LSP specification, and VS Code extension API. The recommendation to use existing data structures rather than adding new dependencies significantly reduces risk -- the analysis pipeline already works correctly for hover and go-to-definition, proving the foundation is solid.

### Gaps to Address

**LLVM static linking strategy** — The recommendation is to build binaries natively on each platform, but the exact LLVM linkage strategy (static vs dynamic, which LLVM shared libraries to bundle if dynamic) needs validation during Phase 2 planning. The inkwell llvm21-1 feature implies static linking, but actual binary size and portability need measurement. Mitigation: Add task in Phase 2 to build test binary on each CI runner and verify it runs on clean system without LLVM installed.

**Scope-aware completion complexity** — The recommendation includes scope-aware completion (all visible names at cursor position) in Phase 3, but the CST upward walk to collect all let bindings, function params, and top-level definitions may be more complex than anticipated. The existing definition.rs shows the pattern for finding one definition, but collecting all visible names is broader. Mitigation: During Phase 3 planning, prototype the scope collection function before committing to scope-aware completion in the same phase. May split into "basic completion" (keywords/types/snippets) and "scope-aware completion" (separate phase).

**Non-ASCII completion offset handling** — The research identifies Pitfall 7 (wrong UTF-16 offsets for non-ASCII) as critical, and the existing offset_to_position function in analysis.rs correctly handles UTF-16. But completion items also need TextEdit ranges which must use the same UTF-16 conversion. Mitigation: During Phase 3 implementation, write tests with multi-byte characters (emoji, CJK) first to verify TextEdit ranges are correct before considering completion feature complete.

**CI LLVM installation time** — GitHub Actions runners do not have LLVM 21 pre-installed. Installing via apt or Homebrew adds time to every CI build. If LLVM installation takes 10+ minutes per runner, the matrix build (4 platforms) becomes 40+ minutes. Mitigation: During Phase 2 planning, measure LLVM install time on each runner type (ubuntu-latest, ubuntu-arm, macos-13, macos-latest) and add caching strategy if needed. Pre-built LLVM packages should be fast, but this needs verification.

## Sources

### Primary (HIGH confidence)

**Direct codebase analysis:**
- crates/mesh-lsp/src/server.rs — MeshBackend, DocumentState, LanguageServer trait impl, ServerCapabilities advertisement
- crates/mesh-lsp/src/analysis.rs — AnalysisResult struct, analyze_document(), offset_to_position (UTF-16), position_to_offset
- crates/mesh-lsp/src/definition.rs — find_definition(), source_to_tree_offset, tree_to_source_offset, CST traversal patterns
- crates/mesh-typeck/src/lib.rs — TypeckResult (types map, type_registry, trait_registry, qualified_modules)
- crates/mesh-typeck/src/ty.rs — Ty enum, Display impl for type formatting
- crates/mesh-typeck/src/builtins.rs — Built-in function registrations (println, print, default, compare)
- crates/mesh-parser/src/syntax_kind.rs — All SyntaxKind variants (FN_DEF, STRUCT_DEF, CALL_EXPR, etc.)
- editors/vscode-mesh/package.json — Extension config, publisher, dependencies (vscode-languageclient ^9.0.1, @vscode/vsce ^2.22.0)
- editors/vscode-mesh/src/extension.ts — findMeshc() checking ~/.mesh/bin/meshc, LanguageClient setup
- editors/vscode-mesh/syntaxes/mesh.tmLanguage.json — TextMate grammar with keyword patterns
- .cargo/config.toml — LLVM_SYS_211_PREFIX hardcoded to /opt/homebrew/opt/llvm
- Cargo.toml — workspace members, tower-lsp 0.20, inkwell with llvm21-1 feature

**Official documentation:**
- tower-lsp 0.20 docs (docs.rs/tower-lsp/0.20.0) — LanguageServer trait methods, ServerCapabilities
- LSP 3.17 Specification (microsoft.github.io) — UTF-16 position requirements, DocumentSymbol range/selection_range, CompletionItem, SignatureHelp
- VS Code Language Extensions (code.visualstudio.com/api/language-extensions) — Syntax highlighting, semantic highlighting, LSP client integration
- VS Code Publishing Extensions (code.visualstudio.com/api/working-with-extensions/publishing-extension) — vsce workflow, PAT setup, .vsixignore, Marketplace requirements
- TextMate Language Grammars (macromates.com/manual/en/language_grammars) — Pattern priority, first-match-wins semantics
- lsp-types crate docs — DocumentSymbol, CompletionItem, SignatureHelp struct definitions

### Secondary (MEDIUM confidence)

**Community consensus, multiple sources agree:**
- Gleam Installation Guide (gleam.run) — Install script pattern, binary distribution for Rust-built language toolchain
- Rust rustup-init installer architecture (deepwiki.com/rust-lang/rustup) — Platform detection, PATH configuration, shell profile handling
- cargo-dist documentation (axodotdev.github.io/cargo-dist) — Binary distribution patterns, LLVM dependency limitations
- Tokio shared state guide (tokio.rs/tokio/tutorial/shared-state) — Mutex in async code, deadlock prevention
- tower-lsp GitHub issues #284 — Concurrent handler execution, std::sync::Mutex blocking in async context
- Wiz Research: Supply chain risk in VS Code extensions (wiz.io/blog) — 550+ secrets leaked in published extensions, .vsixignore importance
- Chef: 5 ways to deal with curl|bash (chef.io/blog) — Install script partial download risk, SHA-256 verification best practices
- Cross-compiling Rust on GitHub Actions (blog.timhutt.co.uk) — Platform matrix patterns, LLVM cross-compile challenges
- Open VSX Registry (open-vsx.org) — Alternative marketplace for VS Code forks (Cursor, VSCodium, etc.)

### Tertiary (LOW confidence)

None — all research grounded in primary sources (direct codebase reading and official documentation).

---
*Research completed: 2026-02-13*
*Ready for roadmap: yes*
