---
phase: 84-vscode-extension-publishing
plan: 02
subsystem: infra
tags: [vscode, marketplace, open-vsx, github-actions, ci-cd, publishing]

# Dependency graph
requires:
  - phase: 84-01-marketplace-metadata
    provides: "Marketplace-ready package.json, icon, README, CHANGELOG, .vscodeignore"
provides:
  - "GitHub Actions workflow for dual-registry publishing on ext-v* tags"
  - "Extension live on VS Code Marketplace as OpenWorthTechnologies.mesh-lang"
affects: []

# Tech tracking
tech-stack:
  added: ["HaaLeo/publish-vscode-extension@v2"]
  patterns: ["package-once-publish-twice VSIX workflow", "ext-v* tag prefix to avoid compiler release tag conflict"]

key-files:
  created:
    - ".github/workflows/publish-extension.yml"
  modified:
    - "editors/vscode-mesh/package.json"
    - "editors/vscode-mesh/README.md"

key-decisions:
  - "Publisher ID changed from mesh-lang to OpenWorthTechnologies to match actual VS Code Marketplace publisher"
  - "Open VSX publish step uses continue-on-error to avoid blocking Marketplace publish when token unavailable"
  - "VSIX packaged in separate step with path via step output instead of glob for reliability"

patterns-established:
  - "ext-v* tag prefix separates extension releases from compiler releases"
  - "Package VSIX once, publish to both registries from same artifact"

# Metrics
duration: ~15min
completed: 2026-02-14
---

# Phase 84 Plan 02: Publish to Marketplace Summary

**GitHub Actions dual-registry publish workflow on ext-v* tags with extension live on VS Code Marketplace**

## Performance

- **Duration:** ~15 min (including checkpoint for human verification and publishing)
- **Started:** 2026-02-14T16:51:00Z
- **Completed:** 2026-02-14T17:22:00Z
- **Tasks:** 2 (1 auto + 1 checkpoint)
- **Files modified:** 3

## Accomplishments
- GitHub Actions workflow publishing to both VS Code Marketplace and Open VSX Registry
- Package-once-publish-twice pattern ensures identical VSIX artifacts on both registries
- ext-v* tag convention avoids conflict with compiler v* release tags
- Extension successfully published and live on VS Code Marketplace via `ext-v0.2.0` tag

## Task Commits

Each task was committed atomically:

1. **Task 1: Create dual-registry publish workflow** - `0154d121` (feat)
2. **Task 2: Verify extension packaging and review publish workflow** - checkpoint (human-verify, approved)

Post-checkpoint fixes (applied during publishing):
- `0063739d` - fix: update publisher ID to OpenWorthTechnologies
- `920a2288` - fix: decouple publish steps, make Open VSX non-blocking
- `9c885b0d` - fix: resolve VSIX path via step output instead of glob

## Files Created/Modified
- `.github/workflows/publish-extension.yml` - Dual-registry publish workflow triggered on ext-v* tags
- `editors/vscode-mesh/package.json` - Publisher ID corrected to OpenWorthTechnologies
- `editors/vscode-mesh/README.md` - Badge URLs updated for correct publisher ID

## Decisions Made
- Publisher ID changed from mesh-lang to OpenWorthTechnologies -- the actual publisher account created on VS Code Marketplace used this organization name
- Open VSX publish step set to continue-on-error: true -- avoids blocking VS Code Marketplace publish when Open VSX token is not configured or namespace not claimed
- VSIX packaged in a dedicated step with path emitted via GITHUB_OUTPUT -- more reliable than glob matching or relying on the HaaLeo action's internal packaging

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Publisher ID mismatch**
- **Found during:** Task 2 checkpoint (publishing)
- **Issue:** package.json had publisher "mesh-lang" but the actual VS Code Marketplace publisher was created as "OpenWorthTechnologies"
- **Fix:** Updated publisher field in package.json and badge URLs in README.md
- **Files modified:** editors/vscode-mesh/package.json, editors/vscode-mesh/README.md
- **Verification:** Workflow succeeded after fix
- **Committed in:** `0063739d`

**2. [Rule 1 - Bug] Open VSX failure blocking Marketplace publish**
- **Found during:** Task 2 checkpoint (publishing)
- **Issue:** Open VSX publish step failed (no token/namespace configured) and blocked the VS Code Marketplace step
- **Fix:** Separated packaging into its own step; added continue-on-error: true to Open VSX step
- **Files modified:** .github/workflows/publish-extension.yml
- **Verification:** Workflow completed successfully with Open VSX step gracefully failing
- **Committed in:** `920a2288`

**3. [Rule 3 - Blocking] VSIX path resolution failure**
- **Found during:** Task 2 checkpoint (publishing)
- **Issue:** Glob pattern for VSIX path did not resolve correctly in the workflow
- **Fix:** Used explicit step output (echo to GITHUB_OUTPUT) to pass VSIX filename between steps
- **Files modified:** .github/workflows/publish-extension.yml
- **Verification:** Workflow completed successfully, VSIX published to Marketplace
- **Committed in:** `9c885b0d`

---

**Total deviations:** 3 auto-fixed (2 bugs, 1 blocking)
**Impact on plan:** All fixes were necessary for successful publishing. The plan's original architecture was sound; fixes addressed real-world account naming and CI environment differences.

## Issues Encountered
- The original plan assumed the HaaLeo action's built-in packaging + output chaining would work seamlessly, but separating packaging into its own step proved more reliable in practice.

## User Setup Required

GitHub repository secrets were configured by the user during the checkpoint:
- `VS_MARKETPLACE_TOKEN` - Azure DevOps PAT with Marketplace manage scope
- Extension published via `ext-v0.2.0` tag push

## Next Phase Readiness
- Extension is live on VS Code Marketplace
- Publishing workflow is fully automated for future releases
- Phase 84 (VS Code Extension Publishing) is complete
- Ready to proceed to next phase in v8.0 Developer Tooling milestone

## Self-Check: PASSED

All 3 files verified present. All 4 task commits (0154d121, 0063739d, 920a2288, 9c885b0d) confirmed in git log.

---
*Phase: 84-vscode-extension-publishing*
*Completed: 2026-02-14*
