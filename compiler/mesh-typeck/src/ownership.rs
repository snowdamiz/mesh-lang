//! Flow-sensitive validation for affine resource values.

use mesh_parser::ast::expr::{
    CallExpr, CaseExpr, ClosureExpr, Expr, IfExpr, NameRef, ReceiveExpr, StructLiteral,
    StructUpdate,
};
use mesh_parser::ast::item::{
    ActorDef, Block, FnDef, Item, LetBinding, ModuleDef, Param, ParamOwnership,
};
use mesh_parser::ast::pat::Pattern;
use mesh_parser::ast::AstNode;
use mesh_parser::{Parse, SyntaxKind};
use rowan::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::TypeError;
use crate::infer::TypeRegistry;
use crate::ty::Ty;
use crate::{ImportContext, ModuleExports};

#[derive(Clone)]
struct Binding {
    ty: Ty,
    moved: bool,
    definitely_moved: bool,
    borrowed: bool,
}

#[derive(Clone)]
struct FunctionSignature {
    modes: Vec<ParamOwnership>,
    formal_types: Vec<Option<Ty>>,
}

#[derive(Clone, Copy)]
enum Usage {
    Read,
    Move,
}

struct Checker<'a> {
    types: &'a FxHashMap<TextRange, Ty>,
    registry: &'a TypeRegistry,
    scopes: Vec<FxHashMap<String, Binding>>,
    signatures: FxHashMap<String, FunctionSignature>,
    errors: Vec<TypeError>,
}

pub(crate) struct OwnershipCheck {
    pub errors: Vec<TypeError>,
    pub function_ownership: FxHashMap<String, Vec<ParamOwnership>>,
}

pub(crate) fn check(
    parse: &Parse,
    types: &FxHashMap<TextRange, Ty>,
    registry: &TypeRegistry,
    import_ctx: &ImportContext,
) -> OwnershipCheck {
    let functions: Vec<FnDef> = parse
        .syntax()
        .descendants()
        .filter_map(FnDef::cast)
        .collect();
    let actors: Vec<ActorDef> = parse
        .syntax()
        .descendants()
        .filter_map(ActorDef::cast)
        .collect();
    let top_level_bindings: Vec<LetBinding> = parse
        .syntax()
        .descendants()
        .filter_map(LetBinding::cast)
        .filter(|binding| {
            !binding.syntax().ancestors().skip(1).any(|ancestor| {
                matches!(
                    ancestor.kind(),
                    SyntaxKind::FN_DEF | SyntaxKind::ACTOR_DEF | SyntaxKind::CLOSURE_EXPR
                )
            })
        })
        .collect();
    let mut signatures = FxHashMap::default();
    let mut ambiguous_bare_signatures = FxHashSet::default();
    for function in &functions {
        if let Some(name) = function.name().and_then(|name| name.text()) {
            let inferred_formals = match types.get(&function.syntax().text_range()) {
                Some(Ty::Fun(parameters, _)) => parameters.clone(),
                _ => Vec::new(),
            };
            let (modes, formal_types) = function
                .param_list()
                .map(|parameters| {
                    parameters
                        .params()
                        .enumerate()
                        .map(|(index, parameter)| {
                            let formal = inferred_formals.get(index).cloned().or_else(|| {
                                parameter.type_annotation().and_then(|annotation| {
                                    annotation
                                        .type_name()
                                        .map(|name| Ty::Con(crate::ty::TyCon::new(name.text())))
                                })
                            });
                            (parameter.ownership(), formal)
                        })
                        .unzip()
                })
                .unwrap_or_default();
            let module_name = function
                .syntax()
                .ancestors()
                .skip(1)
                .find_map(ModuleDef::cast)
                .and_then(|module| module.name())
                .and_then(|name| name.text());
            register_signature(
                &mut signatures,
                &mut ambiguous_bare_signatures,
                name,
                module_name,
                FunctionSignature {
                    modes,
                    formal_types,
                },
            );
        }
    }
    for actor in &actors {
        if let Some(name) = actor.name().and_then(|name| name.text()) {
            let inferred_formals = match types.get(&actor.syntax().text_range()) {
                Some(Ty::Fun(parameters, _)) => parameters.clone(),
                _ => Vec::new(),
            };
            let (modes, formal_types) = actor
                .param_list()
                .map(|parameters| {
                    parameters
                        .params()
                        .enumerate()
                        .map(|(index, parameter)| {
                            let formal = inferred_formals.get(index).cloned().or_else(|| {
                                parameter.type_annotation().and_then(|annotation| {
                                    annotation
                                        .type_name()
                                        .map(|name| Ty::Con(crate::ty::TyCon::new(name.text())))
                                })
                            });
                            (parameter.ownership(), formal)
                        })
                        .unzip()
                })
                .unwrap_or_default();
            let module_name = actor
                .syntax()
                .ancestors()
                .skip(1)
                .find_map(ModuleDef::cast)
                .and_then(|module| module.name())
                .and_then(|name| name.text());
            register_signature(
                &mut signatures,
                &mut ambiguous_bare_signatures,
                name,
                module_name,
                FunctionSignature {
                    modes,
                    formal_types,
                },
            );
        }
    }
    let destroy_signature = FunctionSignature {
        modes: vec![ParamOwnership::Consume],
        formal_types: vec![Some(Ty::secret_bytes())],
    };
    signatures.insert("Secret.destroy".to_string(), destroy_signature.clone());
    signatures.insert("secret_destroy".to_string(), destroy_signature);
    let concat_signature = FunctionSignature {
        modes: vec![ParamOwnership::Consume, ParamOwnership::Consume],
        formal_types: vec![Some(Ty::secret_bytes()), Some(Ty::secret_bytes())],
    };
    signatures.insert("Secret.concat".to_string(), concat_signature.clone());
    signatures.insert("secret_concat".to_string(), concat_signature);
    let bytes_builder = Ty::bytes_builder();
    for name in [
        "bytes_builder_write_u8",
        "bytes_builder_write_u16_be",
        "bytes_builder_write_u32_be",
        "bytes_builder_write_bytes",
    ] {
        let signature = FunctionSignature {
            modes: vec![ParamOwnership::Borrow, ParamOwnership::Move],
            formal_types: vec![Some(bytes_builder.clone()), None],
        };
        signatures.insert(name.to_string(), signature.clone());
        signatures.insert(
            format!("BytesBuilder.{}", name.trim_start_matches("bytes_builder_")),
            signature,
        );
    }
    let finish_builder = FunctionSignature {
        modes: vec![ParamOwnership::Consume],
        formal_types: vec![Some(bytes_builder)],
    };
    signatures.insert("BytesBuilder.finish".to_string(), finish_builder.clone());
    signatures.insert("bytes_builder_finish".to_string(), finish_builder);

    register_pg_signature(&mut signatures, "close", vec![ParamOwnership::Consume]);
    for (operation, arity) in [
        ("execute", 3),
        ("query", 3),
        ("execute_values", 3),
        ("query_values", 3),
        ("begin", 1),
        ("commit", 1),
        ("rollback", 1),
        ("transaction", 2),
        ("query_as", 4),
    ] {
        let mut modes = vec![ParamOwnership::Move; arity];
        modes[0] = ParamOwnership::Borrow;
        register_pg_signature(&mut signatures, operation, modes);
    }

    register_crypto_signature(
        &mut signatures,
        "hmac_sha256",
        vec![ParamOwnership::Borrow, ParamOwnership::Move],
        vec![Ty::secret_bytes(), Ty::bytes()],
    );
    register_crypto_signature(
        &mut signatures,
        "hkdf_sha256",
        vec![
            ParamOwnership::Borrow,
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Move,
        ],
        vec![Ty::secret_bytes(), Ty::bytes(), Ty::bytes(), Ty::int()],
    );
    register_crypto_signature(
        &mut signatures,
        "x25519_public",
        vec![ParamOwnership::Borrow],
        vec![Ty::x25519_private_key()],
    );
    register_crypto_signature(
        &mut signatures,
        "x25519_shared",
        vec![ParamOwnership::Borrow, ParamOwnership::Move],
        vec![Ty::x25519_private_key(), Ty::x25519_public_key()],
    );
    register_crypto_signature(
        &mut signatures,
        "sign",
        vec![ParamOwnership::Borrow, ParamOwnership::Move],
        vec![Ty::signing_private_key(), Ty::bytes()],
    );
    register_crypto_signature(
        &mut signatures,
        "aead_key",
        vec![ParamOwnership::Consume],
        vec![Ty::secret_bytes()],
    );
    for operation in ["aead_seal", "aead_open"] {
        register_crypto_signature(
            &mut signatures,
            operation,
            vec![
                ParamOwnership::Borrow,
                ParamOwnership::Move,
                ParamOwnership::Move,
                ParamOwnership::Move,
            ],
            vec![Ty::aead_key(), Ty::bytes(), Ty::bytes(), Ty::bytes()],
        );
    }
    register_imported_signatures(parse, import_ctx, &mut signatures);

    let mut checker = Checker {
        types,
        registry,
        scopes: vec![FxHashMap::default()],
        signatures,
        errors: Vec::new(),
    };

    checker.check_resource_parameter_patterns(parse);
    checker.check_resource_pattern_wildcards(parse);
    for binding in &top_level_bindings {
        checker.check_top_level_binding(binding);
    }
    for function in &functions {
        checker.check_function(function);
    }
    for actor in &actors {
        checker.check_actor(actor);
    }

    OwnershipCheck {
        function_ownership: checker
            .signatures
            .iter()
            .map(|(name, signature)| (name.clone(), signature.modes.clone()))
            .collect(),
        errors: checker.errors,
    }
}

fn register_imported_signatures(
    parse: &Parse,
    import_ctx: &ImportContext,
    signatures: &mut FxHashMap<String, FunctionSignature>,
) {
    for item in parse.tree().items() {
        match item {
            Item::ImportDecl(import) => {
                let Some(path) = import.module_path() else {
                    continue;
                };
                let Some(namespace) = path.segments().last().cloned() else {
                    continue;
                };
                let Some(exports) = import_ctx.module_exports.get(&namespace) else {
                    continue;
                };
                for export_name in exports.function_ownership.keys() {
                    let source_name = source_function_name(export_name);
                    let Some(signature) = exported_signature(exports, export_name) else {
                        continue;
                    };
                    signatures.insert(format!("{namespace}.{source_name}"), signature.clone());
                    // Cross-module lowering links public functions by their bare symbol.
                    signatures.entry(source_name).or_insert(signature);
                }
            }
            Item::FromImportDecl(import) => {
                let Some(path) = import.module_path() else {
                    continue;
                };
                let Some(namespace) = path.segments().last().cloned() else {
                    continue;
                };
                let Some(exports) = import_ctx.module_exports.get(&namespace) else {
                    continue;
                };
                let Some(imports) = import.import_list() else {
                    continue;
                };
                for imported in imports.names().filter_map(|name| name.text()) {
                    let mut matches = exports
                        .function_ownership
                        .keys()
                        .filter(|exported| source_function_name(exported) == imported);
                    let Some(export_name) = matches.next() else {
                        continue;
                    };
                    // ponytail: overloaded resource-call metadata needs an arity key;
                    // fail closed until an exported resource API actually overloads.
                    if matches.next().is_some() {
                        continue;
                    }
                    if let Some(signature) = exported_signature(exports, export_name) {
                        signatures.entry(imported).or_insert(signature);
                    }
                }
            }
            _ => {}
        }
    }
}

fn source_function_name(export_name: &str) -> String {
    export_name
        .rsplit_once("__")
        .filter(|(_, arity)| !arity.is_empty() && arity.chars().all(|c| c.is_ascii_digit()))
        .map(|(name, _)| name)
        .unwrap_or(export_name)
        .to_string()
}

fn exported_signature(exports: &ModuleExports, export_name: &str) -> Option<FunctionSignature> {
    let modes = exports.function_ownership.get(export_name)?.clone();
    let formal_types = match &exports.functions.get(export_name)?.ty {
        Ty::Fun(parameters, _) => parameters.iter().cloned().map(Some).collect(),
        _ => vec![None; modes.len()],
    };
    Some(FunctionSignature {
        modes,
        formal_types,
    })
}

fn register_crypto_signature(
    signatures: &mut FxHashMap<String, FunctionSignature>,
    name: &str,
    modes: Vec<ParamOwnership>,
    formal_types: Vec<Ty>,
) {
    let signature = FunctionSignature {
        modes,
        formal_types: formal_types.into_iter().map(Some).collect(),
    };
    signatures.insert(format!("Crypto.{name}"), signature.clone());
    signatures.insert(format!("crypto_{name}"), signature);
}

fn register_pg_signature(
    signatures: &mut FxHashMap<String, FunctionSignature>,
    name: &str,
    modes: Vec<ParamOwnership>,
) {
    let mut formal_types = vec![None; modes.len()];
    formal_types[0] = Some(Ty::Con(crate::ty::TyCon::new("PgConn")));
    let signature = FunctionSignature {
        modes,
        formal_types,
    };
    for alias in [
        format!("Pg.{name}"),
        format!("pg_{name}"),
        format!("mesh_pg_{name}"),
    ] {
        signatures.insert(alias, signature.clone());
    }
}

impl Checker<'_> {
    fn check_resource_parameter_patterns(&mut self, parse: &Parse) {
        for parameter in parse.syntax().descendants().filter_map(Param::cast) {
            let Some(pattern) = parameter.pattern() else {
                continue;
            };
            if !self.pattern_is_resource(&pattern) {
                continue;
            }

            // ponytail: synthesize branch-local drop scopes before permitting implicit resource discards.
            self.errors.push(TypeError::ResourceViolation {
                reason: "resource-bearing parameter patterns are unsupported".to_string(),
                span: pattern.syntax().text_range(),
            });
        }
    }

    fn check_resource_pattern_wildcards(&mut self, parse: &Parse) {
        for pattern in parse.syntax().descendants().filter_map(Pattern::cast) {
            if !matches!(pattern, Pattern::Wildcard(_)) {
                continue;
            }
            let belongs_to_rejected_parameter = pattern
                .syntax()
                .ancestors()
                .find_map(Param::cast)
                .and_then(|parameter| parameter.pattern())
                .is_some_and(|parameter_pattern| self.pattern_is_resource(&parameter_pattern));
            if !belongs_to_rejected_parameter && self.pattern_is_resource(&pattern) {
                self.errors.push(TypeError::ResourceViolation {
                    reason: "resource value cannot be discarded with `_` in a pattern".to_string(),
                    span: pattern.syntax().text_range(),
                });
            }
        }
    }

    fn pattern_is_resource(&self, pattern: &Pattern) -> bool {
        self.types
            .get(&pattern.syntax().text_range())
            .is_some_and(|ty| self.registry.is_resource_type(ty))
    }

    fn check_top_level_binding(&mut self, binding: &LetBinding) {
        let ty = binding
            .initializer()
            .and_then(|initializer| self.known_expr_type(&initializer))
            .or_else(|| {
                binding.type_annotation().and_then(|annotation| {
                    annotation
                        .type_name()
                        .map(|name| Ty::Con(crate::ty::TyCon::new(name.text())))
                })
            });
        if !ty
            .as_ref()
            .is_some_and(|ty| self.registry.is_resource_type(ty))
        {
            return;
        }
        let name = binding
            .name()
            .and_then(|name| name.text())
            .unwrap_or_else(|| "<binding>".to_string());
        self.errors.push(TypeError::ResourceViolation {
            reason: format!("resource-bearing top-level binding `{name}` is unsupported"),
            span: binding.syntax().text_range(),
        });
    }

    fn check_actor(&mut self, actor: &ActorDef) {
        self.scopes.push(FxHashMap::default());

        let parameter_types = match self.types.get(&actor.syntax().text_range()) {
            Some(Ty::Fun(parameters, _)) => parameters.clone(),
            _ => Vec::new(),
        };
        if let Some(parameters) = actor.param_list() {
            for (index, parameter) in parameters.params().enumerate() {
                let ty = parameter_types.get(index).cloned().or_else(|| {
                    parameter.type_annotation().and_then(|annotation| {
                        annotation
                            .type_name()
                            .map(|name| Ty::Con(crate::ty::TyCon::new(name.text())))
                    })
                });
                let Some(ty) = ty else {
                    continue;
                };
                if let Some(name) = parameter.name() {
                    self.insert_binding(
                        name.text().to_string(),
                        ty,
                        parameter.ownership() == ParamOwnership::Borrow,
                    );
                }
            }
        }

        if let Some(body) = actor.body() {
            self.check_block(&body);
        }

        self.scopes.pop();
    }

    fn check_function(&mut self, function: &FnDef) {
        self.scopes.push(FxHashMap::default());

        let (parameter_types, return_type) = match self.types.get(&function.syntax().text_range()) {
            Some(Ty::Fun(parameters, return_type)) => {
                (parameters.clone(), Some(return_type.as_ref().clone()))
            }
            _ => (Vec::new(), None),
        };
        if let Some(return_type) = return_type.filter(|_| function.return_type().is_some()) {
            if is_unrestricted_collection_type(&return_type)
                && self.registry.is_resource_type(&return_type)
            {
                self.errors.push(TypeError::ResourceViolation {
                    reason: format!(
                        "resource-bearing type `{return_type}` cannot be used as an unrestricted collection"
                    ),
                    span: function
                        .return_type()
                        .map(|annotation| annotation.syntax().text_range())
                        .unwrap_or_else(|| function.syntax().text_range()),
                });
            } else if is_unsupported_resource_wrapper(self.registry, &return_type) {
                self.errors.push(TypeError::ResourceViolation {
                    reason: unsupported_wrapper_reason(&return_type),
                    span: function
                        .return_type()
                        .map(|annotation| annotation.syntax().text_range())
                        .unwrap_or_else(|| function.syntax().text_range()),
                });
            }
        }
        if let Some(parameters) = function.param_list() {
            for (index, parameter) in parameters.params().enumerate() {
                let ty = parameter_types.get(index).cloned().or_else(|| {
                    parameter.type_annotation().and_then(|annotation| {
                        annotation
                            .type_name()
                            .map(|name| Ty::Con(crate::ty::TyCon::new(name.text())))
                    })
                });
                let Some(ty) = ty else {
                    continue;
                };
                if is_unrestricted_collection_type(&ty) && self.registry.is_resource_type(&ty) {
                    self.errors.push(TypeError::ResourceViolation {
                        reason: format!(
                            "resource-bearing type `{ty}` cannot be used as an unrestricted collection"
                        ),
                        span: parameter.syntax().text_range(),
                    });
                } else if is_unsupported_resource_wrapper(self.registry, &ty) {
                    self.errors.push(TypeError::ResourceViolation {
                        reason: unsupported_wrapper_reason(&ty),
                        span: parameter.syntax().text_range(),
                    });
                }
                if let Some(name) = parameter.name() {
                    self.insert_binding(
                        name.text().to_string(),
                        ty,
                        parameter.ownership() == ParamOwnership::Borrow,
                    );
                }
            }
        }

        if let Some(body) = function.body() {
            self.check_block(&body);
        } else if let Some(body) = function.expr_body() {
            self.check_expr(&body, Usage::Move);
        }

        self.scopes.pop();
    }

    fn check_block(&mut self, block: &Block) {
        self.scopes.push(FxHashMap::default());
        for child in block.syntax().children() {
            if let Some(item) = Item::cast(child.clone()) {
                if let Item::LetBinding(binding) = item {
                    self.check_let(&binding);
                }
            } else if let Some(expr) = Expr::cast(child) {
                self.check_expr(&expr, Usage::Move);
            }
        }
        self.scopes.pop();
    }

    fn check_let(&mut self, binding: &LetBinding) {
        let Some(initializer) = binding.initializer() else {
            return;
        };
        let initializer_ty = self.known_expr_type(&initializer);
        if let Some(ty) = &initializer_ty {
            if is_unrestricted_collection_type(ty)
                && self.registry.is_resource_type(ty)
                && (!matches!(initializer, Expr::ListLiteral(_) | Expr::MapLiteral(_))
                    || binding.type_annotation().is_some())
            {
                self.errors.push(TypeError::ResourceViolation {
                    reason: format!(
                        "resource-bearing type `{ty}` cannot be used as an unrestricted collection"
                    ),
                    span: binding.syntax().text_range(),
                });
            } else if is_unsupported_resource_wrapper(self.registry, ty) {
                self.errors.push(TypeError::ResourceViolation {
                    reason: unsupported_wrapper_reason(ty),
                    span: binding.syntax().text_range(),
                });
            }
        }
        let usage = if initializer_ty
            .as_ref()
            .is_some_and(|ty| self.registry.is_resource_type(ty))
        {
            Usage::Move
        } else {
            Usage::Read
        };
        self.check_expr(&initializer, usage);

        if let Some(pattern) = binding.pattern() {
            self.bind_pattern(&pattern);
        } else if let (Some(name), Some(ty)) =
            (binding.name().and_then(|name| name.text()), initializer_ty)
        {
            self.insert(name, ty);
        }
    }

    fn check_expr(&mut self, expr: &Expr, usage: Usage) {
        match expr {
            Expr::NameRef(name) => self.check_name(name, usage),
            Expr::CallExpr(call) => self.check_call(call),
            Expr::StructLiteral(literal) => self.check_struct_literal(literal),
            Expr::StructUpdate(update) => self.check_struct_update(update),
            Expr::IfExpr(if_expr) => self.check_if(if_expr),
            Expr::CaseExpr(case_expr) => self.check_case(case_expr),
            Expr::ReceiveExpr(receive_expr) => self.check_receive(receive_expr),
            Expr::WhileExpr(while_expr) => self.check_while(while_expr),
            Expr::ForInExpr(for_expr) => self.check_for(for_expr),
            Expr::FieldAccess(access) => {
                if let Some(base) = access.base() {
                    let base_usage = if matches!(usage, Usage::Move)
                        && self
                            .types
                            .get(&access.syntax().text_range())
                            .is_some_and(|ty| self.registry.is_resource_type(ty))
                    {
                        Usage::Move
                    } else {
                        Usage::Read
                    };
                    self.check_expr(&base, base_usage);
                }
            }
            Expr::StringExpr(string) => {
                for interpolation in string
                    .syntax()
                    .children()
                    .filter(|node| node.kind() == SyntaxKind::INTERPOLATION)
                {
                    for inner in interpolation.children().filter_map(Expr::cast) {
                        if self.expr_is_resource(&inner) {
                            self.errors.push(TypeError::ResourceViolation {
                                reason: format!(
                                    "resource `{}` cannot be interpolated or formatted",
                                    self.expr_label(&inner)
                                ),
                                span: inner.syntax().text_range(),
                            });
                        }
                        self.check_expr(&inner, Usage::Read);
                    }
                }
            }
            Expr::BinaryExpr(binary) => {
                let lhs = binary.lhs();
                let rhs = binary.rhs();
                let is_comparison = binary.op().is_some_and(|operator| {
                    matches!(
                        operator.kind(),
                        SyntaxKind::EQ_EQ
                            | SyntaxKind::NOT_EQ
                            | SyntaxKind::LT
                            | SyntaxKind::LT_EQ
                            | SyntaxKind::GT
                            | SyntaxKind::GT_EQ
                    )
                });
                if is_comparison {
                    if let Some(resource) = lhs
                        .as_ref()
                        .filter(|operand| self.expr_is_resource(operand))
                        .or_else(|| {
                            rhs.as_ref()
                                .filter(|operand| self.expr_is_resource(operand))
                        })
                    {
                        self.errors.push(TypeError::ResourceViolation {
                            reason: format!(
                                "resource `{}` cannot be compared or hashed",
                                self.expr_label(resource)
                            ),
                            span: binary.syntax().text_range(),
                        });
                    }
                }
                if let Some(lhs) = lhs {
                    self.check_expr(&lhs, Usage::Read);
                }
                if let Some(rhs) = rhs {
                    self.check_expr(&rhs, Usage::Read);
                }
            }
            Expr::SendExpr(send) => {
                if let Some(arguments) = send.arg_list() {
                    for (index, argument) in arguments.args().enumerate() {
                        if index == 1 && self.expr_is_resource(&argument) {
                            self.errors.push(TypeError::ResourceViolation {
                                reason: format!(
                                    "resource `{}` cannot cross an actor mailbox boundary",
                                    self.expr_label(&argument)
                                ),
                                span: argument.syntax().text_range(),
                            });
                        }
                        self.check_expr(&argument, Usage::Read);
                    }
                }
            }
            Expr::SpawnExpr(spawn) => {
                if let Some(arguments) = spawn.arg_list() {
                    for (index, argument) in arguments.args().enumerate() {
                        if index > 0 && self.expr_is_resource(&argument) {
                            self.errors.push(TypeError::ResourceViolation {
                                reason: format!(
                                    "resource `{}` cannot be transferred into a spawned actor",
                                    self.expr_label(&argument)
                                ),
                                span: argument.syntax().text_range(),
                            });
                        }
                        self.check_expr(&argument, Usage::Read);
                    }
                }
            }
            Expr::ListLiteral(list) => {
                for element in list.elements() {
                    if self.expr_is_resource(&element) {
                        self.errors.push(TypeError::ResourceViolation {
                            reason: format!(
                                "resource `{}` cannot enter an unrestricted collection",
                                self.expr_label(&element)
                            ),
                            span: element.syntax().text_range(),
                        });
                    }
                    self.check_expr(&element, Usage::Read);
                }
            }
            Expr::MapLiteral(map) => {
                for entry in map.entries() {
                    let key = (!entry.is_keyword_entry()).then(|| entry.key()).flatten();
                    for element in key.into_iter().chain(entry.value()) {
                        if self.expr_is_resource(&element) {
                            self.errors.push(TypeError::ResourceViolation {
                                reason: format!(
                                    "resource `{}` cannot enter an unrestricted collection",
                                    self.expr_label(&element)
                                ),
                                span: element.syntax().text_range(),
                            });
                        }
                        self.check_expr(&element, Usage::Read);
                    }
                }
            }
            Expr::JsonExpr(json) => {
                for value in json.fields().filter_map(|field| field.value()) {
                    if self.expr_is_resource(&value) {
                        self.errors.push(TypeError::ResourceViolation {
                            reason: format!(
                                "resource `{}` cannot cross JSON or serialization boundaries",
                                self.expr_label(&value)
                            ),
                            span: value.syntax().text_range(),
                        });
                    }
                    self.check_expr(&value, Usage::Read);
                }
            }
            Expr::ClosureExpr(closure) => self.check_closure(closure),
            Expr::Block(block) => self.check_block(block),
            Expr::TupleExpr(tuple) => {
                for element in tuple.elements() {
                    let usage = if self.expr_is_resource(&element) {
                        Usage::Move
                    } else {
                        Usage::Read
                    };
                    self.check_expr(&element, usage);
                }
            }
            Expr::ReturnExpr(return_expr) => {
                if let Some(value) = return_expr.value() {
                    let usage = if self.expr_is_resource(&value) {
                        Usage::Move
                    } else {
                        Usage::Read
                    };
                    self.check_expr(&value, usage);
                }
            }
            Expr::TryExpr(try_expr) => {
                if let Some(operand) = try_expr.operand() {
                    let usage = if self.expr_is_resource(&operand) {
                        Usage::Move
                    } else {
                        Usage::Read
                    };
                    self.check_expr(&operand, usage);
                }
            }
            _ => {
                for child in expr.syntax().children() {
                    if let Some(child_expr) = Expr::cast(child) {
                        self.check_expr(&child_expr, Usage::Read);
                    }
                }
            }
        }
    }

    fn check_call(&mut self, call: &CallExpr) {
        let callee = call.callee();
        let callee_name = callee.as_ref().and_then(direct_callee_name);
        let transaction_api = match callee_name.as_deref() {
            Some("Pg.transaction" | "pg_transaction") => Some("Pg.transaction"),
            Some("Repo.transaction" | "repo_transaction") => Some("Repo.transaction"),
            _ => None,
        };
        if let Some(transaction_api) = transaction_api {
            let callback = call
                .arg_list()
                .and_then(|arguments| arguments.args().nth(1));
            if let Some(callback) = callback {
                let borrows_connection = match &callback {
                    Expr::ClosureExpr(closure) => closure
                        .param_list()
                        .and_then(|parameters| parameters.params().next())
                        .is_some_and(|parameter| parameter.ownership() == ParamOwnership::Borrow),
                    callback => direct_callee_name(callback)
                        .and_then(|name| self.signatures.get(&name))
                        .and_then(|signature| signature.modes.first())
                        .is_some_and(|mode| *mode == ParamOwnership::Borrow),
                };
                if !borrows_connection {
                    self.errors.push(TypeError::ResourceViolation {
                        reason: format!(
                            "{transaction_api} callback must borrow its PgConn parameter"
                        ),
                        span: callback.syntax().text_range(),
                    });
                }
            }
        }
        if let Some(Expr::FieldAccess(access)) = &callee {
            if let Some(base) = access.base().filter(|base| self.expr_is_resource(base)) {
                let field = access
                    .field()
                    .map(|field| field.text().to_ascii_lowercase())
                    .unwrap_or_default();
                let reason = match field.as_str() {
                    "hash" | "eq" | "lt" => Some("cannot be compared or hashed"),
                    "inspect" | "to_string" | "format" => {
                        Some("cannot be interpolated or formatted")
                    }
                    "to_json" | "serialize" => {
                        Some("cannot cross JSON or serialization boundaries")
                    }
                    _ => None,
                };
                if let Some(reason) = reason {
                    self.errors.push(TypeError::ResourceViolation {
                        reason: format!("resource `{}` {reason}", self.expr_label(&base)),
                        span: access.syntax().text_range(),
                    });
                }
            }
        }
        if let Some(callee) = &callee {
            self.check_expr(callee, Usage::Read);
        }
        let signature = callee_name
            .as_ref()
            .and_then(|name| self.signatures.get(name).cloned());
        let forbidden_reason = signature
            .is_none()
            .then(|| callee_name.as_deref().and_then(forbidden_call_reason))
            .flatten();
        let allowed_resource_constructor = callee_name.as_deref().is_some_and(|callee| {
            self.types
                .get(&call.syntax().text_range())
                .is_some_and(|ty| is_resource_sum_constructor(self.registry, ty, callee))
        });
        if let Some(arguments) = call.arg_list() {
            for (index, argument) in arguments.args().enumerate() {
                let is_resource = self.expr_is_resource(&argument);
                if is_resource {
                    if let Some(reason) = forbidden_reason {
                        let is_existing_collection = reason
                            == "cannot enter an unrestricted collection"
                            && self
                                .known_expr_type(&argument)
                                .as_ref()
                                .is_some_and(is_unrestricted_collection_type);
                        if !is_existing_collection {
                            self.errors.push(TypeError::ResourceViolation {
                                reason: format!(
                                    "resource `{}` {reason}",
                                    self.expr_label(&argument)
                                ),
                                span: argument.syntax().text_range(),
                            });
                        }
                    }
                }
                let formal_is_resource = signature
                    .as_ref()
                    .and_then(|signature| signature.formal_types.get(index))
                    .and_then(Option::as_ref)
                    .is_some_and(|formal| self.registry.is_resource_type(formal));
                let lacks_resource_aware_formal = is_resource
                    && forbidden_reason.is_none()
                    && !formal_is_resource
                    && !allowed_resource_constructor;
                if lacks_resource_aware_formal {
                    self.errors.push(TypeError::ResourceViolation {
                        reason: format!(
                            "resource `{}` cannot be passed through generic or indirect call `{}`",
                            self.expr_label(&argument),
                            callee_name.as_deref().unwrap_or("<indirect>")
                        ),
                        span: argument.syntax().text_range(),
                    });
                }
                let usage = if forbidden_reason.is_some() || lacks_resource_aware_formal {
                    Usage::Read
                } else {
                    match signature
                        .as_ref()
                        .and_then(|signature| signature.modes.get(index))
                    {
                        Some(ParamOwnership::Move | ParamOwnership::Consume) => Usage::Move,
                        Some(ParamOwnership::Borrow) => Usage::Read,
                        None if allowed_resource_constructor && is_resource => Usage::Move,
                        None if is_resource => Usage::Move,
                        None => Usage::Read,
                    }
                };
                self.check_expr(&argument, usage);
            }
        }
    }

    fn check_struct_literal(&mut self, literal: &StructLiteral) {
        for field in literal.fields() {
            if let Some(value) = field.value() {
                let usage = if self
                    .types
                    .get(&value.syntax().text_range())
                    .is_some_and(|ty| self.registry.is_resource_type(ty))
                {
                    Usage::Move
                } else {
                    Usage::Read
                };
                self.check_expr(&value, usage);
            }
        }
    }

    fn check_struct_update(&mut self, update: &StructUpdate) {
        if let Some(base) = update.base_expr() {
            let usage = if self.expr_is_resource(&base) {
                Usage::Move
            } else {
                Usage::Read
            };
            self.check_expr(&base, usage);
        }
        for field in update.override_fields() {
            if let Some(value) = field.value() {
                let usage = if self.expr_is_resource(&value) {
                    Usage::Move
                } else {
                    Usage::Read
                };
                self.check_expr(&value, usage);
            }
        }
    }

    fn check_closure(&mut self, closure: &ClosureExpr) {
        // ponytail: closure types do not carry affine environment metadata yet;
        // reject resource captures until closure values can move and drop that environment.
        let mut local_names = FxHashSet::default();
        if let Some(parameters) = closure.param_list() {
            for parameter in parameters.params() {
                if let Some(name) = parameter.name() {
                    local_names.insert(name.text().to_string());
                }
            }
        }
        for binding in closure.syntax().descendants().filter_map(LetBinding::cast) {
            if let Some(name) = binding.name().and_then(|name| name.text()) {
                local_names.insert(name);
            }
        }

        let mut reported = FxHashSet::default();
        for name_ref in closure.syntax().descendants().filter_map(NameRef::cast) {
            let Some(name) = name_ref.text() else {
                continue;
            };
            if local_names.contains(&name) || reported.contains(&name) {
                continue;
            }
            let is_outer_resource = self
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(&name))
                .is_some_and(|binding| self.registry.is_resource_type(&binding.ty));
            if is_outer_resource {
                reported.insert(name.clone());
                self.errors.push(TypeError::ResourceViolation {
                    reason: format!("resource `{name}` cannot be captured by a closure"),
                    span: name_ref.syntax().text_range(),
                });
            }
        }

        if reported.is_empty() {
            if let Some(body) = closure.body() {
                self.check_block(&body);
            }
        }
    }

    fn check_if(&mut self, if_expr: &IfExpr) {
        if let Some(condition) = if_expr.condition() {
            self.check_expr(&condition, Usage::Read);
        }

        let before_branches = self.scopes.clone();
        self.scopes = before_branches.clone();
        if let Some(then_branch) = if_expr.then_branch() {
            self.check_block(&then_branch);
        }
        let then_scopes = self.scopes.clone();

        self.scopes = before_branches.clone();
        if let Some(else_branch) = if_expr.else_branch() {
            if let Some(block) = else_branch.block() {
                self.check_block(&block);
            } else if let Some(nested) = else_branch.if_expr() {
                self.check_if(&nested);
            }
        }
        let else_scopes = self.scopes.clone();

        self.scopes = before_branches;
        self.merge_branch_states(&[then_scopes, else_scopes]);
    }

    fn check_case(&mut self, case_expr: &CaseExpr) {
        if let Some(scrutinee) = case_expr.scrutinee() {
            let usage = if self
                .types
                .get(&scrutinee.syntax().text_range())
                .is_some_and(|ty| self.registry.is_resource_type(ty))
            {
                Usage::Move
            } else {
                Usage::Read
            };
            self.check_expr(&scrutinee, usage);
        }

        let before_arms = self.scopes.clone();
        let mut arm_states = Vec::new();
        for arm in case_expr.arms() {
            self.scopes = before_arms.clone();
            self.scopes.push(FxHashMap::default());
            let pattern = arm.pattern();
            if let Some(pattern) = &pattern {
                self.bind_pattern(pattern);
            }
            let guard = arm.guard();
            let has_guard = guard.is_some();
            if has_guard {
                if let Some(pattern) = &pattern {
                    self.check_unconsumed_pattern_resources(pattern);
                }
            }
            if let Some(guard) = guard {
                self.check_expr(&guard, Usage::Read);
            }
            if let Some(body) = arm.body() {
                self.check_expr(&body, Usage::Move);
            }
            if !has_guard {
                if let Some(pattern) = &pattern {
                    self.check_unconsumed_pattern_resources(pattern);
                }
            }
            self.scopes.pop();
            arm_states.push(self.scopes.clone());
        }

        self.scopes = before_arms;
        self.merge_branch_states(&arm_states);
    }

    fn check_receive(&mut self, receive_expr: &ReceiveExpr) {
        let before_arms = self.scopes.clone();
        let mut arm_states = Vec::new();

        for arm in receive_expr.arms() {
            self.scopes = before_arms.clone();
            self.scopes.push(FxHashMap::default());
            let pattern = arm.pattern();
            if let Some(pattern) = &pattern {
                self.bind_pattern(pattern);
            }
            let guard = arm
                .syntax()
                .children_with_tokens()
                .any(|element| element.kind() == SyntaxKind::WHEN_KW)
                .then(|| arm.syntax().children().find_map(Expr::cast))
                .flatten();
            let has_guard = guard.is_some();
            if has_guard {
                if let Some(pattern) = &pattern {
                    self.check_unconsumed_pattern_resources(pattern);
                }
            }
            if let Some(guard) = guard {
                self.check_expr(&guard, Usage::Read);
            }
            if let Some(body) = arm.body() {
                self.check_expr(&body, Usage::Move);
            }
            if !has_guard {
                if let Some(pattern) = &pattern {
                    self.check_unconsumed_pattern_resources(pattern);
                }
            }
            self.scopes.pop();
            arm_states.push(self.scopes.clone());
        }

        if let Some(after) = receive_expr.after_clause() {
            self.scopes = before_arms.clone();
            if let Some(timeout) = after.timeout() {
                self.check_expr(&timeout, Usage::Read);
            }
            if let Some(body) = after.body() {
                self.check_expr(&body, Usage::Move);
            }
            arm_states.push(self.scopes.clone());
        }

        self.scopes = before_arms;
        self.merge_branch_states(&arm_states);
    }

    fn check_while(&mut self, while_expr: &mesh_parser::ast::expr::WhileExpr) {
        if let Some(condition) = while_expr.condition() {
            self.check_expr(&condition, Usage::Read);
        }

        let before_body = self.scopes.clone();
        if let Some(body) = while_expr.body() {
            self.check_block(&body);
        }
        let after_body = self.scopes.clone();

        for (before_scope, after_scope) in before_body.iter().zip(&after_body) {
            for (name, before) in before_scope {
                if !before.moved
                    && after_scope.get(name).is_some_and(|after| after.moved)
                    && self.registry.is_resource_type(&before.ty)
                {
                    self.errors.push(TypeError::ResourceViolation {
                        reason: format!(
                            "resource `{name}` may be moved more than once by this loop"
                        ),
                        span: while_expr.syntax().text_range(),
                    });
                }
            }
        }

        self.scopes = before_body;
        self.merge_moved_states(&after_body);
    }

    fn check_for(&mut self, for_expr: &mesh_parser::ast::expr::ForInExpr) {
        if let Some(iterable) = for_expr.iterable() {
            self.check_expr(&iterable, Usage::Read);
        }
        if let Some(filter) = for_expr.filter() {
            self.check_expr(&filter, Usage::Read);
        }

        let before_body = self.scopes.clone();
        if let Some(body) = for_expr.body() {
            self.check_block(&body);
        }
        let after_body = self.scopes.clone();

        for (before_scope, after_scope) in before_body.iter().zip(&after_body) {
            for (name, before) in before_scope {
                if !before.moved
                    && after_scope.get(name).is_some_and(|after| after.moved)
                    && self.registry.is_resource_type(&before.ty)
                {
                    self.errors.push(TypeError::ResourceViolation {
                        reason: format!(
                            "resource `{name}` may be moved more than once by this loop"
                        ),
                        span: for_expr.syntax().text_range(),
                    });
                }
            }
        }

        self.scopes = before_body;
        self.merge_moved_states(&after_body);
    }

    fn bind_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Ident(identifier) => {
                if let (Some(name), Some(ty)) = (
                    identifier.name(),
                    self.types.get(&pattern.syntax().text_range()).cloned(),
                ) {
                    let text = name.text().to_string();
                    if !text.starts_with(|character: char| character.is_uppercase()) {
                        self.insert(text, ty);
                    }
                }
            }
            Pattern::Tuple(tuple) => {
                for child in tuple.patterns() {
                    self.bind_pattern(&child);
                }
            }
            Pattern::Constructor(constructor) => {
                for field in constructor.fields() {
                    self.bind_pattern(&field);
                }
            }
            Pattern::Or(or_pattern) => {
                if let Some(first) = or_pattern.alternatives().next() {
                    self.bind_pattern(&first);
                }
            }
            Pattern::As(as_pattern) => {
                if let Some(inner) = as_pattern.pattern() {
                    self.bind_pattern(&inner);
                }
                if let (Some(name), Some(ty)) = (
                    as_pattern.binding_name(),
                    self.types.get(&pattern.syntax().text_range()).cloned(),
                ) {
                    self.insert(name.text().to_string(), ty);
                }
            }
            Pattern::Cons(cons_pattern) => {
                if let Some(head) = cons_pattern.head() {
                    self.bind_pattern(&head);
                }
                if let Some(tail) = cons_pattern.tail() {
                    self.bind_pattern(&tail);
                }
            }
            Pattern::Wildcard(_) | Pattern::Literal(_) => {}
        }
    }

    fn check_unconsumed_pattern_resources(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Ident(identifier) => {
                let Some(name) = identifier.name().map(|name| name.text().to_string()) else {
                    return;
                };
                if name.starts_with(|character: char| character.is_uppercase()) {
                    return;
                }
                self.check_unconsumed_resource_binding(&name, identifier.syntax().text_range());
            }
            Pattern::Tuple(tuple) => {
                for child in tuple.patterns() {
                    self.check_unconsumed_pattern_resources(&child);
                }
            }
            Pattern::Constructor(constructor) => {
                for field in constructor.fields() {
                    self.check_unconsumed_pattern_resources(&field);
                }
            }
            Pattern::Or(or_pattern) => {
                if let Some(first) = or_pattern.alternatives().next() {
                    self.check_unconsumed_pattern_resources(&first);
                }
            }
            Pattern::As(as_pattern) => {
                if let Some(inner) = as_pattern.pattern() {
                    self.check_unconsumed_pattern_resources(&inner);
                }
                if let Some(binding) = as_pattern.binding_name() {
                    self.check_unconsumed_resource_binding(binding.text(), binding.text_range());
                }
            }
            Pattern::Cons(cons_pattern) => {
                if let Some(head) = cons_pattern.head() {
                    self.check_unconsumed_pattern_resources(&head);
                }
                if let Some(tail) = cons_pattern.tail() {
                    self.check_unconsumed_pattern_resources(&tail);
                }
            }
            Pattern::Wildcard(_) | Pattern::Literal(_) => {}
        }
    }

    fn check_unconsumed_resource_binding(&mut self, name: &str, span: TextRange) {
        let is_unconsumed_resource = self
            .scopes
            .last()
            .and_then(|scope| scope.get(name))
            .is_some_and(|binding| {
                self.registry.is_resource_type(&binding.ty) && !binding.definitely_moved
            });
        if is_unconsumed_resource {
            self.errors.push(TypeError::ResourceViolation {
                reason: format!("resource pattern binding `{name}` must be consumed in this arm"),
                span,
            });
        }
    }

    fn merge_moved_states(&mut self, branch: &[FxHashMap<String, Binding>]) {
        for (scope, branch_scope) in self.scopes.iter_mut().zip(branch) {
            for (name, binding) in scope {
                if branch_scope.get(name).is_some_and(|state| state.moved) {
                    binding.moved = true;
                }
            }
        }
    }

    fn merge_branch_states(&mut self, branches: &[Vec<FxHashMap<String, Binding>>]) {
        if branches.is_empty() {
            return;
        }

        for (scope_index, scope) in self.scopes.iter_mut().enumerate() {
            for (name, binding) in scope {
                binding.moved |= branches.iter().any(|branch| {
                    branch
                        .get(scope_index)
                        .and_then(|scope| scope.get(name))
                        .is_some_and(|state| state.moved)
                });
                binding.definitely_moved = branches.iter().all(|branch| {
                    branch
                        .get(scope_index)
                        .and_then(|scope| scope.get(name))
                        .is_some_and(|state| state.definitely_moved)
                });
            }
        }
    }

    fn check_name(&mut self, name_ref: &NameRef, usage: Usage) {
        let Some(name) = name_ref.text() else {
            return;
        };
        let is_resource = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&name))
            .is_some_and(|binding| self.registry.is_resource_type(&binding.ty));
        if !is_resource {
            return;
        }
        let Some(binding) = self.lookup_mut(&name) else {
            return;
        };
        if binding.moved {
            self.errors.push(TypeError::ResourceViolation {
                reason: format!("resource `{name}` was used after it moved"),
                span: name_ref.syntax().text_range(),
            });
        } else if matches!(usage, Usage::Move) {
            if binding.borrowed {
                self.errors.push(TypeError::ResourceViolation {
                    reason: format!("borrowed resource `{name}` cannot be moved"),
                    span: name_ref.syntax().text_range(),
                });
            } else {
                binding.moved = true;
                binding.definitely_moved = true;
            }
        }
    }

    fn insert(&mut self, name: String, ty: Ty) {
        self.insert_binding(name, ty, false);
    }

    fn insert_binding(&mut self, name: String, ty: Ty, borrowed: bool) {
        self.scopes
            .last_mut()
            .expect("ownership checker always has a scope")
            .insert(
                name,
                Binding {
                    ty,
                    moved: false,
                    definitely_moved: false,
                    borrowed,
                },
            );
    }

    fn lookup_mut(&mut self, name: &str) -> Option<&mut Binding> {
        self.scopes
            .iter_mut()
            .rev()
            .find_map(|scope| scope.get_mut(name))
    }

    fn known_expr_type(&self, expr: &Expr) -> Option<Ty> {
        self.types
            .get(&expr.syntax().text_range())
            .cloned()
            .or_else(|| match expr {
                Expr::NameRef(name_ref) => name_ref.text().and_then(|name| {
                    self.scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(&name))
                        .map(|binding| binding.ty.clone())
                }),
                _ => None,
            })
    }

    fn expr_is_resource(&self, expr: &Expr) -> bool {
        self.known_expr_type(expr)
            .as_ref()
            .is_some_and(|ty| self.registry.is_resource_type(ty))
    }

    fn expr_label(&self, expr: &Expr) -> String {
        if let Expr::NameRef(name) = expr {
            if let Some(text) = name.text() {
                return text;
            }
        }
        self.known_expr_type(expr)
            .map(|ty| ty.to_string())
            .unwrap_or_else(|| "value".to_string())
    }
}

fn direct_callee_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::NameRef(name) => name.text(),
        Expr::FieldAccess(access) => {
            let base = access.base().and_then(|base| direct_callee_name(&base))?;
            let field = access.field()?.text().to_string();
            Some(format!("{base}.{field}"))
        }
        _ => None,
    }
}

fn register_signature(
    signatures: &mut FxHashMap<String, FunctionSignature>,
    ambiguous_bare_signatures: &mut FxHashSet<String>,
    bare_name: String,
    module_name: Option<String>,
    signature: FunctionSignature,
) {
    if let Some(module_name) = module_name {
        signatures.insert(format!("{module_name}.{bare_name}"), signature.clone());
    }

    if ambiguous_bare_signatures.contains(&bare_name) {
        return;
    }
    if signatures.insert(bare_name.clone(), signature).is_some() {
        signatures.remove(&bare_name);
        ambiguous_bare_signatures.insert(bare_name);
    }
}

fn forbidden_call_reason(callee: &str) -> Option<&'static str> {
    let lower = callee.to_ascii_lowercase();
    if lower == "json.encode"
        || lower == "json.serialize"
        || lower == "to_json"
        || lower.ends_with(".to_json")
        || lower.ends_with(".serialize")
    {
        Some("cannot cross JSON or serialization boundaries")
    } else if lower.starts_with("list.") || lower.starts_with("map.") || lower.starts_with("set.") {
        Some("cannot enter an unrestricted collection")
    } else if matches!(
        lower.as_str(),
        "print" | "println" | "inspect" | "to_string" | "format"
    ) || [".print", ".println", ".inspect", ".to_string", ".format"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        Some("cannot be interpolated or formatted")
    } else {
        None
    }
}

fn is_unrestricted_collection_type(ty: &Ty) -> bool {
    match ty {
        Ty::Con(constructor) => matches!(constructor.name.as_str(), "List" | "Map" | "Set"),
        Ty::App(constructor, _) => matches!(
            constructor.as_ref(),
            Ty::Con(constructor) if matches!(constructor.name.as_str(), "List" | "Map" | "Set")
        ),
        _ => false,
    }
}

fn is_resource_sum_constructor(registry: &TypeRegistry, ty: &Ty, callee: &str) -> bool {
    if !registry.is_resource_type(ty) {
        return false;
    }
    let type_name = match ty {
        Ty::Con(constructor) => constructor.name.as_str(),
        Ty::App(constructor, _) => match constructor.as_ref() {
            Ty::Con(constructor) => constructor.name.as_str(),
            _ => return false,
        },
        _ => return false,
    };
    let variant_name = callee.rsplit('.').next().unwrap_or(callee);
    registry
        .sum_type_defs
        .get(type_name)
        .is_some_and(|definition| {
            definition
                .variants
                .iter()
                .any(|variant| variant.name == variant_name)
        })
}

fn is_unsupported_resource_wrapper(registry: &TypeRegistry, ty: &Ty) -> bool {
    let Ty::App(constructor, arguments) = ty else {
        return false;
    };
    let Ty::Con(constructor) = constructor.as_ref() else {
        return false;
    };

    arguments
        .iter()
        .any(|argument| registry.is_resource_type(argument))
        && !matches!(
            constructor.name.as_str(),
            "List" | "Map" | "Set" | "Pid" | "Option" | "Result"
        )
        && !registry.is_resource_name(&constructor.name)
        && !registry.struct_defs.contains_key(&constructor.name)
        && !registry.sum_type_defs.contains_key(&constructor.name)
}

fn unsupported_wrapper_reason(ty: &Ty) -> String {
    format!("resource-bearing wrapper `{ty}` has no registered resource destructor")
}
