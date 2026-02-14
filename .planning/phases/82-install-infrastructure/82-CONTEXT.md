# Phase 82: Install Infrastructure - Context

**Gathered:** 2026-02-14
**Status:** Ready for planning

<domain>
## Phase Boundary

Users can install meshc on macOS, Linux, or Windows with a single `curl | sh` command (or equivalent) backed by prebuilt binaries from GitHub Releases. CI pipeline produces binaries for all supported targets. Install script handles platform detection, checksum verification, PATH configuration, updates, and uninstall.

</domain>

<decisions>
## Implementation Decisions

### Script UX & output
- Minimal output style — just what was installed, where, and the version. No banner or ASCII art
- Subtle colors: green for success, red for errors, bold for emphasis. Respect NO_COLOR and piped output
- No interactive prompts — `curl | sh` installs immediately. No confirmation step needed (user already chose to run it)

### Update & versioning
- Re-running the install script checks the installed version against latest. Skips if already up-to-date, updates if newer is available
- Supports `--version 0.2.0` flag to install a specific version (for CI pinning). Default is latest
- No `meshc self-update` command — users re-run the install script to update
- Supports `--uninstall` flag to remove meshc and clean up PATH changes

### CI build targets
- 6 targets: x86_64-apple-darwin, aarch64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-unknown-linux-musl, x86_64-pc-windows-msvc
- PR pushes build all targets (catch build failures early). Only tag pushes (e.g., v0.2.0) create GitHub Releases
- Release artifacts: .tar.gz per target (.zip for Windows) + SHA256SUMS file. No auto-generated release notes

### Claude's Discretion
- Install location (~/.mesh/bin, /usr/local/bin, etc.) and PATH configuration method
- LLVM linking strategy (static vs dynamic) — pick what gives the best user experience
- Error verbosity on failure — pick appropriate level of diagnostic detail
- musl static linking details
- Windows install experience (PowerShell equivalent of curl | sh, or .msi, etc.)

</decisions>

<specifics>
## Specific Ideas

No specific requirements — open to standard approaches. The success criteria from the roadmap are the reference: `curl -sSf https://mesh-lang.org/install.sh | sh` should just work on a clean system, with auto-detection, checksum verification, and working `meshc --version` after install.

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 82-install-infrastructure*
*Context gathered: 2026-02-14*
