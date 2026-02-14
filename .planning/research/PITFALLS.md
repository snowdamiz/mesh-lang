# Domain Pitfalls

**Domain:** Developer tooling additions to existing Mesh programming language -- install script, VS Code extension updates, LSP features, REPL/formatter audit, docs corrections
**Researched:** 2026-02-13
**Confidence:** HIGH (codebase-informed, ecosystem research verified)

## Critical Pitfalls

Mistakes that cause broken user experience, security incidents, or require significant rework.

---

### Pitfall 1: Install Script Downloads 50MB+ Binary Without Verification

**What goes wrong:** The `meshc` binary statically links LLVM 21 (via `inkwell` with `llvm21-1` feature), producing a binary in the 50-100MB range. A `curl | sh` install script that downloads this binary without SHA256 checksum verification leaves users vulnerable to supply chain attacks, and large downloads are more likely to be interrupted mid-stream -- a partially downloaded binary that is `chmod +x`'d and placed in `$PATH` will produce confusing "exec format error" or segfault messages rather than a clear error.

**Why it happens:** Most install script tutorials show a simple `curl -L | tar xz && mv binary /usr/local/bin/` pattern without verification. When the binary is small (<5MB), the risk of interrupted downloads is low. At 50MB+, network interruptions become common, especially on CI runners, slower connections, and corporate proxies that time out large downloads.

**Consequences:**
- Users who `curl | sh` on a flaky connection get a corrupted binary that segfaults, and blame Mesh
- No checksum verification means a compromised CDN/GitHub release could serve a malicious binary
- The install script cannot distinguish between "download interrupted" and "successful download" without verification
- Corporate environments and CI systems often require checksum verification for security policy compliance

**Prevention:**
1. Generate SHA256 checksums for every release artifact and publish them alongside the binaries in GitHub Releases (e.g., `meshc-darwin-arm64.tar.gz.sha256`).
2. The install script must: (a) download the tarball to a temp file, (b) download the checksum file, (c) verify the checksum BEFORE extracting or installing, (d) fail with a clear error message if verification fails.
3. Use `curl -fSL` (fail on HTTP errors, show errors, follow redirects) rather than just `curl -sSf` to ensure HTTP errors are caught.
4. Always `set -euo pipefail` at the top of the install script so any failure aborts immediately rather than continuing with partial state.

**Detection:** Test the install script with artificially truncated downloads (truncate the tarball at 50%, run the script). It must fail clearly, not install a broken binary.

**Phase mapping:** Install script phase. Must be implemented correctly from the first version -- a broken install experience will be users' first interaction with Mesh.

---

### Pitfall 2: Platform Detection Fails on Non-Standard Environments

**What goes wrong:** The install script uses `uname -s` and `uname -m` to detect OS and architecture, then maps these to GitHub Release asset names. But the mapping has well-known failure modes: `uname -m` returns `aarch64` on Linux but `arm64` on macOS for the same ARM64 architecture. WSL2 reports `Linux` but users may want the Windows binary. Some CI containers report unexpected values. If the script does not handle these variations, it downloads the wrong binary or fails with an opaque "asset not found" error.

**Why it happens:** There is no universal standard for how `uname` reports architecture. Common variations:
- macOS: `arm64` or `x86_64`
- Linux: `aarch64` or `x86_64`
- Some Linux ARM: `armv7l` (32-bit ARM, which Mesh probably does not support)
- Rosetta on macOS: `uname -m` may report `x86_64` even on ARM hardware if the shell is running under Rosetta

**Consequences:**
- Users on ARM Linux get a confusing "no matching release asset" error
- Users running a Rosetta terminal on Apple Silicon download the x86_64 binary, which works but runs under emulation (significantly slower, especially for a compiler)
- The install script silently downloads the wrong binary for the platform

**Prevention:**
1. Normalize architecture names in the script: map both `aarch64` and `arm64` to a single canonical name that matches the GitHub Release asset naming convention.
2. On macOS, additionally check `sysctl -n machdep.cpu.brand_string` or `arch` to detect Apple Silicon even when running under Rosetta.
3. Explicitly list supported platforms and fail with a clear message for unsupported ones: "Mesh does not provide prebuilt binaries for armv7l. Build from source instead."
4. Use a consistent asset naming convention: `meshc-{os}-{arch}.tar.gz` where `os` is `linux`/`darwin` and `arch` is `x86_64`/`arm64` (not `aarch64`).

**Detection:** Test the install script in: native macOS ARM64, macOS x86_64, Rosetta terminal, Linux x86_64, Linux aarch64 (GitHub's arm64 runner), and WSL2.

**Phase mapping:** Install script phase. The detection logic must be correct before the first release or every user hitting the wrong case will file a bug.

---

### Pitfall 3: LSP Mutex Deadlock Under Concurrent Requests

**What goes wrong:** The existing `MeshBackend` in `server.rs` uses `std::sync::Mutex<HashMap<String, DocumentState>>` to protect the document store. tower-lsp executes up to 4 async handlers concurrently. If handler A locks the mutex and then calls `self.client.log_message().await` (or any other `.await`), the Tokio runtime may schedule handler B on the same thread, which also tries to lock the mutex -- causing a deadlock. The existing code already has this pattern: `analyze_and_publish()` locks the mutex, does work, drops the lock, then awaits `publish_diagnostics`. This is currently safe because the lock is dropped before the `.await`, but adding new LSP features (completion, document symbols, signature help) increases the risk of accidentally holding the lock across an `.await`.

**Why it happens:** `tower-lsp` unconditionally executes pending async tasks concurrently (up to 4 at a time) without guaranteeing execution order. `std::sync::Mutex` blocks the OS thread when contended. If task A holds the mutex and is suspended at an `.await` point, and task B is scheduled on the same OS thread and tries to acquire the same mutex, the thread deadlocks because task A cannot be resumed to release the lock.

**Consequences:**
- The LSP server hangs completely -- the editor shows "Mesh Language Server" as not responding
- The user must restart VS Code or kill the process
- The deadlock is non-deterministic and hard to reproduce, making it appear as intermittent "server hangs"
- Adding new features (completion, document symbols) that read from the document store increases contention and makes deadlocks more likely

**Prevention:**
1. NEVER hold `std::sync::Mutex` guard across an `.await` point. The current code does this correctly, but it must be enforced as a pattern rule for all new handlers.
2. For new LSP features, follow the existing pattern in `hover()` and `goto_definition()`: lock the mutex, clone/extract the needed data, drop the lock, then do async work.
3. Consider switching to `tokio::sync::RwLock` for the document store. Completion, document symbols, and hover are all read-only operations. Using an `RwLock` allows multiple readers concurrently while only blocking on writes (did_open, did_change). This dramatically reduces contention.
4. Keep the lock scope as small as possible: lock, extract what you need into local variables, unlock immediately. Never call any method on `self` while holding the lock.

**Detection:** Run the LSP server with rapid successive requests (type a character, immediately hover, immediately request completions). If the server hangs, there is a deadlock. Clippy lint `clippy::await_holding_lock` catches some cases statically.

**Phase mapping:** LSP features phase. Must be reviewed for EVERY new handler implementation.

---

### Pitfall 4: Forgetting to Advertise New LSP Capabilities

**What goes wrong:** In tower-lsp, the `initialize` method returns `ServerCapabilities` that tell the editor which features the server supports. If you implement `completion()`, `document_symbol()`, or `signature_help()` on the `LanguageServer` trait but forget to set the corresponding capability in `initialize`, the editor will NEVER send those requests to the server. The feature silently does nothing, and the developer wastes hours debugging why their handler is never called.

**Why it happens:** The capability declaration and the handler implementation are in different parts of the code (capabilities in `initialize()`, handler as a separate `async fn`). There is no compile-time check that links them -- tower-lsp provides default no-op implementations for all handlers, so missing a capability produces no error.

**Consequences:**
- Completion, document symbols, or signature help silently do not work
- The developer thinks the handler has a bug, but the handler is never invoked
- Hours wasted on debugging the handler logic when the problem is a missing one-liner in `initialize()`

**Prevention:**
1. Add capabilities and handler implementations in the same commit, reviewed together.
2. For each new feature, add a test that calls `server.initialize()` and asserts the relevant capability is present. The existing `server_capabilities` test in `server.rs:213-224` checks for `hover_provider` and `text_document_sync` -- extend it for each new capability:
   - `completion_provider: Some(CompletionOptions { trigger_characters: Some(vec![".".into(), ":".into()]), .. })`
   - `document_symbol_provider: Some(OneOf::Left(true))`
   - `signature_help_provider: Some(SignatureHelpOptions { trigger_characters: Some(vec!["(".into(), ",".into()]), .. })`
3. Write an integration test that sends a `textDocument/completion` request and verifies a non-empty response for a known-good file. If capabilities are missing, this test fails immediately.

**Detection:** The capability test catches this at CI time. Alternatively, enable LSP tracing in VS Code (`"mesh.trace.server": "verbose"`) and verify the initialize response includes the expected capabilities.

**Phase mapping:** LSP features phase. Check for EVERY new LSP method added.

---

### Pitfall 5: VS Code Extension Published with Secrets or Wrong Scope

**What goes wrong:** Publishing a VS Code extension to the Marketplace requires an Azure DevOps Personal Access Token (PAT). Two critical mistakes are common: (1) the PAT is created with the wrong scope (must be "Marketplace (Manage)" with "All accessible organizations"), causing 401/403 errors during `vsce publish`; (2) the `.vsix` package accidentally includes `.env` files, PAT tokens, or API keys from the development environment, which are then publicly downloadable by anyone who installs the extension.

**Why it happens:** The `vsce package` command bundles everything in the extension directory that is not excluded by `.vsixignore`. If `.vsixignore` is missing or incomplete, sensitive files from the development process leak into the published package. Microsoft's Wiz Research found over 550 validated secrets across more than 500 published VS Code extensions -- this is a widespread problem, not a theoretical one.

**Consequences:**
- Leaked PAT tokens can be used to publish malicious updates to your extension, affecting all users
- Microsoft now blocks extensions with detected secrets, causing publish failures and delays
- Wrong PAT scope causes `vsce publish` to fail with opaque authentication errors that are difficult to debug

**Prevention:**
1. Create a `.vsixignore` file that excludes everything except the required files:
   ```
   **
   !out/**
   !syntaxes/**
   !language-configuration.json
   !package.json
   !LICENSE
   !README.md
   !CHANGELOG.md
   ```
2. Before publishing, inspect the `.vsix` contents: `vsce ls` shows what will be included. Review for any files that should not be there.
3. Store the PAT as a GitHub Actions secret (`VSCE_PAT`), never in the repo. Automate publishing via CI so the PAT is never on a developer's machine.
4. When creating the Azure DevOps PAT: select "All accessible organizations" in the Organization dropdown, and set scope to "Marketplace (Manage)". These are the two most common mistakes.
5. The existing `package.json` already has `"package": "vsce package --no-dependencies"` -- this is good, as it avoids bundling `node_modules`. Verify that `--no-dependencies` continues to be used.

**Detection:** Run `vsce ls` before every publish and review the file list. Add a CI step that runs `vsce package` and then inspects the `.vsix` for known secret patterns (`.env`, `token`, `key`, `secret` in filenames).

**Phase mapping:** VS Code extension phase. Must be correct for the first marketplace publish -- you cannot un-publish secrets.

---

### Pitfall 6: TextMate Grammar Conflicts Between Keywords and Type Names

**What goes wrong:** The existing `mesh.tmLanguage.json` has a broad `entity.name.type.mesh` pattern that matches `\b[A-Z][a-zA-Z0-9_]*\b` (any PascalCase identifier). This pattern also matches keywords that happen to be PascalCase, like `Some`, `None`, `Ok`, `Err`, `Int`, `Float`, `String`, `Bool`, etc. The grammar has a separate `support.function.mesh` for `Some|None|Ok|Err` and `support.type.mesh` for `Int|Float|String|...`, but TextMate grammar rule priority depends on declaration ORDER within the patterns array, not specificity. If the `types` repository (which contains the broad PascalCase pattern) is included BEFORE `keywords`, user-defined types like `Some` or `Result` could be incorrectly highlighted.

**Why it happens:** TextMate grammars use first-match-wins within a pattern list. The current top-level `patterns` array includes `#keywords` before `#types`, which is correct -- keywords match first. But when adding new language features (new keywords, new built-in types), developers often add them to the wrong section or forget to add them to the keyword exclusion list, causing them to be matched by the catch-all type pattern instead.

**Consequences:**
- New keywords added to Mesh but not to the grammar show up as type names (wrong color)
- User-defined types that happen to share a name with a built-in get inconsistent highlighting
- String interpolation expressions lose their highlighting if the interpolation grammar includes are in the wrong order

**Prevention:**
1. When adding new Mesh keywords or built-in types, ALWAYS update the TextMate grammar's keyword patterns first.
2. Keep the pattern include order as: `#comments`, `#strings`, `#numbers`, `#keywords`, `#operators`, `#functions`, `#types`, `#variables`. This ensures specific patterns win over general ones.
3. Test grammar changes with a comprehensive test file that includes every keyword, every built-in type, user-defined types, and edge cases (type names that are substrings of keywords, keywords inside string interpolation).
4. Use the "TextMate Syntax Highlighting and Intellisense" extension (`RedCMD.tmlanguage-syntax-highlighter`) during development to get live feedback on grammar changes.

**Detection:** Create a `test.mpl` file with all keywords and types, open in VS Code, and visually inspect. Use the "Developer: Inspect Editor Tokens and Scopes" command to verify each token's scope.

**Phase mapping:** VS Code extension phase. Must be verified whenever the grammar is updated for new language features.

---

### Pitfall 7: Completion Items with Wrong UTF-16 Position Offsets

**What goes wrong:** The LSP specification requires positions in UTF-16 code units, not byte offsets or Unicode scalar values. The existing `analysis.rs` correctly handles UTF-16 conversion for diagnostics, hover, and go-to-definition. But completion items require `TextEdit` ranges that specify where the completion text should be inserted, and these ranges must also use UTF-16 character offsets. If completion ranges use byte offsets instead of UTF-16 offsets, completion inserts text at the wrong position for any line containing non-ASCII characters (emoji, CJK characters, multi-byte UTF-8 sequences).

**Why it happens:** Internally, Mesh source code is stored as a Rust `String` (UTF-8 bytes). Rowan's `TextRange` uses byte offsets. The temptation when building completion items is to pass the byte offset directly as the LSP `Position.character`, which works for ASCII-only code but breaks for multi-byte characters. The existing `offset_to_position` function in `analysis.rs:73-88` correctly counts UTF-16 code units, but a developer adding completion support might bypass it and construct `Position` directly from byte offsets.

**Consequences:**
- Completion inserts text at the wrong column for non-ASCII source files
- The misalignment can corrupt source code (inserting in the middle of a multi-byte character)
- The bug only manifests with non-ASCII content, so it passes most English-only tests

**Prevention:**
1. ALWAYS use the existing `offset_to_position` and `position_to_offset` functions from `analysis.rs` when constructing LSP positions. Never construct `Position { line, character }` manually from byte counts.
2. Prefer using `textEdit` over `insertText` in completion items. The LSP spec says `insertText` is "subject to interpretation by the client side" and some clients modify it unexpectedly. `textEdit` with explicit range is unambiguous.
3. Add tests with non-ASCII source code (e.g., variable names with Unicode, comments with emoji) that verify completion inserts at the correct position.

**Detection:** Write a test with source like `let emoji_var = "test" # comment` where there are multi-byte characters before the completion position. Verify the `TextEdit.range` uses UTF-16 offsets.

**Phase mapping:** LSP completion phase. Must be verified with non-ASCII test cases.

---

## Moderate Pitfalls

### Pitfall 8: Install Script Does Not Handle Existing Installation

**What goes wrong:** The install script downloads and installs `meshc` to `$HOME/.mesh/bin/meshc` (matching the well-known path in the VS Code extension's `findMeshc()`). If the user already has a previous version installed, the script must either upgrade cleanly or warn about the existing installation. A naive script that silently overwrites the binary can break if the old process is still running (common with LSP servers that are kept alive by the editor).

**Prevention:**
1. Check for an existing installation and display the current version before overwriting.
2. On upgrade, write the new binary to a temp file first, then atomically rename it to the final path. This avoids a window where the binary is partially written.
3. If the old binary is currently running (check with `pgrep meshc` or `lsof`), warn the user to restart their editor after the upgrade.
4. The VS Code extension checks `$HOME/.mesh/bin/meshc` as a well-known path (line 42 of `extension.ts`). The install script MUST use this exact path or the extension will not find the binary.

**Phase mapping:** Install script phase.

---

### Pitfall 9: Install Script Does Not Update PATH

**What goes wrong:** The script installs the binary to `$HOME/.mesh/bin/` but the user's `$PATH` does not include this directory. After installation, `meshc --version` fails with "command not found". The user thinks installation failed.

**Prevention:**
1. After installing the binary, check if `$HOME/.mesh/bin` is already in `$PATH`. If not, append the appropriate line to `~/.bashrc`, `~/.zshrc`, or `~/.profile` (detect the user's shell first).
2. Print a clear message: "Add `export PATH=$HOME/.mesh/bin:$PATH` to your shell profile, then restart your terminal."
3. Also create a symlink in `/usr/local/bin/meshc` if the user has write access, as a convenience fallback.
4. Test with a fresh user profile that has no prior Mesh configuration.

**Phase mapping:** Install script phase.

---

### Pitfall 10: GitHub Actions CI Does Not Have LLVM for Cross-Platform Builds

**What goes wrong:** Building `meshc` requires LLVM 21 (`inkwell` with `llvm21-1` feature). The current `.cargo/config.toml` hardcodes `LLVM_SYS_211_PREFIX = "/opt/homebrew/opt/llvm"` which is specific to Apple Silicon macOS with Homebrew. A GitHub Actions workflow building for Linux x86_64, Linux arm64, macOS x86_64, and macOS arm64 needs LLVM 21 installed on each runner, and the LLVM path differs per platform.

**Why it happens:** LLVM is not pre-installed on GitHub Actions runners. Installing LLVM 21 from source takes 30-60 minutes per build. Pre-built LLVM packages exist but are platform-specific and may not match the exact version/configuration required by `inkwell`.

**Consequences:**
- CI builds fail because LLVM is not found
- Build times become 30+ minutes if LLVM is compiled from source on each run
- The `.cargo/config.toml` `LLVM_SYS_211_PREFIX` path breaks on non-macOS runners
- Cross-compilation for ARM64 Linux from x86_64 runners requires cross-compiled LLVM libraries, which is complex

**Prevention:**
1. Do NOT hardcode `LLVM_SYS_211_PREFIX` in `.cargo/config.toml` for CI builds. Instead, set it as an environment variable per CI job, varying by runner OS and architecture.
2. Use pre-built LLVM packages for CI: `apt-get install llvm-21-dev` on Ubuntu, `brew install llvm@21` on macOS.
3. Cache the LLVM installation between CI runs to avoid repeated downloads.
4. Build native binaries on native runners (macOS arm64 on `macos-14`, macOS x86_64 on `macos-13`, Linux x86_64 on `ubuntu-latest`, Linux arm64 on `ubuntu-latest-arm64`) rather than cross-compiling. Cross-compilation with LLVM is fragile.
5. Consider `cargo-dist` which automates multi-platform Rust binary releases with install script generation, handling all the LLVM complexity.

**Detection:** The CI workflow must successfully build on all target platforms. Test with a clean runner (no cache) to verify the LLVM installation step works independently.

**Phase mapping:** Install script / release infrastructure phase. Blocks the entire install script feature since there are no binaries to download without CI builds.

---

### Pitfall 11: Document Symbols Return Wrong `selection_range`

**What goes wrong:** The LSP `DocumentSymbol` struct requires two ranges: `range` (the full extent of the symbol, including its body) and `selection_range` (the identifier itself, e.g., the function name). The spec requires that `selection_range` is CONTAINED within `range`. If the whitespace-skipping coordinate system issue (documented in `definition.rs` comments) causes `selection_range` to extend beyond `range`, editors may crash or refuse to display the symbol outline.

**Why it happens:** The Mesh lexer skips whitespace, so rowan CST offsets differ from source byte offsets. The existing `tree_to_source_offset` and `source_to_tree_offset` functions handle this translation, but they add complexity. If `range` and `selection_range` are computed using different coordinate systems (one in tree offsets, one in source offsets), the containment invariant may be violated.

**Prevention:**
1. Always compute BOTH `range` and `selection_range` in the same coordinate system. Use source byte offsets for both, then convert both through `offset_to_position()`.
2. After computing both ranges, add a debug assertion that `selection_range.start >= range.start && selection_range.end <= range.end`.
3. For functions: `range` = from `fn` keyword to `end` keyword; `selection_range` = just the function name. For structs: `range` = from `struct` keyword to `end`; `selection_range` = just the struct name.
4. Reuse the tree-to-source offset mapping from `definition.rs` rather than reimplementing it.

**Phase mapping:** LSP document symbols phase.

---

### Pitfall 12: Completion Trigger Characters Conflict with Mesh Syntax

**What goes wrong:** LSP completion trigger characters tell the editor "send a completion request when the user types this character." Common trigger characters are `.` (member access) and `:` (type annotations). In Mesh, `::` is the type annotation operator and `|>` is the pipe operator. If `.` is a trigger character, typing `IO.println` triggers completion after the `.`, which is correct. But if `:` is a trigger character, typing `x :: Int` triggers completion after the first `:`, which is likely wrong (the user is typing a type annotation, not requesting completions).

**Why it happens:** Trigger characters are single characters, not sequences. There is no way to say "trigger on `::` but not on `:`". Some editors handle this by checking context (are we in a type annotation position?), but many just fire the completion request on every `:`.

**Consequences:**
- Unnecessary completion popups appear while typing type annotations (`x :: Int`)
- The completion popup interrupts the user's flow
- If the completion handler is slow (re-parses the document), it causes perceptible lag on every `:` keystroke

**Prevention:**
1. Start with ONLY `.` as a trigger character. This handles module member access (`IO.println`, `List.map`) which is the most useful completion context.
2. Do NOT add `:` as a trigger character for v1. Users can still trigger completion manually with Ctrl+Space for type annotations.
3. If `:` is needed later, the completion handler must check context: if the character before the cursor is also `:` (forming `::`), filter completion results to only return type names.
4. For pipe operator completions (`x |> ___`), do not use `|` or `>` as trigger characters. Instead, rely on the editor's built-in identifier completion after the user starts typing a function name.

**Detection:** Type `x :: Int` in the editor and verify no completion popup appears. Type `IO.` and verify completions appear.

**Phase mapping:** LSP completion phase.

---

### Pitfall 13: REPL and Formatter Audit Misses New Language Features

**What goes wrong:** When new language features are added to the parser and type checker (e.g., `from X import Y`, `impl From<A> for B`, iterators), the REPL and formatter may not support them. The REPL uses the full compiler pipeline (`mesh-parser` -> `mesh-typeck` -> `mesh-codegen` with JIT), so parser changes are picked up automatically, but the REPL's session state management may not handle new top-level forms. The formatter (`mesh-fmt`) uses its own CST walker (`walker.rs`) that explicitly matches on `SyntaxKind` variants -- new variants are silently ignored, producing unformatted output.

**Why it happens:** The formatter's `walker.rs` has explicit match arms for each `SyntaxKind`. When a new syntax node is added to the parser (e.g., `SyntaxKind::ASSOC_TYPE_DEF`), if no match arm is added to the formatter, the node's children are not visited and their formatting is lost (output is empty for that node).

**Consequences:**
- `meshc fmt` silently drops new syntax constructs from the output, corrupting source files
- The REPL may error on valid new syntax that works in compiled mode
- Users discover the bug only after running `meshc fmt` on their files and losing code

**Prevention:**
1. The formatter should have a catch-all arm for unknown `SyntaxKind` variants that preserves the original text verbatim rather than dropping it. This is the "do no harm" principle: unknown syntax should pass through unchanged.
2. When adding new syntax to the parser, immediately check: does the formatter handle it? Does the REPL handle it? Add this to the review checklist for every parser change.
3. Add snapshot tests (`insta` is already a dependency) for the formatter that cover every syntax construct, including new ones.
4. For the REPL, test each new syntax construct interactively and verify it works in the JIT pipeline.

**Detection:** Format a file containing every new syntax feature and diff the output against the input. Any disappearing lines indicate a formatter bug.

**Phase mapping:** REPL/formatter audit phase. Must be done AFTER all language features for the current milestone are implemented.

---

### Pitfall 14: Documentation Shows Wrong Install Commands

**What goes wrong:** The current getting-started docs show `./target/release/mesh --version` (the binary is actually `meshc`, not `mesh`) and `mesh hello.mpl` (the actual command is `meshc build <dir>`, not `meshc <file>`). The landing page reportedly shows a fake `curl | sh` command that does not work. If the install script is implemented but the docs are not updated to match, users copy-paste commands that fail.

**Why it happens:** Documentation is written before or independently from the actual implementation. The getting-started guide was written when the CLI interface was different (or aspirational). The docs reference `mesh` as the binary name but the actual binary is `meshc`. The docs show `mesh hello.mpl` (single-file compilation) but the CLI expects `meshc build <dir>` (directory-based project compilation).

**Consequences:**
- New users' very first experience is copy-pasting a command that fails
- Users lose trust in the documentation and in the project
- Support burden increases with "basic install doesn't work" issues

**Prevention:**
1. After the install script is working, manually walk through the entire getting-started guide on a clean system, executing every command.
2. Update the getting-started guide to show the actual install command (`curl -fsSL https://mesh-lang.dev/install.sh | sh`), the actual binary name (`meshc`), and the actual compilation command (`meshc build .`).
3. Add a CI job that extracts code blocks from the docs and runs them in a Docker container to verify they work (doc-testing).
4. The landing page's fake install command must be replaced with the real one, or removed entirely until the install script exists.

**Detection:** Fresh install on a clean macOS and Linux VM, following only the docs. Every command must succeed.

**Phase mapping:** Docs correction phase. Must be done AFTER the install script is working to ensure docs match reality.

---

## Minor Pitfalls

### Pitfall 15: VS Code Extension Version Not Bumped Before Publish

**What goes wrong:** The VS Code Marketplace rejects extension uploads with the same version number as an already-published version. The current `package.json` has `"version": "0.1.0"`. If a developer publishes 0.1.0, makes changes, and forgets to bump the version before publishing again, the publish fails.

**Prevention:**
1. Automate version bumping in the publish CI workflow: `vsce publish patch` automatically increments the patch version.
2. Alternatively, tie the extension version to the git tag: extract version from the release tag and inject it into `package.json` before packaging.
3. Add a CI check that the version in `package.json` is different from the currently published version.

**Phase mapping:** VS Code extension publish phase.

---

### Pitfall 16: `vsce create-publisher` Is No Longer Available

**What goes wrong:** The `vsce create-publisher` command was removed from `vsce`. Developers following old tutorials will get an error. Publisher accounts must now be created at the Visual Studio Marketplace publisher management page (https://marketplace.visualstudio.com/manage).

**Prevention:** Document the correct publisher creation process. The publisher name in `package.json` (`"publisher": "mesh-lang"`) must exactly match the publisher created on the Marketplace portal.

**Phase mapping:** VS Code extension publish phase.

---

### Pitfall 17: Extension Does Not Bundle LSP Binary

**What goes wrong:** The VS Code extension expects `meshc` to be in the user's PATH or at a well-known location (`$HOME/.mesh/bin/meshc`). If neither exists, the extension shows an error dialog. But the error message says "Install Mesh or configure the path to meshc" -- it does not tell the user HOW to install Mesh. If the install script does not exist yet when the extension is published, or the extension is published before the install script is well-known, users will be stuck.

**Prevention:**
1. The extension's error message should include a direct link to the installation instructions: "Install Mesh: https://mesh-lang.dev/docs/getting-started"
2. Consider adding an automatic download option: the extension could offer to download the `meshc` binary directly, similar to how the Rust analyzer extension can download `rust-analyzer`.
3. Ensure the install script is published and documented BEFORE the VS Code extension is published to the Marketplace.

**Phase mapping:** VS Code extension + install script. The install script should be completed first.

---

### Pitfall 18: Signature Help Fires on Every `(` Including Non-Function Calls

**What goes wrong:** If `(` is registered as a signature help trigger character, the editor sends a signature help request every time `(` is typed. In Mesh, `(` is used for function calls (`add(1, 2)`), grouped expressions (`(a + b) * c`), and tuple construction. Only function calls should show signature help. If the handler does not distinguish these contexts, it returns wrong or empty signature information for non-call contexts, causing visual noise.

**Prevention:**
1. In the signature help handler, check if the `(` follows an identifier that resolves to a function. If not, return `None`.
2. Use the existing analysis infrastructure (CST traversal, name resolution) to determine if the cursor is inside a `CALL_EXPR` node before computing signatures.
3. Return `None` (no signature help) rather than an empty `SignatureHelp` for non-call contexts. Some editors handle empty vs. None differently.

**Phase mapping:** LSP signature help phase.

---

### Pitfall 19: GitHub Release Assets Not Available for All Platforms at Launch

**What goes wrong:** The install script is published but GitHub Release assets only exist for macOS arm64 (the developer's machine). Linux users and macOS x86_64 users get "no matching asset found" errors. The install script is blamed, but the issue is missing release artifacts.

**Prevention:**
1. Do not publish the install script until release assets exist for ALL advertised platforms.
2. The CI release workflow must build for at minimum: `darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, `linux-arm64`.
3. The install script should list supported platforms and clearly state which are available. If an asset is missing, say "No prebuilt binary available for [platform]. Build from source: [instructions]" rather than a generic error.

**Phase mapping:** Install script + CI release workflow. The CI workflow must be working before the install script is published.

---

### Pitfall 20: LSP Full Document Sync Is Expensive for Large Files

**What goes wrong:** The current LSP server uses `TextDocumentSyncKind::FULL`, meaning the entire document is sent on every keystroke. For small files this is fine, but for large Mesh files (hundreds of lines), re-parsing and re-typechecking the entire document on every keystroke causes perceptible lag, especially since the type checker is not incremental.

**Prevention:**
1. For v1 of the new LSP features, keep `FULL` sync. It is simpler and correctness matters more than performance at this stage.
2. Add a debounce in the `did_change` handler: do not analyze immediately on every change, but wait 100-200ms after the last change before re-analyzing. This batches rapid keystrokes into a single analysis pass.
3. Log timing information (`analyze_document` duration) to identify when performance becomes a problem.
4. Future optimization: switch to `INCREMENTAL` sync, which only sends the changed range, and incrementally re-parse using rowan's green tree editing.

**Phase mapping:** LSP features phase. Performance optimization can be deferred, but timing instrumentation should be added now.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| Install script | Corrupted binary from interrupted download | SHA256 verification before install (Pitfall 1) |
| Install script | Wrong platform detection | Normalize arch names, test on all platforms (Pitfall 2) |
| Install script | PATH not updated | Detect shell, update profile, print instructions (Pitfall 9) |
| Install script | No CI builds for all platforms | Build matrix with LLVM per-platform (Pitfall 10) |
| Install script | Missing release assets at launch | Block script publish on asset availability (Pitfall 19) |
| VS Code grammar | New keywords not highlighted | Update grammar when parser changes (Pitfall 6) |
| VS Code publish | Secrets in .vsix package | .vsixignore, `vsce ls` review (Pitfall 5) |
| VS Code publish | Version collision on Marketplace | Auto-bump version in CI (Pitfall 15) |
| VS Code publish | `create-publisher` removed from vsce | Use Marketplace web portal (Pitfall 16) |
| LSP completion | Wrong UTF-16 offsets for non-ASCII | Use existing `offset_to_position` (Pitfall 7) |
| LSP completion | Unwanted popups during `::` typing | Only `.` as trigger character (Pitfall 12) |
| LSP document symbols | selection_range outside range | Same coordinate system for both (Pitfall 11) |
| LSP signature help | Fires on non-call `(` | Check CST context before returning (Pitfall 18) |
| LSP all features | Capabilities not advertised | Extend initialize test (Pitfall 4) |
| LSP all features | Mutex deadlock | Never hold lock across `.await` (Pitfall 3) |
| LSP all features | Slow for large files | Debounce did_change, add timing (Pitfall 20) |
| REPL/formatter | Silent data loss for new syntax | Catch-all arm in formatter, snapshot tests (Pitfall 13) |
| Docs | Wrong commands on getting-started page | Walk through on clean system (Pitfall 14) |
| Docs | Extension error does not link to install | Add install URL to error message (Pitfall 17) |

## Sources

### Primary (HIGH confidence -- direct codebase analysis)
- `crates/mesh-lsp/src/server.rs` -- MeshBackend, Mutex<HashMap>, handler implementations, ServerCapabilities
- `crates/mesh-lsp/src/analysis.rs` -- offset_to_position (UTF-16), position_to_offset, analyze_document
- `crates/mesh-lsp/src/definition.rs` -- source_to_tree_offset, tree_to_source_offset (whitespace-skipping coordinate system)
- `crates/mesh-lsp/src/lib.rs` -- run_server, tower-lsp Server setup
- `editors/vscode-mesh/package.json` -- Extension configuration, publisher, scripts, dependencies
- `editors/vscode-mesh/src/extension.ts` -- findMeshc() well-known paths, LanguageClient setup
- `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json` -- Grammar patterns, rule ordering
- `.cargo/config.toml` -- LLVM_SYS_211_PREFIX hardcoded to /opt/homebrew
- `Cargo.toml` -- inkwell with llvm21-1 feature, tower-lsp 0.20
- `crates/meshc/src/main.rs` -- CLI subcommands (build, fmt, repl, lsp)
- `website/docs/docs/getting-started/index.md` -- Current install instructions (build from source only)

### Secondary (MEDIUM confidence -- official documentation, multiple sources agree)
- [tower-lsp LanguageServer trait docs](https://docs.rs/tower-lsp/latest/tower_lsp/trait.LanguageServer.html) -- Handler signatures, capability requirements
- [LSP Specification 3.17](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/) -- UTF-16 positions, TextEdit range requirements, capability negotiation
- [DocumentSymbol struct docs](https://docs.rs/lsp-types/latest/lsp_types/struct.DocumentSymbol.html) -- range/selection_range containment requirement
- [VS Code Publishing Extensions](https://code.visualstudio.com/api/working-with-extensions/publishing-extension) -- PAT creation, vsce workflow, .vsixignore
- [Tokio shared state guide](https://tokio.rs/tokio/tutorial/shared-state) -- Mutex in async code, deadlock prevention
- [tower-lsp concurrent handler issue #284](https://github.com/ebkalderon/tower-lsp/issues/284) -- Concurrent handler execution correctness concerns
- [How to deadlock Tokio with a single mutex](https://turso.tech/blog/how-to-deadlock-tokio-application-in-rust-with-just-a-single-mutex) -- std::sync::Mutex blocking in async context
- [cargo-dist](https://axodotdev.github.io/cargo-dist/) -- Automated Rust binary distribution with install scripts
- [Wiz Research: Supply chain risk in VS Code extensions](https://www.wiz.io/blog/supply-chain-risk-in-vscode-extension-marketplaces) -- 550+ secrets leaked in published extensions
- [Cross-compiling Rust on GitHub Actions](https://blog.timhutt.co.uk/cross-compiling-rust/) -- Platform matrix, LLVM cross-compile challenges
- [Chef: 5 ways to deal with curl|bash](https://www.chef.io/blog/5-ways-to-deal-with-the-install-sh-curl-pipe-bash-problem) -- Partial download risk, verification best practices
- [TextMate Language Grammars manual](https://manual.macromates.com/en/language_grammars) -- Pattern priority, first-match-wins semantics
