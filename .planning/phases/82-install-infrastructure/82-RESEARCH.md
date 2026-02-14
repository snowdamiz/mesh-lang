# Phase 82: Install Infrastructure - Research

**Researched:** 2026-02-14
**Domain:** CI/CD pipeline (GitHub Actions), shell install scripting, LLVM-dependent Rust binary distribution
**Confidence:** HIGH (CI patterns well-established), MEDIUM (LLVM 21 CI availability has known issues)

<user_constraints>

## User Constraints (from CONTEXT.md)

### Locked Decisions

**Script UX & output:**
- Minimal output style -- just what was installed, where, and the version. No banner or ASCII art
- Subtle colors: green for success, red for errors, bold for emphasis. Respect NO_COLOR and piped output
- No interactive prompts -- `curl | sh` installs immediately. No confirmation step needed (user already chose to run it)

**Update & versioning:**
- Re-running the install script checks the installed version against latest. Skips if already up-to-date, updates if newer is available
- Supports `--version 0.2.0` flag to install a specific version (for CI pinning). Default is latest
- No `meshc self-update` command -- users re-run the install script to update
- Supports `--uninstall` flag to remove meshc and clean up PATH changes

**CI build targets:**
- 6 targets: x86_64-apple-darwin, aarch64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-unknown-linux-musl, x86_64-pc-windows-msvc
- PR pushes build all targets (catch build failures early). Only tag pushes (e.g., v0.2.0) create GitHub Releases
- Release artifacts: .tar.gz per target (.zip for Windows) + SHA256SUMS file. No auto-generated release notes

### Claude's Discretion
- Install location (~/.mesh/bin, /usr/local/bin, etc.) and PATH configuration method
- LLVM linking strategy (static vs dynamic) -- pick what gives the best user experience
- Error verbosity on failure -- pick appropriate level of diagnostic detail
- musl static linking details
- Windows install experience (PowerShell equivalent of curl | sh, or .msi, etc.)

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope

</user_constraints>

## Summary

This phase requires two major deliverables: (1) a GitHub Actions CI pipeline that builds meshc for 6 targets and creates GitHub Releases on tag push, and (2) a POSIX shell install script that downloads, verifies, and installs the correct binary for the user's platform.

The primary challenge is the LLVM 21 dependency. meshc depends on `inkwell 0.8.0` with the `llvm21-1` feature, which wraps `llvm-sys 211.0.0`. This means every CI build target needs access to LLVM 21 development libraries (static `.a` files and `llvm-config`). The official LLVM 21.1.8 GitHub releases provide `clang+llvm` archives for Linux x86_64 and Windows, but NOT for Linux ARM64 or macOS (either architecture). The community `ycm-core/llvm` project fills this gap with `clang+llvm` archives for all four major platforms at version 21.1.3. For macOS, Homebrew provides `llvm@21` on both architectures. The recommended strategy is: **statically link LLVM** (`force-static` feature) to produce self-contained binaries with zero LLVM runtime dependency for end users.

The install script follows the well-established `curl -sSf URL | sh` pattern used by rustup, uv, and bun. The script must handle platform detection via `uname`, with special Rosetta detection on Apple Silicon using `sysctl hw.optional.arm64`. For Windows, the standard equivalent is `powershell -ExecutionPolicy ByPass -c "irm URL | iex"`. The script installs to `~/.mesh/bin/meshc`, modifies shell profiles (bash, zsh, fish) for PATH, and supports `--version` and `--uninstall` flags.

**Primary recommendation:** Use native GitHub Actions runners for each platform (macOS ARM64: `macos-14`, macOS x86_64: `macos-15-intel`, Linux x86_64: `ubuntu-24.04`, Linux ARM64: `ubuntu-24.04-arm`, Windows: `windows-latest`), install LLVM 21 via platform-appropriate methods, and statically link LLVM into meshc for zero-dependency distribution.

## Standard Stack

### Core CI

| Tool | Version | Purpose | Why Standard |
|------|---------|---------|--------------|
| GitHub Actions | N/A | CI/CD platform | Already used for deploy.yml; native runners for all target platforms |
| `dtolnay/rust-toolchain` | `@stable` | Install Rust toolchain with target | Standard Rust CI action, handles target addition |
| `Swatinem/rust-cache` | `@v2` | Cache cargo dependencies and build artifacts | Purpose-built for Rust; auto-keys on lockfile and target |
| `actions/upload-artifact` | `@v4` | Share build artifacts between jobs | Standard for multi-job workflows |
| `softprops/action-gh-release` | `@v2` | Create GitHub Releases with artifacts | Most popular release creation action |

### LLVM Installation per Runner

| Runner | LLVM Install Method | Notes |
|--------|-------------------|-------|
| `ubuntu-24.04` (x86_64) | Download `clang+llvm-21.x.x-x86_64-pc-linux-gnu.tar.xz` from official LLVM releases | Official archive includes llvm-config + static libs |
| `ubuntu-24.04-arm` (aarch64) | Download `clang+llvm-21.1.3-aarch64-linux-gnu.tar.xz` from `ycm-core/llvm` OR use `apt.llvm.org` | Official releases lack clang+llvm for Linux ARM64; ycm-core fills the gap |
| `macos-14` (aarch64) | `brew install llvm@21` | Homebrew has LLVM 21 for Apple Silicon |
| `macos-15-intel` (x86_64) | `brew install llvm@21` | Homebrew has LLVM 21 for Intel; runner available for macOS x86_64 builds |
| `windows-latest` (x86_64) | Download `clang+llvm-21.x.x-x86_64-pc-windows-msvc.tar.xz` from official releases | Official archive includes development libraries |

### Install Script Dependencies (User-side)

| Tool | Purpose | Fallback |
|------|---------|----------|
| `curl` | Download binary and checksums | `wget` as fallback |
| `sha256sum` | Verify checksum (Linux) | `shasum -a 256` on macOS |
| `tar` | Extract archive | Required, universally available |
| `uname` | Platform/arch detection | Required, universally available |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Hand-written workflow | `cargo-dist` | cargo-dist generates CI + installers automatically, BUT it cannot handle custom LLVM pre-build steps needed for this project. Revisit if cargo-dist adds better system dependency support |
| `houseabsolute/actions-rust-cross` | `cross-rs` | Good for simple projects but LLVM dependency makes Docker-based cross-compilation impractical; native runners are simpler |
| `KyleMayes/install-llvm-action` | Direct download | Action only supports up to LLVM 20 (v2.0.7); would need `download-url` workaround anyway |
| `apt.llvm.org/llvm.sh` | Direct tarball download | Known failures with LLVM 21 on Ubuntu 22.04/24.04 (GitHub issues #150822, #149877); unreliable for CI |

## Architecture Patterns

### Recommended Project Structure

```
.github/
  workflows/
    ci.yml              # PR builds (all targets, no release)
    release.yml         # Tag-triggered builds + GitHub Release
install/
  install.sh            # POSIX shell install script (served at mesh-lang.org/install.sh)
  install.ps1           # PowerShell install script for Windows
```

### Pattern 1: Matrix Build with Native Runners

**What:** Use GitHub Actions matrix strategy with one job per target, running on the native platform for that target. No cross-compilation needed.
**When to use:** When native runners are available for all targets (they are).
**Why:** Avoids all cross-compilation complexity. LLVM headers/libs match the target. Each runner compiles natively.

```yaml
# Source: Standard GitHub Actions pattern, verified with GH docs
strategy:
  fail-fast: false
  matrix:
    include:
      - target: x86_64-apple-darwin
        os: macos-15-intel
        llvm_method: brew
      - target: aarch64-apple-darwin
        os: macos-14
        llvm_method: brew
      - target: x86_64-unknown-linux-gnu
        os: ubuntu-24.04
        llvm_method: tarball
      - target: aarch64-unknown-linux-gnu
        os: ubuntu-24.04-arm
        llvm_method: tarball
      - target: x86_64-unknown-linux-musl
        os: ubuntu-24.04
        llvm_method: tarball
      - target: x86_64-pc-windows-msvc
        os: windows-latest
        llvm_method: tarball
```

### Pattern 2: Two-Phase Workflow (Build + Release)

**What:** Separate the build step (per-target matrix) from the release step (single job that collects all artifacts and creates the GitHub Release).
**When to use:** Always for release workflows.
**Why:** Build jobs run in parallel. Release job runs once after all builds succeed. Prevents partial releases.

```yaml
# Source: Standard pattern from cargo-dist, GitHub docs
jobs:
  build:
    strategy:
      matrix: # ... targets
    steps:
      - # Build and upload artifact

  release:
    needs: build
    if: startsWith(github.ref, 'refs/tags/v')
    steps:
      - # Download all artifacts
      - # Generate SHA256SUMS
      - # Create GitHub Release
```

### Pattern 3: Wrapped Shell Script (Prevent Partial Execution)

**What:** Wrap the entire install script in a `main()` function called at the end.
**When to use:** Always for `curl | sh` scripts.
**Why:** If the network drops mid-download during `curl | sh`, a partially downloaded script could execute incomplete commands. Wrapping in a function ensures the entire script is downloaded before any execution begins.

```sh
#!/bin/sh
# Source: rustup-init.sh pattern (rust-lang/rustup)
set -eu

main() {
    # ... all installation logic here ...
}

main "$@"
```

### Pattern 4: Install Location with Environment Script

**What:** Install binary to `~/.mesh/bin/meshc` and create an `env` script at `~/.mesh/env` that sets up PATH.
**When to use:** For tools that need discoverable, user-writable install locations.
**Why:** Follows rustup convention (`~/.cargo/bin`). User-writable (no sudo). Isolated from system paths. The `env` script can be sourced in shell profiles: `. "$HOME/.mesh/env"`.

```
~/.mesh/
  bin/
    meshc              # The binary
  env                  # Shell snippet: export PATH="$HOME/.mesh/bin:$PATH"
  version              # Current installed version (for update checks)
```

### Anti-Patterns to Avoid

- **Installing to /usr/local/bin:** Requires sudo, fails in rootless containers, conflicts with system packages. Use `~/.mesh/bin` instead.
- **Building LLVM from source in CI:** Takes 30-60+ minutes. Always use prebuilt binaries.
- **Using `apt.llvm.org/llvm.sh` for LLVM 21:** Known broken on Ubuntu 22.04/24.04 as of 2025. Use direct tarball downloads instead.
- **Using `cross-rs` with LLVM dependency:** Docker-based cross-compilation cannot easily provide LLVM development libraries inside the container. Native runners are simpler.
- **Dynamic linking LLVM in release binaries:** End users would need LLVM 21 installed. Statically linking LLVM into meshc eliminates this dependency entirely.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| GitHub Release creation | Custom API calls | `softprops/action-gh-release@v2` | Handles upload, checksums, idempotency, draft releases |
| Rust toolchain setup | Manual `curl rustup` | `dtolnay/rust-toolchain@stable` | Handles target addition, component installation, caching key |
| Cargo build caching | Manual `actions/cache` | `Swatinem/rust-cache@v2` | Auto-keys on Cargo.lock, target, handles cache eviction |
| Platform detection in shell | Simple `uname` checks | Rustup-style detection with Rosetta handling | Edge cases: Rosetta, musl vs gnu, 32-bit userland on 64-bit kernel |
| Checksum verification in script | Basic `sha256sum` | Multi-tool detection (`sha256sum` OR `shasum -a 256`) | macOS doesn't have `sha256sum`; Linux may not have `shasum` |

**Key insight:** The install script is the highest-risk component because it runs on unpredictable user systems. Every edge case (missing tools, unusual PATHs, read-only filesystems, corporate proxies) matters. Model it closely on rustup-init.sh, which has been battle-tested across millions of installs.

## Common Pitfalls

### Pitfall 1: LLVM 21 Not Available via apt.llvm.org

**What goes wrong:** CI builds fail because `llvm.sh 21` doesn't work on Ubuntu 24.04/22.04.
**Why it happens:** The `llvm.sh` script has bugs with LLVM 21 detection (GitHub issue #150822), and packages may be missing (issue #149877).
**How to avoid:** Download prebuilt `clang+llvm` tarballs directly from GitHub releases instead of using `apt`. Set `LLVM_SYS_211_PREFIX` to the extracted directory.
**Warning signs:** CI builds fail at the LLVM installation step with apt dependency errors.

### Pitfall 2: No Official clang+llvm Tarball for All Platforms

**What goes wrong:** Official LLVM 21.1.8 releases only have `clang+llvm` archives for Linux x86_64 and Windows. macOS and Linux ARM64 only have `LLVM-` toolchain packages (which may lack `llvm-config` and static libraries).
**Why it happens:** LLVM release infrastructure doesn't build `clang+llvm` archives for all platforms consistently.
**How to avoid:** For macOS: use Homebrew (`brew install llvm@21`), which provides `llvm-config` and static libraries. For Linux ARM64: use `ycm-core/llvm` community builds OR `apt.llvm.org` (which IS more reliable for installing dev packages than for the `llvm.sh` script directly). Alternative: check if the `LLVM-` archives actually do include `llvm-config` on non-Windows platforms (needs runtime validation).
**Warning signs:** Build fails with "can't find llvm-config" or "llvm-sys build error".

### Pitfall 3: Rosetta Detection on Apple Silicon

**What goes wrong:** Install script detects `x86_64` on an Apple Silicon Mac running under Rosetta, downloads the wrong binary.
**Why it happens:** `uname -m` returns `x86_64` when the calling process runs under Rosetta translation.
**How to avoid:** Use `sysctl -n hw.optional.arm64 2>/dev/null` to detect true Apple Silicon hardware. If it returns `1`, the machine is ARM64 regardless of what `uname -m` says.
**Warning signs:** Users report "wrong architecture" errors on M1/M2/M3 Macs.

### Pitfall 4: musl Binary Performance Regression

**What goes wrong:** meshc compiled with musl target is significantly slower than glibc version, especially in multi-threaded codegen.
**Why it happens:** musl's default allocator has high lock contention in multi-threaded workloads (up to 10x slower in some cases).
**How to avoid:** Use a custom allocator. Add `mimalloc` or `jemalloc` as the global allocator for the musl target. This brings performance on par with glibc builds.
**Warning signs:** Performance benchmarks on musl binary show unexpected slowdowns vs glibc binary.

### Pitfall 5: Shell Profile Modification Idempotency

**What goes wrong:** Running the install script multiple times appends duplicate PATH entries to shell profiles.
**Why it happens:** Script blindly appends `export PATH=...` without checking if it already exists.
**How to avoid:** Use a marker comment (e.g., `# Mesh compiler`) and check for its existence before modifying. Or better: write a dedicated `~/.mesh/env` file and add a single `. "$HOME/.mesh/env"` line to profiles, checking for the line before adding.
**Warning signs:** `echo $PATH` shows `~/.mesh/bin` repeated multiple times.

### Pitfall 6: LLVM Static Linking Produces Large Binaries

**What goes wrong:** Statically linked meshc binary is 100-300+ MB because it includes all of LLVM.
**Why it happens:** LLVM is massive. Static linking includes all referenced code.
**How to avoid:** This is expected and acceptable -- the binary is self-contained. Use `strip` to remove debug symbols (can reduce size by 50%+). Consider using `LLVM_TARGETS_TO_BUILD` to limit which LLVM backends are included if possible (requires building LLVM from source with limited targets, which conflicts with using prebuilt binaries). The project currently uses inkwell without target feature flags, so all LLVM targets may be linked. Adding specific target features (e.g., `target-x86`, `target-aarch64`) to the inkwell dependency could help if prebuilt LLVM is built with all targets.
**Warning signs:** Release tarballs are unexpectedly large.

### Pitfall 7: macOS Code Signing / Quarantine

**What goes wrong:** Users on macOS cannot run the downloaded binary -- macOS shows "cannot be opened because it is from an unidentified developer" or silently kills the process.
**Why it happens:** macOS quarantines files downloaded from the internet. Unsigned binaries trigger Gatekeeper.
**How to avoid:** The install script should use `xattr -d com.apple.quarantine` on the downloaded binary after extraction. For a better long-term solution, sign binaries with an Apple Developer certificate, but this is not required for the initial release.
**Warning signs:** macOS users report the binary "doesn't work" despite successful installation.

### Pitfall 8: fish Shell PATH Syntax

**What goes wrong:** Install script writes `export PATH=...` to fish config, which is invalid fish syntax.
**Why it happens:** fish uses different syntax from bash/zsh (`set -gx` instead of `export`, `fish_add_path` for PATH).
**How to avoid:** Detect fish separately and write `fish_add_path $HOME/.mesh/bin` to `~/.config/fish/config.fish` instead.
**Warning signs:** fish users report PATH not being set after installation.

### Pitfall 9: musl 1.2.5 Breaking Change (Rust 1.93+)

**What goes wrong:** musl builds fail with "undefined reference to `open64`" linker errors.
**Why it happens:** Rust 1.93+ ships with musl 1.2.5, which removed legacy compatibility symbols.
**How to avoid:** Run `cargo update` to get latest crate versions that are compatible with musl 1.2.5. Most crates have already been updated.
**Warning signs:** musl build fails with `extern` function not found errors, specifically `open64`.

## Code Examples

Verified patterns from official sources:

### LLVM Setup in CI (Linux x86_64)

```yaml
# Source: Derived from llvm-sys docs + LLVM release patterns
- name: Install LLVM 21
  run: |
    LLVM_VERSION="21.1.8"
    LLVM_ARCHIVE="clang+llvm-${LLVM_VERSION}-x86_64-pc-linux-gnu.tar.xz"
    LLVM_URL="https://github.com/llvm/llvm-project/releases/download/llvmorg-${LLVM_VERSION}/${LLVM_ARCHIVE}"
    curl -sSfL "$LLVM_URL" -o llvm.tar.xz
    mkdir -p "$HOME/llvm"
    tar xf llvm.tar.xz --strip-components=1 -C "$HOME/llvm"
    echo "LLVM_SYS_211_PREFIX=$HOME/llvm" >> "$GITHUB_ENV"
```

### LLVM Setup in CI (macOS via Homebrew)

```yaml
# Source: Homebrew LLVM formula conventions
- name: Install LLVM 21
  run: |
    brew install llvm@21
    echo "LLVM_SYS_211_PREFIX=$(brew --prefix llvm@21)" >> "$GITHUB_ENV"
```

### Cargo Build with Static LLVM

```yaml
# Source: llvm-sys crate documentation
- name: Build meshc
  run: |
    cargo build --release --target ${{ matrix.target }} \
      --features inkwell/llvm21-1-force-static
  env:
    LLVM_SYS_211_PREFIX: ${{ env.LLVM_SYS_211_PREFIX }}
```

Note: The `force-static` feature is passed through inkwell to llvm-sys. The current Cargo.toml uses `inkwell = { version = "0.8.0", features = ["llvm21-1"] }`. For release builds, add `llvm21-1-force-static` (or override via the workspace `Cargo.toml` for the release profile). Since the default llvm-sys behavior is already `prefer-static`, this may work without the explicit feature, but `force-static` is more deterministic.

### Platform Detection in Install Script

```sh
# Source: rustup-init.sh pattern (github.com/rust-lang/rustup)
detect_platform() {
    local _ostype _cputype

    _ostype="$(uname -s)"
    _cputype="$(uname -m)"

    case "$_ostype" in
        Linux)
            _ostype="unknown-linux-gnu"
            ;;
        Darwin)
            _ostype="apple-darwin"
            # Detect true architecture (handles Rosetta)
            if [ "$_cputype" = "x86_64" ]; then
                if sysctl -n hw.optional.arm64 2>/dev/null | grep -q "1"; then
                    _cputype="aarch64"
                fi
            fi
            ;;
        MINGW* | MSYS* | CYGWIN*)
            _ostype="pc-windows-msvc"
            ;;
        *)
            echo "error: unsupported OS: $_ostype" >&2
            return 1
            ;;
    esac

    case "$_cputype" in
        x86_64 | amd64)
            _cputype="x86_64"
            ;;
        aarch64 | arm64)
            _cputype="aarch64"
            ;;
        *)
            echo "error: unsupported architecture: $_cputype" >&2
            return 1
            ;;
    esac

    echo "${_cputype}-${_ostype}"
}
```

### Checksum Verification in Install Script

```sh
# Source: Common pattern from rustup, uv, cargo-dist installers
verify_checksum() {
    local _file="$1"
    local _expected="$2"
    local _actual

    if command -v sha256sum > /dev/null 2>&1; then
        _actual="$(sha256sum "$_file" | cut -d' ' -f1)"
    elif command -v shasum > /dev/null 2>&1; then
        _actual="$(shasum -a 256 "$_file" | cut -d' ' -f1)"
    else
        echo "warning: no SHA-256 tool found, skipping verification" >&2
        return 0
    fi

    if [ "$_actual" != "$_expected" ]; then
        echo "error: checksum mismatch" >&2
        echo "  expected: $_expected" >&2
        echo "  actual:   $_actual" >&2
        return 1
    fi
}
```

### PATH Configuration (Multi-Shell)

```sh
# Source: Derived from rustup + uv install patterns
configure_path() {
    local _mesh_env="$HOME/.mesh/env"
    local _marker="# Mesh compiler"

    # Create env file
    cat > "$_mesh_env" << 'ENVEOF'
# Mesh compiler
# This file is sourced by shell profiles to add meshc to PATH
export PATH="$HOME/.mesh/bin:$PATH"
ENVEOF

    # Bash
    for _profile in "$HOME/.bashrc" "$HOME/.bash_profile"; do
        if [ -f "$_profile" ]; then
            if ! grep -q "$_marker" "$_profile" 2>/dev/null; then
                printf '\n%s\n. "%s"\n' "$_marker" "$_mesh_env" >> "$_profile"
            fi
            break  # Only modify one bash profile
        fi
    done

    # Zsh
    if [ -f "$HOME/.zshrc" ] || [ "$(basename "$SHELL")" = "zsh" ]; then
        local _zshrc="$HOME/.zshrc"
        if ! grep -q "$_marker" "$_zshrc" 2>/dev/null; then
            printf '\n%s\n. "%s"\n' "$_marker" "$_mesh_env" >> "$_zshrc"
        fi
    fi

    # Fish
    local _fish_config="$HOME/.config/fish/config.fish"
    if [ -f "$_fish_config" ] || command -v fish > /dev/null 2>&1; then
        mkdir -p "$(dirname "$_fish_config")"
        if ! grep -q "$_marker" "$_fish_config" 2>/dev/null; then
            printf '\n%s\nfish_add_path %s/.mesh/bin\n' "$_marker" "$HOME" >> "$_fish_config"
        fi
    fi
}
```

### SHA256SUMS Generation in Release Workflow

```yaml
# Source: Standard pattern from Helm, cargo-dist, and other release workflows
- name: Generate SHA256SUMS
  run: |
    cd artifacts/
    sha256sum *.tar.gz *.zip > SHA256SUMS
    cat SHA256SUMS
```

### Version Check for Idempotent Updates

```sh
# Source: Common pattern from install scripts
check_update_needed() {
    local _install_dir="$HOME/.mesh/bin"
    local _version_file="$HOME/.mesh/version"
    local _target_version="$1"

    if [ -f "$_version_file" ]; then
        local _current
        _current="$(cat "$_version_file")"
        if [ "$_current" = "$_target_version" ]; then
            echo "meshc $_current is already installed and up-to-date."
            return 1  # No update needed
        fi
        echo "Updating meshc from $_current to $_target_version..."
    fi
    return 0  # Update needed
}
```

### Windows PowerShell Install Script Pattern

```powershell
# Source: cargo-dist + uv PowerShell installer patterns
# Usage: powershell -ExecutionPolicy ByPass -c "irm https://mesh-lang.org/install.ps1 | iex"

$ErrorActionPreference = 'Stop'

$MeshHome = "$env:USERPROFILE\.mesh"
$BinDir = "$MeshHome\bin"

# Detect architecture
$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($Arch -ne "X64") {
    Write-Error "Unsupported architecture: $Arch"
    exit 1
}

$Target = "x86_64-pc-windows-msvc"
# ... download, verify, extract, update PATH ...

# Add to PATH via registry (persistent)
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$BinDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$BinDir;$UserPath", "User")
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `cross-rs` for all targets | Native ARM64 runners (`ubuntu-24.04-arm`) | Jan 2025 | Eliminates Docker/QEMU overhead for Linux ARM64 builds |
| `macos-13` for Intel builds | `macos-15-intel` for Intel builds | Oct 2025 (cargo-dist v0.30.1) | `macos-13` deprecated; `macos-15-intel` is the new Intel runner |
| `macos-latest` = macOS 14 | `macos-latest` = macOS 15 | Aug 2025 | Pin to specific versions in CI to avoid surprises |
| `apt.llvm.org/llvm.sh` | Direct tarball download | 2025 (LLVM 21) | llvm.sh has known bugs with LLVM 21 on Ubuntu 22.04/24.04 |
| musl 1.2.3 | musl 1.2.5 | Rust 1.93 (Jan 2026) | DNS improvements but requires `cargo update` for compat |
| `actions/upload-artifact@v3` | `@v4` | 2024 | v3 deprecated; v4 is current |
| GitHub manual checksums | Native SHA256 digests on release assets | Jun 2025 | GitHub now exposes digests; still generate SHA256SUMS file for curl-based verification |

**Deprecated/outdated:**
- `macos-13` runners: Deprecated by GitHub. Use `macos-14` (ARM64) or `macos-15-intel` (x86_64)
- `KyleMayes/install-llvm-action` for LLVM 21: Only supports up to LLVM 20. Would need `download-url` workaround
- `cargo-dist` for LLVM-dependent projects: Cannot easily handle LLVM pre-build setup (would need pre-build hooks which are experimental)

## Discretion Recommendations

### Install Location: `~/.mesh/bin`

**Recommendation:** Install to `~/.mesh/bin/meshc` with `~/.mesh/env` sourced from shell profiles.

**Rationale:**
- Follows the rustup convention (`~/.cargo/bin`) that Rust developers already know
- User-writable, no `sudo` required
- Works in rootless containers and CI environments
- Isolated from system package managers (no conflicts with Homebrew, apt, etc.)
- `~/.mesh/version` file enables cheap update checks without running `meshc --version`

### LLVM Linking Strategy: Static (`force-static`)

**Recommendation:** Use `inkwell/llvm21-1-force-static` feature for release builds to statically link LLVM.

**Rationale:**
- End users should never need to install LLVM. A `curl | sh` install must produce a working `meshc --version` immediately
- Static linking is the default preference in `llvm-sys` anyway (`prefer-static`)
- Binary size will be large (100-300MB before stripping) but this is standard for LLVM-based tools
- Use `strip` on release binaries to reduce size (50%+ reduction typical)
- Dynamic linking would require users to have the exact LLVM 21 shared libraries installed, which defeats the purpose of the install script

### Error Verbosity: Show context, not stack traces

**Recommendation:** On failure, show:
1. What failed (e.g., "Failed to download meshc binary")
2. The HTTP status code or system error
3. The URL that was attempted
4. A suggestion (e.g., "Check your internet connection" or "Try again with --version to specify a version")

Do NOT show: internal script variable state, full curl verbose output, or raw error dumps. Keep it to 3-5 lines.

### musl Static Linking: Use with custom allocator

**Recommendation:** For the `x86_64-unknown-linux-musl` target:
- Compile with `--target x86_64-unknown-linux-musl` (default `crt-static` behavior)
- Add `mimalloc` (or `jemalloc`) as the global allocator to avoid musl's slow allocator
- The musl target produces a fully static binary with zero glibc dependency
- Ideal for Alpine Linux, Docker scratch containers, and systems with old glibc versions
- `musl-tools` package needed on Ubuntu runner: `sudo apt-get install -y musl-tools`

### Windows Install Experience: PowerShell `irm | iex`

**Recommendation:** Provide `install.ps1` served at `mesh-lang.org/install.ps1` with usage:
```
powershell -ExecutionPolicy ByPass -c "irm https://mesh-lang.org/install.ps1 | iex"
```

**Rationale:**
- This is the standard pattern used by cargo-dist, uv, bun, and other modern CLI tools
- No MSI installer needed -- PowerShell script downloads `.zip`, extracts, and sets PATH
- `ExecutionPolicy ByPass` is required because default policy blocks scripts
- Install to `$env:USERPROFILE\.mesh\bin\meshc.exe`
- Set PATH via Windows Registry (`[Environment]::SetEnvironmentVariable`) for persistence

## Open Questions

1. **Do official `LLVM-` tarballs (non-`clang+llvm`) include `llvm-config` on Linux/macOS?**
   - What we know: On Windows, the distinction is clear -- `LLVM-` is toolchain-only, `clang+llvm-` includes dev tools. On non-Windows platforms, naming conventions differ per release.
   - What's unclear: Whether `LLVM-21.1.8-Linux-ARM64.tar.xz` and `LLVM-21.1.8-macOS-ARM64.tar.xz` include `llvm-config` and static `.a` libraries. The official docs only describe the Windows distinction clearly.
   - Recommendation: **Validate at plan time** by downloading one archive and checking contents. If they DO include dev tools, we can use official archives for all platforms. If not, use Homebrew for macOS and ycm-core or apt for Linux ARM64.

2. **Binary size after static LLVM linking**
   - What we know: LLVM is large; statically linked Rust+LLVM binaries can be 100-300+ MB before stripping
   - What's unclear: Actual size of meshc with LLVM 21 statically linked, and how much `strip` reduces it
   - Recommendation: Accept the size. Users download once. The install script can show a progress indication for large downloads.

3. **inkwell target features for reducing LLVM size**
   - What we know: inkwell supports `target-x86`, `target-aarch64`, etc. features that limit which LLVM backends are compiled in
   - What's unclear: Whether this requires LLVM to have been built with only those targets, or if it works with prebuilt all-target LLVM
   - Recommendation: Investigate during implementation. If inkwell target features can exclude unused backends at link time, this could significantly reduce binary size.

4. **GitHub Releases artifact limit and hosting install script**
   - What we know: mesh-lang.org is hosted via GitHub Pages (deploy.yml exists). Install script needs to be served from this domain.
   - What's unclear: Whether GitHub Pages can serve `install.sh` with the correct MIME type for `curl | sh`
   - Recommendation: Add `install.sh` and `install.ps1` to the website build. GitHub Pages serves all files; MIME type doesn't matter for `curl | sh` (curl fetches raw content).

## Sources

### Primary (HIGH confidence)
- [llvm-sys crate documentation](https://crates.io/crates/llvm-sys) - Linking strategy, env vars, version compatibility
- [inkwell crate](https://crates.io/crates/inkwell) - Feature flags for LLVM version and linking mode
- [LLVM 21.1.8 release](https://github.com/llvm/llvm-project/releases/tag/llvmorg-21.1.8) - Available prebuilt binary archives
- [GitHub Actions runner images](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) - Available runners, architectures
- [GitHub Linux ARM64 runners announcement](https://github.com/orgs/community/discussions/148648) - `ubuntu-24.04-arm` availability
- [rustup-init.sh source](https://github.com/rust-lang/rustup) - Platform detection, Rosetta handling, script structure patterns
- [Homebrew LLVM formula](https://formulae.brew.sh/formula/llvm) - `llvm@21` availability
- [Rust musl 1.2.5 update blog](https://blog.rust-lang.org/2025/12/05/Updating-musl-1.2.5/) - musl compatibility changes

### Secondary (MEDIUM confidence)
- [ycm-core/llvm releases](https://github.com/ycm-core/llvm/releases) - Community LLVM builds for all platforms (21.1.3)
- [cargo-dist documentation](https://axodotdev.github.io/cargo-dist/) - Install script patterns, PowerShell conventions
- [houseabsolute/actions-rust-cross](https://github.com/houseabsolute/actions-rust-cross) - Cross-compilation matrix patterns
- [Swatinem/rust-cache](https://github.com/Swatinem/rust-cache) - Cargo caching best practices
- [Apple Developer Forums](https://developer.apple.com/forums/thread/652667) - `sysctl hw.optional.arm64` for Rosetta detection
- [Flutter PR #67970](https://github.com/flutter/flutter/pull/67970) - Rosetta detection with sysctl

### Tertiary (LOW confidence)
- [llvm-project issue #150822](https://github.com/llvm/llvm-project/issues/150822) - llvm.sh failure on Ubuntu 24.04 (confirms apt approach is unreliable, but issue may be fixed by now)
- [savi-lang/llvm-static](https://github.com/savi-lang/llvm-static) - LLVM static build CI approach (reference only, uses Cirrus CI not GitHub Actions)
- [musl allocator performance](https://raniz.blog/2025-02-06_rust-musl-malloc/) - musl allocator slowness (single source, but well-documented)

## Metadata

**Confidence breakdown:**
- Standard stack (CI): HIGH - GitHub Actions patterns are well-established; native runners available for all targets
- Standard stack (LLVM in CI): MEDIUM - LLVM 21 prebuilt availability varies by platform; apt.llvm.org has known issues; needs runtime validation
- Architecture patterns: HIGH - Matrix build + release workflow is industry standard
- Install script: HIGH - rustup-init.sh provides a battle-tested reference implementation
- Pitfalls: HIGH - Well-documented issues from multiple sources
- LLVM static linking: MEDIUM - llvm-sys docs are clear but actual binary size and behavior needs validation

**Research date:** 2026-02-14
**Valid until:** 2026-03-14 (30 days -- CI runner and LLVM availability may change; pin specific versions)
