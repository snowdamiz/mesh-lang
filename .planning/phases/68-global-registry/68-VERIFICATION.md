---
phase: 68-global-registry
verified: 2026-02-13T08:15:00Z
status: passed
score: 5/5 must-haves verified
re_verification: false
---

# Phase 68: Global Registry Verification Report

**Phase Goal:** Processes can be registered by name across the entire cluster and looked up from any node

**Verified:** 2026-02-13T08:15:00Z

**Status:** passed

**Re-verification:** No - initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | When a new node connects, both sides exchange their global registry snapshots and merge them | ✓ VERIFIED | `send_global_sync` called at lines 2209 (server accept) and 2417 (client connect) in node.rs, right after `send_peer_list` |
| 2 | After sync exchange, both nodes have the union of all globally registered names | ✓ VERIFIED | `merge_snapshot` called by DIST_GLOBAL_SYNC handler (line 984-1022 in node.rs), idempotent merge tested in `test_merge_snapshot_idempotent` |
| 3 | Duplicate names in sync are handled idempotently (skip already-registered names) | ✓ VERIFIED | `test_merge_snapshot_skips_existing_names` (line 550-565) proves existing names preserved, new snapshot entries skipped |
| 4 | Wire format roundtrip tests prove DIST_GLOBAL_REGISTER, DIST_GLOBAL_UNREGISTER, and DIST_GLOBAL_SYNC encode/decode correctly | ✓ VERIFIED | 4 wire format tests pass: `test_dist_global_register_wire_format`, `test_dist_global_unregister_wire_format`, `test_dist_global_sync_wire_format`, `test_dist_global_sync_empty` |
| 5 | GlobalRegistry unit tests cover register, whereis, unregister, cleanup_node, cleanup_process, snapshot, and merge_snapshot | ✓ VERIFIED | 18 tests pass covering all operations including edge cases (duplicates, cleanup, concurrency, idempotent merge) |

**Score:** 5/5 truths verified

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/snow-rt/src/dist/node.rs` | send_global_sync function called after send_peer_list on new connections | ✓ VERIFIED | Lines 2209 and 2417 both call `crate::dist::global::send_global_sync(&session)` immediately after `send_peer_list(&session)` |
| `crates/snow-rt/src/dist/global.rs` | Unit tests for GlobalRegistry and wire format roundtrip tests | ✓ VERIFIED | 18 tests present (lines 334-770), all pass. Includes 14 data structure tests and 4 wire format roundtrip tests |

### Key Link Verification

| From | to | Via | Status | Details |
|------|-----|-----|--------|---------|
| `crates/snow-rt/src/dist/node.rs` (accept loop) | send_global_sync | Called right after send_peer_list in server accept path | ✓ WIRED | Line 2209: `crate::dist::global::send_global_sync(&session);` immediately follows line 2208: `send_peer_list(&session);` |
| `crates/snow-rt/src/dist/node.rs` (connect) | send_global_sync | Called right after send_peer_list in client connect path | ✓ WIRED | Line 2417: `crate::dist::global::send_global_sync(&session);` immediately follows line 2416: `send_peer_list(&session);` |
| `crates/snow-rt/src/dist/node.rs` | `crates/snow-rt/src/dist/global.rs` | send_global_sync calls global_name_registry().snapshot() and writes DIST_GLOBAL_SYNC to session | ✓ WIRED | Lines 303-327 in global.rs: `send_global_sync` gets registry, takes snapshot (line 305), builds DIST_GLOBAL_SYNC payload (lines 311-323), writes to session stream (line 326) |

### Requirements Coverage

| Requirement | Status | Blocking Issue |
|-------------|--------|----------------|
| CLUST-01: User can register a name globally with `Global.register(name, pid)` visible across all nodes | ✓ SATISFIED | Runtime APIs (snow_global_register line 1291 in actor/mod.rs), compiler integration (Global module in typeck line 803-819 in infer.rs), and broadcast on register (broadcast_global_register in global.rs) all present and wired |
| CLUST-02: User can look up global names with `Global.whereis(name)` returning PID from any node | ✓ SATISFIED | Runtime API (snow_global_whereis line 1337 in actor/mod.rs), compiler integration (Global.whereis in typeck line 810-813), local-only lookup with replicated data (whereis line 97-100 in global.rs) all verified |
| CLUST-03: Global registrations are cleaned up automatically when owning node disconnects | ✓ SATISFIED | cleanup_node called in handle_node_disconnect (line 1297 in node.rs), cleanup_process called in handle_process_exit (line 674 in scheduler.rs), both broadcast unregister messages to remaining nodes |

### Anti-Patterns Found

None. All implementations are substantive and properly wired.

### Human Verification Required

None. All automated checks passed and phase goal is fully testable via unit tests.

### Success Criteria Verification

**From ROADMAP.md:**

1. **User can register a process globally with `Global.register(name, pid)` and the name is visible from all connected nodes**
   - ✓ VERIFIED: Runtime API exports snow_global_register (lib.rs line 48), compiler has Global.register type signature (infer.rs line 805-808), MIR lowers to snow_global_register (lower.rs line 9542), codegen unpacks string and emits correct LLVM call (expr.rs line 1985-2010), runtime broadcasts DIST_GLOBAL_REGISTER to all sessions (global.rs line 223-245)

2. **User can look up a globally registered name with `Global.whereis(name)` from any node and get back the correct PID**
   - ✓ VERIFIED: Runtime API exports snow_global_whereis (lib.rs line 48), compiler has Global.whereis type signature (infer.rs line 810-813), MIR and codegen wired (lower.rs, expr.rs line 697-699), whereis is local-only (no network call) because registry is fully replicated (global.rs line 97-100 comment "Always local")

3. **When a node disconnects, all global registrations owned by processes on that node are automatically cleaned up**
   - ✓ VERIFIED: handle_node_disconnect calls cleanup_node (node.rs line 1297), cleanup_node removes all names for that node (global.rs line 134-157), removed names are broadcast as DIST_GLOBAL_UNREGISTER (node.rs line 1298-1300), cleanup_process also called on process exit (scheduler.rs line 674-678)

---

_Verified: 2026-02-13T08:15:00Z_  
_Verifier: Claude (gsd-verifier)_
