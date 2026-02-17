# Requirements: Mesh

**Defined:** 2026-02-16
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.

## v10.1 Requirements

Requirements for stabilization milestone. Fix Mesher compilation errors and verify endpoints.

### Compilation Fixes

- [ ] **FIX-01**: All type mismatches in queries.mpl resolved (Ptr vs Map, Map vs Result)
- [ ] **FIX-02**: All undefined variable errors in service files resolved (org.mpl, project.mpl, user.mpl)
- [ ] **FIX-03**: All `?` operator errors resolved (functions using `?` on non-Result returns)
- [ ] **FIX-04**: All module reference errors resolved (team.mpl, main.mpl)
- [ ] **FIX-05**: All argument count mismatches resolved
- [ ] **FIX-06**: `meshc build mesher` completes with zero errors

### Verification

- [ ] **VER-01**: Mesher binary starts and connects to PostgreSQL
- [ ] **VER-02**: HTTP API endpoints return correct responses (GET/POST tested)
- [ ] **VER-03**: WebSocket endpoints accept connections and respond

## v10.0 Requirements (Complete)

All v10.0 ORM requirements shipped. 50 requirements across 8 phases (96-103).

### Compiler Additions

- [x] **COMP-01**: Atom literal syntax -- v10.0 Phase 96
- [x] **COMP-02**: Keyword argument syntax -- v10.0 Phase 96
- [x] **COMP-03**: Multi-line pipe chain support -- v10.0 Phase 96
- [x] **COMP-04**: Struct update syntax -- v10.0 Phase 96
- [x] **COMP-05**: deriving(Schema) infrastructure -- v10.0 Phase 96
- [x] **COMP-06**: Relationship declaration syntax -- v10.0 Phase 96, 100
- [x] **COMP-07**: Fix Map.collect string key propagation -- v10.0 Phase 96
- [x] **COMP-08**: Fix cross-module from_row/from_json resolution -- v10.0 Phase 96

### Schema Definition

- [x] **SCHM-01**: Table name from struct name -- v10.0 Phase 97
- [x] **SCHM-02**: Field metadata with SQL type mapping -- v10.0 Phase 97
- [x] **SCHM-03**: Primary key configuration -- v10.0 Phase 97
- [x] **SCHM-04**: Timestamps support -- v10.0 Phase 97
- [x] **SCHM-05**: Column accessor functions -- v10.0 Phase 97

### Query Builder

- [x] **QBLD-01** through **QBLD-09**: Full query builder -- v10.0 Phase 98

### Repo Operations

- [x] **REPO-01** through **REPO-11**: Full repo operations -- v10.0 Phases 98, 100

### Changesets

- [x] **CHST-01** through **CHST-09**: Full changeset system -- v10.0 Phase 99

### Migrations

- [x] **MIGR-01** through **MIGR-08**: Full migration tooling -- v10.0 Phase 101

### Mesher Rewrite

- [x] **MSHR-01** through **MSHR-04**: Mesher ORM conversion -- v10.0 Phase 102-103
- [ ] **MSHR-05**: All existing Mesher functionality verified working -- **v10.1**

## Out of Scope

| Feature | Reason |
|---------|--------|
| New ORM features | Stabilization only -- no new functionality |
| Compiler changes | Unless required to fix a compilation error |
| Documentation updates | Not in scope for this patch milestone |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| FIX-01 | Phase 104 | Pending |
| FIX-02 | Phase 104 | Pending |
| FIX-03 | Phase 104 | Pending |
| FIX-04 | Phase 104 | Pending |
| FIX-05 | Phase 104 | Pending |
| FIX-06 | Phase 104 | Pending |
| VER-01 | Phase 105 | Pending |
| VER-02 | Phase 105 | Pending |
| VER-03 | Phase 105 | Pending |

**Coverage:**
- v10.1 requirements: 9 total
- Mapped to phases: 9
- Unmapped: 0

---
*Requirements defined: 2026-02-16*
*Last updated: 2026-02-16 after v10.1 milestone definition*
