---
phase: 81-grammar-document-symbols
verified: 2026-02-14T08:30:00Z
status: human_needed
score: 4/4 must-haves verified
human_verification:
  - test: "Open a Mesh file in VS Code and check Outline panel"
    expected: "Hierarchical symbol tree shows fn, struct, module, actor, service, supervisor, interface, impl, let definitions with proper nesting (functions inside modules, methods inside interfaces)"
    why_human: "Visual UI verification - need to confirm VS Code Outline panel renders correctly"
  - test: "Use Go-to-Symbol (Cmd+Shift+O) in a Mesh file"
    expected: "Symbol picker lists all definitions with correct icons (function, struct, module, class, interface, variable, enum, type)"
    why_human: "Visual UI verification - need to confirm symbol picker works and displays correct icons"
  - test: "Use Breadcrumbs navigation in a nested Mesh file"
    expected: "Breadcrumbs show correct nesting path (e.g., 'module_name > function_name')"
    why_human: "Visual UI verification - need to confirm breadcrumbs render correctly with proper hierarchy"
  - test: "Check syntax highlighting in VS Code for all new keywords and operators"
    expected: "for, while, cond, break, continue, trait, alias, monitor, terminate, trap, after, .., <>, ++, =>, ?, &&, || all highlighted with appropriate colors"
    why_human: "Visual appearance - need to confirm colors render correctly in editor"
  - test: "Check doc comments (## and ##!) appear visually distinct from regular comments"
    expected: "Doc comments use greener, non-italic color; regular comments use standard gray italic"
    why_human: "Visual appearance - need to confirm theme styling is applied correctly"
  - test: "Check hex (0xFF), binary (0b1010), scientific (1.0e10) literals are highlighted"
    expected: "All number formats highlighted as constants with appropriate color"
    why_human: "Visual appearance - need to confirm all number literal patterns are recognized"
  - test: "Check triple-quoted strings with interpolation syntax"
    expected: 'Triple-quoted strings ("""...""") highlighted with interpolation ${} recognized'
    why_human: "Visual appearance - need to confirm string interpolation styling works"
  - test: "Check module-qualified calls (e.g., List.map(...))"
    expected: "Module name highlighted as type, function name highlighted as function"
    why_human: "Visual appearance - need to confirm module-qualified call pattern works"
  - test: "Check website code blocks use updated Shiki themes"
    expected: "Mesh code blocks on website show doc comments in distinct color, all new keywords/operators highlighted"
    why_human: "Visual appearance on website - need to confirm Shiki integration works"
---

# Phase 81: Grammar + Document Symbols Verification Report

**Phase Goal:** Complete syntax highlighting + LSP outline/breadcrumbs/go-to-symbol
**Verified:** 2026-02-14T08:30:00Z
**Status:** human_needed
**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | VS Code Outline panel shows hierarchical symbols for Mesh files | ? HUMAN_NEEDED | document_symbol handler exists with hierarchical DocumentSymbolResponse::Nested, all 11 definition types handled, container recursion implemented. Needs visual UI verification. |
| 2 | Go-to-symbol (Cmd+Shift+O) lists all fn, struct, module, actor, service, supervisor, interface, impl, and let definitions | ? HUMAN_NEEDED | document_symbol capability advertised, all 11 SyntaxKind types matched in collect_symbols. Needs visual UI verification of symbol picker. |
| 3 | Breadcrumbs navigation shows correct nesting (function inside module) | ? HUMAN_NEEDED | Container symbols (MODULE_DEF, ACTOR_DEF, SERVICE_DEF, INTERFACE_DEF, IMPL_DEF) recurse into BLOCK children and set children field. Needs visual UI verification. |
| 4 | Symbol ranges cover entire definitions (fn to end), selection ranges cover just the name | ✓ VERIFIED | make_symbol computes full range from node.text_range() and selection_range from NAME child text_range(), both using tree_to_source_offset conversion chain. |
| 5 | All Mesh keywords are syntax-highlighted in VS Code and on the website | ? HUMAN_NEEDED | Grammar contains all keywords (for, while, cond, break, continue, trait, alias, monitor, terminate, trap, after), website imports grammar via config.mts line 4. Needs visual verification. |
| 6 | Doc comments (## and ##!) appear visually distinct from regular comments | ? HUMAN_NEEDED | Grammar has comment.line.documentation scope, Shiki themes have distinct colors (#7a9a6a light, #8aaa7a dark). Needs visual verification. |
| 7 | Hex, binary, and scientific number literals are highlighted as numbers | ? HUMAN_NEEDED | Grammar contains constant.numeric.hex, constant.numeric.binary, constant.numeric.float patterns. Needs visual verification. |
| 8 | Triple-quoted strings with interpolation are highlighted correctly | ? HUMAN_NEEDED | Grammar contains string.quoted.triple.mesh scope with interpolation patterns. Needs visual verification. |
| 9 | Module-qualified calls (List.map) highlight module as type and function as function | ? HUMAN_NEEDED | Grammar contains module-call pattern with entity.name.type.module and entity.name.function captures. Needs visual verification. |
| 10 | nil is NOT highlighted as a language constant (Mesh uses None) | ✓ VERIFIED | Grammar constant.language pattern is "\\b(true|false)\\b" (nil removed). Plan 01 summary confirms removal. |

**Score:** 2/10 truths verified (8 require human verification)

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| crates/mesh-lsp/src/server.rs | textDocument/documentSymbol handler with hierarchical symbol response | ✓ VERIFIED | document_symbol method exists (lines 211-227), returns DocumentSymbolResponse::Nested, contains "document_symbol" pattern |
| editors/vscode-mesh/syntaxes/mesh.tmLanguage.json | Complete TextMate grammar for Mesh language | ✓ VERIFIED | File exists, valid JSON, contains "for\|while\|cond\|break\|continue" pattern |
| website/docs/.vitepress/theme/shiki/mesh-light.json | Light theme with doc comment styling | ✓ VERIFIED | File exists, contains "comment.line.documentation" pattern (1 match) |
| website/docs/.vitepress/theme/shiki/mesh-dark.json | Dark theme with doc comment styling | ✓ VERIFIED | File exists, contains "comment.line.documentation" pattern (1 match) |

**All 4 artifacts verified (exists, substantive, wired)**

### Key Link Verification

| From | To | Via | Status | Details |
|------|-----|-----|--------|---------|
| crates/mesh-lsp/src/server.rs | crates/mesh-parser/src/syntax_kind.rs | SyntaxKind matching for FN_DEF, STRUCT_DEF, MODULE_DEF, etc. | ✓ WIRED | SyntaxKind imported (line 17), SyntaxKind::FN_DEF pattern found (line 239), all 11 definition types matched in collect_symbols |
| crates/mesh-lsp/src/server.rs | crates/mesh-lsp/src/definition.rs | tree_to_source_offset for range conversion | ✓ WIRED | tree_to_source_offset called 8 times (lines 196, 198, 416, 418, 432, 434, 446, 448), crate::definition:: namespace used |
| crates/mesh-lsp/src/server.rs | crates/mesh-lsp/src/analysis.rs | offset_to_position for LSP position conversion | ✓ WIRED | offset_to_position called 8 times (lines 200, 201, 421, 422, 436, 437, 450, 451), analysis:: module imported (line 20) |
| editors/vscode-mesh/syntaxes/mesh.tmLanguage.json | website/docs/.vitepress/config.mts | Direct import path for Shiki highlighting | ✓ WIRED | config.mts line 4 imports meshGrammar from '../../../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json' |

**All 4 key links verified (wired)**

### Requirements Coverage

| Requirement | Status | Blocking Issue |
|-------------|--------|----------------|
| GRAM-01: All control flow keywords highlighted | ? NEEDS_HUMAN | Grammar contains pattern, needs visual verification |
| GRAM-02: All declaration keywords highlighted | ? NEEDS_HUMAN | Grammar contains pattern, needs visual verification |
| GRAM-03: All actor/supervision keywords highlighted | ? NEEDS_HUMAN | Grammar contains pattern, needs visual verification |
| GRAM-04: All operators highlighted | ? NEEDS_HUMAN | Grammar contains patterns, needs visual verification |
| GRAM-05: Doc comments highlighted with distinct scope | ? NEEDS_HUMAN | Grammar contains scope, themes styled, needs visual verification |
| GRAM-06: Hex, binary, scientific number literals highlighted | ? NEEDS_HUMAN | Grammar contains patterns, needs visual verification |
| GRAM-07: Triple-quoted strings with interpolation highlighted | ? NEEDS_HUMAN | Grammar contains pattern, needs visual verification |
| GRAM-08: Module-qualified calls highlight module as type | ? NEEDS_HUMAN | Grammar contains pattern, needs visual verification |
| GRAM-09: nil removed from constants | ✓ SATISFIED | Verified in grammar pattern |
| GRAM-10: Website highlighting automatically updated | ✓ SATISFIED | Grammar imported by config.mts |
| SYM-01: Hierarchical symbols for fn, struct, module, etc. | ? NEEDS_HUMAN | Handler exists with all types, needs visual verification |
| SYM-02: Correct SymbolKind mapping | ✓ SATISFIED | All 11 types mapped correctly in code |
| SYM-03: Correct range and selection_range | ✓ SATISFIED | Verified in make_symbol implementation |

**Score:** 4/13 requirements satisfied, 9 need human verification

### Anti-Patterns Found

None - no anti-patterns detected.

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| - | - | - | - | - |

### Human Verification Required

#### 1. VS Code Outline Panel Hierarchical Symbols

**Test:** Open a Mesh file containing functions, structs, modules, actors, services, supervisors, interfaces, impl blocks, and let bindings in VS Code. Open the Outline panel (Cmd+Shift+O or View > Outline).

**Expected:**
- Outline panel displays a hierarchical tree
- All definition types appear with correct icons:
  - Functions: function icon
  - Structs: struct icon
  - Modules: module/folder icon
  - Actors/Services/Supervisors: class icon
  - Interfaces: interface icon
  - Impl blocks: object icon with "impl TraitName" label
  - Let bindings: variable icon
  - Sum types: enum icon
  - Type aliases: type parameter icon
- Container symbols (modules, actors, services, interfaces, impls) show nested children
- Example: function inside module appears indented under module

**Why human:** Visual UI verification - programmatic checks can't verify the Outline panel renders correctly or that icons are displayed.

#### 2. Go-to-Symbol (Cmd+Shift+O) Symbol Picker

**Test:** In a Mesh file, press Cmd+Shift+O to open the symbol picker.

**Expected:**
- Symbol picker lists all function, struct, module, actor, service, supervisor, interface, impl, let, sum type, and type alias definitions
- Each symbol has the correct icon
- Typing filters the list
- Selecting a symbol navigates to its definition

**Why human:** Visual UI verification - programmatic checks can't verify the symbol picker UI or icon rendering.

#### 3. Breadcrumbs Navigation

**Test:** Open a Mesh file with nested definitions (e.g., function inside a module, method inside an interface). Click in the nested function body and check the breadcrumbs bar at the top of the editor.

**Expected:**
- Breadcrumbs show the nesting path (e.g., "module_name > function_name")
- Clicking breadcrumb elements navigates to those symbols
- Breadcrumbs update as cursor moves between different symbol scopes

**Why human:** Visual UI verification - programmatic checks can't verify breadcrumbs render or navigate correctly.

#### 4. Syntax Highlighting: All Keywords and Operators

**Test:** Open a Mesh file containing examples of all new keywords (for, while, cond, break, continue, trait, alias, monitor, terminate, trap, after) and operators (.., <>, ++, =>, ?, &&, ||).

**Expected:**
- All keywords highlighted with keyword color (purple/blue in typical themes)
- All operators highlighted with operator color (often same as keywords or distinct)
- Highlighting matches the TextMate scopes defined in grammar

**Why human:** Visual appearance - colors are theme-dependent and need visual confirmation.

#### 5. Doc Comment Visual Distinction

**Test:** Open a Mesh file containing regular comments (#), item doc comments (##), and module doc comments (##!).

**Expected:**
- Doc comments (## and ##!) appear in a greener, non-italic color
- Regular comments (#) appear in standard gray italic
- Visual difference is clear and consistent

**Why human:** Visual appearance - need to confirm theme styling is applied and colors are distinct.

#### 6. Number Literal Highlighting

**Test:** Open a Mesh file containing:
```mesh
let hex = 0xFF
let binary = 0b1010
let octal = 0o755
let scientific = 1.0e10
let float = 3.14
let integer = 42
```

**Expected:**
- All literals highlighted as constants (typically orange/green)
- Hex, binary, octal, scientific, float, and integer all recognized

**Why human:** Visual appearance - need to confirm all number patterns are recognized by grammar.

#### 7. Triple-Quoted String Highlighting

**Test:** Open a Mesh file containing triple-quoted strings with interpolation:
```mesh
let message = """
  Hello ${name}!
  Welcome to ${place}.
"""
```

**Expected:**
- Triple-quoted string delimiters (""") highlighted
- String content highlighted as string
- Interpolation expressions (${...}) highlighted with distinct scope (often variables/expressions inside strings)

**Why human:** Visual appearance - need to confirm interpolation patterns work correctly.

#### 8. Module-Qualified Call Highlighting

**Test:** Open a Mesh file containing module-qualified calls:
```mesh
List.map(xs, fn)
Map.get(m, key)
String.concat(a, b)
```

**Expected:**
- Module name (List, Map, String) highlighted as type (typically class color, often cyan/green)
- Function name (map, get, concat) highlighted as function (typically yellow/gold)
- Distinct colors for module vs function

**Why human:** Visual appearance - need to confirm the lookahead pattern only matches call sites and colors are correct.

#### 9. Website Code Block Highlighting

**Test:** Visit the Mesh website (locally via `npm run docs:dev` or deployed) and view code blocks in documentation.

**Expected:**
- Mesh code blocks use updated Shiki themes
- Doc comments appear in distinct color (greener, non-italic)
- All new keywords and operators highlighted
- Highlighting matches VS Code extension

**Why human:** Visual appearance on website - need to confirm Shiki integration picks up grammar and theme changes.

### Gaps Summary

No gaps found. All automated verification checks passed:
- All 4 artifacts exist, are substantive (not stubs), and are wired
- All 4 key links are wired and functional
- All 11 definition types handled in document_symbol
- SyntaxKind matching uses proper offset conversion
- Capability advertisement includes document_symbol_provider
- Tests pass (31/31)
- Commits verified (de1fdeee, 47fc8d09, 16141d5f)
- No anti-patterns (TODO, FIXME, stub returns, debug-only logging)

However, 8 of 10 observable truths require human verification because they depend on visual UI rendering, color appearance, or interactive behavior that cannot be verified programmatically. The implementation is complete and correct according to all automated checks.

---

_Verified: 2026-02-14T08:30:00Z_
_Verifier: Claude (gsd-verifier)_
