//! Algorithm J inference engine for Snow.
//!
//! Walks the Snow AST, generates type constraints, and solves them via
//! unification. Implements Hindley-Milner type inference with:
//! - Let-polymorphism (generalize + instantiate)
//! - Occurs check (rejects infinite types)
//! - Level-based generalization (Remy's algorithm)
//! - Error provenance via ConstraintOrigin

use rowan::TextRange;
use snow_parser::ast::expr::{
    BinaryExpr, CallExpr, CaseExpr, ClosureExpr, Expr, IfExpr, Literal, NameRef, PipeExpr,
    ReturnExpr, TupleExpr, UnaryExpr,
};
use snow_parser::ast::item::{Block, FnDef, Item, LetBinding};
use snow_parser::ast::pat::Pattern;
use snow_parser::ast::AstNode;
use snow_parser::syntax_kind::SyntaxKind;
use snow_parser::Parse;

use crate::builtins;
use crate::env::TypeEnv;
use crate::error::{ConstraintOrigin, TypeError};
use crate::ty::{Scheme, Ty};
use crate::unify::InferCtx;
use crate::TypeckResult;

use rustc_hash::FxHashMap;

/// Infer types for a parsed Snow program.
///
/// This is the main entry point. Creates an inference context and type
/// environment, registers builtins, then walks the AST inferring types.
pub fn infer(parse: &Parse) -> TypeckResult {
    let mut ctx = InferCtx::new();
    let mut env = TypeEnv::new();
    builtins::register_builtins(&mut ctx, &mut env);

    let mut types = FxHashMap::default();
    let mut result_type = None;

    let tree = parse.tree();

    // Walk all children of SourceFile. Items are handled via Item::cast,
    // bare expressions (top-level expressions not wrapped in items) are
    // handled via Expr::cast.
    for child in tree.syntax().children() {
        if let Some(item) = Item::cast(child.clone()) {
            let ty = infer_item(&mut ctx, &mut env, &item, &mut types);
            if let Some(ty) = ty {
                result_type = Some(ty);
            }
        } else if let Some(expr) = Expr::cast(child.clone()) {
            match infer_expr(&mut ctx, &mut env, &expr, &mut types) {
                Ok(ty) => {
                    let resolved = ctx.resolve(ty.clone());
                    types.insert(expr.syntax().text_range(), resolved.clone());
                    result_type = Some(resolved);
                }
                Err(_) => {
                    // Error already recorded in ctx.errors
                }
            }
        }
    }

    // Resolve all types in the type table through the union-find.
    let resolved_types: FxHashMap<TextRange, Ty> = types
        .into_iter()
        .map(|(range, ty)| (range, ctx.resolve(ty)))
        .collect();

    // Resolve the result type as well.
    let resolved_result = result_type.map(|ty| ctx.resolve(ty));

    TypeckResult {
        types: resolved_types,
        errors: ctx.errors,
        result_type: resolved_result,
    }
}

// ── Item Inference ─────────────────────────────────────────────────────

/// Infer the type of a top-level or nested item.
/// Returns the type of the item (for let bindings, the type of the initializer;
/// for function defs, the function type).
fn infer_item(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    item: &Item,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Option<Ty> {
    match item {
        Item::LetBinding(let_) => infer_let_binding(ctx, env, let_, types).ok(),
        Item::FnDef(fn_) => infer_fn_def(ctx, env, fn_, types).ok(),
        // Declarations that don't produce a value type in this plan:
        Item::ModuleDef(_)
        | Item::ImportDecl(_)
        | Item::FromImportDecl(_)
        | Item::StructDef(_)
        | Item::InterfaceDef(_)
        | Item::ImplDef(_)
        | Item::TypeAliasDef(_) => None,
    }
}

/// Infer a let binding: `let x = expr`
///
/// Uses enter_level/leave_level for let-polymorphism:
/// 1. Enter a new level
/// 2. Infer the initializer's type
/// 3. Leave the level
/// 4. Generalize the type into a scheme
/// 5. Bind the name in the environment
fn infer_let_binding(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    let_: &LetBinding,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    // Enter level for generalization.
    ctx.enter_level();

    // Infer the initializer expression.
    let init_expr = let_.initializer().ok_or_else(|| {
        let err = TypeError::Mismatch {
            expected: Ty::Never,
            found: Ty::Never,
            origin: ConstraintOrigin::Builtin,
        };
        ctx.errors.push(err.clone());
        err
    })?;

    let init_ty = infer_expr(ctx, env, &init_expr, types)?;

    // Leave level and generalize.
    ctx.leave_level();
    let scheme = ctx.generalize(init_ty.clone());

    // Bind the name in the environment.
    if let Some(name) = let_.name() {
        if let Some(name_text) = name.text() {
            env.insert(name_text, scheme);
        }
    } else if let Some(pat) = let_.pattern() {
        // Pattern destructuring -- bind pattern variables.
        let pat_ty = infer_pattern(ctx, env, &pat, types)?;
        ctx.unify(
            pat_ty,
            init_ty.clone(),
            ConstraintOrigin::LetBinding {
                binding_span: let_.syntax().text_range(),
            },
        )?;
    }

    let resolved = ctx.resolve(init_ty);
    types.insert(let_.syntax().text_range(), resolved.clone());

    Ok(resolved)
}

/// Infer a named function definition: `fn name(params) do body end`
///
/// Pre-binds the function name to a fresh variable (for recursion),
/// infers the body, then generalizes the function type.
fn infer_fn_def(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    fn_: &FnDef,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    let fn_name = fn_
        .name()
        .and_then(|n| n.text())
        .unwrap_or_else(|| "<anonymous>".to_string());

    // Enter level for generalization.
    ctx.enter_level();

    // Pre-bind function name to a fresh var for recursion.
    let self_var = ctx.fresh_var();
    env.insert(fn_name.clone(), Scheme::mono(self_var.clone()));

    // Create fresh type variables for parameters.
    env.push_scope();
    let mut param_types = Vec::new();

    if let Some(param_list) = fn_.param_list() {
        for param in param_list.params() {
            let param_ty = ctx.fresh_var();
            if let Some(name_tok) = param.name() {
                let name_text = name_tok.text().to_string();
                env.insert(name_text, Scheme::mono(param_ty.clone()));
            }
            param_types.push(param_ty);
        }
    }

    // Infer body.
    let body_ty = if let Some(body) = fn_.body() {
        infer_block(ctx, env, &body, types)?
    } else {
        Ty::Tuple(vec![]) // unit
    };

    env.pop_scope();

    // Build function type.
    let fn_ty = Ty::Fun(param_types, Box::new(body_ty));

    // Unify self_var with inferred function type (for recursion).
    ctx.unify(self_var, fn_ty.clone(), ConstraintOrigin::Builtin)?;

    // Leave level and generalize.
    ctx.leave_level();
    let scheme = ctx.generalize(fn_ty.clone());

    // Re-bind the function name with the generalized scheme.
    env.insert(fn_name, scheme);

    let resolved = ctx.resolve(fn_ty);
    types.insert(fn_.syntax().text_range(), resolved.clone());

    Ok(resolved)
}

// ── Expression Inference ───────────────────────────────────────────────

/// Infer the type of an expression.
///
/// This is the main expression dispatcher. It matches on all Expr variants
/// and delegates to specialized inference functions.
fn infer_expr(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    expr: &Expr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    let ty = match expr {
        Expr::Literal(lit) => infer_literal(lit),
        Expr::NameRef(name_ref) => infer_name_ref(ctx, env, name_ref)?,
        Expr::BinaryExpr(bin) => infer_binary(ctx, env, bin, types)?,
        Expr::UnaryExpr(un) => infer_unary(ctx, env, un, types)?,
        Expr::CallExpr(call) => infer_call(ctx, env, call, types)?,
        Expr::PipeExpr(pipe) => infer_pipe(ctx, env, pipe, types)?,
        Expr::IfExpr(if_) => infer_if(ctx, env, if_, types)?,
        Expr::ClosureExpr(closure) => infer_closure(ctx, env, closure, types)?,
        Expr::Block(block) => infer_block(ctx, env, block, types)?,
        Expr::TupleExpr(tuple) => infer_tuple(ctx, env, tuple, types)?,
        Expr::CaseExpr(case) => infer_case(ctx, env, case, types)?,
        Expr::ReturnExpr(ret) => infer_return(ctx, env, ret, types)?,
        Expr::StringExpr(_) => Ty::string(),
        Expr::FieldAccess(_) => ctx.fresh_var(), // Deferred to 03-03
        Expr::IndexExpr(_) => ctx.fresh_var(),   // Deferred to 03-03
    };

    let resolved = ctx.resolve(ty.clone());
    types.insert(expr.syntax().text_range(), resolved.clone());

    Ok(ty)
}

/// Infer the type of a literal expression.
fn infer_literal(lit: &Literal) -> Ty {
    if let Some(token) = lit.token() {
        match token.kind() {
            SyntaxKind::INT_LITERAL => Ty::int(),
            SyntaxKind::FLOAT_LITERAL => Ty::float(),
            SyntaxKind::TRUE_KW | SyntaxKind::FALSE_KW => Ty::bool(),
            SyntaxKind::NIL_KW => Ty::Tuple(vec![]), // nil as unit for now
            SyntaxKind::STRING_START => Ty::string(),
            _ => Ty::Tuple(vec![]), // unit fallback
        }
    } else {
        Ty::Tuple(vec![]) // unit fallback
    }
}

/// Infer the type of a name reference (variable lookup).
///
/// Looks up the name in the environment and instantiates its scheme.
/// Returns UnboundVariable error if not found.
fn infer_name_ref(
    ctx: &mut InferCtx,
    env: &TypeEnv,
    name_ref: &NameRef,
) -> Result<Ty, TypeError> {
    let name = name_ref
        .text()
        .unwrap_or_else(|| "<unknown>".to_string());

    match env.lookup(&name) {
        Some(scheme) => Ok(ctx.instantiate(scheme)),
        None => {
            let err = TypeError::UnboundVariable {
                name,
                span: name_ref.syntax().text_range(),
            };
            ctx.errors.push(err.clone());
            Err(err)
        }
    }
}

/// Infer the type of a binary expression.
///
/// For arithmetic ops (+, -, *, /, %): unify both operands, return same type.
/// For comparison ops (<, >, <=, >=, ==, !=): unify both operands, return Bool.
/// For logical ops (and, or): unify both operands with Bool, return Bool.
fn infer_binary(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    bin: &BinaryExpr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    let lhs_expr = bin.lhs().ok_or_else(|| {
        let err = TypeError::Mismatch {
            expected: Ty::Never,
            found: Ty::Never,
            origin: ConstraintOrigin::Builtin,
        };
        ctx.errors.push(err.clone());
        err
    })?;
    let rhs_expr = bin.rhs().ok_or_else(|| {
        let err = TypeError::Mismatch {
            expected: Ty::Never,
            found: Ty::Never,
            origin: ConstraintOrigin::Builtin,
        };
        ctx.errors.push(err.clone());
        err
    })?;

    let lhs_ty = infer_expr(ctx, env, &lhs_expr, types)?;
    let rhs_ty = infer_expr(ctx, env, &rhs_expr, types)?;

    let op = bin.op();
    let op_kind = op.as_ref().map(|t| t.kind());

    let origin = ConstraintOrigin::BinOp {
        op_span: bin.syntax().text_range(),
    };

    match op_kind {
        // Arithmetic: unify both sides, return the common type
        Some(
            SyntaxKind::PLUS
            | SyntaxKind::MINUS
            | SyntaxKind::STAR
            | SyntaxKind::SLASH
            | SyntaxKind::PERCENT,
        ) => {
            ctx.unify(lhs_ty.clone(), rhs_ty, origin)?;
            Ok(lhs_ty)
        }

        // Comparison: unify both sides, return Bool
        Some(
            SyntaxKind::LT
            | SyntaxKind::GT
            | SyntaxKind::LT_EQ
            | SyntaxKind::GT_EQ
            | SyntaxKind::EQ_EQ
            | SyntaxKind::NOT_EQ,
        ) => {
            ctx.unify(lhs_ty, rhs_ty, origin)?;
            Ok(Ty::bool())
        }

        // Logical: unify both sides with Bool, return Bool
        Some(SyntaxKind::AND_KW | SyntaxKind::OR_KW | SyntaxKind::AMP_AMP | SyntaxKind::PIPE_PIPE) => {
            ctx.unify(lhs_ty, Ty::bool(), origin.clone())?;
            ctx.unify(rhs_ty, Ty::bool(), origin)?;
            Ok(Ty::bool())
        }

        // Concatenation operators: unify both sides, return same type
        Some(SyntaxKind::DIAMOND | SyntaxKind::PLUS_PLUS) => {
            ctx.unify(lhs_ty.clone(), rhs_ty, origin)?;
            Ok(lhs_ty)
        }

        // Unknown op: return a fresh variable
        _ => {
            let result = ctx.fresh_var();
            Ok(result)
        }
    }
}

/// Infer the type of a unary expression.
fn infer_unary(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    un: &UnaryExpr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    let operand = un.operand().ok_or_else(|| {
        let err = TypeError::Mismatch {
            expected: Ty::Never,
            found: Ty::Never,
            origin: ConstraintOrigin::Builtin,
        };
        ctx.errors.push(err.clone());
        err
    })?;

    let operand_ty = infer_expr(ctx, env, &operand, types)?;

    let op_kind = un.op().map(|t| t.kind());

    match op_kind {
        Some(SyntaxKind::MINUS) => {
            // Numeric negation: operand stays the same type.
            Ok(operand_ty)
        }
        Some(SyntaxKind::BANG | SyntaxKind::NOT_KW) => {
            // Logical not: operand must be Bool, result is Bool.
            ctx.unify(
                operand_ty,
                Ty::bool(),
                ConstraintOrigin::Builtin,
            )?;
            Ok(Ty::bool())
        }
        _ => Ok(operand_ty),
    }
}

/// Infer the type of a function call expression.
///
/// 1. Infer the callee type
/// 2. Infer each argument type
/// 3. Create a fresh return type variable
/// 4. Unify callee with Fun(arg_types, ret_var)
/// 5. Return ret_var
fn infer_call(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    call: &CallExpr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    let callee_expr = call.callee().ok_or_else(|| {
        let err = TypeError::Mismatch {
            expected: Ty::Never,
            found: Ty::Never,
            origin: ConstraintOrigin::Builtin,
        };
        ctx.errors.push(err.clone());
        err
    })?;

    let callee_ty = infer_expr(ctx, env, &callee_expr, types)?;

    // Collect argument types.
    let mut arg_types = Vec::new();
    if let Some(arg_list) = call.arg_list() {
        for arg in arg_list.args() {
            let arg_ty = infer_expr(ctx, env, &arg, types)?;
            arg_types.push(arg_ty);
        }
    }

    // Create fresh return type variable.
    let ret_var = ctx.fresh_var();

    // Build the expected function type.
    let expected_fn_ty = Ty::Fun(arg_types, Box::new(ret_var.clone()));

    // Unify callee type with expected function type.
    let origin = ConstraintOrigin::FnArg {
        call_site: call.syntax().text_range(),
        param_idx: 0,
    };
    ctx.unify(callee_ty, expected_fn_ty, origin)?;

    Ok(ret_var)
}

/// Infer the type of a pipe expression: `lhs |> rhs`
///
/// The rhs must be a function that accepts lhs as its first argument.
fn infer_pipe(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    pipe: &PipeExpr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    let lhs = pipe.lhs().ok_or_else(|| {
        let err = TypeError::Mismatch {
            expected: Ty::Never,
            found: Ty::Never,
            origin: ConstraintOrigin::Builtin,
        };
        ctx.errors.push(err.clone());
        err
    })?;
    let rhs = pipe.rhs().ok_or_else(|| {
        let err = TypeError::Mismatch {
            expected: Ty::Never,
            found: Ty::Never,
            origin: ConstraintOrigin::Builtin,
        };
        ctx.errors.push(err.clone());
        err
    })?;

    let lhs_ty = infer_expr(ctx, env, &lhs, types)?;
    let rhs_ty = infer_expr(ctx, env, &rhs, types)?;

    let ret_var = ctx.fresh_var();
    let expected_fn = Ty::Fun(vec![lhs_ty], Box::new(ret_var.clone()));

    ctx.unify(rhs_ty, expected_fn, ConstraintOrigin::Builtin)?;

    Ok(ret_var)
}

/// Infer the type of an if expression.
///
/// - Condition must be Bool.
/// - Then and else branches must have the same type.
/// - If no else branch, the type is unit.
fn infer_if(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    if_: &IfExpr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    // Infer condition.
    if let Some(cond) = if_.condition() {
        let cond_ty = infer_expr(ctx, env, &cond, types)?;
        ctx.unify(
            cond_ty,
            Ty::bool(),
            ConstraintOrigin::Builtin,
        )?;
    }

    // Infer then branch.
    let then_ty = if let Some(then_block) = if_.then_branch() {
        infer_block(ctx, env, &then_block, types)?
    } else {
        Ty::Tuple(vec![]) // unit
    };

    // Infer else branch.
    if let Some(else_branch) = if_.else_branch() {
        let else_ty = if let Some(else_if) = else_branch.if_expr() {
            infer_if(ctx, env, &else_if, types)?
        } else if let Some(else_block) = else_branch.block() {
            infer_block(ctx, env, &else_block, types)?
        } else {
            Ty::Tuple(vec![]) // unit
        };

        // Unify then and else branch types.
        let origin = ConstraintOrigin::IfBranches {
            if_span: if_.syntax().text_range(),
            then_span: if_
                .then_branch()
                .map(|b| b.syntax().text_range())
                .unwrap_or_else(|| if_.syntax().text_range()),
            else_span: else_branch.syntax().text_range(),
        };
        ctx.unify(then_ty.clone(), else_ty, origin)?;

        Ok(then_ty)
    } else {
        // No else: the type is the then-branch type (could be unit).
        Ok(then_ty)
    }
}

/// Infer the type of a closure expression: `fn (params) -> body end`
///
/// 1. Create fresh type variables for each parameter
/// 2. Push scope, bind parameters
/// 3. Infer body
/// 4. Pop scope
/// 5. Return Fun(param_types, body_type)
fn infer_closure(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    closure: &ClosureExpr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    env.push_scope();

    let mut param_types = Vec::new();

    if let Some(param_list) = closure.param_list() {
        for param in param_list.params() {
            let param_ty = ctx.fresh_var();
            if let Some(name_tok) = param.name() {
                let name_text = name_tok.text().to_string();
                env.insert(name_text, Scheme::mono(param_ty.clone()));
            }
            param_types.push(param_ty);
        }
    }

    // Infer body.
    let body_ty = if let Some(body) = closure.body() {
        infer_block(ctx, env, &body, types)?
    } else {
        Ty::Tuple(vec![]) // unit
    };

    env.pop_scope();

    Ok(Ty::Fun(param_types, Box::new(body_ty)))
}

/// Infer the type of a block.
///
/// A block contains items/statements and optionally a tail expression.
/// The type of the block is the type of the last expression.
fn infer_block(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    block: &Block,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    let mut last_ty = Ty::Tuple(vec![]); // unit

    // Infer all statements (items).
    for stmt in block.stmts() {
        if let Some(ty) = infer_item(ctx, env, &stmt, types) {
            last_ty = ty;
        }
    }

    // Infer tail expression if present.
    // The tail_expr is the last child that casts to Expr.
    // But if it was already captured as an Item (e.g. let binding),
    // we need to check if there's a bare expression at the end.
    if let Some(tail) = block.tail_expr() {
        // Check if this tail expression is different from the last statement.
        // It might be the initializer of a let binding, in which case we
        // already inferred it. We need to check by comparing text ranges.
        let tail_range = tail.syntax().text_range();
        let is_already_part_of_stmt = block.stmts().any(|stmt| {
            let item_range = match &stmt {
                Item::LetBinding(lb) => lb.syntax().text_range(),
                Item::FnDef(fd) => fd.syntax().text_range(),
                _ => return false,
            };
            // The tail expression's range is contained within an item's range.
            item_range.start() <= tail_range.start() && tail_range.end() <= item_range.end()
        });

        if !is_already_part_of_stmt {
            match infer_expr(ctx, env, &tail, types) {
                Ok(ty) => {
                    last_ty = ty;
                }
                Err(_) => {
                    // Error already recorded
                }
            }
        }
    }

    Ok(last_ty)
}

/// Infer the type of a tuple expression.
fn infer_tuple(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    tuple: &TupleExpr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    let mut elem_types = Vec::new();
    for elem in tuple.elements() {
        let ty = infer_expr(ctx, env, &elem, types)?;
        elem_types.push(ty);
    }

    // Single-element tuple is just the element type (grouping parens).
    if elem_types.len() == 1 {
        Ok(elem_types.into_iter().next().unwrap())
    } else {
        Ok(Ty::Tuple(elem_types))
    }
}

/// Infer the type of a case/match expression.
fn infer_case(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    case: &CaseExpr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    // Infer scrutinee.
    let scrutinee_ty = if let Some(scrutinee) = case.scrutinee() {
        infer_expr(ctx, env, &scrutinee, types)?
    } else {
        ctx.fresh_var()
    };

    let mut result_ty: Option<Ty> = None;

    for arm in case.arms() {
        env.push_scope();

        // Infer pattern type and bind variables.
        if let Some(pat) = arm.pattern() {
            let pat_ty = infer_pattern(ctx, env, &pat, types)?;
            ctx.unify(
                pat_ty,
                scrutinee_ty.clone(),
                ConstraintOrigin::Builtin,
            )?;
        }

        // Infer arm body.
        if let Some(body) = arm.body() {
            let body_ty = infer_expr(ctx, env, &body, types)?;
            if let Some(ref prev_ty) = result_ty {
                ctx.unify(
                    prev_ty.clone(),
                    body_ty.clone(),
                    ConstraintOrigin::Builtin,
                )?;
            } else {
                result_ty = Some(body_ty);
            }
        }

        env.pop_scope();
    }

    Ok(result_ty.unwrap_or_else(|| Ty::Tuple(vec![])))
}

/// Infer the type of a return expression.
fn infer_return(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    ret: &ReturnExpr,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    if let Some(value) = ret.value() {
        let _ty = infer_expr(ctx, env, &value, types)?;
    }
    Ok(Ty::Never)
}

// ── Pattern Inference ──────────────────────────────────────────────────

/// Infer the type of a pattern, binding any variables into the environment.
fn infer_pattern(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    pat: &Pattern,
    types: &mut FxHashMap<TextRange, Ty>,
) -> Result<Ty, TypeError> {
    match pat {
        Pattern::Ident(ident) => {
            let ty = ctx.fresh_var();
            if let Some(name_tok) = ident.name() {
                let name_text = name_tok.text().to_string();
                env.insert(name_text, Scheme::mono(ty.clone()));
            }
            types.insert(pat.syntax().text_range(), ty.clone());
            Ok(ty)
        }
        Pattern::Wildcard(_) => {
            let ty = ctx.fresh_var();
            types.insert(pat.syntax().text_range(), ty.clone());
            Ok(ty)
        }
        Pattern::Literal(lit) => {
            let ty = if let Some(token) = lit.token() {
                match token.kind() {
                    SyntaxKind::INT_LITERAL => Ty::int(),
                    SyntaxKind::FLOAT_LITERAL => Ty::float(),
                    SyntaxKind::TRUE_KW | SyntaxKind::FALSE_KW => Ty::bool(),
                    SyntaxKind::NIL_KW => Ty::Tuple(vec![]),
                    SyntaxKind::STRING_START => Ty::string(),
                    _ => ctx.fresh_var(),
                }
            } else {
                ctx.fresh_var()
            };
            types.insert(pat.syntax().text_range(), ty.clone());
            Ok(ty)
        }
        Pattern::Tuple(tuple_pat) => {
            let mut elem_types = Vec::new();
            for sub_pat in tuple_pat.patterns() {
                let ty = infer_pattern(ctx, env, &sub_pat, types)?;
                elem_types.push(ty);
            }
            let ty = Ty::Tuple(elem_types);
            types.insert(pat.syntax().text_range(), ty.clone());
            Ok(ty)
        }
    }
}
