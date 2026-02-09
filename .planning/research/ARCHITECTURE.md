# Architecture: Loops & Iteration Integration

**Domain:** Adding `for..in` loops, `while` loops, and `break`/`continue` to an existing compiler with expression-oriented semantics, immutable data, and GC-managed collections
**Researched:** 2026-02-08
**Confidence:** HIGH (based on direct analysis of all six compiler stages + established LLVM loop patterns)

---

## Current Pipeline (Relevant Stages)

```
Source: "for x in items do println(x) end"
  |
  v
[snow-lexer]     FOR_KW  IDENT("x")  IN_KW  IDENT("items")  DO_KW  ...  END_KW
  Tokens exist: FOR_KW, IN_KW, DO_KW, END_KW already in TokenKind/SyntaxKind.
  Missing: WHILE_KW, BREAK_KW, CONTINUE_KW not yet reserved.
  |
  v
[snow-parser]    No CST node for loops.
  Missing: FOR_EXPR, WHILE_EXPR SyntaxKind variants.
  Missing: parse_for_expr(), parse_while_expr() parser productions.
  Missing: break/continue as expression atoms in lhs().
  |
  v
[snow-typeck]    No type inference for loops.
  Missing: infer_for(), infer_while() functions + Expr enum variants.
  Missing: loop context tracking for break/continue validation.
  Key decision: loops are expressions that return Unit (or break-value).
  |
  v
[snow-codegen/mir]  No MIR loop nodes.
  Missing: MirExpr::ForIn, MirExpr::While, MirExpr::Break, MirExpr::Continue.
  Missing: lower_for_expr(), lower_while_expr() in lower.rs.
  |
  v
[snow-codegen/codegen]  No LLVM loop block patterns.
  Missing: codegen_for_in(), codegen_while(), codegen_break(), codegen_continue().
  Pattern needed: loop_header -> loop_body -> loop_latch -> loop_exit basic blocks.
  |
  v
[snow-rt]  Runtime iteration support.
  Exists: snow_list_length(), snow_list_get() -- sufficient for indexed iteration.
  Exists: snow_range_new(), snow_range_length() -- range iteration.
  Missing: snow_map_entries() / snow_set_to_list() for map/set iteration.
  Missing: Generic iterator protocol (not needed for v1; index-based is sufficient).
```

### What Already Exists (Foundation)

1. **Lexer:** `FOR_KW` and `IN_KW` tokens are already defined in `TokenKind` (snow-common/src/token.rs line 44, 50) and mapped to `SyntaxKind::FOR_KW` / `SyntaxKind::IN_KW` (syntax_kind.rs line 38, 43). These are recognized by `keyword_from_str` (line 207, 212). They are simply unused in the parser today.

2. **Parser infrastructure:** The `do...end` block pattern used by `if`, `case`, closures, and actors is exactly the pattern loops need. `parse_block_body()` already handles block termination at `END_KW`. The `lhs()` function in `expressions.rs` dispatches on `SyntaxKind` atoms -- adding `FOR_KW` and `WHILE_KW` is mechanical.

3. **Runtime collections:** `snow_list_length(list) -> i64` and `snow_list_get(list, index) -> u64` (list.rs lines 70, 117) provide indexed access for list iteration. `snow_range_new(start, end) -> ptr` creates ranges. No iterator protocol needed -- for..in desugars to indexed loops.

4. **LLVM block patterns:** `codegen_if()` (expr.rs line 856) demonstrates the alloca+basic-block+merge pattern: create result alloca, branch to then/else blocks, store result in alloca, branch to merge, load from alloca. Loops follow the same pattern with a back-edge.

5. **Return/early exit:** `codegen_return()` (expr.rs line 1329) demonstrates emitting a terminator mid-expression and returning a dummy value. Break/continue follow this exact pattern (emit branch, return dummy).

### What Is Missing (The Gap)

| Layer | What | Effort |
|-------|------|--------|
| snow-common/token.rs | `While`, `Break`, `Continue` keywords + `keyword_from_str` entries | Trivial |
| snow-parser/syntax_kind.rs | `WHILE_KW`, `BREAK_KW`, `CONTINUE_KW` token kinds + `FOR_EXPR`, `WHILE_EXPR`, `BREAK_EXPR`, `CONTINUE_EXPR` composite nodes | Small |
| snow-parser/expressions.rs | `parse_for_expr()`, `parse_while_expr()`, break/continue atoms in `lhs()` | Medium |
| snow-parser/ast/expr.rs | `ForExpr`, `WhileExpr`, `BreakExpr`, `ContinueExpr` AST node types + Expr enum variants | Small |
| snow-typeck/infer.rs | `infer_for()`, `infer_while()`, break/continue tracking, loop context | Medium |
| snow-codegen/mir/mod.rs | `MirExpr::ForIn`, `MirExpr::While`, `MirExpr::Break`, `MirExpr::Continue` + `MirType` additions if needed | Small |
| snow-codegen/mir/lower.rs | `lower_for_expr()`, `lower_while_expr()` | Medium |
| snow-codegen/codegen/expr.rs | `codegen_for_in()`, `codegen_while()`, `codegen_break()`, `codegen_continue()` | Medium-Large |
| snow-rt | `snow_map_entries()`, `snow_set_to_list()` for collection iteration | Small |

---

## Recommended Architecture

### Design Principle: Loops as Imperative Sugar, Not New Control Flow Primitives

Snow is expression-oriented. `if/else` returns a value. `case` returns a value. Loops break this pattern because they are inherently imperative -- they produce side effects via their body, not a meaningful return value.

**Design decisions:**

1. **Loops return Unit.** `for x in items do body end` has type `Unit`. This is consistent with functional languages (Elixir's `Enum.each`, Haskell's `mapM_`). The loop's purpose is side effects.

2. **`break` with optional value is deferred.** Languages like Rust allow `break value` from loops. This adds significant complexity (break-value type must unify across all break sites + the loop's natural exit). For v1, `break` exits with no value (Unit). `break value` can be added later.

3. **`for..in` desugars to indexed iteration at MIR level.** No iterator protocol. `for x in list do body end` becomes a while loop over index 0..length with `list_get(list, i)` calls. This avoids inventing an iterator trait/protocol. Works for List, Range, Map (via keys), Set (via to_list).

4. **`while` is the primitive loop form in MIR.** `for..in` desugars to `while` during MIR lowering. Codegen only handles `while`, `break`, and `continue`. This keeps codegen simple.

5. **Break/continue are non-local branches.** At the LLVM level, `break` is an unconditional branch to the loop's exit block. `continue` is an unconditional branch to the loop's header. These require tracking the current loop's blocks during codegen.

### Component Boundaries

| Component | Responsibility | What Changes |
|-----------|---------------|--------------|
| snow-common | Token definitions | **Small** -- add 3 new keywords (`while`, `break`, `continue`) |
| snow-lexer | Tokenization | **Nothing** -- keyword_from_str handles new keywords automatically |
| snow-parser | CST construction | **Medium** -- new parser productions, new SyntaxKind variants, new AST nodes |
| snow-typeck | Type inference | **Medium** -- new infer functions, loop context for break/continue validation |
| snow-codegen/mir | MIR definitions | **Small** -- new MirExpr variants |
| snow-codegen/mir/lower.rs | AST -> MIR | **Medium** -- for..in desugaring to while, break/continue lowering |
| snow-codegen/codegen | MIR -> LLVM IR | **Medium** -- loop basic block structure, loop context for break/continue |
| snow-rt | Runtime | **Small** -- map/set iteration helpers |

### Data Flow: `for x in items do body end`

```
Source: for x in [1, 2, 3] do println(x) end

Parser Output:
  FOR_EXPR
    NAME "x"              --> loop variable
    IN_KW
    LIST_LITERAL [1,2,3]  --> iterable expression
    DO_KW
    BLOCK                 --> loop body
      CALL_EXPR println(x)
    END_KW

Type Checker (infer_for):
  1. Infer iterable: [1,2,3] :: List<Int>
  2. Extract element type: Int (from List<Int>)
  3. Bind loop variable: x :: Int in body scope
  4. Infer body in extended scope: println(x) :: Unit
  5. Loop result type: Unit
  6. Record: FOR_EXPR range -> Unit

MIR Lowering (lower_for_expr):
  DESUGARS for..in to while-based indexed iteration:

  MirExpr::Block([
    Let { name: "__iter_list", value: <iterable>, ... },
    Let { name: "__iter_len", value: Call("snow_list_length", [Var("__iter_list")]), ... },
    Let { name: "__iter_idx", value: IntLit(0), ... },
    While {
      cond: BinOp(Lt, Var("__iter_idx"), Var("__iter_len")),
      body: Block([
        Let { name: "x", value: Call("snow_list_get", [Var("__iter_list"), Var("__iter_idx")]), ... },
        <body>,
        // increment: __iter_idx = __iter_idx + 1  (mutation! -- see note below)
      ]),
      ty: Unit,
    }
  ], Unit)

  NOTE: The index variable __iter_idx requires mutation. MIR currently uses
  Let-with-continuation (functional style). Two options:
    (a) Add MirExpr::Assign for loop counter mutation (simpler codegen)
    (b) Emit the while loop as a single MIR node with implicit counter

  RECOMMENDATION: Option (b) -- MirExpr::ForIn as a first-class MIR node that
  codegen translates directly. This avoids adding general mutation to MIR.

LLVM Codegen (codegen_for_in):
  ; Entry: evaluate iterable, get length
  %list = call @snow_list_length(%iterable)
  %len = ...
  %idx_alloca = alloca i64
  store i64 0, %idx_alloca
  br label %loop_header

  loop_header:
    %idx = load i64, %idx_alloca
    %cond = icmp slt i64 %idx, %len
    br i1 %cond, label %loop_body, label %loop_exit

  loop_body:
    %elem = call @snow_list_get(%list, %idx)
    ; bind "x" to %elem
    <body codegen>
    ; increment
    %next_idx = add i64 %idx, 1
    store i64 %next_idx, %idx_alloca
    br label %loop_header

  loop_exit:
    ; result = Unit
```

### Data Flow: `while cond do body end`

```
Source: while x > 0 do x = x - 1 end

  NOTE: Snow has no mutable variables, so a pure while loop with decreasing
  counter is not directly expressible. while is primarily useful for:
  - Infinite loops with break: while true do ... break ... end
  - Loops over mutable state (if added later)
  - Internal use by for..in desugaring

Parser Output:
  WHILE_EXPR
    BINARY_EXPR (x > 0)   --> condition
    DO_KW
    BLOCK                  --> body
    END_KW

Type Checker (infer_while):
  1. Infer condition: x > 0 :: Bool
  2. Infer body: <body> :: T (any type, discarded)
  3. Loop result type: Unit

MIR:
  MirExpr::While {
    cond: BinOp(Gt, Var("x"), IntLit(0)),
    body: <body_mir>,
    ty: Unit,
  }

LLVM Codegen (codegen_while):
  br label %loop_header

  loop_header:
    %cond = <codegen condition>
    br i1 %cond, label %loop_body, label %loop_exit

  loop_body:
    <body codegen>
    br label %loop_header    ; back-edge

  loop_exit:
    ; result = Unit
```

### Data Flow: `break` and `continue`

```
Break:
  Parser: BREAK_EXPR atom (no arguments for v1)
  Typeck: Must be inside a loop. Type is Never (control flow leaves).
  MIR:    MirExpr::Break
  LLVM:   br label %loop_exit  (of enclosing loop)

Continue:
  Parser: CONTINUE_EXPR atom (no arguments)
  Typeck: Must be inside a loop. Type is Never (control flow leaves).
  MIR:    MirExpr::Continue
  LLVM:   br label %loop_header  (of enclosing loop)
          For for..in: br label %loop_latch (increment before re-test)
```

---

## New Components Per Compiler Stage

### Stage 1: Lexer / Tokens (snow-common + snow-lexer)

**New `TokenKind` variants (snow-common/src/token.rs):**
```rust
// Add to enum TokenKind:
While,     // "while"
Break,     // "break"
Continue,  // "continue"
```

**New `keyword_from_str` entries (snow-common/src/token.rs):**
```rust
"while" => Some(TokenKind::While),
"break" => Some(TokenKind::Break),
"continue" => Some(TokenKind::Continue),
```

**No changes to snow-lexer/src/lib.rs.** The `lex_ident` function calls `keyword_from_str` which handles the new keywords automatically.

**Impact:** TokenKind variant count goes from 93 to 96. The test `token_kind_variant_count` must be updated.

### Stage 2: Parser (snow-parser)

**New `SyntaxKind` variants (syntax_kind.rs):**
```rust
// Token kinds (add after existing keywords):
WHILE_KW,
BREAK_KW,
CONTINUE_KW,

// Composite node kinds (add after existing expression nodes):
FOR_EXPR,      // for x in iterable do body end
WHILE_EXPR,    // while cond do body end
BREAK_EXPR,    // break
CONTINUE_EXPR, // continue
```

**New `SyntaxKind::from(TokenKind)` arms:**
```rust
TokenKind::While => SyntaxKind::WHILE_KW,
TokenKind::Break => SyntaxKind::BREAK_KW,
TokenKind::Continue => SyntaxKind::CONTINUE_KW,
```

**New parser productions (expressions.rs):**

```
parse_for_expr:
  FOR_EXPR
    FOR_KW
    pattern        (NAME or destructuring pattern)
    IN_KW
    expr           (iterable expression)
    DO_KW
    BLOCK          (loop body via parse_block_body)
    END_KW

parse_while_expr:
  WHILE_EXPR
    WHILE_KW
    expr           (condition)
    DO_KW
    BLOCK          (loop body via parse_block_body)
    END_KW
```

**Integration into `lhs()` (expressions.rs):**
```rust
// Add to the match in lhs():
SyntaxKind::FOR_KW => Some(parse_for_expr(p)),
SyntaxKind::WHILE_KW => Some(parse_while_expr(p)),
SyntaxKind::BREAK_KW => {
    let m = p.open();
    p.advance(); // BREAK_KW
    Some(p.close(m, SyntaxKind::BREAK_EXPR))
}
SyntaxKind::CONTINUE_KW => {
    let m = p.open();
    p.advance(); // CONTINUE_KW
    Some(p.close(m, SyntaxKind::CONTINUE_EXPR))
}
```

**Integration into `parse_block_body()` -- block terminators:**
`parse_block_body()` checks for `END_KW | ELSE_KW | EOF` as block terminators (line 457). No change needed: loops use `do...end`, and the existing `END_KW` termination handles them.

**New AST node types (ast/expr.rs):**
```rust
// Add to Expr enum:
ForExpr(ForExpr),
WhileExpr(WhileExpr),
BreakExpr(BreakExpr),
ContinueExpr(ContinueExpr),

// New AST nodes:
ast_node!(ForExpr, FOR_EXPR);
impl ForExpr {
    pub fn pattern(&self) -> Option<Pattern> { ... }  // loop variable pattern
    pub fn iterable(&self) -> Option<Expr> { ... }    // the collection expression
    pub fn body(&self) -> Option<Block> { ... }       // loop body block
}

ast_node!(WhileExpr, WHILE_EXPR);
impl WhileExpr {
    pub fn condition(&self) -> Option<Expr> { ... }
    pub fn body(&self) -> Option<Block> { ... }
}

ast_node!(BreakExpr, BREAK_EXPR);
ast_node!(ContinueExpr, CONTINUE_EXPR);
```

### Stage 3: Type Checker (snow-typeck/infer.rs)

**New `infer_expr` arms:**
```rust
// Add to infer_expr match:
Expr::ForExpr(for_) => infer_for(ctx, env, for_, types, ...),
Expr::WhileExpr(while_) => infer_while(ctx, env, while_, types, ...),
Expr::BreakExpr(_) => infer_break(ctx, env, ...),
Expr::ContinueExpr(_) => infer_continue(ctx, env, ...),
```

**Loop context tracking:** The type checker needs to know whether we are inside a loop to validate break/continue. Add a loop depth counter to the inference context:

```rust
// In the inference state (passed through or stored):
loop_depth: u32,   // 0 = not in loop, >0 = inside loop(s)
```

Increment when entering a for/while body, decrement when leaving. `break` and `continue` at loop_depth 0 produce a type error: "break/continue outside of loop".

**`infer_for` logic:**
1. Infer iterable expression type.
2. Extract element type from iterable:
   - `List<T>` -> element type is `T`
   - `Range` -> element type is `Int`
   - `Map<K, V>` -> element type is `Tuple(K, V)` (or just K for key iteration -- design choice)
   - `Set<T>` -> element type is `T`
   - `String` -> element type is `String` (char iteration -- or defer)
3. Bind loop variable pattern with element type in a new scope.
4. Infer body in extended scope.
5. Return `Ty::unit()` (loops produce Unit).

**`infer_while` logic:**
1. Infer condition -- must unify with `Bool`.
2. Infer body (result discarded).
3. Return `Ty::unit()`.

**`infer_break` / `infer_continue` logic:**
1. Check `loop_depth > 0`. If not, emit error.
2. Return `Ty::Never` (like `return`, these expressions never produce a value at their call site).

### Stage 4: MIR (snow-codegen/mir/mod.rs)

**New `MirExpr` variants:**

```rust
/// For-in loop: iterates over a collection.
/// Desugared from `for pattern in iterable do body end`.
/// Codegen handles the indexed iteration directly.
ForIn {
    /// The loop variable name.
    var_name: String,
    /// The loop variable's type (element type of the collection).
    var_ty: MirType,
    /// The iterable expression (List, Range, Map, Set).
    iterable: Box<MirExpr>,
    /// What kind of collection is being iterated.
    iter_kind: IterKind,
    /// The loop body.
    body: Box<MirExpr>,
    /// Always Unit.
    ty: MirType,
},

/// While loop.
While {
    /// Loop condition (must be Bool).
    cond: Box<MirExpr>,
    /// Loop body.
    body: Box<MirExpr>,
    /// Always Unit.
    ty: MirType,
},

/// Break from the enclosing loop.
Break,

/// Continue to the next iteration of the enclosing loop.
Continue,
```

**New `IterKind` enum (describes how to iterate):**
```rust
#[derive(Debug, Clone)]
pub enum IterKind {
    /// List iteration: snow_list_length + snow_list_get
    List,
    /// Range iteration: range start/end direct access
    Range,
    /// Map iteration: snow_map_keys then iterate keys
    Map,
    /// Set iteration: snow_set_to_list then iterate
    Set,
}
```

**Update `MirExpr::ty()`:**
```rust
MirExpr::ForIn { ty, .. } => ty,
MirExpr::While { ty, .. } => ty,
MirExpr::Break => &MirType::Never,
MirExpr::Continue => &MirType::Never,
```

### Stage 5: MIR Lowering (snow-codegen/mir/lower.rs)

**New `lower_expr` arms:**
```rust
Expr::ForExpr(for_) => self.lower_for_expr(for_),
Expr::WhileExpr(while_) => self.lower_while_expr(while_),
Expr::BreakExpr(_) => MirExpr::Break,
Expr::ContinueExpr(_) => MirExpr::Continue,
```

**`lower_for_expr` -- for..in to MirExpr::ForIn:**
```rust
fn lower_for_expr(&mut self, for_: &ForExpr) -> MirExpr {
    let iterable = self.lower_expr(&for_.iterable().unwrap());
    let iter_ty = iterable.ty().clone();

    // Determine iteration kind and element type from iterable type
    let (iter_kind, elem_ty) = match &iter_ty {
        MirType::Struct(name) if name == "List" => (IterKind::List, /* extract elem */),
        // ... Range, Map, Set cases ...
        _ => panic!("cannot iterate over {:?}", iter_ty),
    };

    // Extract loop variable name from pattern
    let var_name = extract_pattern_name(&for_.pattern().unwrap());

    // Lower body with loop variable in scope
    self.push_scope();
    self.insert_var(var_name.clone(), elem_ty.clone());
    let body = self.lower_block(&for_.body().unwrap());
    self.pop_scope();

    MirExpr::ForIn {
        var_name,
        var_ty: elem_ty,
        iterable: Box::new(iterable),
        iter_kind,
        body: Box::new(body),
        ty: MirType::Unit,
    }
}
```

**`lower_while_expr` -- straightforward:**
```rust
fn lower_while_expr(&mut self, while_: &WhileExpr) -> MirExpr {
    let cond = self.lower_expr(&while_.condition().unwrap());
    self.push_scope();
    let body = self.lower_block(&while_.body().unwrap());
    self.pop_scope();

    MirExpr::While {
        cond: Box::new(cond),
        body: Box::new(body),
        ty: MirType::Unit,
    }
}
```

### Stage 6: LLVM Codegen (snow-codegen/codegen/expr.rs)

**Loop context for break/continue:** Codegen needs to know which basic blocks to branch to for `break` and `continue`. Add a stack to `CodeGen`:

```rust
// In CodeGen struct:
/// Stack of loop contexts for break/continue target resolution.
/// Each entry: (loop_header_bb, loop_exit_bb, loop_latch_bb_option)
loop_stack: Vec<LoopContext<'ctx>>,

struct LoopContext<'ctx> {
    header: BasicBlock<'ctx>,   // continue target (re-test condition)
    exit: BasicBlock<'ctx>,     // break target
    latch: Option<BasicBlock<'ctx>>,  // for..in: increment before header
}
```

**`codegen_while`:**
```rust
fn codegen_while(
    &mut self,
    cond: &MirExpr,
    body: &MirExpr,
) -> Result<BasicValueEnum<'ctx>, String> {
    let fn_val = self.current_function();

    let loop_header = self.context.append_basic_block(fn_val, "while_header");
    let loop_body = self.context.append_basic_block(fn_val, "while_body");
    let loop_exit = self.context.append_basic_block(fn_val, "while_exit");

    // Branch to header
    self.builder.build_unconditional_branch(loop_header)?;

    // Header: evaluate condition
    self.builder.position_at_end(loop_header);
    let cond_val = self.codegen_expr(cond)?.into_int_value();
    self.builder.build_conditional_branch(cond_val, loop_body, loop_exit)?;

    // Body
    self.builder.position_at_end(loop_body);
    self.loop_stack.push(LoopContext {
        header: loop_header,
        exit: loop_exit,
        latch: None,
    });
    let _ = self.codegen_expr(body)?;
    self.loop_stack.pop();

    // Back-edge (only if body didn't already terminate via break/return)
    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
        self.builder.build_unconditional_branch(loop_header)?;
    }

    // Exit
    self.builder.position_at_end(loop_exit);
    Ok(self.context.struct_type(&[], false).const_zero().into())
}
```

**`codegen_for_in` (for List iteration):**
```rust
fn codegen_for_in(
    &mut self,
    var_name: &str,
    var_ty: &MirType,
    iterable: &MirExpr,
    iter_kind: &IterKind,
    body: &MirExpr,
) -> Result<BasicValueEnum<'ctx>, String> {
    let fn_val = self.current_function();
    let i64_ty = self.context.i64_type();

    // Evaluate iterable
    let iter_val = self.codegen_expr(iterable)?;

    // Get length
    let len = match iter_kind {
        IterKind::List => {
            let len_fn = get_intrinsic(self.module, "snow_list_length");
            self.builder.build_call(len_fn, &[iter_val.into()], "len")?
                .try_as_basic_value().left().unwrap().into_int_value()
        }
        IterKind::Range => {
            let len_fn = get_intrinsic(self.module, "snow_range_length");
            self.builder.build_call(len_fn, &[iter_val.into()], "len")?
                .try_as_basic_value().left().unwrap().into_int_value()
        }
        // ... other kinds
    };

    // Index alloca
    let idx_alloca = self.builder.build_alloca(i64_ty, "__iter_idx")?;
    self.builder.build_store(idx_alloca, i64_ty.const_zero())?;

    let loop_header = self.context.append_basic_block(fn_val, "for_header");
    let loop_body = self.context.append_basic_block(fn_val, "for_body");
    let loop_latch = self.context.append_basic_block(fn_val, "for_latch");
    let loop_exit = self.context.append_basic_block(fn_val, "for_exit");

    self.builder.build_unconditional_branch(loop_header)?;

    // Header: idx < len?
    self.builder.position_at_end(loop_header);
    let idx = self.builder.build_load(i64_ty, idx_alloca, "idx")?.into_int_value();
    let cond = self.builder.build_int_compare(IntPredicate::SLT, idx, len, "cond")?;
    self.builder.build_conditional_branch(cond, loop_body, loop_exit)?;

    // Body: bind loop variable, execute body
    self.builder.position_at_end(loop_body);
    let elem = match iter_kind {
        IterKind::List => {
            let get_fn = get_intrinsic(self.module, "snow_list_get");
            self.builder.build_call(get_fn, &[iter_val.into(), idx.into()], "elem")?
                .try_as_basic_value().left().unwrap()
        }
        // Range: start + idx
        IterKind::Range => { /* load range start, add idx */ }
        // ...
    };

    // Bind loop variable
    let var_llvm_ty = self.llvm_type(var_ty);
    let var_alloca = self.builder.build_alloca(var_llvm_ty, var_name)?;
    self.builder.build_store(var_alloca, elem)?;
    let old = self.locals.insert(var_name.to_string(), var_alloca);
    let old_ty = self.local_types.insert(var_name.to_string(), var_ty.clone());

    // Push loop context (continue goes to latch, not header)
    self.loop_stack.push(LoopContext {
        header: loop_header,
        exit: loop_exit,
        latch: Some(loop_latch),
    });
    let _ = self.codegen_expr(body)?;
    self.loop_stack.pop();

    // Restore variable binding
    // ... restore old/old_ty ...

    // Fall through to latch (if not already terminated)
    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
        self.builder.build_unconditional_branch(loop_latch)?;
    }

    // Latch: increment index
    self.builder.position_at_end(loop_latch);
    let idx = self.builder.build_load(i64_ty, idx_alloca, "idx")?;
    let next_idx = self.builder.build_int_add(idx.into_int_value(),
        i64_ty.const_int(1, false), "next_idx")?;
    self.builder.build_store(idx_alloca, next_idx)?;
    self.builder.build_unconditional_branch(loop_header)?;

    // Exit
    self.builder.position_at_end(loop_exit);
    Ok(self.context.struct_type(&[], false).const_zero().into())
}
```

**`codegen_break` and `codegen_continue`:**
```rust
fn codegen_break(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
    let loop_ctx = self.loop_stack.last()
        .ok_or("break outside of loop")?;
    self.builder.build_unconditional_branch(loop_ctx.exit)?;
    // Return dummy (same pattern as codegen_return)
    Ok(self.context.struct_type(&[], false).const_zero().into())
}

fn codegen_continue(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
    let loop_ctx = self.loop_stack.last()
        .ok_or("continue outside of loop")?;
    // For for..in, continue goes to latch (increment before re-testing)
    let target = loop_ctx.latch.unwrap_or(loop_ctx.header);
    self.builder.build_unconditional_branch(target)?;
    Ok(self.context.struct_type(&[], false).const_zero().into())
}
```

### Stage 7: Runtime (snow-rt)

**New runtime functions for collection iteration:**

```rust
// Map iteration support: return keys as a List for indexed access
#[no_mangle]
pub extern "C" fn snow_map_entries(map: *mut u8) -> *mut u8
// Already exists: snow_map_keys() returns a List of keys

// Set iteration: convert to List
#[no_mangle]
pub extern "C" fn snow_set_to_list(set: *mut u8) -> *mut u8
```

For Range iteration, no new runtime function is needed. Ranges have `start` and `end` fields accessible at known offsets. Codegen can emit direct GEP to read the start value and compute `start + idx` for each iteration.

---

## Patterns to Follow

### Pattern 1: alloca + branch + merge (Established in codegen_if)

**What:** Use stack allocas for mutable loop state (index counter), basic block structure for control flow, and check for existing terminators before emitting branches.

**When:** All loop codegen.

**Existing example (codegen_if, line 856):**
```rust
let result_alloca = self.builder.build_alloca(result_ty, "if_result")?;
// ... build_conditional_branch to then/else ...
// In each block: store result, check terminator, build_unconditional_branch to merge
if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
    self.builder.build_unconditional_branch(merge_bb)?;
}
```

**Why:** The terminator check is critical. If a branch contains `return` or `break`, it already has a terminator. Emitting a second unconditional branch on a terminated block is an LLVM error.

### Pattern 2: Loop Stack for Nested Contexts

**What:** Maintain a stack of loop contexts during codegen. Push when entering a loop, pop when leaving. Break/continue reference the top of the stack.

**When:** Nested loops like `for x in xs do for y in ys do ... break ... end end`.

**Why:** Break must branch to the innermost loop's exit, not the outer loop's exit. A stack makes this automatic.

### Pattern 3: Desugaring at MIR Level (Established by pipe desugaring)

**What:** Complex source syntax desugars to simpler MIR forms. `for..in` becomes indexed iteration. The parser/typeck see the high-level form; MIR and codegen see the low-level form.

**Existing example:** `x |> f(a)` desugars to `f(x, a)` during MIR lowering. The parser produces PIPE_EXPR, but MIR only has MirExpr::Call.

**Applied here:** `for x in list do body end` retains its high-level structure through parsing and typechecking (where element type extraction needs the `for` semantics), then desugars to MirExpr::ForIn which codegen translates to indexed iteration.

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: General Mutable Variables in MIR

**What:** Adding `MirExpr::Assign { target, value }` to support the loop counter.

**Why bad:** Snow is an immutable language. Adding general mutation to MIR opens the door to mutable variables everywhere. The loop counter is an internal implementation detail that users never see.

**Instead:** Keep mutation internal to codegen. The `ForIn` MIR node encapsulates the counter; codegen uses an alloca for the index. The MIR level remains purely functional except for the loop construct itself.

### Anti-Pattern 2: Iterator Protocol at This Stage

**What:** Defining an `Iterable` trait/interface with `next()`, `has_next()` methods, and implementing it for each collection.

**Why bad:** Massively complex. Requires: trait definition, impl for each collection, stateful iterator objects (mutable state), allocation of iterator state, Option return type for `next()`. This is a v2+ feature.

**Instead:** Index-based iteration for v1. `snow_list_length` + `snow_list_get` is simple, efficient, and works. Range iteration is even simpler (just arithmetic). Map/Set iteration converts to List first.

### Anti-Pattern 3: Break-with-Value in v1

**What:** Supporting `break expr` where the loop evaluates to the break value.

**Why bad:** Requires every break site to produce the same type. The loop's "natural exit" (condition becomes false) must also produce a value of that type. Type inference must unify break values across all sites. Significant complexity for a rarely-used feature.

**Instead:** `break` exits with Unit. Loops always return Unit. If users need to compute a value from a loop, use a let binding before the loop and set it from within (or use functional combinators like `map`/`reduce`).

### Anti-Pattern 4: Separate Loop Latch Block Omission for While

**What:** Having `continue` in a `while` loop branch directly to the header, but in a `for..in` loop branch to the latch (increment). Sharing the same codegen path and hoping it works.

**Why bad:** If `continue` in a `for..in` loop branches directly to the header (skipping the latch), the index is never incremented, causing an infinite loop.

**Instead:** Use the `LoopContext.latch` field. For `while` loops, `latch` is `None` and `continue` targets the header. For `for..in` loops, `latch` is `Some(latch_bb)` and `continue` targets the latch. The `codegen_continue` function uses `latch.unwrap_or(header)`.

---

## Collection Iteration Strategy

| Collection | Iterable? | Element Type | Iteration Method | Runtime Functions |
|-----------|-----------|--------------|-----------------|-------------------|
| `List<T>` | Yes | `T` | Indexed: `length` + `get(i)` | `snow_list_length`, `snow_list_get` |
| `Range` (Int..Int) | Yes | `Int` | Direct arithmetic: `start + i` for `i in 0..length` | `snow_range_length` (or inline) |
| `Map<K, V>` | Yes (over keys) | `K` | Convert keys to List, then indexed | `snow_map_keys` (returns List) |
| `Set<T>` | Yes | `T` | Convert to List, then indexed | `snow_set_to_list` (new) |
| `String` | Deferred | `String` (single char) | Not in v1 | -- |
| `Int`, `Float`, `Bool` | No | -- | Type error | -- |

**Type extraction in typeck:**

The type checker must extract element types from collection types. The `Ty` representation uses `Ty::App(Ty::Con("List"), [T])` for `List<T>`, and `Ty::Con("Range")` for ranges.

```
Ty::App(Con("List"), [T])    -> element type: T
Ty::Con("Range")             -> element type: Int
Ty::App(Con("Map"), [K, V])  -> element type: K (iterate keys)
Ty::App(Con("Set"), [T])     -> element type: T
```

---

## Break/Continue: Interaction with Nested Constructs

### Break/Continue Inside If-Else

```snow
for x in items do
  if x > 10 do
    break       # valid: branches to loop exit
  end
end
```

This works because `break` emits `br label %loop_exit` regardless of which basic block it appears in. The LLVM verifier ensures the branch target is valid within the function.

**Caveat:** After `break` or `continue`, the current basic block has a terminator. Any code after them in the same block is unreachable. The existing pattern of checking `get_terminator().is_none()` before emitting the back-edge handles this.

### Break/Continue Inside Case/Match

```snow
for x in items do
  case x do
    0 -> continue
    42 -> break
    n -> println(n)
  end
end
```

This works because case arms are generated as separate basic blocks. Each arm that contains break/continue will have its branch to the loop's exit/header instead of branching to the case merge block.

**Important:** The case merge block may become unreachable if ALL arms break/continue. The LLVM verifier tolerates unreachable blocks, but they should ideally be cleaned up by the optimizer.

### Break/Continue Inside Closures -- PROHIBITED

```snow
for x in items do
  items.map(fn y ->
    break  # ERROR: break is not inside a loop in this scope
  end)
end
```

Break/continue must not escape closure boundaries. The loop context stack is scoped to the current function being compiled. When codegen enters a closure body (a separate `MirFunction`), the loop stack is empty. The typeck loop_depth counter should similarly reset to 0 when entering a closure body.

---

## Suggested Build Order

Based on dependency analysis:

### Phase 1: Tokens + Parser (foundation, no behavioral change)
1. Add `While`, `Break`, `Continue` to `TokenKind` and `keyword_from_str`
2. Add `WHILE_KW`, `BREAK_KW`, `CONTINUE_KW`, `FOR_EXPR`, `WHILE_EXPR`, `BREAK_EXPR`, `CONTINUE_EXPR` to `SyntaxKind`
3. Implement `parse_for_expr()`, `parse_while_expr()`, break/continue atoms in `lhs()`
4. Add `ForExpr`, `WhileExpr`, `BreakExpr`, `ContinueExpr` AST nodes to Expr enum
5. Parser snapshot tests for all four forms

### Phase 2: Type Checker (semantic correctness)
1. Add loop_depth tracking to inference state
2. Implement `infer_for()` with element type extraction
3. Implement `infer_while()` with Bool condition check
4. Implement `infer_break()` and `infer_continue()` with loop context validation
5. Ensure closures reset loop_depth to 0
6. Type error tests: break outside loop, non-bool while condition, non-iterable for

### Phase 3: MIR + Lowering (desugaring)
1. Add `ForIn`, `While`, `Break`, `Continue` to `MirExpr`
2. Add `IterKind` enum
3. Implement `lower_for_expr()` and `lower_while_expr()`
4. Update `lower_expr` dispatch
5. MIR snapshot tests

### Phase 4: Codegen + Integration (LLVM IR generation)
1. Add `LoopContext` and `loop_stack` to `CodeGen`
2. Implement `codegen_while()` with basic block structure
3. Implement `codegen_for_in()` for List iteration
4. Implement `codegen_break()` and `codegen_continue()`
5. Add `for_exit` handling for codegen dispatch
6. Extend for Range, Map, Set iteration
7. Add runtime functions if needed (`snow_set_to_list`)
8. End-to-end tests: compile and run loop programs

### Build Order Rationale

```
Phase 1 (Parser) --> Phase 2 (Typeck) --> Phase 3 (MIR) --> Phase 4 (Codegen)
    |                    |                    |                    |
    No deps              Needs AST nodes     Needs typed AST     Needs MIR nodes
```

Strictly sequential because each stage consumes the output of the previous one. Within each phase, the order is: while (simpler) before for..in (complex desugaring), break/continue last (needs loop context).

---

## LLVM IR Structure Reference

### While Loop

```llvm
entry:
  br label %while_header

while_header:                      ; loop condition
  %cond = ...                      ; evaluate condition
  br i1 %cond, label %while_body, label %while_exit

while_body:                        ; loop body
  ...                              ; body code
  br label %while_header           ; back-edge

while_exit:                        ; after loop
  ; result = {} (unit)
```

### For-In Loop (List)

```llvm
entry:
  %list = ...                      ; evaluate iterable
  %len = call i64 @snow_list_length(ptr %list)
  %idx = alloca i64
  store i64 0, ptr %idx
  br label %for_header

for_header:                        ; index check
  %i = load i64, ptr %idx
  %cond = icmp slt i64 %i, %len
  br i1 %cond, label %for_body, label %for_exit

for_body:                          ; element access + body
  %elem = call i64 @snow_list_get(ptr %list, i64 %i)
  %x = alloca i64                  ; loop variable
  store i64 %elem, ptr %x
  ...                              ; body code
  br label %for_latch

for_latch:                         ; increment
  %i2 = load i64, ptr %idx
  %i3 = add i64 %i2, 1
  store i64 %i3, ptr %idx
  br label %for_header             ; back-edge

for_exit:                          ; after loop
  ; result = {} (unit)
```

### Break Inside Conditional

```llvm
for_body:
  %x_val = load i64, ptr %x
  %test = icmp sgt i64 %x_val, 10
  br i1 %test, label %break_bb, label %no_break

break_bb:
  br label %for_exit               ; break -> loop exit

no_break:
  ...                              ; rest of body
  br label %for_latch
```

---

## Sources

### Codebase Analysis (HIGH confidence)
- `crates/snow-common/src/token.rs` -- TokenKind enum (93 variants), `keyword_from_str` (line 191), `For` (line 44), `In` (line 50) already defined
- `crates/snow-parser/src/syntax_kind.rs` -- SyntaxKind enum, `FOR_KW` (line 38), `IN_KW` (line 43) already defined but unused in parser
- `crates/snow-parser/src/parser/expressions.rs` -- Pratt parser `lhs()` (line 165), `parse_block_body()` (line 445), `parse_if_expr()` (line 557) as pattern
- `crates/snow-parser/src/ast/expr.rs` -- Expr enum (line 16), AST node definitions pattern
- `crates/snow-typeck/src/infer.rs` -- `infer_expr` dispatch (line 2355), `infer_if` pattern (line 2380)
- `crates/snow-typeck/src/ty.rs` -- Ty enum, `Ty::App` for parameterized types (line 55)
- `crates/snow-codegen/src/mir/mod.rs` -- MirExpr enum (line 143), MirType (line 61)
- `crates/snow-codegen/src/mir/lower.rs` -- Lowerer struct (line 120), `lower_expr` dispatch (line 3056), scope management (line 182)
- `crates/snow-codegen/src/codegen/expr.rs` -- `codegen_if` with alloca+branch pattern (line 856), `codegen_return` with dummy value (line 1329), `codegen_block` (line 975)
- `crates/snow-codegen/src/codegen/mod.rs` -- CodeGen struct with locals/local_types (line 43)
- `crates/snow-codegen/src/codegen/intrinsics.rs` -- `snow_list_length`, `snow_list_get`, `snow_range_length` declarations (lines 233-278)
- `crates/snow-rt/src/collections/list.rs` -- List layout: `{len: u64, cap: u64, data: [u64]}`, `snow_list_get` (line 117)
- `crates/snow-rt/src/collections/range.rs` -- Range layout: `{start: i64, end: i64}`, `snow_range_new` (line 24), `snow_range_length` (line 125)

### LLVM Loop Patterns (HIGH confidence -- well-established)
- LLVM Language Reference: Loop structure with header/body/latch/exit blocks
- Standard indexed loop pattern with alloca counter and icmp+br termination

---
*Architecture research for: Snow Loops & Iteration Milestone*
*Researched: 2026-02-08*
