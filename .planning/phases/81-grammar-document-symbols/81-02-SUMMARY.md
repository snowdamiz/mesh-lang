---
phase: 81-grammar-document-symbols
plan: 02
subsystem: lsp
tags: [lsp, document-symbols, outline, breadcrumbs, tower-lsp, rowan, cst]

# Dependency graph
requires:
  - phase: 80-lsp-foundation
    provides: "LSP server with hover, diagnostics, go-to-definition, offset conversion"
provides:
  - "textDocument/documentSymbol handler returning hierarchical DocumentSymbolResponse::Nested"
  - "collect_symbols CST walker covering 11 definition types + call/cast handlers"
  - "VS Code Outline panel, Breadcrumbs, and Cmd+Shift+O support for Mesh files"
affects: [83-completion, 84-semantic-tokens]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "CST walk for symbol collection using SyntaxKind matching"
    - "tree_to_source_offset for rowan-to-source coordinate conversion in symbol ranges"
    - "override_name pattern for IMPL_DEF nodes without NAME children"

key-files:
  created: []
  modified:
    - "crates/mesh-lsp/src/server.rs"

key-decisions:
  - "Used DocumentSymbolResponse::Nested for hierarchical symbol tree (not flat SymbolInformation)"
  - "IMPL_DEF named as 'impl TraitName' from PATH child (not unnamed)"
  - "CALL_HANDLER and CAST_HANDLER included as FUNCTION symbols inside service bodies"
  - "ACTOR_DEF, SERVICE_DEF, SUPERVISOR_DEF all mapped to SymbolKind::CLASS"

patterns-established:
  - "collect_symbols + make_symbol pattern for CST-to-LSP symbol extraction"
  - "Container recursion: MODULE_DEF, ACTOR_DEF, SERVICE_DEF, INTERFACE_DEF, IMPL_DEF recurse into BLOCK children"

# Metrics
duration: 3min
completed: 2026-02-14
---

# Phase 81 Plan 02: Document Symbols Summary

**Hierarchical textDocument/documentSymbol handler with CST walk covering 11 definition types, proper rowan-to-source offset conversion, and recursive container nesting**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-14T08:18:44Z
- **Completed:** 2026-02-14T08:21:18Z
- **Tasks:** 1
- **Files modified:** 1

## Accomplishments
- LSP server now advertises and handles textDocument/documentSymbol requests
- Hierarchical symbol tree for VS Code Outline panel with proper nesting (functions inside modules, methods inside interfaces, etc.)
- All 11 Mesh definition types mapped to correct SymbolKind constants: FN_DEF->FUNCTION, STRUCT_DEF->STRUCT, MODULE_DEF->MODULE, ACTOR_DEF->CLASS, SERVICE_DEF->CLASS, SUPERVISOR_DEF->CLASS, INTERFACE_DEF->INTERFACE, IMPL_DEF->OBJECT, LET_BINDING->VARIABLE, SUM_TYPE_DEF->ENUM, TYPE_ALIAS_DEF->TYPE_PARAMETER
- Correct offset conversion using tree_to_source_offset for both range and selection_range
- IMPL_DEF blocks extract trait name from PATH child for display as "impl TraitName"

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement document_symbol handler with hierarchical CST walk** - `16141d5f` (feat)

**Plan metadata:** pending (docs: complete plan)

## Files Created/Modified
- `crates/mesh-lsp/src/server.rs` - Added document_symbol method, collect_symbols CST walker, make_symbol range converter, extract_impl_name helper, capability advertisement, and test assertion

## Decisions Made
- Used DocumentSymbolResponse::Nested (not flat SymbolInformation) for proper hierarchy in VS Code Outline
- Mapped ACTOR_DEF, SERVICE_DEF, SUPERVISOR_DEF all to SymbolKind::CLASS since LSP has no dedicated actor/service/supervisor kinds
- IMPL_DEF uses "impl TraitName" format from PATH child since it has no NAME child
- Added CALL_HANDLER and CAST_HANDLER as FUNCTION symbols to show service handlers in the Outline (deviation Rule 2)
- Selection ranges use NAME node for most types, PATH node for IMPL_DEF
- Gracefully skip symbols where offset conversion returns None rather than panicking

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Added CALL_HANDLER and CAST_HANDLER symbol support**
- **Found during:** Task 1 (collect_symbols implementation)
- **Issue:** Plan listed 11 definition types but did not include CALL_HANDLER and CAST_HANDLER, which are the primary definitions inside SERVICE_DEF blocks. Without these, the service body would appear empty in the Outline panel.
- **Fix:** Added match arms for SyntaxKind::CALL_HANDLER and SyntaxKind::CAST_HANDLER as SymbolKind::FUNCTION symbols in collect_symbols
- **Files modified:** crates/mesh-lsp/src/server.rs
- **Verification:** cargo build and cargo test pass
- **Committed in:** 16141d5f (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 missing critical)
**Impact on plan:** Essential for service body symbols to appear in Outline. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Document symbol support is complete and tested
- SYM-01 (hierarchical symbols), SYM-02 (kind mapping), SYM-03 (range computation) satisfied
- Ready for Phase 82 (Install) or Phase 83 (Completion) which may build on the CST walking patterns established here

## Self-Check: PASSED

- [x] crates/mesh-lsp/src/server.rs exists
- [x] 81-02-SUMMARY.md exists
- [x] Commit 16141d5f exists in git log
- [x] All 11 SyntaxKind types handled in collect_symbols (verified: 11 matches)
- [x] document_symbol_provider capability advertised and tested
- [x] tree_to_source_offset used for offset conversion (8 call sites)
- [x] cargo build -p mesh-lsp succeeds
- [x] cargo test -p mesh-lsp passes (31/31 tests)

---
*Phase: 81-grammar-document-symbols*
*Completed: 2026-02-14*
