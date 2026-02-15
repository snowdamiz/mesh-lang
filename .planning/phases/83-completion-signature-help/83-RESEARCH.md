# Phase 83: LSP Completion + Signature Help - Research

**Researched:** 2026-02-14
**Domain:** LSP textDocument/completion and textDocument/signatureHelp for Mesh
**Confidence:** HIGH

## Summary

This phase adds two closely related LSP features to the Mesh language server: code completion (keyword, type, snippet, and scope-aware variable/function completions) and signature help (parameter info and active parameter tracking inside function call parentheses). Both features build on the existing tower-lsp 0.20 / lsp-types 0.94.1 infrastructure already in `crates/mesh-lsp/`.

The existing LSP server (`MeshBackend`) already implements hover, diagnostics, go-to-definition, and document symbols. It stores `DocumentState` per open document containing parsed CST (`mesh_parser::Parse`) and type-check results (`TypeckResult`). The completion and signature help handlers will follow the identical pattern: look up the document state, find the cursor position, and compute results from the CST and/or type environment.

For **completion**, the three simpler tiers (keywords, built-in types, snippets) require no CST analysis -- they can be static lists filtered by prefix. The fourth tier (scope-aware variable/function names from CST walk) reuses the same upward-walking pattern already proven in `definition.rs::find_variable_or_function_def`, but collects all in-scope names rather than searching for one specific name. For **signature help**, the challenge is locating the enclosing CALL_EXPR, extracting the function name, looking up its type (Ty::Fun), and counting comma tokens to determine the active parameter index.

**Primary recommendation:** Implement completion as a static keyword/type/snippet provider augmented by a CST-walk scope collector. Implement signature help by walking the CST upward from cursor position to find the enclosing CALL_EXPR/ARG_LIST, resolving the callee name via the existing definition infrastructure, then extracting parameter info from the typeck result's type map.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| tower-lsp | 0.20.0 | LSP server framework | Already in use; provides `completion` and `signature_help` trait methods |
| lsp-types | 0.94.1 | LSP type definitions | Provides `CompletionItem`, `CompletionItemKind`, `SignatureHelp`, `SignatureInformation`, `ParameterInformation` |
| rowan | 0.16.1 | CST library | Already in use for all CST traversal |
| mesh-parser | workspace | Parse source into CST/AST | CST node kinds for scope walk and CALL_EXPR detection |
| mesh-typeck | workspace | Type inference results | `TypeckResult.types` for resolving function types |
| mesh-lexer | workspace | Tokenization | Offset mapping via `source_to_tree_offset`/`tree_to_source_offset` |
| mesh-common | workspace | Token/keyword definitions | `keyword_from_str` for keyword list, `TokenKind` for keyword enumeration |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| serde_json | workspace | Already a dependency | Not needed for this feature specifically |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| CST walk for scope completion | Full semantic analysis from TypeEnv | TypeEnv is not persisted in the LSP state; the CST walk is simpler and sufficient for variable/function names without needing to re-run type inference with custom scope tracking |
| Static keyword list | Re-derive from TokenKind | Static list is simpler and more maintainable for completion |

## Architecture Patterns

### Current File Structure (relevant files)
```
crates/mesh-lsp/
  src/server.rs         # LanguageServer trait impl -- add completion + signature_help handlers
  src/analysis.rs       # Document analysis, offset/position conversion (reuse)
  src/definition.rs     # CST traversal for go-to-definition (reuse patterns)
  src/lib.rs            # Module structure -- add new modules
  Cargo.toml            # Dependencies (no new deps needed)
crates/mesh-parser/
  src/syntax_kind.rs    # All CST node/token kinds (CALL_EXPR, ARG_LIST, etc.)
  src/ast/item.rs       # Typed AST wrappers (FnDef, ParamList, Param)
  src/ast/expr.rs       # CallExpr, ArgList typed wrappers
crates/mesh-typeck/
  src/ty.rs             # Ty::Fun(params, ret) -- function type representation
  src/builtins.rs       # Built-in function type schemes (parameter types)
  src/lib.rs            # TypeckResult with types map
crates/mesh-common/
  src/token.rs          # keyword_from_str, TokenKind enum (48 keywords)
```

### Pattern 1: Completion Provider Architecture

**What:** A module that computes `CompletionItem` lists based on cursor context.

**Structure:**
- New file `crates/mesh-lsp/src/completion.rs`
- Four completion tiers, all combined into a single response:
  1. **Keywords** -- static list of 48 keywords, filtered by typed prefix
  2. **Built-in types** -- static list (Int, Float, String, Bool, List, Map, Set, Option, Result, Queue, Range, Pid, Json, Request, Response, Router, SqliteConn, PgConn, PoolHandle)
  3. **Snippets** -- `InsertTextFormat::Snippet` items for common patterns
  4. **Scope-aware names** -- CST walk upward from cursor position collecting in-scope let/fn/param names

**When to use:** Called by the `completion` handler in server.rs.

**Capability advertisement:**
```rust
completion_provider: Some(CompletionOptions {
    trigger_characters: None, // completions triggered by typing any identifier character
    resolve_provider: Some(false), // no lazy resolution needed
    ..Default::default()
}),
```

### Pattern 2: Scope-Aware CST Walk for Completion

**What:** Walk the CST upward from the cursor position, collecting all visible names.

**Existing go-to-definition pattern (from `definition.rs`):**
```rust
// Walk up from current node through BLOCKs and FN_DEFs
let mut current = name_ref_node.parent()?;
loop {
    match current.kind() {
        SyntaxKind::BLOCK | SyntaxKind::SOURCE_FILE => {
            // Search earlier siblings in this block
            search_block_for_def(&current, name_ref_node, name)
        }
        SyntaxKind::FN_DEF => {
            // Check function parameters
            search_params_for_name(&current, name)
        }
        SyntaxKind::CLOSURE_EXPR => {
            // Check closure parameters
            search_params_for_name(&current, name)
        }
        _ => {}
    }
    current = current.parent()?;
}
```

**Adapted for completion (collect ALL names, not search for one):**
- Walk upward from the token at cursor offset
- At each BLOCK/SOURCE_FILE: collect names from LET_BINDING, FN_DEF, ACTOR_DEF, SERVICE_DEF, MODULE_DEF, STRUCT_DEF, SUM_TYPE_DEF children that appear before the cursor (or all for SOURCE_FILE)
- At each FN_DEF/CLOSURE_EXPR: collect parameter names from PARAM_LIST
- Deduplicate (inner scopes shadow outer)
- Filter by typed prefix

**Key difference from go-to-definition:** Go-to-definition searches for ONE specific name upward. Completion collects ALL visible names upward. The traversal pattern is identical.

### Pattern 3: Snippet Completions

**What:** `InsertTextFormat::Snippet` completions that expand common Mesh patterns.

**Required snippets (COMP-03):**

| Trigger | Expansion | Note |
|---------|-----------|------|
| `fn` | `fn ${1:name}(${2:params}) do\n  ${0}\nend` | Function definition |
| `let` | `let ${1:name} = ${0}` | Let binding |
| `struct` | `struct ${1:Name} do\n  ${0}\nend` | Struct definition |
| `match` / `case` | `case ${1:expr} do\n  ${2:pattern} -> ${0}\nend` | Pattern match |
| `for` | `for ${1:item} in ${2:collection} do\n  ${0}\nend` | For loop |
| `while` | `while ${1:condition} do\n  ${0}\nend` | While loop |
| `actor` | `actor ${1:Name}(${2:state}) do\n  ${0}\nend` | Actor definition |
| `interface` | `interface ${1:Name} do\n  ${0}\nend` | Interface definition |
| `impl` | `impl ${1:Trait} for ${2:Type} do\n  ${0}\nend` | Impl block |

### Pattern 4: Signature Help Provider Architecture

**What:** A module that computes `SignatureHelp` when the cursor is inside function call parentheses.

**Structure:**
- New file `crates/mesh-lsp/src/signature_help.rs`
- Steps:
  1. Convert LSP position to byte offset
  2. Convert byte offset to tree offset (reuse `source_to_tree_offset`)
  3. Find the enclosing CALL_EXPR / ARG_LIST by walking CST upward
  4. Extract the callee function name from the CALL_EXPR's first child (NAME_REF or FIELD_ACCESS)
  5. Look up the function's type in `TypeckResult.types` or resolve via the existing CST definition infrastructure
  6. If the type is `Ty::Fun(params, ret)`, build `SignatureInformation` with parameter labels
  7. Count COMMA tokens before the cursor in the ARG_LIST to determine `active_parameter`

**Capability advertisement:**
```rust
signature_help_provider: Some(SignatureHelpOptions {
    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
    retrigger_characters: None,
    ..Default::default()
}),
```

### Pattern 5: Active Parameter Tracking

**What:** Determine which parameter is currently being typed by counting commas.

**Algorithm:**
```
Given: cursor position inside ARG_LIST
1. Find the ARG_LIST node containing the cursor
2. Count COMMA tokens that appear before the cursor's tree offset
3. active_parameter = number of commas before cursor
```

**Edge cases:**
- Cursor right after `(` -> active_parameter = 0
- Cursor right after first `,` -> active_parameter = 1
- Nested calls: `foo(bar(x), y)` -- walk UP to find the outermost ARG_LIST where cursor is directly inside (not in a nested CALL_EXPR)

### Pattern 6: Parameter Name Extraction from CST

**What:** The type system stores `Ty::Fun(Vec<Ty>, Box<Ty>)` which has parameter TYPES but not parameter NAMES. Parameter names must come from the CST.

**Strategy for user-defined functions:**
1. Use the go-to-definition infrastructure to find the FN_DEF node for the called function
2. Extract parameter names from the FN_DEF's PARAM_LIST children
3. Combine with types from `Ty::Fun` to produce labeled parameters

**Strategy for built-in functions:**
- Built-in functions (registered in `builtins.rs`) have types but no CST nodes
- For builtins, use only type labels: `param0: Int`, `param1: String`, etc.
- OR maintain a static map of builtin function name -> parameter name list
- Recommendation: start with type-only labels for builtins; enhance with a static name map later if users request it

### Anti-Patterns to Avoid
- **Don't rerun type inference for completion:** The existing `AnalysisResult` already stores the parse and typeck result. Use them directly.
- **Don't try to complete in broken parse states initially:** The CST from the error-recovering parser is sufficient for scope walks even with parse errors. Don't add special incomplete-token detection initially.
- **Don't block on full scope analysis:** Simple prefix matching on CST-extracted names is fast. Don't try to build a full semantic model.
- **Don't return too many completions:** Filter by prefix. Cap results if needed (though unlikely given the static lists are small).

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| LSP completion protocol | Custom JSON-RPC | tower-lsp's `completion` trait method | Protocol compliance, edge cases |
| Snippet syntax | Custom template engine | LSP snippet syntax (`${1:placeholder}`) | VS Code and all LSP clients natively support this |
| Position conversion | New offset functions | Existing `position_to_offset_pub`, `offset_to_position`, `source_to_tree_offset`, `tree_to_source_offset` | Already tested and working |
| Scope walking | New tree traversal | Adapt `definition.rs` patterns | Already handles blocks, functions, closures, shadowing |

**Key insight:** The hardest part of completion (scope-aware name collection) is a generalization of go-to-definition (scope-aware name search). The existing code in `definition.rs` provides a proven traversal pattern that just needs to collect rather than search.

## Common Pitfalls

### Pitfall 1: Whitespace-Stripped CST Coordinate Mismatch
**What goes wrong:** The Mesh lexer skips whitespace, so rowan tree offsets differ from source byte offsets. Cursor positions from the editor are in source coordinates.
**Why it happens:** LSP positions are in source coordinates (line/character). The CST uses tree coordinates (no whitespace).
**How to avoid:** ALWAYS convert source offsets to tree offsets via `source_to_tree_offset` before doing any CST traversal. ALWAYS convert back via `tree_to_source_offset` before returning results to the client. This is already handled correctly by the go-to-definition code.
**Warning signs:** Completion or signature help triggers in the wrong place; off-by-N errors in parameter highlighting.

### Pitfall 2: Cursor Between Tokens
**What goes wrong:** When the user is typing a partial identifier, the cursor may be at a position that doesn't correspond to any complete token in the CST.
**Why it happens:** The parser's error recovery may not create a node at the exact cursor position.
**How to avoid:** For completion, use `token_at_offset` with `right_biased()` (like go-to-definition does). If no token is found, fall back to prefix matching against the raw source text from the last word boundary to the cursor position.
**Warning signs:** No completions shown while typing.

### Pitfall 3: Nested Call Expressions for Signature Help
**What goes wrong:** `foo(bar(x, |), y)` -- the cursor at `|` should show signature help for `bar`, not `foo`.
**Why it happens:** Walking upward from the cursor hits ARG_LIST nodes for both calls.
**How to avoid:** When walking upward to find the enclosing CALL_EXPR, take the INNERMOST one whose ARG_LIST directly contains the cursor position (not inside a nested child CALL_EXPR).
**Warning signs:** Wrong function's parameter info shown in nested calls.

### Pitfall 4: Type Resolution for Callee
**What goes wrong:** The function being called may not have a type entry in `TypeckResult.types` at the exact range expected.
**Why it happens:** TypeckResult maps `TextRange -> Ty`. The callee of a call expr may be a NAME_REF whose range doesn't exactly match what's stored.
**How to avoid:** Try multiple lookup strategies: (a) look up the callee NAME_REF's text range in the types map, (b) look up the entire CALL_EXPR's text range, (c) fall back to searching by function name in the types map. The hover implementation already handles similar range-matching issues.
**Warning signs:** Signature help shows no info even when the function type is known.

### Pitfall 5: Snippet Completion Conflicts with Keywords
**What goes wrong:** Typing `fn` shows both the keyword completion AND the snippet completion.
**Why it happens:** Both matchers activate on the same prefix.
**How to avoid:** Make snippets have a `CompletionItemKind::Snippet` and keywords have `CompletionItemKind::Keyword`. Let the editor's built-in deduplication handle it. Alternatively, suppress keyword completions when a matching snippet exists.
**Warning signs:** Duplicate entries in the completion list.

### Pitfall 6: Parameter Names Unavailable for Built-in Functions
**What goes wrong:** Signature help for `println(...)` shows `(String) -> ()` but no parameter name.
**Why it happens:** Built-in functions are registered with type schemes only; there's no CST to extract parameter names from.
**How to avoid:** For Phase 83, accept type-only labels for builtins (e.g., `String` as the parameter label). Optionally, create a static map `fn_name -> &[&str]` for the most common builtins (println, print, map, filter, reduce, head, tail). This is a quality-of-life enhancement, not a blocker.
**Warning signs:** Signature help shows raw type names instead of meaningful parameter names.

## Code Examples

### Example 1: Advertising Completion and Signature Help Capabilities

```rust
// In server.rs, inside the `initialize` method:
async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
    Ok(InitializeResult {
        capabilities: ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(
                TextDocumentSyncKind::FULL,
            )),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            definition_provider: Some(OneOf::Left(true)),
            document_symbol_provider: Some(OneOf::Left(true)),
            // NEW: Completion
            completion_provider: Some(CompletionOptions {
                trigger_characters: None,
                resolve_provider: Some(false),
                ..Default::default()
            }),
            // NEW: Signature help
            signature_help_provider: Some(SignatureHelpOptions {
                trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                retrigger_characters: None,
                work_done_progress_options: Default::default(),
            }),
            ..Default::default()
        },
        ..Default::default()
    })
}
```

### Example 2: Completion Handler Skeleton

```rust
// In server.rs:
async fn completion(
    &self,
    params: CompletionParams,
) -> Result<Option<CompletionResponse>> {
    let uri_str = params.text_document_position.text_document.uri.to_string();
    let position = params.text_document_position.position;

    let docs = self.documents.lock().unwrap();
    let doc = match docs.get(&uri_str) {
        Some(doc) => doc,
        None => return Ok(None),
    };

    let items = crate::completion::compute_completions(
        &doc.source,
        &doc.analysis,
        &position,
    );

    Ok(Some(CompletionResponse::Array(items)))
}
```

### Example 3: Keyword/Type/Snippet Completion (Static Lists)

```rust
// In completion.rs:
const KEYWORDS: &[&str] = &[
    "actor", "after", "alias", "and", "break", "call", "case", "cast",
    "cond", "continue", "def", "do", "else", "end", "false", "fn",
    "for", "if", "impl", "import", "in", "interface", "let", "link",
    "match", "module", "monitor", "nil", "not", "or", "pub", "receive",
    "return", "self", "send", "service", "spawn", "struct", "supervisor",
    "terminate", "trait", "trap", "true", "type", "when", "where",
    "while", "with",
];

const BUILTIN_TYPES: &[&str] = &[
    "Int", "Float", "String", "Bool", "List", "Map", "Set", "Option",
    "Result", "Queue", "Range", "Pid", "Json", "Request", "Response",
    "Router", "SqliteConn", "PgConn", "PoolHandle",
];

fn keyword_completions(prefix: &str) -> Vec<CompletionItem> {
    KEYWORDS.iter()
        .filter(|kw| kw.starts_with(prefix))
        .map(|kw| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        })
        .collect()
}
```

### Example 4: Scope Walk for Variable/Function Collection

```rust
// In completion.rs -- adapted from definition.rs patterns:
fn collect_in_scope_names(
    source: &str,
    root: &SyntaxNode,
    source_offset: usize,
) -> Vec<(String, CompletionItemKind)> {
    let tree_offset = source_to_tree_offset(source, source_offset)?;
    let target = rowan::TextSize::from(tree_offset as u32);
    let token = root.token_at_offset(target).right_biased()?;

    let mut names = Vec::new();
    let mut current = token.parent()?;

    loop {
        match current.kind() {
            SyntaxKind::BLOCK | SyntaxKind::SOURCE_FILE => {
                // Collect names from children before cursor (or all for SOURCE_FILE)
                collect_block_names(&current, target, &mut names);
            }
            SyntaxKind::FN_DEF | SyntaxKind::CLOSURE_EXPR => {
                // Collect parameter names
                collect_param_names(&current, &mut names);
            }
            _ => {}
        }
        current = match current.parent() {
            Some(p) => p,
            None => break,
        };
    }

    names
}
```

### Example 5: Signature Help -- Finding Enclosing Call

```rust
// In signature_help.rs:
fn find_enclosing_call(
    root: &SyntaxNode,
    tree_offset: rowan::TextSize,
) -> Option<(SyntaxNode, usize)> {
    // Find the token at the cursor position
    let token = root.token_at_offset(tree_offset).right_biased()?;

    // Walk up to find the innermost CALL_EXPR
    let mut node = token.parent()?;
    loop {
        if node.kind() == SyntaxKind::ARG_LIST {
            // Found an arg list -- parent should be CALL_EXPR
            let call_expr = node.parent()?;
            if call_expr.kind() == SyntaxKind::CALL_EXPR {
                // Count commas before cursor in the ARG_LIST
                let comma_count = node.children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .filter(|t| t.kind() == SyntaxKind::COMMA)
                    .filter(|t| t.text_range().end() <= tree_offset)
                    .count();
                return Some((call_expr, comma_count));
            }
        }
        node = node.parent()?;
    }
}
```

### Example 6: Extracting Function Parameter Info

```rust
// In signature_help.rs:
fn build_signature_info(
    source: &str,
    root: &SyntaxNode,
    callee_name: &str,
    fn_type: &Ty,
) -> Option<SignatureInformation> {
    match fn_type {
        Ty::Fun(params, ret) => {
            // Try to find parameter names from FN_DEF in CST
            let param_names = find_fn_def_param_names(root, callee_name);

            let param_infos: Vec<ParameterInformation> = params.iter()
                .enumerate()
                .map(|(i, ty)| {
                    let label = match param_names.as_ref().and_then(|names| names.get(i)) {
                        Some(name) => format!("{}: {}", name, ty),
                        None => format!("{}", ty),
                    };
                    ParameterInformation {
                        label: ParameterLabel::Simple(label),
                        documentation: None,
                    }
                })
                .collect();

            let label = format!("{}({}) -> {}",
                callee_name,
                param_infos.iter()
                    .map(|p| match &p.label {
                        ParameterLabel::Simple(s) => s.clone(),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                ret,
            );

            Some(SignatureInformation {
                label,
                documentation: None,
                parameters: Some(param_infos),
                active_parameter: None,
            })
        }
        _ => None,
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No completion | Phase 83 adds completion | This phase | Users get keyword, type, snippet, and scope completions |
| No signature help | Phase 83 adds signature help | This phase | Users see parameter info while typing function calls |

**Existing tower-lsp completion support:**
- tower-lsp 0.20 provides `async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>>` on the `LanguageServer` trait
- tower-lsp 0.20 provides `async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>>` on the `LanguageServer` trait
- Both are already available in the trait -- just need to override the default (which returns `Ok(None)`)

## Open Questions

1. **Parameter names for built-in functions**
   - What we know: Built-in functions have `Ty::Fun` types with parameter types but no names. The CST has no FN_DEF nodes for builtins.
   - What's unclear: Whether users will find type-only parameter labels acceptable, or if a static name map is needed from the start.
   - Recommendation: Start with type-only labels. Add a static name map as a follow-up enhancement if feedback warrants it. This keeps initial scope manageable.

2. **Completion trigger for dot-access (`Module.func`)**
   - What we know: Module-qualified access uses `expr.field` syntax. Typing `IO.` could trigger member completion.
   - What's unclear: Whether COMP-01 through COMP-04 require dot-triggered completion for module members.
   - Recommendation: Defer dot-triggered member completion to a future phase. The current requirements focus on unqualified completions. Dot-completion would require knowing module exports, which is a more complex feature.

3. **Completion in type annotation position**
   - What we know: When typing after `::` (type annotation), the user likely wants type names, not variable names.
   - What's unclear: Whether COMP-02 requires context-sensitive filtering (only types after `::`, only values in expression position).
   - Recommendation: For Phase 83, always return the full set (keywords + types + snippets + scope names) and let the editor's built-in filtering handle context. Context-sensitive filtering can be added incrementally.

4. **Interaction with the whitespace-stripped coordinate system**
   - What we know: The existing `source_to_tree_offset` / `tree_to_source_offset` functions handle the coordinate conversion correctly (tested, roundtrip verified).
   - What's unclear: Edge cases when the cursor is inside whitespace (between tokens) -- `source_to_tree_offset` returns `None` for offsets inside whitespace.
   - Recommendation: For completion, extract the prefix by scanning the source text backwards from cursor to the last non-identifier character. This avoids the tree offset entirely for prefix extraction. Only use tree offsets for the CST scope walk.

5. **Scope-aware completion complexity (blocker noted in state)**
   - What we know: The state notes "Scope-aware CST walk complexity -- may need prototype before full implementation."
   - What's unclear: How complex the walk actually is beyond what go-to-definition already does.
   - Recommendation: The go-to-definition code already implements the exact upward-walking, block-searching, parameter-checking pattern needed. The only change is collecting all names instead of searching for one. This is a straightforward generalization. The blocker concern is overstated -- the existing code proves the pattern works.

## Sources

### Primary (HIGH confidence)
- **Codebase analysis**: Direct examination of `crates/mesh-lsp/src/server.rs`, `analysis.rs`, `definition.rs` -- all LSP handler patterns, offset conversion, CST traversal
- **Codebase analysis**: Direct examination of `crates/mesh-parser/src/syntax_kind.rs` -- all 48 keywords, all composite node kinds (CALL_EXPR, ARG_LIST, PARAM_LIST, etc.)
- **Codebase analysis**: Direct examination of `crates/mesh-typeck/src/ty.rs` -- `Ty::Fun(Vec<Ty>, Box<Ty>)` representation, `Scheme` for polymorphic types
- **Codebase analysis**: Direct examination of `crates/mesh-typeck/src/builtins.rs` -- all built-in type names and function signatures
- **Codebase analysis**: Direct examination of `crates/mesh-common/src/token.rs` -- `keyword_from_str` with complete keyword list
- **Codebase analysis**: Direct examination of `crates/mesh-parser/src/ast/item.rs` -- FnDef.param_list(), ParamList.params(), Param.name()
- **Codebase analysis**: Direct examination of `crates/mesh-parser/src/ast/expr.rs` -- CallExpr.callee(), CallExpr.arg_list(), ArgList.args()
- **Codebase analysis**: `Cargo.lock` confirms tower-lsp 0.20.0, lsp-types 0.94.1
- **Phase 81 research**: Confirmed patterns for CST traversal and offset conversion

### Secondary (MEDIUM confidence)
- **tower-lsp 0.20 API**: The `LanguageServer` trait includes `completion` and `signature_help` methods with default implementations returning `Ok(None)`. Verified by the fact that the current server compiles without implementing them.
- **LSP specification**: CompletionItem, SignatureHelp, ParameterInformation are standard LSP 3.17 types available in lsp-types 0.94.1.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all libraries already in use, no new dependencies
- Architecture: HIGH -- patterns directly derived from existing code (go-to-definition, hover, document symbols)
- Pitfalls: HIGH -- identified from direct analysis of coordinate system, CST structure, and type representation
- Scope walk complexity: HIGH -- existing go-to-definition proves the pattern; generalization is straightforward

**Research date:** 2026-02-14
**Valid until:** 60 days (stable domain; tower-lsp and codebase unlikely to change)
