---
phase: 82-install-infrastructure
plan: 01
subsystem: infra
tags: [github-actions, ci, llvm, mimalloc, musl, release, matrix-build]

# Dependency graph
requires:
  - phase: none
    provides: existing meshc binary crate and workspace Cargo.toml
provides:
  - GitHub Actions release workflow building meshc for 6 targets
  - mimalloc global allocator for musl target performance
  - SHA256SUMS checksums for release verification
affects: [82-02-install-script, website-hosting]

# Tech tracking
tech-stack:
  added: [mimalloc, github-actions-matrix, softprops/action-gh-release, Swatinem/rust-cache, actions/cache]
  patterns: [native-runner-per-target, two-phase-build-release, llvm-tarball-caching]

key-files:
  created:
    - .github/workflows/release.yml
  modified:
    - Cargo.toml
    - crates/meshc/Cargo.toml
    - crates/meshc/src/main.rs

key-decisions:
  - "LLVM 21 installed via Homebrew on macOS, tarball on Linux/Windows -- avoids broken apt.llvm.org"
  - "LLVM tarballs cached via actions/cache keyed on target to avoid re-downloading each build"
  - "No inkwell/llvm21-1-force-static feature flag -- default prefer-static behavior used initially"

patterns-established:
  - "Native runner per target: no cross-compilation, each platform builds natively"
  - "Two-phase workflow: matrix build job uploads artifacts, release job collects and publishes"
  - "cfg-guarded global allocator: musl-only code with zero impact on other targets"

# Metrics
duration: 3min
completed: 2026-02-14
---

# Phase 82 Plan 01: Release CI Pipeline Summary

**GitHub Actions 6-target matrix build workflow with LLVM 21 per-platform installation, mimalloc musl allocator, and tag-triggered GitHub Releases with SHA256SUMS**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-14T15:48:54Z
- **Completed:** 2026-02-14T15:51:24Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Added mimalloc as cfg-guarded global allocator for musl target to prevent 10x perf regression
- Created comprehensive release workflow building meshc on all 6 targets using native runners
- LLVM 21 installation automated per platform: Homebrew (macOS), official tarballs (Linux x86_64, Windows), ycm-core (Linux ARM64)
- LLVM download caching prevents re-downloading on every CI run
- Tag pushes create GitHub Releases with .tar.gz/.zip archives and SHA256SUMS file

## Task Commits

Each task was committed atomically:

1. **Task 1: Add mimalloc global allocator for musl target** - `e97d4aa5` (feat)
2. **Task 2: Create GitHub Actions release workflow with 6-target matrix build** - `9f5c2dee` (feat)

## Files Created/Modified
- `Cargo.toml` - Added mimalloc to workspace dependencies
- `crates/meshc/Cargo.toml` - Added target-specific musl dependency for mimalloc
- `crates/meshc/src/main.rs` - Added cfg-guarded global_allocator static for musl
- `.github/workflows/release.yml` - Full CI pipeline: 6-target matrix build + release job

## Decisions Made
- LLVM 21 installed via Homebrew on macOS (both architectures), official tarball on Linux x86_64/Windows, ycm-core community build on Linux ARM64 -- avoids broken apt.llvm.org script
- LLVM tarballs cached with actions/cache keyed on LLVM version + target to speed up CI
- Did not add `inkwell/llvm21-1-force-static` feature flag -- the default `prefer-static` behavior in llvm-sys should work when static libraries are available in the prefix directory; flag can be added if needed after first CI run
- Windows LLVM extraction uses 7z (pre-installed on windows-latest) to handle .tar.xz format

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required. The workflow will activate automatically when pushed to the repository.

## Next Phase Readiness
- Release workflow is ready to build meshc on all 6 targets once pushed
- GitHub Releases will be created automatically on tag pushes (v*)
- Plan 82-02 (install script) can reference the release artifact URLs created by this workflow

## Self-Check: PASSED

- [x] `.github/workflows/release.yml` exists
- [x] `82-01-SUMMARY.md` exists
- [x] Commit `e97d4aa5` (Task 1) exists
- [x] Commit `9f5c2dee` (Task 2) exists
- [x] mimalloc in workspace Cargo.toml
- [x] global_allocator in main.rs
- [x] musl target dep in meshc Cargo.toml

---
*Phase: 82-install-infrastructure*
*Completed: 2026-02-14*
