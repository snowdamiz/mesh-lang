---
phase: 82-install-infrastructure
plan: 02
subsystem: infra
tags: [shell, powershell, install-script, curl-sh, platform-detection, checksum]

# Dependency graph
requires:
  - phase: none
    provides: N/A (standalone install scripts)
provides:
  - POSIX shell install script for macOS/Linux (install/install.sh)
  - PowerShell install script for Windows (install/install.ps1)
  - Website-served copies at mesh-lang.org/install.sh and mesh-lang.org/install.ps1
affects: [82-01-ci-pipeline, release-workflow, documentation]

# Tech tracking
tech-stack:
  added: []
  patterns: [curl-sh-install, main-wrapper-pattern, marker-based-idempotency, env-file-path-config]

key-files:
  created:
    - install/install.sh
    - install/install.ps1
    - website/docs/public/install.sh
    - website/docs/public/install.ps1
  modified: []

key-decisions:
  - "Install location ~/.mesh/bin with ~/.mesh/env sourced from shell profiles (rustup convention)"
  - "Marker-based idempotent PATH configuration using '# Mesh compiler' comment"
  - "Fish shell uses fish_add_path instead of export PATH (correct fish syntax)"
  - "PowerShell uses Windows Registry for persistent PATH (SetEnvironmentVariable User scope)"
  - "Checksum verification gracefully degrades (warns and continues if tools unavailable)"

patterns-established:
  - "curl|sh safety: entire script wrapped in main() called at end"
  - "Multi-shell PATH config: env file + marker-based profile modification for bash/zsh/fish"
  - "Version pinning: --version flag reads VERSION_FILE for skip-if-current logic"
  - "Platform detection: uname + Rosetta sysctl check for accurate Apple Silicon detection"

# Metrics
duration: 3min
completed: 2026-02-14
---

# Phase 82 Plan 02: Install Scripts Summary

**POSIX and PowerShell install scripts with platform detection, SHA-256 verification, and multi-shell PATH configuration for curl|sh one-liner installation**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-14T15:48:57Z
- **Completed:** 2026-02-14T15:51:35Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- POSIX shell install script handling macOS (x86_64/ARM64 with Rosetta detection) and Linux (x86_64/ARM64)
- SHA-256 checksum verification with automatic tool detection (sha256sum on Linux, shasum on macOS)
- Idempotent PATH configuration for bash, zsh, and fish shells using marker comments
- PowerShell install script for Windows with registry-based PATH persistence
- Scripts deployed to website/docs/public/ for serving at mesh-lang.org

## Task Commits

Each task was committed atomically:

1. **Task 1: Create POSIX shell install script** - `0d982176` (feat)
2. **Task 2: Create PowerShell install script and deploy to website** - `a7035017` (feat)

## Files Created/Modified
- `install/install.sh` - POSIX shell install script for macOS and Linux (~270 lines)
- `install/install.ps1` - PowerShell install script for Windows (~210 lines)
- `website/docs/public/install.sh` - Website-served copy of POSIX script
- `website/docs/public/install.ps1` - Website-served copy of PowerShell script

## Decisions Made
- Used `~/.mesh/bin` as install location with `~/.mesh/env` sourced from shell profiles, following rustup convention
- Marker-based idempotent PATH configuration using `# Mesh compiler` comment to prevent duplicate entries on re-run
- Fish shell handled separately with `fish_add_path` instead of `export PATH` (correct fish syntax)
- PowerShell uses Windows Registry `[Environment]::SetEnvironmentVariable` for persistent PATH changes
- Checksum verification gracefully degrades -- warns and continues if no SHA tool available
- Direct file copies to website/docs/public/ rather than symlinks for portability

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
- shellcheck and pwsh not available on host system; verification relied on `sh -n` syntax check and manual function/feature verification instead

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Install scripts are ready for use once the CI pipeline (Plan 01) creates GitHub Releases with the expected artifact naming convention
- Website copies will be served automatically once deployed via existing deploy.yml workflow
- Scripts expect tarball naming pattern: `meshc-v{VERSION}-{PLATFORM}.tar.gz` and `SHA256SUMS` file in releases

## Self-Check: PASSED

All 4 files verified on disk. Both commit hashes (0d982176, a7035017) confirmed in git log.

---
*Phase: 82-install-infrastructure*
*Completed: 2026-02-14*
