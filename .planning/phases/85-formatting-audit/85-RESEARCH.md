# Phase 85: LSP Formatting + Audit - Research

**Researched:** 2026-02-14
**Domain:** LSP formatting handler, formatter completeness audit, REPL JIT symbol coverage
**Confidence:** HIGH

## Summary

This phase has three distinct work areas: (1) wiring `mesh_fmt::format_source` into the LSP server as a `textDocument/formatting` handler, (2) auditing the formatter's CST walker for completeness against all v1.7-v7.0 syntax constructs, and (3) auditing the REPL's JIT symbol registration for v7.0 features (iterators, From/Into, numeric traits).

The LSP formatting handler is straightforward -- tower-lsp 0.20 provides a `formatting` method on the `LanguageServer` trait, the `mesh-fmt` crate already has a clean `format_source(source, config) -> String` API, and the pattern is a single `TextEdit` replacing the entire document. The main work is adding `mesh-fmt` as a dependency and advertising `document_formatting_provider` in server capabilities.

The formatter audit reveals that the CST walker already handles most syntax constructs explicitly (for/while loops, trait/impl blocks, closures, pipe expressions, etc.) but several v5.0-v7.0 nodes fall through to the generic `walk_tokens_inline` fallback: `ASSOC_TYPE_DEF`, `ASSOC_TYPE_BINDING`, `MAP_LITERAL`, `MAP_ENTRY`, `LIST_LITERAL`, `FUN_TYPE`, and `CONS_PAT`. The REPL audit reveals a significant gap: the JIT symbol registration in `jit.rs` does NOT register any iterator runtime functions (`mesh_iter_*`), From/Into conversion functions (`mesh_int_to_float`, `mesh_float_to_int`), or collect operations (`mesh_list_collect`, etc.), meaning these v7.0 features will fail at JIT execution time.

**Primary recommendation:** Implement the LSP formatting handler (small, well-defined), add dedicated walker handlers for the 7 missing CST node types, and register all missing runtime symbols in the REPL's JIT engine.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| tower-lsp | 0.20.0 | LSP protocol framework | Already used by mesh-lsp |
| lsp-types | 0.94.1 | LSP type definitions (TextEdit, etc.) | Transitive dep of tower-lsp |
| mesh-fmt | 0.1.0 | Mesh code formatter | Existing crate, `format_source` API |
| mesh-parser | 0.1.0 | CST parsing (SyntaxKind enum) | Walker dispatches on SyntaxKind |
| mesh-rt | 0.1.0 | Runtime functions for JIT | Iterator/From/Into symbols needed |
| rowan | 0.16 | CST library | Used by parser and formatter walker |
| insta | 1.46 | Snapshot testing | Used by formatter tests |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| mesh-codegen | 0.1.0 | MIR lowering + LLVM codegen | REPL JIT uses its pipeline |
| inkwell | 0.8.0 | LLVM bindings | JIT execution engine |

### Alternatives Considered
None -- all libraries are already in use. This phase adds no new dependencies.

**Installation:**
```toml
# Add to crates/mesh-lsp/Cargo.toml [dependencies]
mesh-fmt = { path = "../mesh-fmt" }
```

## Architecture Patterns

### Recommended Project Structure
```
crates/
├── mesh-lsp/
│   └── src/
│       ├── server.rs      # Add formatting handler + capability
│       └── lib.rs          # No changes needed
├── mesh-fmt/
│   └── src/
│       ├── walker.rs       # Add dedicated handlers for 7 missing nodes
│       └── lib.rs          # Add new test cases
└── mesh-repl/
    └── src/
        └── jit.rs          # Register ~30 missing runtime symbols
```

### Pattern 1: LSP Formatting Handler (Full-Document Replace)
**What:** The standard LSP formatting pattern returns a single TextEdit that replaces the entire document content with the formatted version.
**When to use:** When the formatter operates on the whole file (as `mesh_fmt::format_source` does).
**Example:**
```rust
// Source: tower-lsp docs + existing mesh-lsp patterns
async fn formatting(
    &self,
    params: DocumentFormattingParams,
) -> Result<Option<Vec<TextEdit>>> {
    let uri_str = params.text_document.uri.to_string();
    let docs = self.documents.lock().unwrap();
    let doc = match docs.get(&uri_str) {
        Some(doc) => doc,
        None => return Ok(None),
    };

    let config = mesh_fmt::FormatConfig {
        indent_size: params.options.tab_size as usize,
        ..Default::default()
    };
    let formatted = mesh_fmt::format_source(&doc.source, &config);

    if formatted == doc.source {
        return Ok(None); // Already formatted
    }

    // Single edit replacing entire document
    let line_count = doc.source.lines().count() as u32;
    let last_line_len = doc.source.lines().last().map_or(0, |l| l.len()) as u32;
    Ok(Some(vec![TextEdit {
        range: Range::new(
            Position::new(0, 0),
            Position::new(line_count, last_line_len),
        ),
        new_text: formatted,
    }]))
}
```

### Pattern 2: CST Walker Node Handler
**What:** Each SyntaxKind gets a dedicated handler function in the walker that produces optimal FormatIR.
**When to use:** When `walk_tokens_inline` produces incorrect or suboptimal formatting for a node type.
**Example:**
```rust
// Pattern for collection literals: [1, 2, 3] and %{a => 1, b => 2}
fn walk_list_literal(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::L_BRACKET => parts.push(ir::text("[")),
                SyntaxKind::R_BRACKET => parts.push(ir::text("]")),
                SyntaxKind::COMMA => {
                    parts.push(ir::text(","));
                    parts.push(ir::space());
                }
                SyntaxKind::NEWLINE => {}
                _ => { add_token_with_context(&tok, &mut parts); }
            },
            NodeOrToken::Node(n) => parts.push(walk_node(&n)),
        }
    }
    ir::group(ir::concat(parts))
}
```

### Pattern 3: JIT Symbol Registration
**What:** Each runtime function must be registered with LLVM's symbol table via `LLVMAddSymbol` for JIT resolution.
**When to use:** When new runtime functions are added that JIT-compiled code may call.
**Example:**
```rust
// From existing jit.rs pattern
add_sym("mesh_iter_map", mesh_rt::mesh_iter_map as *const ());
add_sym("mesh_iter_filter", mesh_rt::mesh_iter_filter as *const ());
add_sym("mesh_list_collect", mesh_rt::mesh_list_collect as *const ());
```

### Anti-Patterns to Avoid
- **Incremental text edits for formatting:** Computing minimal diffs is complex and error-prone. Use full-document replacement -- VS Code handles it efficiently.
- **Spawning external process for formatting:** The formatter is in-process (`mesh-fmt` crate). Do NOT shell out to `meshc fmt`.
- **Ignoring idempotency:** Every formatter change MUST be tested for idempotency (format(format(x)) == format(x)).

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Document range calculation | Manual byte-to-line conversion | `analysis::offset_to_position` | Already exists in mesh-lsp |
| Tab size handling | Custom indent config parsing | `params.options.tab_size` | LSP spec provides it |
| Format IR | Custom string builder | `mesh_fmt::ir::*` constructors | Wadler-Lindig IR already battle-tested |
| Runtime symbol registration | Manual dlsym/linking | `LLVMAddSymbol` via existing `add_sym` | Proven pattern in jit.rs |

**Key insight:** All three sub-tasks compose existing, well-tested infrastructure. The LSP handler calls `format_source`. The walker adds more match arms. The REPL adds more `add_sym` calls. No new abstractions needed.

## Common Pitfalls

### Pitfall 1: Formatter Idempotency Regression
**What goes wrong:** A new walker handler formats code differently on first vs second pass, causing infinite format-on-save loops.
**Why it happens:** The formatted output doesn't parse back to the same CST structure (e.g., missing spaces between tokens that the parser needs).
**How to avoid:** Every new test case MUST include an idempotency assertion: `assert_eq!(fmt(code), fmt(&fmt(code)))`.
**Warning signs:** Insta snapshot tests that look correct but fail the double-format check.

### Pitfall 2: Off-by-One in Document End Position
**What goes wrong:** The TextEdit range doesn't cover the last line or character, causing partial formatting.
**Why it happens:** `lines().count()` vs 0-indexed line numbers, or forgetting the trailing newline.
**How to avoid:** Use the full document range: line 0 col 0 to line_count col last_line_len. Test with files that do/don't end in newlines.
**Warning signs:** Last line of file not reformatted.

### Pitfall 3: Missing REPL Symbol Causes Silent JIT Failure
**What goes wrong:** JIT-compiled code calls an unregistered runtime function, causing a segfault or "symbol not found" error.
**Why it happens:** New runtime functions added in v7.0 were never registered in the REPL's `register_runtime_symbols()`.
**How to avoid:** Systematically cross-reference every `mesh_rt::mesh_*` public extern function against the JIT registration list. Use a test that calls each feature.
**Warning signs:** REPL crashes on iterator/collect/From/Into expressions that work fine in compiled code.

### Pitfall 4: Walk Tokens Inline Swallows Significant Tokens
**What goes wrong:** The `walk_tokens_inline` fallback emits tokens with `needs_space_before` logic that doesn't account for all delimiters.
**Why it happens:** The generic inline walker was designed for simple leaf nodes, not complex structures like map literals with `=>` operators.
**How to avoid:** Add explicit walker handlers for complex node types instead of relying on the fallback.
**Warning signs:** Missing spaces around `=>` in map literals, or collapsed `[` `]` brackets.

### Pitfall 5: Server Capability Not Advertised
**What goes wrong:** VS Code never sends `textDocument/formatting` requests because the server didn't declare the capability.
**Why it happens:** Adding the handler method without updating `initialize()` to include `document_formatting_provider`.
**How to avoid:** Always add the capability alongside the handler. Test by checking `server_capabilities()` test.
**Warning signs:** "Format Document" does nothing in VS Code, no LSP log entries for formatting.

## Code Examples

Verified patterns from the existing codebase:

### LSP Capability Advertisement
```rust
// Source: crates/mesh-lsp/src/server.rs, line 79-101
// Add document_formatting_provider to existing capabilities
ServerCapabilities {
    // ... existing capabilities ...
    document_formatting_provider: Some(OneOf::Left(true)),
    ..Default::default()
}
```

### Format Source API
```rust
// Source: crates/mesh-fmt/src/lib.rs, line 34-39
pub fn format_source(source: &str, config: &FormatConfig) -> String {
    let parse = mesh_parser::parse(source);
    let root = parse.syntax();
    let doc = walker::walk_node(&root);
    printer::print(&doc, config)
}
```

### Existing Walker Dispatch Pattern
```rust
// Source: crates/mesh-fmt/src/walker.rs, line 31-123
// Add new entries to the match in walk_node():
SyntaxKind::MAP_LITERAL => walk_map_literal(node),
SyntaxKind::MAP_ENTRY => walk_map_entry(node),
SyntaxKind::LIST_LITERAL => walk_list_literal(node),
SyntaxKind::ASSOC_TYPE_DEF => walk_tokens_inline(node),  // Simple enough
SyntaxKind::ASSOC_TYPE_BINDING => walk_assoc_type_binding(node),
SyntaxKind::FUN_TYPE => walk_tokens_inline(node),  // Already handled well
SyntaxKind::CONS_PAT => walk_tokens_inline(node),  // Already handled well
```

### Existing JIT Symbol Pattern
```rust
// Source: crates/mesh-repl/src/jit.rs, line 40-194
// Missing iterator symbols to add:
add_sym("mesh_iter_generic_next", mesh_rt::mesh_iter_generic_next as *const ());
add_sym("mesh_iter_map", mesh_rt::mesh_iter_map as *const ());
add_sym("mesh_iter_filter", mesh_rt::mesh_iter_filter as *const ());
add_sym("mesh_iter_take", mesh_rt::mesh_iter_take as *const ());
add_sym("mesh_iter_skip", mesh_rt::mesh_iter_skip as *const ());
add_sym("mesh_iter_enumerate", mesh_rt::mesh_iter_enumerate as *const ());
add_sym("mesh_iter_zip", mesh_rt::mesh_iter_zip as *const ());
// ... adapter next functions ...
// ... terminal operations ...
// ... collect operations ...
// ... From/Into conversion intrinsics ...
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No LSP formatting | `textDocument/formatting` via mesh-fmt | Phase 85 (now) | VS Code "Format Document" works |
| 7 nodes via walk_tokens_inline | Dedicated walker handlers | Phase 85 (now) | Correct formatting for map/list literals, assoc types |
| REPL lacks v7.0 runtime symbols | Full symbol registration | Phase 85 (now) | Iterator pipelines work in REPL |

**Known limitations in current formatter:**
- Pipe operator multiline idempotency: after formatting, `|>` at line start causes parser issues (noted in test comments, line 137-148 of lib.rs)
- Interface methods with do/end body have a known formatting bug (noted in test comments, line 202-203 of lib.rs)

## Open Questions

1. **Should `tab_size` from LSP params override `indent_size`?**
   - What we know: `FormatConfig::default()` uses `indent_size: 2`. LSP `FormattingOptions` provides `tab_size`.
   - What's unclear: Whether users expect the LSP indent to match the Mesh canonical style (always 2) or respect their editor setting.
   - Recommendation: Use `tab_size` from LSP params -- this is the standard LSP behavior. Users who want canonical formatting can set tab_size=2 in VS Code settings.

2. **How thorough should the REPL audit be for v7.0 features?**
   - What we know: Iterator symbols, From/Into symbols, collect symbols, and some list iterator functions are missing.
   - What's unclear: Whether ALL runtime symbols should be registered exhaustively, or only those that the compiler pipeline actually emits calls to.
   - Recommendation: Register all public `#[no_mangle] extern "C"` functions from mesh-rt. This is future-proof and the cost is negligible (just more `add_sym` calls).

3. **Should the formatter audit include correctness tests for every syntax combination?**
   - What we know: Existing tests cover most basic constructs. Missing: associated types in interface/impl, map literals, list literals, iterator pipeline formatting.
   - What's unclear: How exhaustive to be -- testing every combination is impractical.
   - Recommendation: Add one idempotent format test per missing node type, plus one "comprehensive" test that combines multiple v7.0 features in one file. Focus on constructs that actually use dedicated walker handlers.

## Detailed Gap Analysis

### Formatter Walker: Missing Dedicated Handlers

These SyntaxKind values currently fall through to `walk_tokens_inline`:

| SyntaxKind | Current Behavior | Needs Dedicated Handler? | Priority |
|------------|-----------------|--------------------------|----------|
| `ASSOC_TYPE_DEF` | Inline tokens | No -- simple `type Item` is fine inline | LOW |
| `ASSOC_TYPE_BINDING` | Inline tokens | Yes -- `type Item = Int` needs space around `=` | MEDIUM |
| `MAP_LITERAL` | Inline tokens | Yes -- `%{k => v}` needs special delimiters | HIGH |
| `MAP_ENTRY` | Inline tokens | Yes -- `k => v` needs space around `=>` | HIGH |
| `LIST_LITERAL` | Inline tokens | Yes -- `[1, 2, 3]` needs bracket/comma handling | HIGH |
| `FUN_TYPE` | Inline tokens | No -- already handled well by inline | LOW |
| `CONS_PAT` | Inline tokens | No -- `head :: tail` already has `::` spacing | LOW |

### REPL JIT: Missing Runtime Symbols

Cross-referencing `mesh-rt/src/iter.rs` and `mesh-rt/src/lib.rs` exports against `jit.rs` registrations:

**Iterator adapter constructors (6 missing):**
- `mesh_iter_map`, `mesh_iter_filter`, `mesh_iter_take`
- `mesh_iter_skip`, `mesh_iter_enumerate`, `mesh_iter_zip`

**Iterator adapter next functions (6 missing):**
- `mesh_iter_map_next`, `mesh_iter_filter_next`, `mesh_iter_take_next`
- `mesh_iter_skip_next`, `mesh_iter_enumerate_next`, `mesh_iter_zip_next`

**Generic dispatch (1 missing):**
- `mesh_iter_generic_next`

**Terminal operations (6 missing):**
- `mesh_iter_count`, `mesh_iter_sum`, `mesh_iter_any`
- `mesh_iter_all`, `mesh_iter_find`, `mesh_iter_reduce`

**Collect operations (4 missing):**
- `mesh_list_collect`, `mesh_map_collect`, `mesh_set_collect`, `mesh_string_collect`

**Collection iterator constructors (4 missing):**
- `mesh_list_iter_new`, `mesh_map_iter_new`, `mesh_set_iter_new`, `mesh_range_iter_new`

**From/Into conversion intrinsics (2 missing, built into codegen):**
- `mesh_int_to_float`, `mesh_float_to_int` -- these are codegen intrinsics, may not need registration if inlined

**Total: ~27 missing symbols**

## Sources

### Primary (HIGH confidence)
- Codebase analysis of `crates/mesh-lsp/src/server.rs` -- current LSP capabilities and handler patterns
- Codebase analysis of `crates/mesh-fmt/src/walker.rs` -- complete SyntaxKind dispatch table (lines 31-123)
- Codebase analysis of `crates/mesh-repl/src/jit.rs` -- complete symbol registration list (lines 40-194)
- Codebase analysis of `crates/mesh-rt/src/iter.rs` -- all iterator runtime functions
- Codebase analysis of `crates/mesh-parser/src/syntax_kind.rs` -- all ~84 composite node kinds

### Secondary (MEDIUM confidence)
- [tower-lsp LanguageServer trait docs](https://docs.rs/tower-lsp/latest/tower_lsp/trait.LanguageServer.html) -- formatting method signature
- [tower-lsp GitHub](https://github.com/ebkalderon/tower-lsp) -- version 0.20.0 with lsp-types 0.94.1

### Tertiary (LOW confidence)
- None -- all findings verified against codebase

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all libraries already in use, versions confirmed from Cargo.lock
- Architecture: HIGH -- patterns extracted directly from existing codebase
- Pitfalls: HIGH -- derived from known issues documented in code comments and test notes
- Gap analysis: HIGH -- systematic cross-reference of SyntaxKind enum vs walker dispatch, mesh-rt exports vs JIT registration

**Research date:** 2026-02-14
**Valid until:** 2026-03-14 (stable -- no external dependency changes expected)
