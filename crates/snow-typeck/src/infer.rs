//! Algorithm J inference engine for Snow.
//!
//! Walks the Snow AST, generates type constraints, and solves them via
//! unification. Implements Hindley-Milner type inference with:
//! - Let-polymorphism (generalize + instantiate)
//! - Occurs check (rejects infinite types)
//! - Level-based generalization (Remy's algorithm)
//! - Error provenance via ConstraintOrigin
//! - Trait system: interface definitions, impl blocks, where-clause enforcement
//! - Compiler-known traits for operator dispatch (Add, Eq, Ord, etc.)
//! - Struct definitions, struct literals, and field access (03-03)

use rowan::TextRange;
use snow_parser::ast::expr::{
    BinaryExpr, CallExpr, CaseExpr, ClosureExpr, Expr, FieldAccess, IfExpr, Literal, NameRef,
    PipeExpr, ReturnExpr, StructLiteral, TupleExpr, UnaryExpr,
};
use snow_parser::ast::item::{
    Block, FnDef, InterfaceDef, ImplDef as AstImplDef, Item, LetBinding, StructDef, TypeAliasDef,
};
use snow_parser::ast::pat::Pattern;
use snow_parser::ast::AstNode;
use snow_parser::syntax_kind::SyntaxKind;
use snow_parser::Parse;

use crate::builtins;
use crate::env::TypeEnv;
use crate::error::{ConstraintOrigin, TypeError};
use crate::traits::{
    ImplDef as TraitImplDef, ImplMethodSig, TraitDef, TraitMethodSig, TraitRegistry,
};
use crate::ty::{Scheme, Ty, TyCon};
use crate::unify::InferCtx;
use crate::TypeckResult;

use rustc_hash::FxHashMap;

// ── Struct & Type Registry (03-03) ────────────────────────────────────

/// A registered struct definition with its fields and generic parameters.
#[derive(Clone, Debug)]
struct StructDefInfo {
    /// The struct's name.
    name: String,
    /// Names of generic type parameters (e.g., ["A", "B"] for `Pair<A, B>`).
    generic_params: Vec<String>,
    /// Field names and their types. Types may reference generic params.
    fields: Vec<(String, Ty)>,
}

/// A registered type alias.
#[derive(Clone, Debug)]
struct TypeAliasInfo {
    /// The alias name.
    #[allow(dead_code)]
    name: String,
    /// Names of generic type parameters.
    #[allow(dead_code)]
    generic_params: Vec<String>,
    /// The aliased type (may reference generic params).
    #[allow(dead_code)]
    aliased_type: Ty,
}

/// Registry for struct definitions and type aliases.
#[derive(Clone, Debug, Default)]
struct TypeRegistry {
    struct_defs: FxHashMap<String, StructDefInfo>,
    type_aliases: FxHashMap<String, TypeAliasInfo>,
}

impl TypeRegistry {
    fn new() -> Self {
        Self::default()
    }

    fn register_struct(&mut self, info: StructDefInfo) {
        self.struct_defs.insert(info.name.clone(), info);
    }

    fn register_alias(&mut self, info: TypeAliasInfo) {
        self.type_aliases.insert(info.name.clone(), info);
    }

    fn lookup_struct(&self, name: &str) -> Option<&StructDefInfo> {
        self.struct_defs.get(name)
    }

    #[allow(dead_code)]
    fn lookup_alias(&self, name: &str) -> Option<&TypeAliasInfo> {
        self.type_aliases.get(name)
    }
}

// ── Per-function metadata for where-clause enforcement (03-04) ────────

/// Per-function metadata for where-clause enforcement.
#[derive(Clone, Debug)]
struct FnConstraints {
    /// Where-clause constraints: (type_param_name, trait_name).
    where_constraints: Vec<(String, String)>,
    /// Type parameter names mapped to their inference type variables.
    type_params: FxHashMap<String, Ty>,
    /// For each function parameter (by index), the type parameter name it
    /// was annotated with (if any). Used to resolve type params from call-site
    /// argument types after instantiation + unification.
    param_type_param_names: Vec<Option<String>>,
}

/// Infer types for a parsed Snow program.
///
/// This is the main entry point. Creates an inference context and type
/// environment, registers builtins, then walks the AST inferring types.
pub fn infer(parse: &Parse) -> TypeckResult {
    let mut ctx = InferCtx::new();
    let mut env = TypeEnv::new();
    let mut trait_registry = TraitRegistry::new();
    let mut type_registry = TypeRegistry::new();
    builtins::register_builtins(&mut ctx, &mut env, &mut trait_registry);
    register_option_result_constructors(&mut ctx, &mut env);

    let mut types = FxHashMap::default();
    let mut result_type = None;
    let mut fn_constraints: FxHashMap<String, FnConstraints> = FxHashMap::default();

    let tree = parse.tree();

    // Walk all children of SourceFile. Items are handled via Item::cast,
    // bare expressions (top-level expressions not wrapped in items) are
    // handled via Expr::cast.
    for child in tree.syntax().children() {
        if let Some(item) = Item::cast(child.clone()) {
            let ty = infer_item(
                &mut ctx,
                &mut env,
                &item,
                &mut types,
                &mut type_registry,
                &mut trait_registry,
                &mut fn_constraints,
            );
            if let Some(ty) = ty {
                result_type = Some(ty);
            }
        } else if let Some(expr) = Expr::cast(child.clone()) {
            match infer_expr(
                &mut ctx,
                &mut env,
                &expr,
                &mut types,
                &type_registry,
                &trait_registry,
                &fn_constraints,
            ) {
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

/// Register Some, None, Ok, Err constructors for Option<T> and Result<T, E>.
///
/// Uses enter_level/leave_level to ensure fresh type variables are created
/// at a higher level than current, so they get properly generalized into
/// polymorphic schemes (forall).
fn register_option_result_constructors(ctx: &mut InferCtx, env: &mut TypeEnv) {
    // Some :: forall T. T -> Option<T>
    {
        ctx.enter_level();
        let t_var = ctx.fresh_var();
        let some_ty = Ty::Fun(
            vec![t_var.clone()],
            Box::new(Ty::option(t_var.clone())),
        );
        ctx.leave_level();
        let scheme = ctx.generalize(some_ty);
        env.insert("Some".into(), scheme);
    }

    // None :: forall T. Option<T>
    {
        ctx.enter_level();
        let t_var = ctx.fresh_var();
        let none_ty = Ty::option(t_var);
        ctx.leave_level();
        let scheme = ctx.generalize(none_ty);
        env.insert("None".into(), scheme);
    }

    // Ok :: forall T E. T -> Result<T, E>
    {
        ctx.enter_level();
        let t_var = ctx.fresh_var();
        let e_var = ctx.fresh_var();
        let ok_ty = Ty::Fun(
            vec![t_var.clone()],
            Box::new(Ty::result(t_var, e_var)),
        );
        ctx.leave_level();
        let scheme = ctx.generalize(ok_ty);
        env.insert("Ok".into(), scheme);
    }

    // Err :: forall T E. E -> Result<T, E>
    {
        ctx.enter_level();
        let t_var = ctx.fresh_var();
        let e_var = ctx.fresh_var();
        let err_ty = Ty::Fun(
            vec![e_var.clone()],
            Box::new(Ty::result(t_var, e_var)),
        );
        ctx.leave_level();
        let scheme = ctx.generalize(err_ty);
        env.insert("Err".into(), scheme);
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
    type_registry: &mut TypeRegistry,
    trait_registry: &mut TraitRegistry,
    fn_constraints: &mut FxHashMap<String, FnConstraints>,
) -> Option<Ty> {
    match item {
        Item::LetBinding(let_) => {
            infer_let_binding(ctx, env, let_, types, type_registry, trait_registry, fn_constraints)
                .ok()
        }
        Item::FnDef(fn_) => {
            infer_fn_def(ctx, env, fn_, types, type_registry, trait_registry, fn_constraints).ok()
        }
        Item::StructDef(struct_def) => {
            register_struct_def(ctx, env, struct_def, type_registry);
            None
        }
        Item::TypeAliasDef(alias_def) => {
            register_type_alias(alias_def, type_registry);
            None
        }
        Item::InterfaceDef(iface) => {
            infer_interface_def(ctx, env, iface, trait_registry);
            None
        }
        Item::ImplDef(impl_) => {
            infer_impl_def(ctx, env, impl_, types, type_registry, trait_registry, fn_constraints);
            None
        }
        // Declarations that don't produce a value type:
        Item::ModuleDef(_) | Item::ImportDecl(_) | Item::FromImportDecl(_) => None,
    }
}

// ── Struct Registration (03-03) ────────────────────────────────────────

/// Register a struct definition: extract field names/types and generic params.
fn register_struct_def(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    struct_def: &StructDef,
    type_registry: &mut TypeRegistry,
) {
    let name = struct_def
        .name()
        .and_then(|n| n.text())
        .unwrap_or_else(|| "<unnamed>".to_string());

    // Extract generic type parameters.
    let generic_params: Vec<String> = struct_def
        .syntax()
        .children()
        .filter(|n| n.kind() == SyntaxKind::GENERIC_PARAM_LIST)
        .flat_map(|gpl| {
            gpl.children_with_tokens()
                .filter_map(|t| t.into_token())
                .filter(|t| t.kind() == SyntaxKind::IDENT)
                .map(|t| t.text().to_string())
        })
        .collect();

    // Extract fields.
    let mut fields = Vec::new();
    for field in struct_def.fields() {
        let field_name = field
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<unnamed>".to_string());

        let field_ty = field
            .type_annotation()
            .and_then(|ann| resolve_type_annotation(ctx, &ann, type_registry))
            .unwrap_or_else(|| ctx.fresh_var());

        fields.push((field_name, field_ty));
    }

    // Register a constructor function: StructName(field1, field2, ...) -> StructName
    let struct_ty = if generic_params.is_empty() {
        Ty::struct_ty(&name, vec![])
    } else {
        let type_args: Vec<Ty> = generic_params.iter().map(|_| ctx.fresh_var()).collect();
        Ty::struct_ty(&name, type_args)
    };

    env.insert(name.clone(), Scheme::mono(struct_ty));

    type_registry.register_struct(StructDefInfo {
        name,
        generic_params,
        fields,
    });
}

/// Register a type alias.
fn register_type_alias(alias_def: &TypeAliasDef, type_registry: &mut TypeRegistry) {
    let name = alias_def
        .name()
        .and_then(|n| n.text())
        .unwrap_or_else(|| "<unnamed>".to_string());

    let generic_params: Vec<String> = alias_def
        .syntax()
        .children()
        .filter(|n| n.kind() == SyntaxKind::GENERIC_PARAM_LIST)
        .flat_map(|gpl| {
            gpl.children_with_tokens()
                .filter_map(|t| t.into_token())
                .filter(|t| t.kind() == SyntaxKind::IDENT)
                .map(|t| t.text().to_string())
        })
        .collect();

    // Parse the aliased type from tokens after the `=` sign.
    let aliased_type = parse_alias_type(alias_def.syntax(), &generic_params);

    type_registry.register_alias(TypeAliasInfo {
        name,
        generic_params,
        aliased_type,
    });
}

/// Parse the aliased type from a TYPE_ALIAS_DEF node.
/// Collects tokens after the `=` sign and parses them as a type.
fn parse_alias_type(node: &snow_parser::SyntaxNode, _generic_params: &[String]) -> Ty {
    let mut tokens: Vec<(SyntaxKind, String)> = Vec::new();
    let mut past_eq = false;

    for child in node.children_with_tokens() {
        match child {
            rowan::NodeOrToken::Token(t) => {
                let kind = t.kind();
                if kind == SyntaxKind::EQ {
                    past_eq = true;
                    continue;
                }
                if past_eq {
                    match kind {
                        SyntaxKind::IDENT | SyntaxKind::LT | SyntaxKind::GT
                        | SyntaxKind::COMMA | SyntaxKind::QUESTION | SyntaxKind::BANG
                        | SyntaxKind::L_PAREN | SyntaxKind::R_PAREN => {
                            tokens.push((kind, t.text().to_string()));
                        }
                        _ => {}
                    }
                }
            }
            rowan::NodeOrToken::Node(n) => {
                if past_eq {
                    collect_annotation_tokens(&n, &mut tokens);
                }
            }
        }
    }

    if tokens.is_empty() {
        return Ty::Never;
    }

    // Parse the tokens, treating generic_params as type variables
    // (they'll be represented as Ty::Con("A"), Ty::Con("B") etc.)
    parse_type_tokens(&tokens, &mut 0)
}

// ── Interface/Impl Registration (03-04) ───────────────────────────────

/// Process an interface definition: register the trait in the registry.
fn infer_interface_def(
    _ctx: &mut InferCtx,
    _env: &mut TypeEnv,
    iface: &InterfaceDef,
    trait_registry: &mut TraitRegistry,
) {
    let trait_name = iface
        .name()
        .and_then(|n| n.text())
        .unwrap_or_else(|| "<unnamed>".to_string());

    let mut methods = Vec::new();
    for method in iface.methods() {
        let method_name = method
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<unnamed>".to_string());

        let mut has_self = false;
        let mut param_count = 0;

        if let Some(param_list) = method.param_list() {
            for param in param_list.params() {
                let is_self = param
                    .syntax()
                    .children_with_tokens()
                    .any(|tok| {
                        tok.as_token()
                            .map(|t| t.kind() == SyntaxKind::SELF_KW)
                            .unwrap_or(false)
                    });
                if is_self {
                    has_self = true;
                } else {
                    param_count += 1;
                }
            }
        }

        let return_type = method.return_type().and_then(|ann| resolve_type_name(&ann));

        methods.push(TraitMethodSig {
            name: method_name,
            has_self,
            param_count,
            return_type,
        });
    }

    trait_registry.register_trait(TraitDef {
        name: trait_name,
        methods,
    });
}

/// Process an impl definition: register the impl and type-check methods.
fn infer_impl_def(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    impl_: &AstImplDef,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &mut TraitRegistry,
    fn_constraints: &mut FxHashMap<String, FnConstraints>,
) {
    // Extract trait name from the first PATH child.
    let paths: Vec<_> = impl_
        .syntax()
        .children()
        .filter(|n| n.kind() == SyntaxKind::PATH)
        .collect();

    let trait_name = paths
        .first()
        .and_then(|path| {
            path.children_with_tokens()
                .filter_map(|t| t.into_token())
                .find(|t| t.kind() == SyntaxKind::IDENT)
                .map(|t| t.text().to_string())
        })
        .unwrap_or_else(|| "<unknown>".to_string());

    // Extract type name from the second PATH child (after `for`).
    let impl_type_name = paths
        .get(1)
        .and_then(|path| {
            path.children_with_tokens()
                .filter_map(|t| t.into_token())
                .find(|t| t.kind() == SyntaxKind::IDENT)
                .map(|t| t.text().to_string())
        })
        .unwrap_or_else(|| "<unknown>".to_string());

    let impl_type = name_to_type(&impl_type_name);

    // Collect methods from the impl block.
    let mut impl_methods = FxHashMap::default();

    for method in impl_.methods() {
        let method_name = method
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<unnamed>".to_string());

        let mut has_self = false;
        let mut param_count = 0;

        if let Some(param_list) = method.param_list() {
            for param in param_list.params() {
                let is_self = param
                    .syntax()
                    .children_with_tokens()
                    .any(|tok| {
                        tok.as_token()
                            .map(|t| t.kind() == SyntaxKind::SELF_KW)
                            .unwrap_or(false)
                    });
                if is_self {
                    has_self = true;
                } else {
                    param_count += 1;
                }
            }
        }

        let return_type = method.return_type().and_then(|ann| resolve_type_name(&ann));

        impl_methods.insert(
            method_name.clone(),
            ImplMethodSig {
                has_self,
                param_count,
                return_type: return_type.clone(),
            },
        );

        // Also infer the method body to check it type-checks.
        env.push_scope();
        env.insert("self".into(), Scheme::mono(impl_type.clone()));

        if let Some(param_list) = method.param_list() {
            for param in param_list.params() {
                let is_self = param
                    .syntax()
                    .children_with_tokens()
                    .any(|tok| {
                        tok.as_token()
                            .map(|t| t.kind() == SyntaxKind::SELF_KW)
                            .unwrap_or(false)
                    });
                if !is_self {
                    if let Some(name_tok) = param.name() {
                        let name_text = name_tok.text().to_string();
                        let param_ty = param
                            .type_annotation()
                            .and_then(|ann| resolve_type_name(&ann))
                            .unwrap_or_else(|| ctx.fresh_var());
                        env.insert(name_text, Scheme::mono(param_ty));
                    }
                }
            }
        }

        if let Some(body) = method.body() {
            match infer_block(
                ctx,
                env,
                &body,
                types,
                type_registry,
                &*trait_registry,
                fn_constraints,
            ) {
                Ok(body_ty) => {
                    if let Some(ref ret_ty) = return_type {
                        let _ = ctx.unify(body_ty, ret_ty.clone(), ConstraintOrigin::Builtin);
                    }
                }
                Err(_) => { /* error already recorded */ }
            }
        }

        env.pop_scope();

        // Register the method as a callable function so `to_string(42)` works.
        let fn_ty = {
            let params = vec![impl_type.clone()];
            let ret = return_type.clone().unwrap_or_else(|| Ty::Tuple(vec![]));
            Ty::Fun(params, Box::new(ret))
        };
        if env.lookup(&method_name).is_none() {
            env.insert(method_name.clone(), Scheme::mono(fn_ty));
        }
    }

    // Register the impl and collect validation errors.
    let errors = trait_registry.register_impl(TraitImplDef {
        trait_name,
        impl_type,
        impl_type_name,
        methods: impl_methods,
    });

    ctx.errors.extend(errors);
}

/// Infer a let binding: `let x = expr`
fn infer_let_binding(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    let_: &LetBinding,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    ctx.enter_level();

    let init_expr = let_.initializer().ok_or_else(|| {
        let err = TypeError::Mismatch {
            expected: Ty::Never,
            found: Ty::Never,
            origin: ConstraintOrigin::Builtin,
        };
        ctx.errors.push(err.clone());
        err
    })?;

    let init_ty = infer_expr(ctx, env, &init_expr, types, type_registry, trait_registry, fn_constraints)?;

    // If there is a type annotation, resolve and unify with the inferred type.
    if let Some(annotation) = let_.type_annotation() {
        if let Some(ann_ty) = resolve_type_annotation(ctx, &annotation, type_registry) {
            let origin = ConstraintOrigin::Annotation {
                annotation_span: annotation.syntax().text_range(),
            };
            ctx.unify(init_ty.clone(), ann_ty, origin)?;
        }
    }

    ctx.leave_level();
    let scheme = ctx.generalize(init_ty.clone());

    if let Some(name) = let_.name() {
        if let Some(name_text) = name.text() {
            env.insert(name_text, scheme);
        }
    } else if let Some(pat) = let_.pattern() {
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

/// Infer a named function definition: `fn name(params) [-> RetType] [where T: Trait] do body end`
fn infer_fn_def(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    fn_: &FnDef,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &mut FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    let fn_name = fn_
        .name()
        .and_then(|n| n.text())
        .unwrap_or_else(|| "<anonymous>".to_string());

    ctx.enter_level();

    let self_var = ctx.fresh_var();
    env.insert(fn_name.clone(), Scheme::mono(self_var.clone()));

    // Extract generic type parameters if present.
    let mut type_params: FxHashMap<String, Ty> = FxHashMap::default();
    for child in fn_.syntax().children() {
        if child.kind() == SyntaxKind::GENERIC_PARAM_LIST {
            for tok in child.children_with_tokens() {
                if let Some(token) = tok.as_token() {
                    if token.kind() == SyntaxKind::IDENT {
                        let param_name = token.text().to_string();
                        let param_ty = ctx.fresh_var();
                        type_params.insert(param_name, param_ty);
                    }
                }
            }
        }
    }

    // Extract where-clause constraints.
    let where_constraints = extract_where_constraints(fn_);

    env.push_scope();

    // Insert type params into the scope.
    for (name, ty) in &type_params {
        env.insert(name.clone(), Scheme::mono(ty.clone()));
    }

    let mut param_types = Vec::new();
    let mut param_type_param_names: Vec<Option<String>> = Vec::new();

    if let Some(param_list) = fn_.param_list() {
        for param in param_list.params() {
            let (param_ty, tp_name) = if let Some(ann) = param.type_annotation() {
                if let Some(type_name) = resolve_type_name_str(&ann) {
                    if let Some(tp_ty) = type_params.get(&type_name) {
                        (tp_ty.clone(), Some(type_name))
                    } else {
                        (name_to_type(&type_name), None)
                    }
                } else {
                    (ctx.fresh_var(), None)
                }
            } else {
                (ctx.fresh_var(), None)
            };

            if let Some(name_tok) = param.name() {
                let name_text = name_tok.text().to_string();
                env.insert(name_text, Scheme::mono(param_ty.clone()));
            }
            param_types.push(param_ty);
            param_type_param_names.push(tp_name);
        }
    }

    if !where_constraints.is_empty() || !type_params.is_empty() {
        fn_constraints.insert(
            fn_name.clone(),
            FnConstraints {
                where_constraints: where_constraints.clone(),
                type_params: type_params.clone(),
                param_type_param_names,
            },
        );
    }

    // Parse return type annotation.
    let return_type_annotation = fn_.return_type().and_then(|ann| {
        let type_name = resolve_type_name_str(&ann)?;
        if let Some(tp_ty) = type_params.get(&type_name) {
            Some(tp_ty.clone())
        } else {
            Some(name_to_type(&type_name))
        }
    });

    let body_ty = if let Some(body) = fn_.body() {
        infer_block(ctx, env, &body, types, type_registry, trait_registry, fn_constraints)?
    } else {
        Ty::Tuple(vec![])
    };

    if let Some(ref ret_ann) = return_type_annotation {
        ctx.unify(body_ty.clone(), ret_ann.clone(), ConstraintOrigin::Builtin)?;
    }

    env.pop_scope();

    let ret_ty = return_type_annotation.unwrap_or(body_ty);
    let fn_ty = Ty::Fun(param_types, Box::new(ret_ty));

    ctx.unify(self_var, fn_ty.clone(), ConstraintOrigin::Builtin)?;

    ctx.leave_level();
    let scheme = ctx.generalize(fn_ty.clone());

    env.insert(fn_name, scheme);

    let resolved = ctx.resolve(fn_ty);
    types.insert(fn_.syntax().text_range(), resolved.clone());

    Ok(resolved)
}

// ── Expression Inference ───────────────────────────────────────────────

/// Infer the type of an expression.
fn infer_expr(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    expr: &Expr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    let ty = match expr {
        Expr::Literal(lit) => infer_literal(lit),
        Expr::NameRef(name_ref) => infer_name_ref(ctx, env, name_ref)?,
        Expr::BinaryExpr(bin) => {
            infer_binary(ctx, env, bin, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::UnaryExpr(un) => {
            infer_unary(ctx, env, un, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::CallExpr(call) => {
            infer_call(ctx, env, call, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::PipeExpr(pipe) => {
            infer_pipe(ctx, env, pipe, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::IfExpr(if_) => {
            infer_if(ctx, env, if_, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::ClosureExpr(closure) => {
            infer_closure(ctx, env, closure, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::Block(block) => {
            infer_block(ctx, env, block, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::TupleExpr(tuple) => {
            infer_tuple(ctx, env, tuple, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::CaseExpr(case) => {
            infer_case(ctx, env, case, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::ReturnExpr(ret) => {
            infer_return(ctx, env, ret, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::StringExpr(_) => Ty::string(),
        Expr::FieldAccess(fa) => {
            infer_field_access(ctx, env, fa, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::StructLiteral(sl) => {
            infer_struct_literal(ctx, env, sl, types, type_registry, trait_registry, fn_constraints)?
        }
        Expr::IndexExpr(_) => ctx.fresh_var(),
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
            SyntaxKind::NIL_KW => Ty::Tuple(vec![]),
            SyntaxKind::STRING_START => Ty::string(),
            _ => Ty::Tuple(vec![]),
        }
    } else {
        Ty::Tuple(vec![])
    }
}

/// Infer the type of a name reference (variable lookup).
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

/// Infer the type of a binary expression with trait-based operator dispatch.
fn infer_binary(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    bin: &BinaryExpr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
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

    let lhs_ty = infer_expr(ctx, env, &lhs_expr, types, type_registry, trait_registry, fn_constraints)?;
    let rhs_ty = infer_expr(ctx, env, &rhs_expr, types, type_registry, trait_registry, fn_constraints)?;

    let op = bin.op();
    let op_kind = op.as_ref().map(|t| t.kind());

    let origin = ConstraintOrigin::BinOp {
        op_span: bin.syntax().text_range(),
    };

    match op_kind {
        // Arithmetic: dispatch via compiler-known traits
        Some(SyntaxKind::PLUS) => {
            infer_trait_binary_op(ctx, "Add", &lhs_ty, &rhs_ty, trait_registry, &origin)
        }
        Some(SyntaxKind::MINUS) => {
            infer_trait_binary_op(ctx, "Sub", &lhs_ty, &rhs_ty, trait_registry, &origin)
        }
        Some(SyntaxKind::STAR) => {
            infer_trait_binary_op(ctx, "Mul", &lhs_ty, &rhs_ty, trait_registry, &origin)
        }
        Some(SyntaxKind::SLASH) => {
            infer_trait_binary_op(ctx, "Div", &lhs_ty, &rhs_ty, trait_registry, &origin)
        }
        Some(SyntaxKind::PERCENT) => {
            infer_trait_binary_op(ctx, "Mod", &lhs_ty, &rhs_ty, trait_registry, &origin)
        }

        // Equality: dispatch via Eq trait, return Bool
        Some(SyntaxKind::EQ_EQ | SyntaxKind::NOT_EQ) => {
            ctx.unify(lhs_ty.clone(), rhs_ty, origin.clone())?;
            let resolved = ctx.resolve(lhs_ty);
            if !is_type_var(&resolved) && !trait_registry.has_impl("Eq", &resolved) {
                let err = TypeError::TraitNotSatisfied {
                    ty: resolved,
                    trait_name: "Eq".to_string(),
                    origin,
                };
                ctx.errors.push(err.clone());
                return Err(err);
            }
            Ok(Ty::bool())
        }

        // Ordering: dispatch via Ord trait, return Bool
        Some(SyntaxKind::LT | SyntaxKind::GT | SyntaxKind::LT_EQ | SyntaxKind::GT_EQ) => {
            ctx.unify(lhs_ty.clone(), rhs_ty, origin.clone())?;
            let resolved = ctx.resolve(lhs_ty);
            if !is_type_var(&resolved) && !trait_registry.has_impl("Ord", &resolved) {
                let err = TypeError::TraitNotSatisfied {
                    ty: resolved,
                    trait_name: "Ord".to_string(),
                    origin,
                };
                ctx.errors.push(err.clone());
                return Err(err);
            }
            Ok(Ty::bool())
        }

        // Logical: unify both sides with Bool, return Bool
        Some(
            SyntaxKind::AND_KW | SyntaxKind::OR_KW | SyntaxKind::AMP_AMP | SyntaxKind::PIPE_PIPE,
        ) => {
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

/// Infer a binary operator using trait dispatch.
fn infer_trait_binary_op(
    ctx: &mut InferCtx,
    trait_name: &str,
    lhs_ty: &Ty,
    rhs_ty: &Ty,
    trait_registry: &TraitRegistry,
    origin: &ConstraintOrigin,
) -> Result<Ty, TypeError> {
    ctx.unify(lhs_ty.clone(), rhs_ty.clone(), origin.clone())?;

    let resolved = ctx.resolve(lhs_ty.clone());

    if is_type_var(&resolved) {
        return Ok(resolved);
    }

    if trait_registry.has_impl(trait_name, &resolved) {
        Ok(resolved)
    } else {
        let err = TypeError::TraitNotSatisfied {
            ty: resolved,
            trait_name: trait_name.to_string(),
            origin: origin.clone(),
        };
        ctx.errors.push(err.clone());
        Err(err)
    }
}

/// Infer the type of a unary expression.
fn infer_unary(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    un: &UnaryExpr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
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

    let operand_ty = infer_expr(ctx, env, &operand, types, type_registry, trait_registry, fn_constraints)?;

    let op_kind = un.op().map(|t| t.kind());

    match op_kind {
        Some(SyntaxKind::MINUS) => Ok(operand_ty),
        Some(SyntaxKind::BANG | SyntaxKind::NOT_KW) => {
            ctx.unify(operand_ty, Ty::bool(), ConstraintOrigin::Builtin)?;
            Ok(Ty::bool())
        }
        _ => Ok(operand_ty),
    }
}

/// Infer the type of a function call expression with where-clause enforcement.
fn infer_call(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    call: &CallExpr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
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

    let callee_ty = infer_expr(ctx, env, &callee_expr, types, type_registry, trait_registry, fn_constraints)?;

    let mut arg_types = Vec::new();
    if let Some(arg_list) = call.arg_list() {
        for arg in arg_list.args() {
            let arg_ty = infer_expr(ctx, env, &arg, types, type_registry, trait_registry, fn_constraints)?;
            arg_types.push(arg_ty);
        }
    }

    let ret_var = ctx.fresh_var();
    let expected_fn_ty = Ty::Fun(arg_types.clone(), Box::new(ret_var.clone()));

    let origin = ConstraintOrigin::FnArg {
        call_site: call.syntax().text_range(),
        param_idx: 0,
    };
    ctx.unify(callee_ty, expected_fn_ty, origin.clone())?;

    // Check where-clause constraints at the call site.
    // After unification, arg_types hold the resolved concrete types for each
    // parameter. Use param_type_param_names to map from arg position back to
    // type parameter name, then check trait constraints on the resolved types.
    if let Expr::NameRef(name_ref) = &callee_expr {
        if let Some(fn_name) = name_ref.text() {
            if let Some(constraints) = fn_constraints.get(&fn_name) {
                if !constraints.where_constraints.is_empty() {
                    let mut resolved_type_args: FxHashMap<String, Ty> = FxHashMap::default();

                    // Build type param -> resolved type mapping from call-site args.
                    for (i, tp_name_opt) in constraints.param_type_param_names.iter().enumerate() {
                        if let Some(tp_name) = tp_name_opt {
                            if i < arg_types.len() {
                                let resolved = ctx.resolve(arg_types[i].clone());
                                resolved_type_args.insert(tp_name.clone(), resolved);
                            }
                        }
                    }

                    // Fallback: also try definition-time vars (may work for non-generic cases).
                    for (param_name, param_ty) in &constraints.type_params {
                        if !resolved_type_args.contains_key(param_name) {
                            let resolved = ctx.resolve(param_ty.clone());
                            resolved_type_args.insert(param_name.clone(), resolved);
                        }
                    }

                    let errors = trait_registry.check_where_constraints(
                        &constraints.where_constraints,
                        &resolved_type_args,
                        origin,
                    );
                    ctx.errors.extend(errors.clone());

                    if let Some(first_err) = errors.into_iter().next() {
                        return Err(first_err);
                    }
                }
            }
        }
    }

    Ok(ret_var)
}

/// Infer the type of a pipe expression: `lhs |> rhs`
fn infer_pipe(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    pipe: &PipeExpr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
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

    let lhs_ty = infer_expr(ctx, env, &lhs, types, type_registry, trait_registry, fn_constraints)?;
    let rhs_ty = infer_expr(ctx, env, &rhs, types, type_registry, trait_registry, fn_constraints)?;

    let ret_var = ctx.fresh_var();
    let expected_fn = Ty::Fun(vec![lhs_ty], Box::new(ret_var.clone()));

    ctx.unify(rhs_ty, expected_fn, ConstraintOrigin::Builtin)?;

    Ok(ret_var)
}

/// Infer the type of an if expression.
fn infer_if(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    if_: &IfExpr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    if let Some(cond) = if_.condition() {
        let cond_ty = infer_expr(ctx, env, &cond, types, type_registry, trait_registry, fn_constraints)?;
        ctx.unify(cond_ty, Ty::bool(), ConstraintOrigin::Builtin)?;
    }

    let then_ty = if let Some(then_block) = if_.then_branch() {
        infer_block(ctx, env, &then_block, types, type_registry, trait_registry, fn_constraints)?
    } else {
        Ty::Tuple(vec![])
    };

    if let Some(else_branch) = if_.else_branch() {
        let else_ty = if let Some(else_if) = else_branch.if_expr() {
            infer_if(ctx, env, &else_if, types, type_registry, trait_registry, fn_constraints)?
        } else if let Some(else_block) = else_branch.block() {
            infer_block(ctx, env, &else_block, types, type_registry, trait_registry, fn_constraints)?
        } else {
            Ty::Tuple(vec![])
        };

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
        Ok(then_ty)
    }
}

/// Infer the type of a closure expression: `fn (params) -> body end`
fn infer_closure(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    closure: &ClosureExpr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
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

    let body_ty = if let Some(body) = closure.body() {
        infer_block(ctx, env, &body, types, type_registry, trait_registry, fn_constraints)?
    } else {
        Ty::Tuple(vec![])
    };

    env.pop_scope();

    Ok(Ty::Fun(param_types, Box::new(body_ty)))
}

/// Infer the type of a block.
fn infer_block(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    block: &Block,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    let mut last_ty = Ty::Tuple(vec![]);

    for stmt in block.stmts() {
        match &stmt {
            Item::LetBinding(let_) => {
                if let Ok(ty) = infer_let_binding(
                    ctx,
                    env,
                    let_,
                    types,
                    type_registry,
                    trait_registry,
                    &FxHashMap::default(),
                ) {
                    last_ty = ty;
                }
            }
            Item::FnDef(fn_) => {
                if let Ok(ty) = infer_fn_def(
                    ctx,
                    env,
                    fn_,
                    types,
                    type_registry,
                    trait_registry,
                    &mut FxHashMap::default(),
                ) {
                    last_ty = ty;
                }
            }
            _ => {
                // Other items (interface, impl, struct, etc.) don't produce values in blocks.
            }
        }
    }

    if let Some(tail) = block.tail_expr() {
        let tail_range = tail.syntax().text_range();
        let is_already_part_of_stmt = block.stmts().any(|stmt| {
            let item_range = match &stmt {
                Item::LetBinding(lb) => lb.syntax().text_range(),
                Item::FnDef(fd) => fd.syntax().text_range(),
                _ => return false,
            };
            item_range.start() <= tail_range.start() && tail_range.end() <= item_range.end()
        });

        if !is_already_part_of_stmt {
            match infer_expr(ctx, env, &tail, types, type_registry, trait_registry, fn_constraints) {
                Ok(ty) => {
                    last_ty = ty;
                }
                Err(_) => {}
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
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    let mut elem_types = Vec::new();
    for elem in tuple.elements() {
        let ty = infer_expr(ctx, env, &elem, types, type_registry, trait_registry, fn_constraints)?;
        elem_types.push(ty);
    }

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
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    let scrutinee_ty = if let Some(scrutinee) = case.scrutinee() {
        infer_expr(ctx, env, &scrutinee, types, type_registry, trait_registry, fn_constraints)?
    } else {
        ctx.fresh_var()
    };

    let mut result_ty: Option<Ty> = None;

    for arm in case.arms() {
        env.push_scope();

        if let Some(pat) = arm.pattern() {
            let pat_ty = infer_pattern(ctx, env, &pat, types)?;
            ctx.unify(pat_ty, scrutinee_ty.clone(), ConstraintOrigin::Builtin)?;
        }

        if let Some(body) = arm.body() {
            let body_ty = infer_expr(ctx, env, &body, types, type_registry, trait_registry, fn_constraints)?;
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
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    if let Some(value) = ret.value() {
        let _ty = infer_expr(ctx, env, &value, types, type_registry, trait_registry, fn_constraints)?;
    }
    Ok(Ty::Never)
}

// ── Struct/Field Inference (03-03) ─────────────────────────────────────

/// Infer the type of a field access expression: `expr.field_name`
fn infer_field_access(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    fa: &FieldAccess,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    let base_expr = fa.base().ok_or_else(|| {
        let err = TypeError::Mismatch {
            expected: Ty::Never,
            found: Ty::Never,
            origin: ConstraintOrigin::Builtin,
        };
        ctx.errors.push(err.clone());
        err
    })?;
    let base_ty = infer_expr(ctx, env, &base_expr, types, type_registry, trait_registry, fn_constraints)?;
    let resolved_base = ctx.resolve(base_ty);

    let field_name = match fa.field() {
        Some(tok) => tok.text().to_string(),
        None => "<unknown>".to_string(),
    };

    let struct_name = match &resolved_base {
        Ty::App(con, _) => {
            if let Ty::Con(tc) = con.as_ref() {
                Some(tc.name.clone())
            } else {
                None
            }
        }
        Ty::Con(tc) => Some(tc.name.clone()),
        _ => None,
    };

    if let Some(name) = struct_name {
        if let Some(struct_info) = type_registry.lookup_struct(&name) {
            let struct_info = struct_info.clone();
            // Get the type arguments from the resolved base type.
            let type_args = match &resolved_base {
                Ty::App(_, args) => args.clone(),
                _ => vec![],
            };
            for (fname, fty) in &struct_info.fields {
                if *fname == field_name {
                    // Substitute generic params with actual type args.
                    let resolved_field = substitute_type_params(
                        fty,
                        &struct_info.generic_params,
                        &type_args,
                    );
                    return Ok(resolved_field);
                }
            }
            // Field not found in struct.
            let err = TypeError::NoSuchField {
                ty: resolved_base,
                field_name,
                span: fa.syntax().text_range(),
            };
            ctx.errors.push(err.clone());
            return Err(err);
        }
    }

    Ok(ctx.fresh_var())
}

/// Infer the type of a struct literal: `StructName { field1: expr1, ... }`
///
/// 1. Look up the struct definition.
/// 2. Create fresh type variables for generic parameters.
/// 3. For each field in the literal, infer value type and unify with expected.
/// 4. Check all required fields are present.
/// 5. Return the struct type with inferred generic arguments.
fn infer_struct_literal(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    sl: &StructLiteral,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    let struct_name = sl
        .name_ref()
        .and_then(|nr| nr.text())
        .unwrap_or_else(|| "<unknown>".to_string());

    let struct_def = match type_registry.lookup_struct(&struct_name) {
        Some(def) => def.clone(),
        None => {
            // Unknown struct -- infer field values anyway, return a basic type.
            for field in sl.fields() {
                if let Some(value) = field.value() {
                    let _ = infer_expr(ctx, env, &value, types, type_registry, trait_registry, fn_constraints);
                }
            }
            return Ok(Ty::struct_ty(&struct_name, vec![]));
        }
    };

    // Create fresh type variables for generic params.
    let generic_vars: Vec<Ty> = struct_def
        .generic_params
        .iter()
        .map(|_| ctx.fresh_var())
        .collect();

    // Track provided fields.
    let mut provided_fields: Vec<String> = Vec::new();

    for field in sl.fields() {
        let field_name = match field.name().and_then(|n| n.text()) {
            Some(n) => n,
            None => continue,
        };

        // Find expected field type.
        let expected_ty = struct_def
            .fields
            .iter()
            .find(|(name, _)| *name == field_name)
            .map(|(_, ty)| {
                substitute_type_params(ty, &struct_def.generic_params, &generic_vars)
            });

        let expected_ty = match expected_ty {
            Some(ty) => ty,
            None => {
                let err = TypeError::UnknownField {
                    struct_name: struct_name.clone(),
                    field_name: field_name.clone(),
                    span: field.syntax().text_range(),
                };
                ctx.errors.push(err.clone());
                return Err(err);
            }
        };

        // Infer field value.
        if let Some(value) = field.value() {
            let value_ty = infer_expr(ctx, env, &value, types, type_registry, trait_registry, fn_constraints)?;
            ctx.unify(
                value_ty,
                expected_ty,
                ConstraintOrigin::Annotation {
                    annotation_span: field.syntax().text_range(),
                },
            )?;
        }

        provided_fields.push(field_name);
    }

    // Check for missing fields.
    for (field_name, _) in &struct_def.fields {
        if !provided_fields.contains(field_name) {
            let err = TypeError::MissingField {
                struct_name: struct_name.clone(),
                field_name: field_name.clone(),
                span: sl.syntax().text_range(),
            };
            ctx.errors.push(err.clone());
            return Err(err);
        }
    }

    Ok(Ty::App(
        Box::new(Ty::Con(TyCon::new(&struct_name))),
        generic_vars,
    ))
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

// ── Helpers ────────────────────────────────────────────────────────────

/// Extract where-clause constraints from a function definition.
fn extract_where_constraints(fn_: &FnDef) -> Vec<(String, String)> {
    let mut constraints = Vec::new();

    for child in fn_.syntax().children() {
        if child.kind() == SyntaxKind::WHERE_CLAUSE {
            for bound in child.children() {
                if bound.kind() == SyntaxKind::TRAIT_BOUND {
                    let tokens: Vec<_> = bound
                        .children_with_tokens()
                        .filter_map(|t| t.into_token())
                        .filter(|t| t.kind() == SyntaxKind::IDENT)
                        .collect();

                    if tokens.len() >= 2 {
                        let type_param = tokens[0].text().to_string();
                        let trait_name = tokens[1].text().to_string();
                        constraints.push((type_param, trait_name));
                    }
                }
            }
        }
    }

    constraints
}

/// Resolve a type annotation to a Ty, from the annotation's type name.
fn resolve_type_name(ann: &snow_parser::ast::item::TypeAnnotation) -> Option<Ty> {
    let name = resolve_type_name_str(ann)?;
    Some(name_to_type(&name))
}

/// Extract the type name string from a type annotation.
fn resolve_type_name_str(ann: &snow_parser::ast::item::TypeAnnotation) -> Option<String> {
    ann.type_name().map(|t| t.text().to_string())
}

/// Resolve a type annotation using the type registry (supports struct types, aliases).
fn resolve_type_annotation(
    _ctx: &mut InferCtx,
    ann: &snow_parser::ast::item::TypeAnnotation,
    type_registry: &TypeRegistry,
) -> Option<Ty> {
    // Collect all significant tokens from the annotation to parse the full type.
    let mut tokens: Vec<(SyntaxKind, String)> = Vec::new();
    collect_annotation_tokens(ann.syntax(), &mut tokens);
    if tokens.is_empty() {
        return None;
    }
    let ty = parse_type_tokens(&tokens, &mut 0);
    Some(resolve_alias(ty, type_registry))
}

/// Collect significant tokens (IDENT, LT, GT, COMMA, QUESTION, BANG,
/// L_PAREN, R_PAREN) from a TYPE_ANNOTATION node tree.
fn collect_annotation_tokens(
    node: &snow_parser::SyntaxNode,
    tokens: &mut Vec<(SyntaxKind, String)>,
) {
    for child in node.children_with_tokens() {
        match child {
            rowan::NodeOrToken::Token(t) => {
                let kind = t.kind();
                match kind {
                    SyntaxKind::IDENT | SyntaxKind::LT | SyntaxKind::GT
                    | SyntaxKind::COMMA | SyntaxKind::QUESTION | SyntaxKind::BANG
                    | SyntaxKind::L_PAREN | SyntaxKind::R_PAREN => {
                        tokens.push((kind, t.text().to_string()));
                    }
                    _ => {}
                }
            }
            rowan::NodeOrToken::Node(n) => {
                collect_annotation_tokens(&n, tokens);
            }
        }
    }
}

/// Parse a Ty from a flat list of significant tokens.
fn parse_type_tokens(tokens: &[(SyntaxKind, String)], pos: &mut usize) -> Ty {
    if *pos >= tokens.len() {
        return Ty::Never;
    }

    // Tuple: (A, B)
    if tokens[*pos].0 == SyntaxKind::L_PAREN {
        *pos += 1;
        let mut elems = Vec::new();
        while *pos < tokens.len() && tokens[*pos].0 != SyntaxKind::R_PAREN {
            elems.push(parse_type_tokens(tokens, pos));
            if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::COMMA {
                *pos += 1;
            }
        }
        if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::R_PAREN {
            *pos += 1;
        }
        let base = Ty::Tuple(elems);
        return apply_type_sugar(tokens, pos, base);
    }

    if tokens[*pos].0 != SyntaxKind::IDENT {
        return Ty::Never;
    }

    let name = tokens[*pos].1.clone();
    *pos += 1;

    // Generic args: Name<A, B>
    let base = if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::LT {
        *pos += 1;
        let mut args = Vec::new();
        while *pos < tokens.len() && tokens[*pos].0 != SyntaxKind::GT {
            args.push(parse_type_tokens(tokens, pos));
            if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::COMMA {
                *pos += 1;
            }
        }
        if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::GT {
            *pos += 1;
        }
        Ty::App(Box::new(Ty::Con(TyCon::new(&name))), args)
    } else {
        name_to_type(&name)
    };

    apply_type_sugar(tokens, pos, base)
}

/// Apply sugar postfix: `?` for Option, `!` for Result.
fn apply_type_sugar(tokens: &[(SyntaxKind, String)], pos: &mut usize, base: Ty) -> Ty {
    if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::QUESTION {
        *pos += 1;
        Ty::option(base)
    } else if *pos < tokens.len() && tokens[*pos].0 == SyntaxKind::BANG {
        *pos += 1;
        let err_ty = parse_type_tokens(tokens, pos);
        Ty::result(base, err_ty)
    } else {
        base
    }
}

/// Recursively resolve type aliases.
fn resolve_alias(ty: Ty, type_registry: &TypeRegistry) -> Ty {
    match ty {
        Ty::App(con, args) => {
            if let Ty::Con(ref tc) = *con {
                if let Some(alias) = type_registry.lookup_alias(&tc.name) {
                    let resolved_args: Vec<Ty> = args
                        .into_iter()
                        .map(|a| resolve_alias(a, type_registry))
                        .collect();
                    return substitute_type_params(
                        &alias.aliased_type,
                        &alias.generic_params,
                        &resolved_args,
                    );
                }
            }
            let resolved_args: Vec<Ty> = args
                .into_iter()
                .map(|a| resolve_alias(a, type_registry))
                .collect();
            Ty::App(con, resolved_args)
        }
        Ty::Con(ref tc) => {
            if let Some(alias) = type_registry.lookup_alias(&tc.name) {
                if alias.generic_params.is_empty() {
                    return resolve_alias(alias.aliased_type.clone(), type_registry);
                }
            }
            ty
        }
        Ty::Fun(params, ret) => {
            let p: Vec<Ty> = params.into_iter().map(|p| resolve_alias(p, type_registry)).collect();
            Ty::Fun(p, Box::new(resolve_alias(*ret, type_registry)))
        }
        Ty::Tuple(elems) => {
            let e: Vec<Ty> = elems.into_iter().map(|e| resolve_alias(e, type_registry)).collect();
            Ty::Tuple(e)
        }
        _ => ty,
    }
}

/// Substitute named type parameters with concrete types.
fn substitute_type_params(ty: &Ty, param_names: &[String], param_values: &[Ty]) -> Ty {
    match ty {
        Ty::Con(tc) => {
            if let Some(idx) = param_names.iter().position(|p| *p == tc.name) {
                if idx < param_values.len() {
                    return param_values[idx].clone();
                }
            }
            ty.clone()
        }
        Ty::App(con, args) => {
            let new_con = substitute_type_params(con, param_names, param_values);
            let new_args: Vec<Ty> = args
                .iter()
                .map(|a| substitute_type_params(a, param_names, param_values))
                .collect();
            Ty::App(Box::new(new_con), new_args)
        }
        Ty::Fun(params, ret) => {
            let p: Vec<Ty> = params.iter().map(|p| substitute_type_params(p, param_names, param_values)).collect();
            Ty::Fun(p, Box::new(substitute_type_params(ret, param_names, param_values)))
        }
        Ty::Tuple(elems) => {
            let e: Vec<Ty> = elems.iter().map(|e| substitute_type_params(e, param_names, param_values)).collect();
            Ty::Tuple(e)
        }
        _ => ty.clone(),
    }
}

/// Convert a type name string to a Ty.
fn name_to_type(name: &str) -> Ty {
    match name {
        "Int" => Ty::int(),
        "Float" => Ty::float(),
        "String" => Ty::string(),
        "Bool" => Ty::bool(),
        other => Ty::Con(TyCon::new(other)),
    }
}

/// Check if a type is an unresolved type variable.
fn is_type_var(ty: &Ty) -> bool {
    matches!(ty, Ty::Var(_))
}
