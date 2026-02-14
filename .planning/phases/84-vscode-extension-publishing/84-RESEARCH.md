# Phase 84: VS Code Extension Publishing - Research

**Researched:** 2026-02-14
**Domain:** VS Code Marketplace publishing, Open VSX publishing, extension packaging, CI/CD automation
**Confidence:** HIGH

## Summary

Publishing a VS Code extension to the VS Code Marketplace and Open VSX involves three distinct concerns: (1) preparing marketplace metadata (icon, README, CHANGELOG, gallery banner), (2) packaging the extension correctly with `.vsixignore` to exclude dev artifacts and secrets, and (3) automating the publish process via GitHub Actions on tag push.

The Mesh extension at `editors/vscode-mesh/` already has a well-structured `package.json` with publisher `mesh-lang`, name `mesh-lang`, and version `0.1.0`. The extension compiles TypeScript to `out/extension.js` and uses `vscode-languageclient`. The current `.vscodeignore` (note: the file is actually named `.vscodeignore` in the repo, not `.vsixignore`) already excludes `src/**`, `tsconfig.json`, and `node_modules/**`. However, several critical gaps exist: no `vscode:prepublish` script (TypeScript won't compile before packaging), no icon (PNG required, only SVG logos exist), no extension-specific README or CHANGELOG, no `repository` field in package.json, and no CI/CD workflow for extension publishing.

**Primary recommendation:** Add a `vscode:prepublish` compile script, convert the SVG logo to a 256x256 PNG icon, create an extension-specific README.md with screenshots, create CHANGELOG.md, enhance `.vscodeignore` to be comprehensive, add all marketplace metadata fields to package.json, and create a GitHub Actions workflow using `HaaLeo/publish-vscode-extension@v2` that triggers on version tags.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `@vscode/vsce` | ^3.7.1 (latest) | Package and publish to VS Code Marketplace | Official Microsoft CLI tool |
| `ovsx` | latest (via npx) | Publish to Open VSX Registry | Official Eclipse CLI tool |
| `HaaLeo/publish-vscode-extension` | v2 | GitHub Action for CI/CD publishing | Supports both registries in one action |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `typescript` | ^5.3.0 | Compile extension source | Already in devDependencies |
| `vscode-languageclient` | ^9.0.1 | LSP client for Mesh | Already in dependencies |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `HaaLeo/publish-vscode-extension@v2` | Manual `vsce publish` + `npx ovsx publish` | More control but more boilerplate; HaaLeo handles packaging once and reusing VSIX |
| `ovsx` CLI directly | `HaaLeo/publish-vscode-extension@v2` | HaaLeo wraps ovsx, simpler in CI |

**Installation (dev):**
```bash
# Already in package.json devDependencies:
npm install --save-dev @vscode/vsce

# ovsx used via npx in CI only, no local install needed
```

## Architecture Patterns

### Extension Directory Structure (Target State)
```
editors/vscode-mesh/
├── .vscodeignore          # Exclude dev files from VSIX
├── CHANGELOG.md           # Extension changelog (shown on Marketplace)
├── README.md              # Extension README (shown on Marketplace)
├── images/
│   └── icon.png           # 256x256 PNG extension icon
├── language-configuration.json
├── package.json           # Enhanced with marketplace metadata
├── package-lock.json
├── src/
│   └── extension.ts       # TypeScript source (excluded from VSIX)
├── syntaxes/
│   └── mesh.tmLanguage.json
├── tsconfig.json          # (excluded from VSIX)
└── out/                   # Compiled JS (included in VSIX, git-ignored)
    └── extension.js
```

### Pattern 1: vscode:prepublish Script
**What:** A script in package.json that runs automatically before `vsce package` or `vsce publish`.
**When to use:** Always -- ensures TypeScript is compiled before packaging.
**Example:**
```json
{
  "scripts": {
    "vscode:prepublish": "npm run compile",
    "compile": "tsc -p ./",
    "watch": "tsc -watch -p ./",
    "package": "vsce package --no-dependencies",
    "install-local": "vsce package --no-dependencies && code --install-extension mesh-lang-0.2.0.vsix"
  }
}
```
Source: https://code.visualstudio.com/api/working-with-extensions/publishing-extension

### Pattern 2: Comprehensive package.json Marketplace Fields
**What:** All fields that affect the Marketplace listing appearance.
**When to use:** Before first publish.
**Example:**
```json
{
  "name": "mesh-lang",
  "displayName": "Mesh Language",
  "description": "Mesh language support — syntax highlighting, diagnostics, hover, go-to-definition, and completions",
  "version": "0.2.0",
  "publisher": "mesh-lang",
  "license": "MIT",
  "icon": "images/icon.png",
  "galleryBanner": {
    "color": "#1a1a2e",
    "theme": "dark"
  },
  "repository": {
    "type": "git",
    "url": "https://github.com/mesh-lang/mesh.git"
  },
  "homepage": "https://meshlang.dev",
  "bugs": {
    "url": "https://github.com/mesh-lang/mesh/issues"
  },
  "categories": ["Programming Languages"],
  "keywords": ["mesh", "language", "syntax highlighting", "lsp", "actor", "concurrency"],
  "engines": {
    "vscode": "^1.75.0"
  },
  "pricing": "Free",
  "markdown": "github"
}
```
Source: https://code.visualstudio.com/api/references/extension-manifest

### Pattern 3: GitHub Actions Dual-Registry Publishing
**What:** Publish to both VS Code Marketplace and Open VSX from a single workflow on tag push.
**When to use:** CI/CD automation for extension releases.
**Example:**
```yaml
name: Publish Extension

on:
  push:
    tags:
      - "ext-v*"

jobs:
  publish:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: editors/vscode-mesh
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
      - run: npm ci

      - name: Publish to Open VSX Registry
        uses: HaaLeo/publish-vscode-extension@v2
        id: publishToOpenVSX
        with:
          pat: ${{ secrets.OPEN_VSX_TOKEN }}
          packagePath: ./editors/vscode-mesh

      - name: Publish to Visual Studio Marketplace
        uses: HaaLeo/publish-vscode-extension@v2
        with:
          pat: ${{ secrets.VS_MARKETPLACE_TOKEN }}
          registryUrl: https://marketplace.visualstudio.com
          extensionFile: ${{ steps.publishToOpenVSX.outputs.vsixPath }}
```
Source: https://github.com/HaaLeo/publish-vscode-extension

### Anti-Patterns to Avoid
- **Publishing without `vscode:prepublish`:** The VSIX will have `main: "./out/extension.js"` but no `out/` directory, causing instant activation failure.
- **Using SVG for icon:** VS Code Marketplace rejects SVG icons for security reasons. Must be PNG.
- **Hardcoding PAT in workflow:** Always use GitHub secrets (`secrets.VS_MARKETPLACE_TOKEN`, `secrets.OPEN_VSX_TOKEN`).
- **Re-packaging for second registry:** Package once (Open VSX step), reuse the `.vsix` for VS Code Marketplace via `extensionFile` input. Avoids inconsistencies.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| VSIX packaging | Custom zip scripts | `vsce package` | Handles manifest validation, file exclusion, prepublish hooks |
| Dual-registry publishing | Two separate workflows | `HaaLeo/publish-vscode-extension@v2` | Single action, reuses VSIX, handles both registries |
| SVG to PNG conversion | Manual export each time | One-time conversion via design tool or `rsvg-convert` | Only needs to happen once; the PNG is committed |
| Version management | Manual edits to package.json | `npm version minor` or `vsce publish minor` | Handles git tag creation and version bumping |

**Key insight:** The publishing toolchain (`vsce`, `ovsx`, HaaLeo action) is mature and handles all the edge cases around VSIX format compliance, license validation, and image URL rewriting. The only manual work is one-time setup: creating the publisher accounts, generating tokens, and preparing the icon/README.

## Common Pitfalls

### Pitfall 1: Missing vscode:prepublish Script
**What goes wrong:** `vsce package` creates a VSIX without compiled JavaScript. The extension activates but immediately fails because `out/extension.js` doesn't exist.
**Why it happens:** The existing package.json has `compile` and `watch` scripts but no `vscode:prepublish`.
**How to avoid:** Add `"vscode:prepublish": "npm run compile"` to package.json scripts.
**Warning signs:** `vsce ls` shows no `out/` files in the package listing.

### Pitfall 2: No README.md in Extension Directory
**What goes wrong:** The Marketplace listing shows a blank page or "No README found" message.
**Why it happens:** The root `README.md` is for the entire Mesh project, not the extension. The extension directory needs its own README.
**How to avoid:** Create `editors/vscode-mesh/README.md` with extension-specific content: features, screenshots, configuration, requirements.
**Warning signs:** `vsce package` warns about missing README.

### Pitfall 3: Azure DevOps PAT Scope Too Narrow
**What goes wrong:** `vsce publish` returns 401 or 403 errors.
**Why it happens:** PAT created with organization-specific scope instead of "All accessible organizations", or without "Marketplace (Manage)" scope.
**How to avoid:** When creating the PAT: select "All accessible organizations" and "Marketplace > Manage" scope.
**Warning signs:** 401/403 on publish; success on `vsce package`.

### Pitfall 4: Open VSX Namespace Not Pre-Created
**What goes wrong:** `ovsx publish` fails with "Namespace not found" error.
**Why it happens:** Unlike VS Code Marketplace, Open VSX requires explicit namespace creation before first publish.
**How to avoid:** Run `npx ovsx create-namespace mesh-lang --pat <token>` before the first publish. This is a one-time manual step.
**Warning signs:** First CI/CD run fails even though VS Code Marketplace publish succeeded.

### Pitfall 5: SVG Icon in package.json
**What goes wrong:** `vsce package` rejects the extension or the Marketplace strips the icon.
**Why it happens:** VS Code Marketplace prohibits SVG icons for security reasons. The project only has SVG logos.
**How to avoid:** Convert the SVG logo to a 256x256 PNG. The existing `logo-icon-black.svg` or `logo-icon-white.svg` from the website can be the source.
**Warning signs:** `vsce package` emits a warning about SVG icons.

### Pitfall 6: Image URLs Not HTTPS in README
**What goes wrong:** Images don't render on the Marketplace page.
**Why it happens:** All image URLs in README.md and CHANGELOG.md must resolve to HTTPS. Relative paths work only if a public GitHub `repository` field is set in package.json (vsce rewrites them).
**How to avoid:** Set the `repository` field in package.json to the public GitHub repo. Use relative image paths in README -- vsce will convert them to raw GitHub URLs.
**Warning signs:** Images appear broken on the Marketplace listing.

### Pitfall 7: Extension Tag Conflicts with Compiler Tags
**What goes wrong:** Pushing a `v0.2.0` tag triggers both the compiler release workflow AND the extension publish workflow.
**Why it happens:** Both workflows trigger on `v*` tags.
**How to avoid:** Use a distinct tag prefix for extension releases, e.g., `ext-v0.2.0` or `vscode-v0.2.0`. The existing `release.yml` triggers on `v*` tags.
**Warning signs:** Extension publish CI runs when only the compiler was intended to be released.

### Pitfall 8: .vscodeignore vs .vsixignore Naming
**What goes wrong:** Confusion about which file to use.
**Why it happens:** The requirements say `.vsixignore` but the actual VS Code convention is `.vscodeignore`. Both names work but `.vscodeignore` is the documented standard.
**How to avoid:** The file in the repo is already named `.vscodeignore` -- keep this name. Note: the requirement EXT-05 says `.vsixignore` but `.vscodeignore` is what vsce actually documents and uses.
**Warning signs:** None -- both work, but consistency with ecosystem matters.

### Pitfall 9: Open VSX Eclipse Publisher Agreement
**What goes wrong:** Cannot publish to Open VSX even with a valid token.
**Why it happens:** Open VSX requires signing the Eclipse Foundation Open VSX Publisher Agreement before publishing.
**How to avoid:** Create an Eclipse account, add GitHub username, log in to open-vsx.org with GitHub, connect Eclipse account, sign the Publisher Agreement.
**Warning signs:** Token creation succeeds but publish is rejected.

## Code Examples

### Complete package.json (Target State)
```json
{
  "name": "mesh-lang",
  "displayName": "Mesh Language",
  "description": "Mesh language support — syntax highlighting, diagnostics, hover, go-to-definition, completions, and signature help",
  "version": "0.2.0",
  "publisher": "mesh-lang",
  "license": "MIT",
  "icon": "images/icon.png",
  "galleryBanner": {
    "color": "#1a1a2e",
    "theme": "dark"
  },
  "repository": {
    "type": "git",
    "url": "https://github.com/mesh-lang/mesh.git"
  },
  "homepage": "https://meshlang.dev",
  "bugs": {
    "url": "https://github.com/mesh-lang/mesh/issues"
  },
  "categories": ["Programming Languages"],
  "keywords": [
    "mesh", "language", "syntax highlighting", "lsp",
    "actor", "concurrency", "elixir-like", "static-types"
  ],
  "pricing": "Free",
  "markdown": "github",
  "engines": {
    "vscode": "^1.75.0"
  },
  "activationEvents": [
    "onLanguage:mesh"
  ],
  "main": "./out/extension.js",
  "contributes": {
    "languages": [
      {
        "id": "mesh",
        "aliases": ["Mesh", "mesh"],
        "extensions": [".mpl"],
        "configuration": "./language-configuration.json",
        "icon": {
          "light": "./images/icon.png",
          "dark": "./images/icon.png"
        }
      }
    ],
    "grammars": [
      {
        "language": "mesh",
        "scopeName": "source.mesh",
        "path": "./syntaxes/mesh.tmLanguage.json"
      }
    ],
    "configuration": {
      "title": "Mesh",
      "properties": {
        "mesh.lsp.path": {
          "type": "string",
          "default": "meshc",
          "description": "Path to the meshc binary (must be in PATH, or provide absolute path)"
        }
      }
    }
  },
  "scripts": {
    "vscode:prepublish": "npm run compile",
    "compile": "tsc -p ./",
    "watch": "tsc -watch -p ./",
    "package": "vsce package --no-dependencies",
    "install-local": "vsce package --no-dependencies && code --install-extension mesh-lang-0.2.0.vsix"
  },
  "dependencies": {
    "vscode-languageclient": "^9.0.1"
  },
  "devDependencies": {
    "@types/node": "^25.2.1",
    "@types/vscode": "^1.75.0",
    "@vscode/vsce": "^3.7.1",
    "typescript": "^5.3.0"
  }
}
```

### Complete .vscodeignore (Target State)
```
# Source files (compiled to out/)
src/**
**/*.ts
!**/*.d.ts

# Config files
tsconfig.json
.vscode/**
.github/**
.gitignore

# Dev artifacts
node_modules/**
out/**/*.map
**/*.test.js
**/*.spec.js

# Package manager
package-lock.json

# CI/CD
.eslintrc*
.prettierrc*
```

Note: `devDependencies` are automatically excluded by vsce, so they don't need listing. The `out/` directory IS included (it's the compiled extension), but source maps (`out/**/*.map`) are excluded.

### CHANGELOG.md Format
```markdown
# Changelog

All notable changes to the Mesh Language extension will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-02-14

### Added
- Published to VS Code Marketplace and Open VSX Registry
- Extension icon and marketplace metadata
- Completion suggestions with snippet support
- Signature help for function calls
- Document symbols (Outline view)

### Changed
- Enhanced TextMate grammar with comprehensive scope coverage

## [0.1.0] - 2026-02-07

### Added
- Initial release
- TextMate grammar for syntax highlighting
- LSP client connecting to meshc language server
- Hover information
- Go-to-definition
- Diagnostics (errors and warnings)
```

### GitHub Actions Workflow (publish-extension.yml)
```yaml
name: Publish Extension

on:
  push:
    tags:
      - "ext-v*"

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-node@v4
        with:
          node-version: 20

      - name: Install dependencies
        run: npm ci
        working-directory: editors/vscode-mesh

      - name: Compile TypeScript
        run: npm run compile
        working-directory: editors/vscode-mesh

      - name: Publish to Open VSX Registry
        uses: HaaLeo/publish-vscode-extension@v2
        id: publishToOpenVSX
        with:
          pat: ${{ secrets.OPEN_VSX_TOKEN }}
          packagePath: ./editors/vscode-mesh

      - name: Publish to Visual Studio Marketplace
        uses: HaaLeo/publish-vscode-extension@v2
        with:
          pat: ${{ secrets.VS_MARKETPLACE_TOKEN }}
          registryUrl: https://marketplace.visualstudio.com
          extensionFile: ${{ steps.publishToOpenVSX.outputs.vsixPath }}
```

### Extension README.md Structure
```markdown
# Mesh Language

![Visual Studio Marketplace Version](https://img.shields.io/visual-studio-marketplace/v/mesh-lang.mesh-lang)
![Visual Studio Marketplace Installs](https://img.shields.io/visual-studio-marketplace/i/mesh-lang.mesh-lang)

Mesh language support for Visual Studio Code, providing a rich editing experience
for the [Mesh programming language](https://meshlang.dev).

## Features

- **Syntax Highlighting** -- Full TextMate grammar for Mesh
- **Diagnostics** -- Real-time error and warning reporting
- **Hover Information** -- Type info and documentation on hover
- **Go to Definition** -- Jump to symbol definitions
- **Completions** -- Context-aware code completions
- **Signature Help** -- Parameter hints for function calls
- **Document Symbols** -- Outline view with hierarchical symbols

![Syntax Highlighting](images/screenshot-syntax.png)

## Requirements

The extension requires the `meshc` compiler binary for LSP features.
Install it via the install script:

\`\`\`bash
curl -sSf https://meshlang.dev/install.sh | sh
\`\`\`

Or build from source -- see [Getting Started](https://meshlang.dev/docs/getting-started/).

## Extension Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `mesh.lsp.path` | `meshc` | Path to the meshc binary |

## Release Notes

See [CHANGELOG.md](CHANGELOG.md) for full release history.
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `vsce` npm package | `@vscode/vsce` npm package | 2023 | Old `vsce` is deprecated; must use `@vscode/vsce` |
| Manual publish to each registry | `HaaLeo/publish-vscode-extension@v2` | 2023+ | Single action handles both registries |
| PAT-only auth (Azure DevOps) | PATs still required for vsce | Ongoing | Microsoft Entra tokens available but vsce still needs PATs |

**Deprecated/outdated:**
- `vsce` (unscoped npm package) -- replaced by `@vscode/vsce`; last version was 2.15.0, 3+ years ago
- OAuth for Azure DevOps -- deprecated April 2025, full sunset planned 2026; does not affect PAT-based vsce publishing

## Open Questions

1. **Publisher Account Creation**
   - What we know: The `publisher` field in package.json is `mesh-lang`. An Azure DevOps organization + PAT with "Marketplace (Manage)" scope is needed. An Eclipse account + Open VSX token is needed.
   - What's unclear: Whether the `mesh-lang` publisher name is already registered by someone else on either registry.
   - Recommendation: This is a one-time manual step the user must do. Document the exact steps in the plan as a prerequisite. Check availability of `mesh-lang` on both registries before starting.

2. **Icon Design**
   - What we know: The project has SVG logos (`logo-icon-black.svg`, `logo-icon-white.svg`) in the website's public directory. The VS Code icon must be PNG, at least 128x128, ideally 256x256.
   - What's unclear: Whether the user wants a custom icon design or just a PNG export of the existing SVG logo.
   - Recommendation: Convert `logo-icon-white.svg` to a 256x256 PNG on a dark background (matching `galleryBanner.color`). This maintains brand consistency. Use `rsvg-convert` or any SVG-to-PNG tool.

3. **Tag Naming Convention**
   - What we know: The existing `release.yml` triggers on `v*` tags for compiler releases. The extension needs its own tag pattern.
   - What's unclear: Whether to use `ext-v*`, `vscode-v*`, or another prefix.
   - Recommendation: Use `ext-v*` (e.g., `ext-v0.2.0`) -- short, clear, and won't conflict with compiler `v*` tags.

4. **Screenshots**
   - What we know: EXT-03 requires screenshots on the Marketplace page. Screenshots are included in the extension README.md as PNG images.
   - What's unclear: What specific screenshots to include.
   - Recommendation: Take 2-3 screenshots showing: (1) syntax highlighting of a Mesh file, (2) hover/completions in action, (3) error diagnostics. Store in `editors/vscode-mesh/images/`.

5. **GitHub Repository URL**
   - What we know: The README references `https://github.com/mesh-lang/mesh.git` and also `https://github.com/snowdamiz/mesh-lang.git` in docs.
   - What's unclear: Which is the canonical public URL.
   - Recommendation: Use `https://github.com/mesh-lang/mesh.git` as it matches the publisher name. Verify this URL is valid before publishing.

## Sources

### Primary (HIGH confidence)
- [VS Code Publishing Extensions](https://code.visualstudio.com/api/working-with-extensions/publishing-extension) -- packaging, PAT creation, vsce commands
- [VS Code Extension Manifest](https://code.visualstudio.com/api/references/extension-manifest) -- all package.json fields
- [HaaLeo/publish-vscode-extension](https://github.com/HaaLeo/publish-vscode-extension) -- GitHub Action inputs/outputs, workflow examples
- [@vscode/vsce npm](https://www.npmjs.com/package/@vscode/vsce) -- current version 3.7.1
- [Open VSX Publishing Extensions](https://github.com/eclipse/openvsx/wiki/Publishing-Extensions) -- ovsx CLI, namespace creation
- [Open VSX Namespace Access](https://github.com/eclipse/openvsx/wiki/Namespace-Access) -- namespace ownership, verification

### Secondary (MEDIUM confidence)
- [ovsx npm](https://www.npmjs.com/package/ovsx) -- CLI installation and commands
- [Azure DevOps PAT docs](https://learn.microsoft.com/en-us/azure/devops/organizations/accounts/use-personal-access-tokens-to-authenticate) -- PAT creation and scoping
- [Eclipse Open VSX FAQ](https://www.eclipse.org/legal/open-vsx-registry-faq/) -- publisher agreement requirements

### Tertiary (LOW confidence)
- Gallery banner color choice (`#1a1a2e`) is a suggestion based on common dark-theme extensions; user may prefer different branding

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- vsce, ovsx, and HaaLeo action are all well-documented official/established tools
- Architecture: HIGH -- package.json manifest fields and .vscodeignore patterns are thoroughly documented by Microsoft
- Pitfalls: HIGH -- common errors (missing prepublish, SVG icon, PAT scope) are well-documented in official docs and community reports
- CI/CD workflow: HIGH -- HaaLeo action v2 is mature with clear documentation; the dual-publish pattern is the documented recommended approach

**Research date:** 2026-02-14
**Valid until:** 2026-04-14 (stable domain; vsce and ovsx rarely have breaking changes)
