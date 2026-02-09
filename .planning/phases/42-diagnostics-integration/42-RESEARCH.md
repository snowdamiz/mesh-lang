# Phase 42: Diagnostics & Integration - Research

**Researched:** 2026-02-09
**Domain:** Compiler diagnostic enrichment (module context in errors, module-qualified type display) and end-to-end multi-module validation
**Confidence:** HIGH

## Summary

Phase 42 has two distinct requirements: (1) DIAG-01 -- enrich error messages with module context (source module name and file path), and (2) DIAG-02 -- display module-qualified type names in type errors (e.g., "expected Math.Vector.Point, got Main.Point"). A third success criterion requires a comprehensive end-to-end integration test with 3+ modules covering structs, traits, generics, and imports. All three are well-scoped and achievable using existing infrastructure.

The current diagnostic system uses ariadne 0.6 for rendering. The `render_diagnostic` function already receives a `filename: &str` parameter, but it is NOT used in the ariadne report or source cache. The output currently shows `<unknown>` as the file location because `Source::from(source)` creates an anonymous source. The fix for DIAG-01 is straightforward: switch from `Range<usize>` spans to `(String, Range<usize>)` named spans, and from `Source::from(source)` to `ariadne::sources([(filename, source)])`. This makes ariadne display the actual file path in diagnostic output. Additionally, the `TypeError` variants for cross-module issues (ImportModuleNotFound, ImportNameNotFound, PrivateItem) already carry `module_name` -- no structural changes needed there. For generic type errors (Mismatch, ArityMismatch, etc.) occurring in multi-module contexts, the file path is already threaded correctly from `snowc/src/main.rs` where `module_path.display().to_string()` is passed as the filename.

For DIAG-02, type names in error messages are rendered via `Ty`'s `Display` impl and `TyCon::name`. Currently, when module B imports `Point` from `Geometry`, the type is registered as `Ty::Con(TyCon::new("Point"))` -- there is no module qualification. To show "expected Geometry.Point, got Main.Point", the type checker needs to register imported types with module-qualified names (e.g., `TyCon::new("Geometry.Point")`). This requires changes in the `infer.rs` import resolution code where struct/sum types from other modules are registered into the local TypeRegistry. The type's `name` field in `StructDefInfo`/`SumTypeDefInfo` would carry the module-qualified name for display purposes. However, care is needed: the MIR lowering and codegen use these names for struct layout resolution, so a display-only qualification approach may be safer.

**Primary recommendation:** Split into two tasks: (1) ariadne named-source integration for file paths in diagnostics, (2) module-qualified type display in error messages. Add a third task for the comprehensive E2E integration test. The ariadne change is mechanical; the type display change needs careful design to avoid breaking codegen.

## Standard Stack

### Core
| Crate | Version | Purpose | Key Files |
|-------|---------|---------|-----------|
| `snow-typeck` | workspace | Type errors, diagnostic rendering, Ty Display | `src/diagnostics.rs`, `src/error.rs`, `src/ty.rs`, `src/infer.rs` |
| `ariadne` | 0.6 | Fancy diagnostic rendering with multi-span labels | Used via `snow-typeck/src/diagnostics.rs` |
| `snowc` | workspace | Build pipeline, error reporting loop, E2E tests | `src/main.rs`, `tests/e2e.rs` |
| `snow-common` | workspace | ModuleGraph with module names and paths | `src/module_graph.rs` |

### Supporting
| Crate | Version | Purpose | When to Use |
|-------|---------|---------|-------------|
| `insta` | workspace | Snapshot testing for diagnostic output | Diagnostic snapshot tests |
| `tempfile` | workspace | Temp dirs for multi-file E2E tests | E2E integration tests |

### No New Dependencies
All changes use existing crates. The ariadne 0.6 named-source API (`sources()` function, `(String, Range<usize>)` spans) is already available but unused.

## Architecture Patterns

### Current Diagnostic Pipeline

```
snowc/src/main.rs::build()
  for each module in compilation_order:
    typeck = check_with_imports(parse, import_ctx)
    for error in typeck.errors:
      render_diagnostic(error, source, file_name, diag_opts, None)  // <-- file_name IS passed
        -> ariadne Report::build(kind, Range<usize>)                // <-- file_name NOT used
        -> report.write(Source::from(source), buf)                  // <-- anonymous source
```

### Pattern 1: Named-Source Ariadne Reports (DIAG-01)

**What:** Switch `render_diagnostic` to use ariadne's named-source API so file paths appear in diagnostic output.

**Current output:**
```
[E0001] Error: expected String, found Int
   +--[ <unknown>:1:5 ]
```

**Target output:**
```
[E0001] Error: expected String, found Int
   +--[ math/vector.snow:1:5 ]
```

**Implementation approach:**
Change the span type from `Range<usize>` to `(String, Range<usize>)` throughout `render_diagnostic`. Change the cache from `Source::from(source)` to `ariadne::sources([(filename.to_string(), source)])`.

```rust
// BEFORE:
let span = clamp(span);
Report::build(ReportKind::Error, span.clone())
// ...
let cache = Source::from(source);
report.write(cache, &mut buf)

// AFTER:
let span = (filename.to_string(), clamp(span));
Report::build(ReportKind::Error, span.clone())
// ...
let cache = ariadne::sources([(filename.to_string(), source)]);
report.write(cache, &mut buf)
```

**Key insight:** Every `Report::build()`, `Label::new()`, and `Report.write()` call must use the same span type. This is a mechanical refactor -- every occurrence of `Range<usize>` span in `diagnostics.rs` must become `(String, Range<usize>)`.

**Scope of change:** Only `snow-typeck/src/diagnostics.rs` needs modification. The `render_diagnostic` function signature stays the same (it already takes `filename: &str`). All callers in `snowc/src/main.rs` already pass the correct filename. The diagnostic test snapshots will change (from `<unknown>` to the test filename).

### Pattern 2: Module-Qualified Type Display (DIAG-02)

**What:** When a type originates from another module, display it with the module prefix in error messages.

**Current output:**
```
type mismatch: expected `Point`, found `Point`
```

**Target output:**
```
type mismatch: expected `Math.Vector.Point`, found `Main.Point`
```

**Implementation approaches (ranked by safety):**

**Approach A -- Display-only qualification (RECOMMENDED):**
Add an optional `module_origin` field to `TyCon` that is used ONLY for display, not for type identity or codegen. The `Display` impl for `TyCon` shows the qualified name, but `PartialEq`/`Hash` use only `name`.

```rust
pub struct TyCon {
    pub name: String,
    /// Module origin for display in error messages (e.g., "Math.Vector").
    /// Not used for type identity or codegen.
    pub display_prefix: Option<String>,
}

impl fmt::Display for TyCon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = &self.display_prefix {
            write!(f, "{}.{}", prefix, self.name)
        } else {
            write!(f, "{}", self.name)
        }
    }
}
```

When the type checker registers imported types from another module, set `display_prefix` to the source module name. Local types keep `display_prefix = None`. The `PartialEq` and `Hash` impls for `TyCon` must continue to use only `name` (not `display_prefix`) to preserve type identity semantics.

**Why this is safest:** Codegen, MIR lowering, and type registry lookups all use `TyCon.name` (the unqualified name). Adding a display-only field avoids changing any lookup or identity logic.

**Approach B -- Qualify names in the TypeRegistry:**
Register imported types with module-qualified names (e.g., `"Geometry.Point"`). This changes type identity, meaning `"Geometry.Point" != "Point"` in the type system. This is the "module-qualified type names from day one" prior decision, but it has deep implications for codegen, struct layout lookup, and variant constructor resolution. NOT recommended for this phase.

**Where to set display_prefix:**
In `infer.rs`, when processing `from Module import TypeName`:
- At line 1509-1515: When creating `Ty::struct_ty(&name, ...)` for an imported struct, use a modified `Ty::struct_ty` that sets the `display_prefix`.
- At line 1521-1528: When registering imported sum type variant constructors.
- At line 1464-1470: When registering struct types for qualified access (`import Module` style).

### Pattern 3: Comprehensive E2E Integration Test

**What:** A realistic multi-module project with 3+ modules exercising structs, traits, generics, and imports.

**Structure:**
```
project/
  main.snow          -- imports Geometry, Utils; calls functions, uses types
  geometry.snow      -- pub struct Point, pub fn distance, pub trait Shape
  math/vector.snow   -- pub struct Vec2, pub fn dot, imports Geometry.Point
  utils.snow         -- pub fn format_point, imports Geometry
```

**What it validates:**
- Cross-module struct construction and field access
- Cross-module function calls (qualified and selective import)
- Cross-module trait usage
- Nested module paths (Math.Vector)
- Private function isolation (no leakage)
- Correct output when all modules compile and link

### Anti-Patterns to Avoid

- **Changing TyCon::PartialEq/Hash to include display_prefix:** This would break type identity. Two types with the same name but different display prefixes must still be equal.
- **Adding module info to Span struct:** The existing Span in snow-common is byte offsets only. File identity belongs in the diagnostic layer (ariadne), not the span type.
- **Separate file reading in diagnostics:** The source text and filename are already available at the call site. Do NOT have `render_diagnostic` read files from disk.
- **Multi-file ariadne reports:** Each diagnostic is about a single file's error. Do not try to create cross-file reports (e.g., showing the import site AND the definition site in different files). That is a future enhancement.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Named source display in diagnostics | Custom filename header formatting | `ariadne::sources([(name, src)])` + `(String, Range)` spans | ariadne already supports this; just not wired up |
| File path for modules | Custom path construction | `project.graph.get(id).path` joined with `dir` | Already computed in `build_project` |
| Module name for types | Custom module tracking in typeck | `ModuleExports.module_name` field | Already available in ImportContext |
| Levenshtein/did-you-mean | Custom fuzzy matching | Existing `find_closest_name` in diagnostics.rs | Already implemented |
| Multi-file E2E test harness | New test framework | `compile_multifile_and_run` helper in e2e.rs | Already works for Phase 39-41 tests |

**Key insight:** The infrastructure for DIAG-01 and DIAG-02 already exists in the codebase. DIAG-01 is literally wiring up the `filename` parameter that is already passed but ignored. DIAG-02 requires a small structural change to `TyCon` for display purposes.

## Common Pitfalls

### Pitfall 1: Breaking TyCon Equality with Display Prefix
**What goes wrong:** Adding `display_prefix` to `TyCon` and including it in `PartialEq`/`Hash` would make `Geometry.Point != Point` even when they represent the same type. Type unification would fail for valid programs.
**Why it happens:** Derive(PartialEq, Hash) includes all fields by default.
**How to avoid:** Manually implement `PartialEq` and `Hash` for `TyCon` to use only `name`. Or use a separate wrapper type for display-only purposes.
**Warning signs:** "type mismatch: expected Point, found Point" -- same type name, different display prefix.

### Pitfall 2: Snapshot Test Mass Breakage
**What goes wrong:** All existing diagnostic snapshot tests will break because the filename changes from `<unknown>` to `test.snow` (or whatever is passed).
**Why it happens:** The named-source change affects ALL diagnostic output, not just cross-module errors.
**How to avoid:** Update all snapshot tests after the ariadne change. Use `insta` review workflow (`cargo insta review`) to batch-accept new snapshots.
**Warning signs:** All diagnostic tests fail after the ariadne change.

### Pitfall 3: Type Display Prefix Propagation Through Unification
**What goes wrong:** After unifying two types, the resulting type may lose its display prefix, or gain an incorrect one.
**Why it happens:** The unification engine replaces type variables with concrete types. If the concrete type from module A has a display prefix, all uses inherit it.
**How to avoid:** The `display_prefix` should be set at type registration time (import resolution) and preserved through unification. Since unification replaces `Ty::Var` with the concrete `Ty`, the concrete type's `TyCon` (with its `display_prefix`) is the one that survives.
**Warning signs:** Type errors showing wrong module prefix for locally-defined types.

### Pitfall 4: Ariadne Span Type Mismatch
**What goes wrong:** Mixing `Range<usize>` and `(String, Range<usize>)` spans in the same report causes compile errors.
**Why it happens:** The `S: Span` generic parameter must be consistent across `Report::build`, `Label::new`, and `report.write`.
**How to avoid:** Consistently use `(String, Range<usize>)` for ALL spans in the refactored `render_diagnostic`. Search-and-replace all span construction.
**Warning signs:** Rust compile errors about mismatched types in `Report::build` or `Label::new`.

### Pitfall 5: Missing Display Prefix for Local Types in Cross-Module Errors
**What goes wrong:** In "expected Geometry.Point, got Point", the local type shows no module prefix. The success criterion says "expected Math.Vector.Point, got Main.Point" -- both types should be module-qualified.
**Why it happens:** Local types (defined in the current module) have no `display_prefix` because they were never imported.
**How to avoid:** When the type checker runs for a module, set the `display_prefix` for all locally-defined types to the current module name. This requires threading the module name into the type checker (it currently does not have it).
**Warning signs:** "expected Geometry.Point, got Point" instead of "expected Geometry.Point, got Main.Point".

### Pitfall 6: Codegen Breakage from TyCon Name Change
**What goes wrong:** If `TyCon.name` is changed to include the module prefix (e.g., "Geometry.Point"), then MIR type resolution (`resolve_con`) will fail to match "Point" in the type registry.
**Why it happens:** Approach B (qualify names in TypeRegistry) changes the lookup keys.
**How to avoid:** Use Approach A (display-only prefix). Keep `TyCon.name` as the unqualified name. Only `Display` shows the qualified form.
**Warning signs:** LLVM codegen errors, "unknown type" fallbacks, struct layout mismatches.

## Code Examples

### Current render_diagnostic Span Usage (to be changed)
```rust
// Source: snow-typeck/src/diagnostics.rs lines 570-573, 1501-1506
let span = clamp(span);
let mut builder = Report::build(ReportKind::Error, span.clone())
    .with_code(code)
    .with_message(&msg)
    .with_config(config);
// ...
let cache = Source::from(source);
report.write(cache, &mut buf)
```

### Target: Named-Source Span Usage
```rust
// Every span becomes (filename, range):
let span = (filename.to_string(), clamp(span));
let mut builder = Report::build(ReportKind::Error, span.clone())
    .with_code(code)
    .with_message(&msg)
    .with_config(config);

// Labels also use named spans:
builder.add_label(
    Label::new((filename.to_string(), then_range))
        .with_message(format!("expected {}", expected))
        .with_color(Color::Red),
);

// Cache uses named sources:
let cache = ariadne::sources([(filename.to_string(), source)]);
report.write(cache, &mut buf)
```

### Current TyCon Definition (to be extended)
```rust
// Source: snow-typeck/src/ty.rs lines 20-35
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TyCon {
    pub name: String,
}

impl fmt::Display for TyCon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}
```

### Target: TyCon with Display Prefix
```rust
#[derive(Clone, Debug)]
pub struct TyCon {
    pub name: String,
    /// Module origin for display in error messages.
    /// NOT used for type identity. Example: "Math.Vector"
    pub display_prefix: Option<String>,
}

impl PartialEq for TyCon {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name  // display_prefix excluded
    }
}

impl Eq for TyCon {}

impl Hash for TyCon {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);  // display_prefix excluded
    }
}

impl TyCon {
    pub fn new(name: impl Into<String>) -> Self {
        TyCon { name: name.into(), display_prefix: None }
    }

    pub fn with_module(name: impl Into<String>, module: impl Into<String>) -> Self {
        TyCon { name: name.into(), display_prefix: Some(module.into()) }
    }
}

impl fmt::Display for TyCon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = &self.display_prefix {
            write!(f, "{}.{}", prefix, self.name)
        } else {
            write!(f, "{}", self.name)
        }
    }
}
```

### Current Build Pipeline Error Reporting (already correct)
```rust
// Source: snowc/src/main.rs lines 309-317
let file_name = module_path.display().to_string();
for error in &typeck.errors {
    has_type_errors = true;
    let rendered = snow_typeck::diagnostics::render_diagnostic(
        error, source, &file_name, diag_opts, None,
    );
    eprint!("{}", rendered);
}
```
This already passes the correct file path. No changes needed in `main.rs` for DIAG-01.

### E2E Multi-Module Test Template
```rust
// Based on existing compile_multifile_and_run in e2e.rs
#[test]
fn e2e_comprehensive_multi_module_integration() {
    let output = compile_multifile_and_run(&[
        ("geometry.snow", r#"
pub struct Point do
  x :: Int
  y :: Int
end

pub fn make_point(x :: Int, y :: Int) -> Point do
  Point { x: x, y: y }
end

pub fn point_sum(p :: Point) -> Int do
  p.x + p.y
end
"#),
        ("math/vector.snow", r#"
from Geometry import Point, make_point, point_sum

pub fn scaled_sum(p :: Point, factor :: Int) -> Int do
  point_sum(p) * factor
end
"#),
        ("utils.snow", r#"
pub fn double(n :: Int) -> Int do
  n * 2
end
"#),
        ("main.snow", r#"
from Geometry import make_point
from Utils import double
import Math.Vector

fn main() do
  let p = make_point(3, 4)
  let sum = Vector.scaled_sum(p, 2)
  let result = double(sum)
  println("${result}")
end
"#),
    ]);
    assert_eq!(output, "28\n");  // (3+4)*2 = 14, double(14) = 28
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Anonymous source in ariadne (`<unknown>`) | filename parameter passed but NOT used | Since inception | File path available but not displayed |
| Unqualified type names everywhere | Unqualified type names everywhere | Current state | No module context in type errors |
| Single-file diagnostic testing | `compile_multifile_and_run` / `_expect_error` helpers | Phase 39 | Multi-module E2E testing infrastructure exists |
| No module-aware error variants | ImportModuleNotFound, ImportNameNotFound, PrivateItem | Phase 39-40 | Module name already in cross-module errors |

**Key current state:**
- `render_diagnostic` takes `filename` but does not pass it to ariadne (anonymous `Source::from`)
- ariadne 0.6 supports named sources via `sources()` function and `(Id, Range<usize>)` span type
- `TyCon` has no module information; `Display` always shows unqualified name
- 108 E2E tests pass; 17 diagnostic snapshot tests exist
- The `module_path` is correctly threaded through the build pipeline in `main.rs`
- `ModuleExports.module_name` carries the full module name (e.g., "Math.Vector")

## Open Questions

1. **Should local types also show module prefix?**
   - What we know: Success criterion says "expected Math.Vector.Point, got Main.Point" -- both types qualified.
   - What's unclear: The type checker currently does not know its own module name. The `check_with_imports` function takes `ImportContext` but not the current module's name.
   - Recommendation: Thread the current module name into the type checker via a new field on `ImportContext` (e.g., `current_module: Option<String>`). For single-file mode, this is `None` and types display without prefix. For multi-module mode, set it to the module's name. Use it to set `display_prefix` on locally-defined TyCons.

2. **Should builtin types show module prefix?**
   - What we know: Int, String, Bool, Float, Option, Result are builtins defined everywhere.
   - What's unclear: Should a type mismatch like "expected Int, found String" ever show "expected Main.Int"?
   - Recommendation: No. Builtins should never have a display prefix. Only user-defined struct/sum types get module-qualified display.

3. **Does the TyCon display_prefix approach work through type applications?**
   - What we know: `Ty::App(Box<Ty>, Vec<Ty>)` wraps a `Ty::Con(TyCon)`. The TyCon inside App carries the name.
   - What's unclear: For `Option<Geometry.Point>`, should it display as `Option<Geometry.Point>`?
   - Recommendation: Yes, and this works automatically. The `Ty::App` Display impl calls `Display` on the inner `Ty::Con`, which calls `TyCon::Display`, which shows the qualified name. No special handling needed.

4. **Impact on existing diagnostic snapshots?**
   - What we know: All 17 diagnostic snapshots show `<unknown>` for filename. The named-source change will change them all to show the test filename.
   - What's unclear: Whether any downstream consumers (LSP, CI) depend on the `<unknown>` text.
   - Recommendation: The LSP uses JSON diagnostics (which already have the correct filename). The snapshot change is purely visual. Use `cargo insta review` to batch-accept.

## Sources

### Primary (HIGH confidence)
- `snow-typeck/src/diagnostics.rs` -- Full render_diagnostic implementation, 1507 lines, all span construction
- `snow-typeck/src/error.rs` -- All TypeError variants with span types and Display implementations
- `snow-typeck/src/ty.rs` -- TyCon definition, Ty Display implementation, type constructors
- `snow-typeck/src/infer.rs` -- Import resolution (lines 1451-1588), struct/sum type registration
- `snow-typeck/src/lib.rs` -- TypeckResult, ImportContext, ModuleExports, render_errors
- `snowc/src/main.rs` -- Build pipeline error reporting loop (lines 309-326), module_path threading
- `snow-common/src/module_graph.rs` -- ModuleInfo with `name` and `path` fields
- `snowc/tests/e2e.rs` -- compile_multifile_and_run and compile_multifile_expect_error helpers
- Phase 41 RESEARCH.md and SUMMARY.md -- Module-qualified naming decisions, qualify_name method
- ariadne 0.6 docs (docs.rs/ariadne/0.6.0) -- Span trait, sources() function, named source API

### Secondary (MEDIUM confidence)
- [ariadne GitHub](https://github.com/zesterer/ariadne) -- Multi-file examples, Cache trait usage
- [ariadne docs.rs](https://docs.rs/ariadne/0.6.0/ariadne/) -- API reference for Span, Source, sources()

## Metadata

**Confidence breakdown:**
- DIAG-01 (file paths in diagnostics): HIGH -- The filename parameter is already threaded; ariadne 0.6 supports named sources; change is mechanical
- DIAG-02 (module-qualified type display): HIGH -- The display_prefix approach avoids all identity/codegen issues; import resolution code is well-understood
- E2E integration test: HIGH -- compile_multifile_and_run helper already proven in Phase 39-41 tests
- Pitfalls: HIGH -- All identified from direct code analysis

**Research date:** 2026-02-09
**Valid until:** 30 days (stable compiler internals, no external dependencies changing)
