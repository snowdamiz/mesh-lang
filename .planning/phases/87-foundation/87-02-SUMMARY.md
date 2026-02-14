---
phase: 87-foundation
plan: 02
subsystem: services
tags: [mesh, services, actor, batch-writer, postgresql, jsonb, retry]

# Dependency graph
requires:
  - phase: 87-foundation plan 01
    provides: data type structs, PostgreSQL schema DDL, query helper functions
provides:
  - OrgService with 3 call handlers (create, get, list orgs)
  - ProjectService with 6 call handlers (CRUD, API key management)
  - UserService with 7 call handlers (auth, sessions, membership)
  - StorageWriter service with 2 cast handlers (Store, Flush)
  - flush_ticker actor for periodic timer-based flush
  - insert_event helper with PostgreSQL jsonb server-side parsing
  - Batch flush with 3-retry exponential backoff
affects: [88-ingestion-pipeline, 89-alerting, 90-dashboard]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Service-per-entity with PoolHandle state"
    - "Per-project StorageWriter with bounded buffer"
    - "Timer actor pattern (recursive sleep + cast) for periodic flush"
    - "JSON string buffer with PostgreSQL server-side jsonb parsing"
    - "Exponential backoff retry (100ms, 500ms) with drop-on-failure"

key-files:
  created:
    - mesher/main.mpl
    - mesher/storage/writer.mpl
  modified:
    - mesher/storage/queries.mpl
    - mesher/storage/schema.mpl
    - mesher/types/event.mpl
    - mesher/types/issue.mpl
    - mesher/types/project.mpl
    - mesher/types/user.mpl
    - mesher/types/alert.mpl
    - crates/mesh-typeck/src/infer.rs
    - crates/mesh-codegen/src/codegen/expr.rs
    - crates/mesh-codegen/src/mir/lower.rs

key-decisions:
  - "All services in main.mpl -- Mesh cannot export services cross-module (ModuleExports lacks ServiceDef)"
  - "Explicit case matching instead of ? operator -- LLVM codegen bug produces wrong return type with ? in Result functions"
  - "Err(_) instead of Err(e) binding -- LLVM codegen produces non-dominating alloca for error variable in case arms"
  - "JSON string buffer (List<String>) instead of Map -- polymorphic type variables cannot cross module boundaries (type var IDs are module-scoped)"
  - "Timer actor pattern instead of Timer.send_after -- raw bytes from send_after don't match service dispatch type tags"
  - "Removed flush_interval from WriterState struct -- interval is owned by the external flush_ticker actor"

patterns-established:
  - "Service delegate pattern: service handlers call standalone helper functions to keep handler bodies minimal"
  - "Cross-module workaround: only import functions with fully concrete (non-polymorphic) parameter types"
  - "Result handling: use explicit case/match instead of ? operator in non-trivial functions"

# Metrics
duration: 60min
completed: 2026-02-14
---

# Phase 87 Plan 02: Service Layer Summary

**4 Mesh services (16 call + 2 cast handlers) with per-project batch writer, timer-based flush, and 3-retry exponential backoff -- compiles to native binary**

## Performance

- **Duration:** ~60 min
- **Started:** 2026-02-14T22:00:00Z
- **Completed:** 2026-02-14T23:06:00Z
- **Tasks:** 2 (combined into single commit due to single-file constraint)
- **Files created:** 2

## Accomplishments
- 4 services totaling 18 handlers (16 call + 2 cast) that compile to a native binary
- StorageWriter with bounded buffer (50 batch / 500 max), drop-oldest backpressure, and dual flush triggers (size + timer)
- Discovered and worked around 3 Mesh compiler bugs: ? operator LLVM codegen, Err(e) domination, polymorphic cross-module type variables
- PostgreSQL jsonb server-side parsing for event ingestion (no Map deserialization needed in Mesh)

## Task Commits

Tasks 1 and 2 were combined into a single commit because all services share `mesher/main.mpl` (services cannot be in separate files):

1. **Tasks 1+2: Entity services + StorageWriter + main entry** - `19af21f7` (feat)

## Files Created/Modified
- `mesher/main.mpl` - All 4 services (Org, Project, User, StorageWriter), flush/retry logic, buffer helpers, flush_ticker actor, main entry point (321 lines)
- `mesher/storage/writer.mpl` - insert_event helper using PostgreSQL jsonb extraction (16 lines)
- `mesher/storage/queries.mpl` - Corrected: PoolHandle types, Map-based row construction, Pool.execute intermediate binding
- `mesher/storage/schema.mpl` - Corrected: removed module declaration
- `mesher/types/*.mpl` - Corrected: removed module declarations, Option<String> to String, module to module_name
- `crates/mesh-typeck/src/infer.rs` - Fix: resolve_type_annotation for generic parameter types (List<String>)
- `crates/mesh-codegen/src/codegen/expr.rs` - Fix: service struct state type detection (pointer vs i64)
- `crates/mesh-codegen/src/mir/lower.rs` - Fix: MIR lowering for service dispatch with struct state

## Decisions Made
1. **All services in main.mpl:** Mesh's ModuleExports does not include ServiceDef. Services register methods locally in the typechecker and cannot be imported cross-module. All 4 services must coexist in main.mpl.
2. **No ? operator:** The ? operator's LLVM codegen produces `ret ptr %call` when the function return type is `{ i8, ptr }` (Result). Replaced all ? usage with explicit case/match.
3. **Err(_) not Err(e):** The Err(e) binding in case arms produces LLVM alloca instructions that don't dominate their uses. Using Err(_) avoids the variable binding entirely.
4. **JSON string buffer:** Functions with inferred (polymorphic) type parameters cannot be imported cross-module. The typechecker's type variable IDs are module-scoped, causing an index-out-of-bounds panic in the unification table. Only functions with fully concrete parameter types can be exported. Chose List<String> buffer to keep insert_event exportable.
5. **Timer actor pattern:** Timer.send_after delivers raw bytes that cannot match service cast dispatch type tags (u64-based switch). Used a separate flush_ticker actor with Timer.sleep + recursive call instead.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed Login handler LLVM codegen crash**
- **Found during:** Task 1 (UserService implementation)
- **Issue:** login_user function using ? operator produced LLVM verification error: function return type mismatch (ret ptr vs { i8, ptr })
- **Fix:** Replaced ? with explicit case auth_result do Ok(user) -> ... | Err(_) -> Err("authentication failed") end
- **Files modified:** mesher/main.mpl
- **Verification:** meshc build mesher/ compiles successfully
- **Committed in:** 19af21f7

**2. [Rule 1 - Bug] Fixed Err(e) LLVM alloca domination**
- **Found during:** Task 2 (start_services function)
- **Issue:** Err(e) variable binding in case arms produced non-dominating alloca: %e = alloca ptr not dominated by %e18 = load ptr
- **Fix:** Changed all Err(e) to Err(_), removed error string interpolation in error messages
- **Files modified:** mesher/main.mpl
- **Verification:** meshc build mesher/ compiles successfully
- **Committed in:** 19af21f7

**3. [Rule 3 - Blocking] Moved services from separate files to main.mpl**
- **Found during:** Task 1 (service file creation)
- **Issue:** Mesh ModuleExports does not include ServiceDef; services cannot be imported cross-module
- **Fix:** Moved all 4 services to main.mpl, deleted mesher/services/*.mpl
- **Files modified:** mesher/main.mpl, deleted mesher/services/org_service.mpl, mesher/services/project_service.mpl, mesher/services/user_service.mpl
- **Verification:** meshc build mesher/ compiles successfully
- **Committed in:** 19af21f7

**4. [Rule 3 - Blocking] Changed buffer type from Map to JSON strings**
- **Found during:** Task 2 (StorageWriter implementation)
- **Issue:** Functions with polymorphic (inferred) type parameters cause typechecker panic when imported cross-module (type variable ID out of bounds in unification table)
- **Fix:** Changed buffer from List<Map<String, String>> to List<String>, events stored as JSON strings. PostgreSQL parses JSON server-side using jsonb extraction operators.
- **Files modified:** mesher/main.mpl, mesher/storage/writer.mpl
- **Verification:** meshc build mesher/ compiles successfully
- **Committed in:** 19af21f7

**5. [Rule 3 - Blocking] Used timer actor instead of Timer.send_after**
- **Found during:** Task 2 (flush timer implementation)
- **Issue:** Timer.send_after delivers raw bytes as actor messages; service dispatch expects messages with u64 type_tag headers. Raw string bytes produce garbage type tags that don't match any handler.
- **Fix:** Created flush_ticker actor that uses Timer.sleep + recursive call + StorageWriter.flush(writer_pid) cast
- **Files modified:** mesher/main.mpl
- **Verification:** meshc build mesher/ compiles successfully
- **Committed in:** 19af21f7

---

**Total deviations:** 5 auto-fixed (2 bugs, 3 blocking)
**Impact on plan:** All auto-fixes necessary to achieve compilation. No scope creep. Service API surface matches plan exactly (16 call + 2 cast handlers). Internal implementation differs from plan due to Mesh compiler limitations.

## Issues Encountered
- Mesh compiler had 3 codegen bugs affecting service development: ? operator return type, Err(e) domination, and service struct state pointer/i64 mismatch. Fixed with compiler patches to expr.rs (state type detection) and lower.rs (service dispatch MIR).
- Typechecker bug: function parameters with generic type annotations (e.g., `List<String>`) were resolved as unparameterized types. Fixed in infer.rs to use `resolve_type_annotation` for parameters.
- Plan assumed separate service files and Map-based event buffers; reality required consolidation to main.mpl and JSON string buffers due to compiler/language limitations.

## User Setup Required
None - no external service configuration required. PostgreSQL database needed at runtime (postgres://mesh:mesh@localhost:5432/mesher).

## Next Phase Readiness
- All 4 services compile and are ready for runtime testing (requires PostgreSQL)
- StorageWriter.start(pool, project_id) + flush_ticker(writer_pid, 5000) pattern ready for Phase 88 ingestion pipeline
- HTTP.serve will replace Timer.sleep(999999999) in main entry point

## Self-Check: PASSED

- mesher/main.mpl: FOUND
- mesher/storage/writer.mpl: FOUND
- Commit 19af21f7: FOUND
- Compilation: Compiled: mesher/mesher
- Call handlers: 16 (expected 16)
- Cast handlers: 2 (expected 2)
- Services: 4 (expected 4)

---
*Phase: 87-foundation*
*Completed: 2026-02-14*
