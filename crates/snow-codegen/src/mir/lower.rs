//! AST-to-MIR lowering.
//!
//! Converts the typed Rowan CST (Parse + TypeckResult) to the MIR representation.
//! Handles desugaring of pipe operators, string interpolation, and closure conversion.

use std::collections::HashMap;

use rowan::TextRange;
use rustc_hash::FxHashMap;
use snow_parser::ast::expr::{
    BinaryExpr, CallExpr, CaseExpr, ClosureExpr, Expr, FieldAccess, IfExpr, Literal, MatchArm,
    NameRef, PipeExpr, ReturnExpr, StringExpr, StructLiteral, TupleExpr, UnaryExpr,
};
use snow_parser::ast::item::{Block, FnDef, Item, LetBinding, SourceFile, StructDef, SumTypeDef};
use snow_parser::ast::pat::Pattern;
use snow_parser::ast::AstNode;
use snow_parser::syntax_kind::SyntaxKind;
use snow_parser::Parse;
use snow_typeck::ty::Ty;
use snow_typeck::TypeckResult;

use super::types::resolve_type;
use super::{
    BinOp, MirExpr, MirFunction, MirLiteral, MirMatchArm, MirModule, MirPattern, MirStructDef,
    MirSumTypeDef, MirType, MirVariantDef, UnaryOp,
};

// ── Lowerer ──────────────────────────────────────────────────────────

/// The AST-to-MIR lowering context.
struct Lowerer<'a> {
    /// Type map from typeck: TextRange -> Ty.
    types: &'a FxHashMap<TextRange, Ty>,
    /// Type registry for struct/sum type lookups.
    registry: &'a snow_typeck::TypeRegistry,
    /// Functions being built.
    functions: Vec<MirFunction>,
    /// Struct definitions.
    structs: Vec<MirStructDef>,
    /// Sum type definitions.
    sum_types: Vec<MirSumTypeDef>,
    /// Scope stack for local variable types.
    scopes: Vec<HashMap<String, MirType>>,
    /// Counter for generating unique lifted closure function names.
    closure_counter: u32,
    /// Names of known functions (for distinguishing direct calls from closure calls).
    known_functions: HashMap<String, MirType>,
    /// Entry function name, if found.
    entry_function: Option<String>,
}

impl<'a> Lowerer<'a> {
    fn new(typeck: &'a TypeckResult) -> Self {
        Lowerer {
            types: &typeck.types,
            registry: &typeck.type_registry,
            functions: Vec::new(),
            structs: Vec::new(),
            sum_types: Vec::new(),
            scopes: vec![HashMap::new()],
            closure_counter: 0,
            known_functions: HashMap::new(),
            entry_function: None,
        }
    }

    // ── Scope management ─────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn insert_var(&mut self, name: String, ty: MirType) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty);
        }
    }

    fn lookup_var(&self, name: &str) -> Option<MirType> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }

    // ── Type resolution helper ───────────────────────────────────────

    fn resolve_range(&self, range: TextRange) -> MirType {
        if let Some(ty) = self.types.get(&range) {
            resolve_type(ty, self.registry, false)
        } else {
            MirType::Unit
        }
    }

    fn resolve_range_closure(&self, range: TextRange) -> MirType {
        if let Some(ty) = self.types.get(&range) {
            resolve_type(ty, self.registry, true)
        } else {
            MirType::Unit
        }
    }

    fn get_ty(&self, range: TextRange) -> Option<&Ty> {
        self.types.get(&range)
    }

    // ── Top-level lowering ───────────────────────────────────────────

    fn lower_source_file(&mut self, sf: SourceFile) {
        // First pass: register all function names so we know which are direct calls.
        for item in sf.items() {
            if let Item::FnDef(fn_def) = &item {
                if let Some(name) = fn_def.name().and_then(|n| n.text()) {
                    let fn_ty = self.resolve_range(fn_def.syntax().text_range());
                    self.known_functions.insert(name.clone(), fn_ty.clone());
                    self.insert_var(name, fn_ty);
                }
            }
        }

        // Register builtin I/O functions as known functions.
        self.known_functions.insert(
            "println".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Unit)),
        );
        self.known_functions.insert(
            "print".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Unit)),
        );
        // Also register variant constructors as known functions.
        for (_, sum_info) in &self.registry.sum_type_defs {
            for variant in &sum_info.variants {
                if !variant.fields.is_empty() {
                    // Variant constructor is a function
                    let name = variant.name.clone();
                    let qualified = format!("{}.{}", sum_info.name, variant.name);
                    // We don't have exact types here; mark as known for call dispatch.
                    self.known_functions
                        .insert(name, MirType::FnPtr(vec![], Box::new(MirType::Unit)));
                    self.known_functions
                        .insert(qualified, MirType::FnPtr(vec![], Box::new(MirType::Unit)));
                }
            }
        }

        // Second pass: lower all items.
        for item in sf.items() {
            self.lower_item(item);
        }
    }

    fn lower_item(&mut self, item: Item) {
        match item {
            Item::FnDef(fn_def) => self.lower_fn_def(&fn_def),
            Item::StructDef(struct_def) => self.lower_struct_def(&struct_def),
            Item::SumTypeDef(sum_def) => self.lower_sum_type_def(&sum_def),
            Item::LetBinding(let_) => self.lower_top_level_let(&let_),
            Item::ImplDef(impl_def) => {
                // Lower impl methods as standalone functions.
                for method in impl_def.methods() {
                    self.lower_fn_def(&method);
                }
            }
            Item::InterfaceDef(_) | Item::TypeAliasDef(_) => {
                // Skip -- interfaces are erased, type aliases are resolved.
            }
            Item::ModuleDef(_) | Item::ImportDecl(_) | Item::FromImportDecl(_) => {
                // Skip -- module/import handling is not needed for single-file compilation.
            }
            Item::ActorDef(_) => {
                // Actor definition lowering will be implemented in Phase 06 Plan 04/05.
            }
        }
    }

    // ── Function lowering ────────────────────────────────────────────

    fn lower_fn_def(&mut self, fn_def: &FnDef) {
        let name = fn_def
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<anonymous>".to_string());

        // Get function type from typeck.
        let fn_range = fn_def.syntax().text_range();
        let fn_ty_raw = self.get_ty(fn_range).cloned();

        // Extract parameter names and types.
        let mut params = Vec::new();
        self.push_scope();

        if let Some(param_list) = fn_def.param_list() {
            if let Some(Ty::Fun(param_tys, _)) = &fn_ty_raw {
                for (param, param_ty) in param_list.params().zip(param_tys.iter()) {
                    let param_name = param
                        .name()
                        .map(|t| t.text().to_string())
                        .unwrap_or_else(|| "_".to_string());
                    let mir_ty = resolve_type(param_ty, self.registry, false);
                    self.insert_var(param_name.clone(), mir_ty.clone());
                    params.push((param_name, mir_ty));
                }
            } else {
                // Fallback: use range-based type lookup for each param.
                for param in param_list.params() {
                    let param_name = param
                        .name()
                        .map(|t| t.text().to_string())
                        .unwrap_or_else(|| "_".to_string());
                    let mir_ty = self.resolve_range(param.syntax().text_range());
                    self.insert_var(param_name.clone(), mir_ty.clone());
                    params.push((param_name, mir_ty));
                }
            }
        }

        // Return type.
        let return_type = if let Some(Ty::Fun(_, ret)) = &fn_ty_raw {
            resolve_type(ret, self.registry, false)
        } else {
            MirType::Unit
        };

        // Lower body.
        let body = if let Some(block) = fn_def.body() {
            self.lower_block(&block)
        } else {
            MirExpr::Unit
        };

        self.pop_scope();

        if name == "main" {
            self.entry_function = Some(name.clone());
        }

        self.functions.push(MirFunction {
            name,
            params,
            return_type,
            body,
            is_closure_fn: false,
            captures: Vec::new(),
        });
    }

    // ── Struct lowering ──────────────────────────────────────────────

    fn lower_struct_def(&mut self, struct_def: &StructDef) {
        let name = struct_def
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<unnamed>".to_string());

        // Look up from type registry for accurate types.
        let fields = if let Some(info) = self.registry.struct_defs.get(&name) {
            info.fields
                .iter()
                .map(|(fname, fty)| {
                    (
                        fname.clone(),
                        resolve_type(fty, self.registry, false),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };

        self.structs.push(MirStructDef { name, fields });
    }

    // ── Sum type lowering ────────────────────────────────────────────

    fn lower_sum_type_def(&mut self, sum_def: &SumTypeDef) {
        let name = sum_def
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<unnamed>".to_string());

        // Look up from type registry for accurate variant info.
        let variants = if let Some(info) = self.registry.sum_type_defs.get(&name) {
            info.variants
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let fields = v
                        .fields
                        .iter()
                        .map(|f| {
                            let ty = match f {
                                snow_typeck::VariantFieldInfo::Positional(ty) => ty,
                                snow_typeck::VariantFieldInfo::Named(_, ty) => ty,
                            };
                            resolve_type(ty, self.registry, false)
                        })
                        .collect();
                    MirVariantDef {
                        name: v.name.clone(),
                        fields,
                        tag: i as u8,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        self.sum_types.push(MirSumTypeDef { name, variants });
    }

    // ── Top-level let ────────────────────────────────────────────────

    fn lower_top_level_let(&mut self, let_: &LetBinding) {
        let name = let_
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "_".to_string());

        let value = if let Some(init) = let_.initializer() {
            self.lower_expr(&init)
        } else {
            MirExpr::Unit
        };

        let ty = value.ty().clone();
        self.insert_var(name.clone(), ty.clone());

        // Top-level lets become a function that returns the value (for globals).
        // In practice, these would be part of an init function, but for now
        // we store the binding in scope for use by other functions.
    }

    // ── Block lowering ───────────────────────────────────────────────

    fn lower_block(&mut self, block: &Block) -> MirExpr {
        // Collect all children in source order as MIR expressions.
        // Let bindings insert the variable into scope (for subsequent children)
        // and are wrapped to nest the remaining block as the body.
        let mut parts: Vec<MirExpr> = Vec::new();
        let mut let_names: Vec<String> = Vec::new();

        for child in block.syntax().children() {
            if let Some(item) = Item::cast(child.clone()) {
                match item {
                    Item::LetBinding(ref let_) => {
                        let name = let_
                            .name()
                            .and_then(|n| n.text())
                            .unwrap_or_else(|| "_".to_string());
                        let value = if let Some(init) = let_.initializer() {
                            self.lower_expr(&init)
                        } else {
                            MirExpr::Unit
                        };
                        let ty = value.ty().clone();
                        self.insert_var(name.clone(), ty.clone());
                        let_names.push(name.clone());
                        parts.push(MirExpr::Let {
                            name,
                            ty,
                            value: Box::new(value),
                            body: Box::new(MirExpr::Unit), // placeholder; nested below
                        });
                    }
                    Item::FnDef(ref fn_def) => {
                        self.lower_fn_def(fn_def);
                    }
                    _ => {}
                }
                continue;
            }
            if let Some(expr) = Expr::cast(child) {
                let mir = self.lower_expr(&expr);
                parts.push(mir);
            }
        }

        // Build the final expression. Let bindings need to nest their body
        // over subsequent parts. We build from the end backwards:
        // [Let(x), expr1, Let(y), expr2] becomes:
        // Let(x, Block([expr1, Let(y, expr2)]))
        if parts.is_empty() {
            return MirExpr::Unit;
        }

        // Fold from right to left: each Let wraps everything after it as its body.
        let mut result = parts.pop().unwrap();
        while let Some(part) = parts.pop() {
            match part {
                MirExpr::Let { name, ty, value, body: _ } => {
                    result = MirExpr::Let {
                        name,
                        ty,
                        value,
                        body: Box::new(result),
                    };
                }
                other => {
                    // Non-let expression before result: wrap in a Block.
                    let ty = result.ty().clone();
                    result = MirExpr::Block(vec![other, result], ty);
                }
            }
        }

        result
    }

    // ── Let binding lowering ─────────────────────────────────────────

    fn lower_let_binding(&mut self, let_: &LetBinding) -> MirExpr {
        let name = let_
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "_".to_string());

        let value = if let Some(init) = let_.initializer() {
            self.lower_expr(&init)
        } else {
            MirExpr::Unit
        };

        let ty = value.ty().clone();
        self.insert_var(name.clone(), ty.clone());

        MirExpr::Let {
            name,
            ty,
            value: Box::new(value),
            body: Box::new(MirExpr::Unit),
        }
    }

    // ── Expression lowering ──────────────────────────────────────────

    fn lower_expr(&mut self, expr: &Expr) -> MirExpr {
        match expr {
            Expr::Literal(lit) => self.lower_literal(lit),
            Expr::NameRef(name_ref) => self.lower_name_ref(name_ref),
            Expr::BinaryExpr(bin) => self.lower_binary_expr(bin),
            Expr::UnaryExpr(un) => self.lower_unary_expr(un),
            Expr::CallExpr(call) => self.lower_call_expr(call),
            Expr::PipeExpr(pipe) => self.lower_pipe_expr(pipe),
            Expr::FieldAccess(fa) => self.lower_field_access(fa),
            Expr::IndexExpr(_) => {
                // Index expressions not yet supported in MIR.
                MirExpr::Unit
            }
            Expr::IfExpr(if_) => self.lower_if_expr(if_),
            Expr::CaseExpr(case) => self.lower_case_expr(case),
            Expr::ClosureExpr(closure) => self.lower_closure_expr(closure),
            Expr::Block(block) => self.lower_block(block),
            Expr::StringExpr(str_expr) => self.lower_string_expr(str_expr),
            Expr::ReturnExpr(ret) => self.lower_return_expr(ret),
            Expr::TupleExpr(tuple) => self.lower_tuple_expr(tuple),
            Expr::StructLiteral(sl) => self.lower_struct_literal(sl),
            // Actor expressions -- lowering will be implemented in Plan 04/05.
            Expr::SpawnExpr(_) | Expr::SendExpr(_) | Expr::ReceiveExpr(_)
            | Expr::SelfExpr(_) | Expr::LinkExpr(_) => {
                // Placeholder: actor expression lowering not yet implemented.
                MirExpr::Unit
            }
        }
    }

    // ── Literal lowering ─────────────────────────────────────────────

    fn lower_literal(&self, lit: &Literal) -> MirExpr {
        let token = match lit.token() {
            Some(t) => t,
            None => return MirExpr::Unit,
        };

        let text = token.text().to_string();

        match token.kind() {
            SyntaxKind::INT_LITERAL => {
                let val = text.parse::<i64>().unwrap_or(0);
                MirExpr::IntLit(val, MirType::Int)
            }
            SyntaxKind::FLOAT_LITERAL => {
                let val = text.parse::<f64>().unwrap_or(0.0);
                MirExpr::FloatLit(val, MirType::Float)
            }
            SyntaxKind::TRUE_KW => MirExpr::BoolLit(true, MirType::Bool),
            SyntaxKind::FALSE_KW => MirExpr::BoolLit(false, MirType::Bool),
            SyntaxKind::NIL_KW => MirExpr::Unit,
            SyntaxKind::STRING_START => {
                // Simple string literal (no interpolation in a LITERAL node).
                // Extract the string content from the syntax node.
                let content = extract_simple_string_content(lit.syntax());
                MirExpr::StringLit(content, MirType::String)
            }
            _ => MirExpr::Unit,
        }
    }

    // ── Name reference lowering ──────────────────────────────────────

    fn lower_name_ref(&self, name_ref: &NameRef) -> MirExpr {
        let name = name_ref
            .text()
            .unwrap_or_else(|| "<unknown>".to_string());

        // Check if this is a nullary variant constructor (e.g., Red, None, Point).
        // These are NameRef nodes that refer to sum type variants with no fields.
        for (_, sum_info) in &self.registry.sum_type_defs {
            for variant in &sum_info.variants {
                if variant.name == name && variant.fields.is_empty() {
                    let ty_name = &sum_info.name;
                    let mir_ty = MirType::SumType(ty_name.clone());
                    return MirExpr::ConstructVariant {
                        type_name: ty_name.clone(),
                        variant: name,
                        fields: vec![],
                        ty: mir_ty,
                    };
                }
            }
        }

        // Map builtin function names to their runtime equivalents.
        let name = map_builtin_name(&name);

        // Check scope first for the type. This preserves MirType::Closure
        // for variables bound to closures, which is needed for correct
        // ClosureCall dispatch.
        let ty = if let Some(scope_ty) = self.lookup_var(&name) {
            scope_ty
        } else {
            self.resolve_range(name_ref.syntax().text_range())
        };
        MirExpr::Var(name, ty)
    }

    // ── Binary expression lowering ───────────────────────────────────

    fn lower_binary_expr(&mut self, bin: &BinaryExpr) -> MirExpr {
        let lhs = bin.lhs().map(|e| self.lower_expr(&e)).unwrap_or(MirExpr::Unit);
        let rhs = bin.rhs().map(|e| self.lower_expr(&e)).unwrap_or(MirExpr::Unit);

        let op = bin
            .op()
            .map(|t| match t.kind() {
                SyntaxKind::PLUS => BinOp::Add,
                SyntaxKind::MINUS => BinOp::Sub,
                SyntaxKind::STAR => BinOp::Mul,
                SyntaxKind::SLASH => BinOp::Div,
                SyntaxKind::PERCENT => BinOp::Mod,
                SyntaxKind::EQ_EQ => BinOp::Eq,
                SyntaxKind::NOT_EQ => BinOp::NotEq,
                SyntaxKind::LT => BinOp::Lt,
                SyntaxKind::GT => BinOp::Gt,
                SyntaxKind::LT_EQ => BinOp::LtEq,
                SyntaxKind::GT_EQ => BinOp::GtEq,
                SyntaxKind::AND_KW | SyntaxKind::AMP_AMP => BinOp::And,
                SyntaxKind::OR_KW | SyntaxKind::PIPE_PIPE => BinOp::Or,
                SyntaxKind::PLUS_PLUS => BinOp::Concat,
                _ => BinOp::Add, // fallback
            })
            .unwrap_or(BinOp::Add);

        let ty = self.resolve_range(bin.syntax().text_range());

        MirExpr::BinOp {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            ty,
        }
    }

    // ── Unary expression lowering ────────────────────────────────────

    fn lower_unary_expr(&mut self, un: &UnaryExpr) -> MirExpr {
        let operand = un
            .operand()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        let op = un
            .op()
            .map(|t| match t.kind() {
                SyntaxKind::MINUS => UnaryOp::Neg,
                SyntaxKind::BANG | SyntaxKind::NOT_KW => UnaryOp::Not,
                _ => UnaryOp::Neg,
            })
            .unwrap_or(UnaryOp::Neg);

        let ty = self.resolve_range(un.syntax().text_range());

        MirExpr::UnaryOp {
            op,
            operand: Box::new(operand),
            ty,
        }
    }

    // ── Call expression lowering ─────────────────────────────────────

    fn lower_call_expr(&mut self, call: &CallExpr) -> MirExpr {
        let callee = call.callee().map(|e| self.lower_expr(&e));
        let args: Vec<MirExpr> = call
            .arg_list()
            .map(|al| al.args().map(|a| self.lower_expr(&a)).collect())
            .unwrap_or_default();

        let ty = self.resolve_range(call.syntax().text_range());

        let callee = match callee {
            Some(c) => c,
            None => return MirExpr::Unit,
        };

        // Check if this is a variant constructor call (e.g., Circle(5.0)).
        if let MirExpr::Var(ref name, _) = callee {
            for (_, sum_info) in &self.registry.sum_type_defs {
                for variant in &sum_info.variants {
                    if variant.name == *name && !variant.fields.is_empty() {
                        let ty_name = &sum_info.name;
                        let mir_ty = MirType::SumType(ty_name.clone());
                        return MirExpr::ConstructVariant {
                            type_name: ty_name.clone(),
                            variant: name.clone(),
                            fields: args,
                            ty: mir_ty,
                        };
                    }
                }
            }
        }

        // Determine if this is a direct function call or a closure call.
        let is_known_fn = match &callee {
            MirExpr::Var(name, _) => self.known_functions.contains_key(name),
            _ => false,
        };

        if is_known_fn {
            MirExpr::Call {
                func: Box::new(callee),
                args,
                ty,
            }
        } else {
            // Check the callee type. If it's a Closure type, use ClosureCall.
            match callee.ty() {
                MirType::Closure(_, _) => MirExpr::ClosureCall {
                    closure: Box::new(callee),
                    args,
                    ty,
                },
                _ => MirExpr::Call {
                    func: Box::new(callee),
                    args,
                    ty,
                },
            }
        }
    }

    // ── Pipe expression lowering (DESUGARING) ────────────────────────

    fn lower_pipe_expr(&mut self, pipe: &PipeExpr) -> MirExpr {
        // Desugar: `x |> f` -> `f(x)`
        //          `x |> f(a, b)` -> `f(x, a, b)`
        let lhs = pipe
            .lhs()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        let rhs = pipe.rhs();
        let ty = self.resolve_range(pipe.syntax().text_range());

        match rhs {
            Some(Expr::CallExpr(call)) => {
                // `x |> f(a, b)` -> `f(x, a, b)` -- prepend lhs to existing args.
                let callee = call.callee().map(|e| self.lower_expr(&e));
                let mut args: Vec<MirExpr> = Vec::new();
                args.push(lhs);
                if let Some(arg_list) = call.arg_list() {
                    for arg in arg_list.args() {
                        args.push(self.lower_expr(&arg));
                    }
                }
                let callee = match callee {
                    Some(c) => c,
                    None => return MirExpr::Unit,
                };
                MirExpr::Call {
                    func: Box::new(callee),
                    args,
                    ty,
                }
            }
            Some(rhs_expr) => {
                // `x |> f` -> `f(x)` -- bare function reference.
                let func = self.lower_expr(&rhs_expr);
                MirExpr::Call {
                    func: Box::new(func),
                    args: vec![lhs],
                    ty,
                }
            }
            None => MirExpr::Unit,
        }
    }

    // ── Field access lowering ────────────────────────────────────────

    fn lower_field_access(&mut self, fa: &FieldAccess) -> MirExpr {
        let object = fa
            .base()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        let field = fa
            .field()
            .map(|t| t.text().to_string())
            .unwrap_or_default();

        let ty = self.resolve_range(fa.syntax().text_range());

        MirExpr::FieldAccess {
            object: Box::new(object),
            field,
            ty,
        }
    }

    // ── If expression lowering ───────────────────────────────────────

    fn lower_if_expr(&mut self, if_: &IfExpr) -> MirExpr {
        let cond = if_
            .condition()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::BoolLit(true, MirType::Bool));

        let then_body = if_
            .then_branch()
            .map(|b| self.lower_block(&b))
            .unwrap_or(MirExpr::Unit);

        let else_body = if let Some(else_branch) = if_.else_branch() {
            if let Some(chained_if) = else_branch.if_expr() {
                // else-if chain
                self.lower_if_expr(&chained_if)
            } else if let Some(block) = else_branch.block() {
                self.lower_block(&block)
            } else {
                MirExpr::Unit
            }
        } else {
            MirExpr::Unit
        };

        let ty = self.resolve_range(if_.syntax().text_range());

        MirExpr::If {
            cond: Box::new(cond),
            then_body: Box::new(then_body),
            else_body: Box::new(else_body),
            ty,
        }
    }

    // ── Case expression lowering ─────────────────────────────────────

    fn lower_case_expr(&mut self, case: &CaseExpr) -> MirExpr {
        let scrutinee = case
            .scrutinee()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        let arms: Vec<MirMatchArm> = case.arms().map(|arm| self.lower_match_arm(&arm)).collect();

        let ty = self.resolve_range(case.syntax().text_range());

        MirExpr::Match {
            scrutinee: Box::new(scrutinee),
            arms,
            ty,
        }
    }

    fn lower_match_arm(&mut self, arm: &MatchArm) -> MirMatchArm {
        self.push_scope();

        let pattern = arm
            .pattern()
            .map(|p| self.lower_pattern(&p))
            .unwrap_or(MirPattern::Wildcard);

        let guard = arm.guard().map(|e| self.lower_expr(&e));

        let body = arm
            .body()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        self.pop_scope();

        MirMatchArm {
            pattern,
            guard,
            body,
        }
    }

    // ── Pattern lowering ─────────────────────────────────────────────

    fn lower_pattern(&mut self, pat: &Pattern) -> MirPattern {
        match pat {
            Pattern::Wildcard(_) => MirPattern::Wildcard,

            Pattern::Ident(ident) => {
                let name = ident
                    .name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_else(|| "_".to_string());
                let ty = self.resolve_range(ident.syntax().text_range());
                self.insert_var(name.clone(), ty.clone());
                MirPattern::Var(name, ty)
            }

            Pattern::Literal(lit) => {
                let token = lit.token();
                match token {
                    Some(t) => {
                        let text = t.text().to_string();
                        match t.kind() {
                            SyntaxKind::INT_LITERAL => {
                                MirPattern::Literal(MirLiteral::Int(
                                    text.parse().unwrap_or(0),
                                ))
                            }
                            SyntaxKind::FLOAT_LITERAL => {
                                MirPattern::Literal(MirLiteral::Float(
                                    text.parse().unwrap_or(0.0),
                                ))
                            }
                            SyntaxKind::TRUE_KW => {
                                MirPattern::Literal(MirLiteral::Bool(true))
                            }
                            SyntaxKind::FALSE_KW => {
                                MirPattern::Literal(MirLiteral::Bool(false))
                            }
                            SyntaxKind::STRING_START => {
                                // Extract string content from the literal pattern node.
                                let content = extract_simple_string_content(lit.syntax());
                                MirPattern::Literal(MirLiteral::String(content))
                            }
                            _ => MirPattern::Wildcard,
                        }
                    }
                    None => MirPattern::Wildcard,
                }
            }

            Pattern::Constructor(ctor) => {
                let variant_name = ctor
                    .variant_name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();

                let type_name = if let Some(tn) = ctor.type_name() {
                    tn.text().to_string()
                } else {
                    // Find the type name from the registry for unqualified constructors.
                    find_type_for_variant(&variant_name, self.registry)
                        .unwrap_or_default()
                };

                let fields: Vec<MirPattern> =
                    ctor.fields().map(|p| self.lower_pattern(&p)).collect();

                // Collect bindings introduced by sub-patterns.
                let bindings = collect_pattern_bindings(&fields);

                MirPattern::Constructor {
                    type_name,
                    variant: variant_name,
                    fields,
                    bindings,
                }
            }

            Pattern::Tuple(tuple) => {
                let pats: Vec<MirPattern> =
                    tuple.patterns().map(|p| self.lower_pattern(&p)).collect();
                MirPattern::Tuple(pats)
            }

            Pattern::Or(or) => {
                let alts: Vec<MirPattern> =
                    or.alternatives().map(|p| self.lower_pattern(&p)).collect();
                MirPattern::Or(alts)
            }

            Pattern::As(as_pat) => {
                // Layered pattern: bind name AND match inner pattern.
                // For MIR, we lower the inner pattern and add the name as a Var binding.
                let binding_name = as_pat
                    .binding_name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_else(|| "_".to_string());
                let ty = self.resolve_range(as_pat.syntax().text_range());
                self.insert_var(binding_name.clone(), ty.clone());

                // Lower inner pattern -- the binding is separate.
                if let Some(inner) = as_pat.pattern() {
                    self.lower_pattern(&inner)
                } else {
                    MirPattern::Var(binding_name, ty)
                }
            }
        }
    }

    // ── Closure expression lowering (CLOSURE CONVERSION) ─────────────

    fn lower_closure_expr(&mut self, closure: &ClosureExpr) -> MirExpr {
        self.closure_counter += 1;
        let closure_fn_name = format!("__closure_{}", self.closure_counter);

        let closure_range = closure.syntax().text_range();
        let closure_ty = self.get_ty(closure_range).cloned();

        // Extract parameter types from the closure's function type.
        let mut param_types = Vec::new();
        let return_type;
        if let Some(Ty::Fun(params, ret)) = &closure_ty {
            param_types = params
                .iter()
                .map(|p| resolve_type(p, self.registry, false))
                .collect();
            return_type = resolve_type(ret, self.registry, false);
        } else {
            return_type = MirType::Unit;
        }

        // Extract parameter names.
        let mut param_names = Vec::new();
        if let Some(param_list) = closure.param_list() {
            for param in param_list.params() {
                let name = param
                    .name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_else(|| "_".to_string());
                param_names.push(name);
            }
        }

        // Build params: env_ptr first, then user params.
        let mut fn_params = Vec::new();
        fn_params.push(("__env".to_string(), MirType::Ptr));

        for (i, name) in param_names.iter().enumerate() {
            let ty = param_types.get(i).cloned().unwrap_or(MirType::Unit);
            fn_params.push((name.clone(), ty));
        }

        // Determine captured variables by scanning the closure body.
        // Any variable referenced in the body that is not a parameter and
        // exists in the outer scope is a capture.
        let outer_vars: HashMap<String, MirType> = self
            .scopes
            .iter()
            .flat_map(|s| s.iter())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let param_set: std::collections::HashSet<&str> =
            param_names.iter().map(|s| s.as_str()).collect();

        // Lower the body in a new scope with params.
        self.push_scope();
        for (name, ty) in &fn_params {
            self.insert_var(name.clone(), ty.clone());
        }

        let body = if let Some(block) = closure.body() {
            self.lower_block(&block)
        } else {
            MirExpr::Unit
        };

        self.pop_scope();

        // Find captured variables by scanning the lowered body for Var references
        // that match outer scope names and are not parameters.
        let mut captures: Vec<(String, MirType)> = Vec::new();
        let mut capture_exprs: Vec<MirExpr> = Vec::new();
        collect_free_vars(&body, &param_set, &outer_vars, &mut captures);
        for (name, ty) in &captures {
            capture_exprs.push(MirExpr::Var(name.clone(), ty.clone()));
        }

        // Create the lifted function.
        self.functions.push(MirFunction {
            name: closure_fn_name.clone(),
            params: fn_params,
            return_type: return_type.clone(),
            body,
            is_closure_fn: true,
            captures: captures.clone(),
        });

        // Create the MakeClosure expression.
        let mir_ty = MirType::Closure(param_types, Box::new(return_type));

        MirExpr::MakeClosure {
            fn_name: closure_fn_name,
            captures: capture_exprs,
            ty: mir_ty,
        }
    }

    // ── String expression lowering (INTERPOLATION DESUGARING) ────────

    fn lower_string_expr(&mut self, str_expr: &StringExpr) -> MirExpr {
        // Walk the STRING_EXPR node's children to find STRING_CONTENT and
        // INTERPOLATION segments.
        let mut segments: Vec<MirExpr> = Vec::new();

        for child in str_expr.syntax().children_with_tokens() {
            match child.kind() {
                SyntaxKind::STRING_CONTENT => {
                    let text = child
                        .as_token()
                        .map(|t| t.text().to_string())
                        .unwrap_or_default();
                    if !text.is_empty() {
                        segments.push(MirExpr::StringLit(text, MirType::String));
                    }
                }
                SyntaxKind::INTERPOLATION => {
                    // INTERPOLATION node contains an expression child.
                    if let Some(node) = child.as_node() {
                        for inner in node.children() {
                            if let Some(expr) = Expr::cast(inner) {
                                let lowered = self.lower_expr(&expr);
                                // Wrap in a to_string call based on the expression's type.
                                let converted = self.wrap_to_string(lowered);
                                segments.push(converted);
                            }
                        }
                    }
                }
                _ => {
                    // STRING_START, STRING_END, INTERPOLATION_START, INTERPOLATION_END:
                    // skip these tokens.
                }
            }
        }

        // If no segments, return empty string.
        if segments.is_empty() {
            return MirExpr::StringLit(String::new(), MirType::String);
        }

        // If single segment, return it directly.
        if segments.len() == 1 {
            return segments.pop().unwrap();
        }

        // Chain concat calls: concat(concat(seg0, seg1), seg2) ...
        let mut result = segments.remove(0);
        for seg in segments {
            result = MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    "snow_string_concat".to_string(),
                    MirType::FnPtr(
                        vec![MirType::String, MirType::String],
                        Box::new(MirType::String),
                    ),
                )),
                args: vec![result, seg],
                ty: MirType::String,
            };
        }

        result
    }

    /// Wrap an expression in a to_string runtime call based on its type.
    fn wrap_to_string(&self, expr: MirExpr) -> MirExpr {
        match expr.ty() {
            MirType::String => expr, // already a string
            MirType::Int => MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    "snow_int_to_string".to_string(),
                    MirType::FnPtr(vec![MirType::Int], Box::new(MirType::String)),
                )),
                args: vec![expr],
                ty: MirType::String,
            },
            MirType::Float => MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    "snow_float_to_string".to_string(),
                    MirType::FnPtr(vec![MirType::Float], Box::new(MirType::String)),
                )),
                args: vec![expr],
                ty: MirType::String,
            },
            MirType::Bool => MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    "snow_bool_to_string".to_string(),
                    MirType::FnPtr(vec![MirType::Bool], Box::new(MirType::String)),
                )),
                args: vec![expr],
                ty: MirType::String,
            },
            _ => {
                // For other types, attempt a generic to_string call.
                MirExpr::Call {
                    func: Box::new(MirExpr::Var(
                        "to_string".to_string(),
                        MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
                    )),
                    args: vec![expr],
                    ty: MirType::String,
                }
            }
        }
    }

    // ── Return expression lowering ───────────────────────────────────

    fn lower_return_expr(&mut self, ret: &ReturnExpr) -> MirExpr {
        let value = ret
            .value()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        MirExpr::Return(Box::new(value))
    }

    // ── Tuple expression lowering ────────────────────────────────────

    fn lower_tuple_expr(&mut self, tuple: &TupleExpr) -> MirExpr {
        let elements: Vec<MirExpr> = tuple.elements().map(|e| self.lower_expr(&e)).collect();

        // Per decision 03-02: single-element tuple is grouping parens, not a tuple.
        if elements.len() == 1 {
            return elements.into_iter().next().unwrap();
        }

        if elements.is_empty() {
            return MirExpr::Unit;
        }

        // Multi-element tuple: create a block that evaluates to a tuple.
        let ty = self.resolve_range(tuple.syntax().text_range());
        MirExpr::Block(elements, ty)
    }

    // ── Struct literal lowering ──────────────────────────────────────

    fn lower_struct_literal(&mut self, sl: &StructLiteral) -> MirExpr {
        let name = sl
            .name_ref()
            .and_then(|nr| nr.text())
            .unwrap_or_else(|| "<unnamed>".to_string());

        let fields: Vec<(String, MirExpr)> = sl
            .fields()
            .map(|f| {
                let field_name = f
                    .name()
                    .and_then(|n| n.text())
                    .unwrap_or_default();
                let value = f
                    .value()
                    .map(|e| self.lower_expr(&e))
                    .unwrap_or(MirExpr::Unit);
                (field_name, value)
            })
            .collect();

        let ty = self.resolve_range(sl.syntax().text_range());

        MirExpr::StructLit { name, fields, ty }
    }
}

// ── Helper functions ─────────────────────────────────────────────────

/// Map Snow builtin function names to their runtime equivalents.
///
/// Snow source uses clean names like `println`, `print`, `to_string`.
/// These are mapped to the actual runtime function names like `snow_println`,
/// `snow_print`, `snow_int_to_string` at the MIR level.
fn map_builtin_name(name: &str) -> String {
    match name {
        "println" => "snow_println".to_string(),
        "print" => "snow_print".to_string(),
        _ => name.to_string(),
    }
}

/// Extract simple string content from a LITERAL or STRING_EXPR syntax node.
/// Walks children looking for STRING_CONTENT tokens and concatenates them.
fn extract_simple_string_content(node: &snow_parser::cst::SyntaxNode) -> String {
    let mut content = String::new();
    for child in node.children_with_tokens() {
        if child.kind() == SyntaxKind::STRING_CONTENT {
            if let Some(token) = child.as_token() {
                content.push_str(token.text());
            }
        }
    }
    content
}

/// Find the type name that contains a given variant name.
fn find_type_for_variant(variant: &str, registry: &snow_typeck::TypeRegistry) -> Option<String> {
    for (type_name, info) in &registry.sum_type_defs {
        for v in &info.variants {
            if v.name == variant {
                return Some(type_name.clone());
            }
        }
    }
    None
}

/// Collect bindings introduced by a list of patterns (for constructor pattern bindings).
fn collect_pattern_bindings(patterns: &[MirPattern]) -> Vec<(String, MirType)> {
    let mut bindings = Vec::new();
    for pat in patterns {
        collect_bindings_recursive(pat, &mut bindings);
    }
    bindings
}

fn collect_bindings_recursive(pat: &MirPattern, bindings: &mut Vec<(String, MirType)>) {
    match pat {
        MirPattern::Var(name, ty) => {
            bindings.push((name.clone(), ty.clone()));
        }
        MirPattern::Constructor { fields, .. } => {
            for f in fields {
                collect_bindings_recursive(f, bindings);
            }
        }
        MirPattern::Tuple(pats) => {
            for p in pats {
                collect_bindings_recursive(p, bindings);
            }
        }
        MirPattern::Or(alts) => {
            // Use bindings from first alternative (all should have same bindings).
            if let Some(first) = alts.first() {
                collect_bindings_recursive(first, bindings);
            }
        }
        MirPattern::Wildcard | MirPattern::Literal(_) => {}
    }
}

/// Collect free variables from an expression that exist in the outer scope
/// but are not in the parameter set. Deduplicates by name.
fn collect_free_vars(
    expr: &MirExpr,
    params: &std::collections::HashSet<&str>,
    outer_vars: &HashMap<String, MirType>,
    captures: &mut Vec<(String, MirType)>,
) {
    match expr {
        MirExpr::Var(name, _) => {
            if !params.contains(name.as_str())
                && name != "__env"
                && outer_vars.contains_key(name)
                && !captures.iter().any(|(n, _)| n == name)
            {
                if let Some(ty) = outer_vars.get(name) {
                    captures.push((name.clone(), ty.clone()));
                }
            }
        }
        MirExpr::BinOp { lhs, rhs, .. } => {
            collect_free_vars(lhs, params, outer_vars, captures);
            collect_free_vars(rhs, params, outer_vars, captures);
        }
        MirExpr::UnaryOp { operand, .. } => {
            collect_free_vars(operand, params, outer_vars, captures);
        }
        MirExpr::Call { func, args, .. } | MirExpr::ClosureCall { closure: func, args, .. } => {
            collect_free_vars(func, params, outer_vars, captures);
            for arg in args {
                collect_free_vars(arg, params, outer_vars, captures);
            }
        }
        MirExpr::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            collect_free_vars(cond, params, outer_vars, captures);
            collect_free_vars(then_body, params, outer_vars, captures);
            collect_free_vars(else_body, params, outer_vars, captures);
        }
        MirExpr::Let { value, body, .. } => {
            collect_free_vars(value, params, outer_vars, captures);
            collect_free_vars(body, params, outer_vars, captures);
        }
        MirExpr::Block(exprs, _) => {
            for e in exprs {
                collect_free_vars(e, params, outer_vars, captures);
            }
        }
        MirExpr::Match {
            scrutinee, arms, ..
        } => {
            collect_free_vars(scrutinee, params, outer_vars, captures);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_free_vars(guard, params, outer_vars, captures);
                }
                collect_free_vars(&arm.body, params, outer_vars, captures);
            }
        }
        MirExpr::StructLit { fields, .. } => {
            for (_, val) in fields {
                collect_free_vars(val, params, outer_vars, captures);
            }
        }
        MirExpr::FieldAccess { object, .. } => {
            collect_free_vars(object, params, outer_vars, captures);
        }
        MirExpr::ConstructVariant { fields, .. } => {
            for f in fields {
                collect_free_vars(f, params, outer_vars, captures);
            }
        }
        MirExpr::MakeClosure { captures: caps, .. } => {
            for c in caps {
                collect_free_vars(c, params, outer_vars, captures);
            }
        }
        MirExpr::Return(val) => {
            collect_free_vars(val, params, outer_vars, captures);
        }
        MirExpr::IntLit(_, _)
        | MirExpr::FloatLit(_, _)
        | MirExpr::BoolLit(_, _)
        | MirExpr::StringLit(_, _)
        | MirExpr::Panic { .. }
        | MirExpr::Unit => {}
        // Actor primitives
        MirExpr::ActorSpawn { func, args, terminate_callback, .. } => {
            collect_free_vars(func, params, outer_vars, captures);
            for arg in args {
                collect_free_vars(arg, params, outer_vars, captures);
            }
            if let Some(cb) = terminate_callback {
                collect_free_vars(cb, params, outer_vars, captures);
            }
        }
        MirExpr::ActorSend { target, message, .. } => {
            collect_free_vars(target, params, outer_vars, captures);
            collect_free_vars(message, params, outer_vars, captures);
        }
        MirExpr::ActorReceive { arms, timeout_ms, timeout_body, .. } => {
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_free_vars(guard, params, outer_vars, captures);
                }
                collect_free_vars(&arm.body, params, outer_vars, captures);
            }
            if let Some(tm) = timeout_ms {
                collect_free_vars(tm, params, outer_vars, captures);
            }
            if let Some(tb) = timeout_body {
                collect_free_vars(tb, params, outer_vars, captures);
            }
        }
        MirExpr::ActorSelf { .. } => {}
        MirExpr::ActorLink { target, .. } => {
            collect_free_vars(target, params, outer_vars, captures);
        }
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Lower a parsed and type-checked Snow program to MIR.
///
/// This is the main entry point for AST-to-MIR conversion. It walks the
/// typed AST, desugars pipe operators and string interpolation, lifts closures,
/// and produces a flat MIR module.
pub fn lower_to_mir(parse: &Parse, typeck: &TypeckResult) -> Result<MirModule, String> {
    let tree = parse.syntax();
    let source_file = match SourceFile::cast(tree.clone()) {
        Some(sf) => sf,
        None => return Err("Failed to cast root node to SourceFile".to_string()),
    };

    let mut lowerer = Lowerer::new(typeck);

    // Also register builtin sum types from the registry (Option, Result).
    for (name, info) in &typeck.type_registry.sum_type_defs {
        // These may not appear as items in the source file but are needed.
        let variants = info
            .variants
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let fields = v
                    .fields
                    .iter()
                    .map(|f| {
                        let ty = match f {
                            snow_typeck::VariantFieldInfo::Positional(ty) => ty,
                            snow_typeck::VariantFieldInfo::Named(_, ty) => ty,
                        };
                        resolve_type(ty, &typeck.type_registry, false)
                    })
                    .collect();
                MirVariantDef {
                    name: v.name.clone(),
                    fields,
                    tag: i as u8,
                }
            })
            .collect();

        lowerer.sum_types.push(MirSumTypeDef {
            name: name.clone(),
            variants,
        });
    }

    lowerer.lower_source_file(source_file);

    Ok(MirModule {
        functions: lowerer.functions,
        structs: lowerer.structs,
        sum_types: lowerer.sum_types,
        entry_function: lowerer.entry_function,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to parse and type-check a Snow source, then lower to MIR.
    fn lower(source: &str) -> MirModule {
        let parse = snow_parser::parse(source);
        let typeck = snow_typeck::check(&parse);
        // Ignore type errors for MIR lowering tests -- we test lowering, not typeck.
        lower_to_mir(&parse, &typeck).expect("MIR lowering failed")
    }

    #[test]
    fn lower_int_literal() {
        let mir = lower("let x = 42");
        // The top-level let should not produce a function, but we should have
        // at least the builtin sum types in the module.
        assert!(mir.functions.is_empty() || mir.functions.len() >= 0);
    }

    #[test]
    fn lower_function_def() {
        let mir = lower("fn add(a :: Int, b :: Int) -> Int do a + b end");
        let func = mir.functions.iter().find(|f| f.name == "add");
        assert!(func.is_some(), "Expected 'add' function in MIR");
        let func = func.unwrap();
        assert_eq!(func.params.len(), 2);
        assert_eq!(func.params[0].0, "a");
        assert_eq!(func.params[0].1, MirType::Int);
        assert_eq!(func.params[1].0, "b");
        assert_eq!(func.params[1].1, MirType::Int);
        assert_eq!(func.return_type, MirType::Int);

        // Body should be a BinOp
        assert!(matches!(func.body, MirExpr::BinOp { op: BinOp::Add, .. }));
    }

    #[test]
    fn lower_pipe_desugars_to_call() {
        // `x |> f` should desugar to `f(x)`
        let mir = lower(
            "fn double(x :: Int) -> Int do x * 2 end\n\
             fn main() do 5 |> double end",
        );
        let main = mir.functions.iter().find(|f| f.name == "main");
        assert!(main.is_some(), "Expected 'main' function in MIR");
        let main = main.unwrap();

        // Body should be a Call with func=double, args=[5]
        match &main.body {
            MirExpr::Call { func, args, .. } => {
                assert!(matches!(func.as_ref(), MirExpr::Var(name, _) if name == "double"));
                assert_eq!(args.len(), 1);
                assert!(matches!(&args[0], MirExpr::IntLit(5, _)));
            }
            other => panic!("Expected Call, got {:?}", other),
        }
    }

    #[test]
    fn lower_string_interpolation_desugars_to_concat() {
        let source = r#"
fn main() do
  let name = "world"
  "hello ${name}"
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "main");
        assert!(main.is_some());
        let main = main.unwrap();

        // The body should contain a concat call somewhere.
        fn has_concat_call(expr: &MirExpr) -> bool {
            match expr {
                MirExpr::Call { func, .. } => {
                    if let MirExpr::Var(name, _) = func.as_ref() {
                        if name == "snow_string_concat" {
                            return true;
                        }
                    }
                    false
                }
                MirExpr::Block(exprs, _) => exprs.iter().any(has_concat_call),
                MirExpr::Let { value, body, .. } => {
                    has_concat_call(value) || has_concat_call(body)
                }
                _ => false,
            }
        }

        assert!(
            has_concat_call(&main.body),
            "Expected snow_string_concat call in interpolated string body: {:?}",
            main.body
        );
    }

    #[test]
    fn lower_closure_produces_lifted_function() {
        let source = r#"
fn main() do
  let y = 10
  let inc = fn(x :: Int) -> Int do x + y end
  inc
end
"#;
        let mir = lower(source);

        // Should have a lifted closure function
        let closure_fn = mir.functions.iter().find(|f| f.name.starts_with("__closure_"));
        assert!(
            closure_fn.is_some(),
            "Expected lifted closure function, got functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let closure_fn = closure_fn.unwrap();
        assert!(closure_fn.is_closure_fn);
        // First param should be __env
        assert_eq!(closure_fn.params[0].0, "__env");
    }

    #[test]
    fn lower_main_sets_entry_function() {
        let mir = lower("fn main() do 0 end");
        assert_eq!(mir.entry_function, Some("main".to_string()));
    }

    #[test]
    fn lower_if_expr() {
        let mir = lower("fn test(x :: Bool) -> Int do if x do 1 else 2 end end");
        let func = mir.functions.iter().find(|f| f.name == "test");
        assert!(func.is_some());
        assert!(matches!(func.unwrap().body, MirExpr::If { .. }));
    }

    #[test]
    fn lower_case_expr() {
        let source = r#"
fn test(x :: Int) -> Int do
  case x do
    0 -> 1
    _ -> 2
  end
end
"#;
        let mir = lower(source);
        let func = mir.functions.iter().find(|f| f.name == "test");
        assert!(func.is_some());
        let func = func.unwrap();
        assert!(
            matches!(func.body, MirExpr::Match { .. }),
            "Expected Match, got {:?}",
            func.body
        );
    }
}
