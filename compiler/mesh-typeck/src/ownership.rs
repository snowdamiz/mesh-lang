//! Flow-sensitive validation for affine resource values.

use mesh_parser::ast::expr::{
    CallExpr, CaseExpr, ClosureExpr, Expr, IfExpr, NameRef, ReceiveExpr, StructLiteral,
    StructUpdate,
};
use mesh_parser::ast::item::{
    ActorDef, Block, FnDef, ImplDef, Item, LetBinding, ModuleDef, Param, ParamList, ParamOwnership,
    TypeAnnotation,
};
use mesh_parser::ast::pat::Pattern;
use mesh_parser::ast::AstNode;
use mesh_parser::{Parse, SyntaxKind, SyntaxNode};
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
    /// The arity (`name__N`) the type checker chose for each call to a
    /// name defined at several.
    call_targets: &'a FxHashMap<TextRange, String>,
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
    overloaded_fn_names: &FxHashSet<String>,
    call_targets: &FxHashMap<TextRange, String>,
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
                    SyntaxKind::FN_DEF
                        | SyntaxKind::ACTOR_DEF
                        | SyntaxKind::CLOSURE_EXPR
                        | SyntaxKind::TRAILING_CLOSURE
                )
            })
        })
        .collect();
    let mut signatures = FxHashMap::default();
    let mut ambiguous_bare_signatures = FxHashSet::default();
    for function in functions
        .iter()
        .filter(|function| !is_later_clause(function))
    {
        let Some(mut name) = function.name().and_then(|name| name.text()) else {
            continue;
        };
        // Each arity of a top-level name defined at several is its own
        // function, `name__N`, as the type checker and lowering name it.
        let top_level = function
            .syntax()
            .parent()
            .is_some_and(|parent| parent.kind() == SyntaxKind::SOURCE_FILE);
        if top_level && overloaded_fn_names.contains(&name) {
            name = format!("{name}__{}", fn_arity(function));
        }
        let signature = signature_of(types, function.syntax(), function.param_list());
        // A method is also called through its type or its interface
        // (`Session.close(s)`, `Closer.close(s)`); two impls defining it
        // make that name as ambiguous as a bare name defined twice.
        if let Some(impl_def) = function.syntax().ancestors().find_map(ImplDef::cast) {
            for owner in [impl_def.type_name(), impl_def.interface_name()]
                .into_iter()
                .flatten()
            {
                register_signature(
                    &mut signatures,
                    &mut ambiguous_bare_signatures,
                    format!("{}.{name}", owner.text()),
                    None,
                    signature.clone(),
                );
            }
        }
        let module_name = enclosing_module(function.syntax());
        register_signature(
            &mut signatures,
            &mut ambiguous_bare_signatures,
            name,
            module_name,
            signature,
        );
    }
    for actor in &actors {
        let Some(name) = actor.name().and_then(|name| name.text()) else {
            continue;
        };
        register_signature(
            &mut signatures,
            &mut ambiguous_bare_signatures,
            name,
            enclosing_module(actor.syntax()),
            signature_of(types, actor.syntax(), actor.param_list()),
        );
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
    let secret_map = Ty::secret_map();
    for name in ["insert", "contains", "copy", "delete", "fork"] {
        let arity = if name == "insert" {
            3
        } else if name == "fork" {
            1
        } else {
            2
        };
        let mut modes = vec![ParamOwnership::Borrow; arity];
        let mut formal_types = vec![None; arity];
        formal_types[0] = Some(secret_map.clone());
        if name == "insert" {
            modes[2] = ParamOwnership::Consume;
            formal_types[2] = Some(Ty::secret_bytes());
        }
        let signature = FunctionSignature {
            modes,
            formal_types,
        };
        signatures.insert(format!("SecretMap.{name}"), signature.clone());
        signatures.insert(format!("secret_map_{name}"), signature);
    }
    let merge_signature = FunctionSignature {
        modes: vec![ParamOwnership::Borrow, ParamOwnership::Consume],
        formal_types: vec![Some(secret_map.clone()), Some(secret_map)],
    };
    signatures.insert("SecretMap.merge".to_string(), merge_signature.clone());
    signatures.insert("secret_map_merge".to_string(), merge_signature);
    for (module, prefix, resource) in [
        ("Secret", "secret", Ty::secret_bytes()),
        ("SecretMap", "secret_map", Ty::secret_map()),
        (
            "X25519PrivateKey",
            "x25519_private_key",
            Ty::x25519_private_key(),
        ),
        (
            "SigningPrivateKey",
            "signing_private_key",
            Ty::signing_private_key(),
        ),
        (
            "MlKemPrivateKey",
            "mlkem_private_key",
            Ty::mlkem_private_key(),
        ),
    ] {
        let seal = FunctionSignature {
            modes: vec![
                ParamOwnership::Borrow,
                ParamOwnership::Borrow,
                ParamOwnership::Move,
            ],
            formal_types: vec![Some(resource), Some(Ty::storage_key()), Some(Ty::bytes())],
        };
        signatures.insert(format!("{module}.seal_for_storage"), seal.clone());
        signatures.insert(format!("{prefix}_seal_for_storage"), seal);
        let unseal = FunctionSignature {
            modes: vec![
                ParamOwnership::Move,
                ParamOwnership::Borrow,
                ParamOwnership::Move,
            ],
            formal_types: vec![
                Some(Ty::bytes()),
                Some(Ty::storage_key()),
                Some(Ty::bytes()),
            ],
        };
        signatures.insert(format!("{module}.unseal_from_storage"), unseal.clone());
        signatures.insert(format!("{prefix}_unseal_from_storage"), unseal);
    }
    for name in ["seal_bytes", "unseal_bytes"] {
        let signature = FunctionSignature {
            modes: vec![
                ParamOwnership::Move,
                ParamOwnership::Borrow,
                ParamOwnership::Move,
            ],
            formal_types: vec![
                Some(Ty::bytes()),
                Some(Ty::storage_key()),
                Some(Ty::bytes()),
            ],
        };
        signatures.insert(format!("StorageKey.{name}"), signature.clone());
        signatures.insert(format!("storage_key_{name}"), signature);
    }
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
        "argon2id",
        vec![
            ParamOwnership::Borrow,
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Move,
        ],
        vec![
            Ty::secret_bytes(),
            Ty::bytes(),
            Ty::int(),
            Ty::int(),
            Ty::int(),
            Ty::int(),
        ],
    );
    register_crypto_signature(
        &mut signatures,
        "x25519_from_secret",
        vec![ParamOwnership::Consume],
        vec![Ty::secret_bytes()],
    );
    for operation in ["mlkem_from_secret", "signing_from_secret"] {
        register_crypto_signature(
            &mut signatures,
            operation,
            vec![ParamOwnership::Consume],
            vec![Ty::secret_bytes()],
        );
    }
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
        "hpke_seal",
        vec![
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Move,
        ],
        vec![
            Ty::x25519_public_key(),
            Ty::bytes(),
            Ty::bytes(),
            Ty::bytes(),
        ],
    );
    register_crypto_signature(
        &mut signatures,
        "hpke_open",
        vec![
            ParamOwnership::Borrow,
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Move,
        ],
        vec![
            Ty::x25519_private_key(),
            Ty::bytes(),
            Ty::bytes(),
            Ty::bytes(),
        ],
    );
    register_crypto_signature(
        &mut signatures,
        "hpke_seal_secret",
        vec![
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Borrow,
        ],
        vec![
            Ty::x25519_public_key(),
            Ty::bytes(),
            Ty::bytes(),
            Ty::secret_bytes(),
        ],
    );
    register_crypto_signature(
        &mut signatures,
        "hpke_open_secret",
        vec![
            ParamOwnership::Borrow,
            ParamOwnership::Move,
            ParamOwnership::Move,
            ParamOwnership::Move,
        ],
        vec![
            Ty::x25519_private_key(),
            Ty::bytes(),
            Ty::bytes(),
            Ty::bytes(),
        ],
    );
    register_crypto_signature(
        &mut signatures,
        "mlkem_decapsulate",
        vec![ParamOwnership::Borrow, ParamOwnership::Move],
        vec![Ty::mlkem_private_key(), Ty::mlkem_ciphertext()],
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
        call_targets,
        scopes: vec![FxHashMap::default()],
        signatures,
        errors: Vec::new(),
    };

    checker.check_resource_patterns(parse);
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
        let (path, names) = match &item {
            Item::ImportDecl(import) => (import.module_path(), None),
            Item::FromImportDecl(import) => {
                let list = import.import_list();
                let names: Vec<String> = list
                    .iter()
                    .flat_map(|list| list.names())
                    .filter_map(|name| name.text())
                    .collect();
                (import.module_path(), Some(names))
            }
            _ => continue,
        };
        let namespace = path.and_then(|path| path.segments().last().cloned());
        let Some((namespace, exports)) = namespace.and_then(|namespace| {
            let exports = import_ctx.module_exports.get(&namespace)?;
            Some((namespace, exports))
        }) else {
            continue;
        };
        let Some(names) = names else {
            // `import Module`: each function as `Module.name`, and by its
            // bare symbol, which cross-module lowering links it by.
            for export_name in exports.function_ownership.keys() {
                let signature = exported_signature(exports, export_name);
                signatures.insert(format!("{namespace}.{export_name}"), signature.clone());
                signatures.entry(export_name.clone()).or_insert(signature);
            }
            continue;
        };
        // An overloaded name brings each of its arities (`name__N`).
        for imported in names {
            for export_name in exports
                .function_ownership
                .keys()
                .filter(|exported| source_function_name(exported) == imported)
            {
                signatures
                    .entry(export_name.clone())
                    .or_insert_with(|| exported_signature(exports, export_name));
            }
        }
    }
}

pub(crate) fn source_function_name(export_name: &str) -> String {
    export_name
        .rsplit_once("__")
        .filter(|(_, arity)| !arity.is_empty() && arity.chars().all(|c| c.is_ascii_digit()))
        .map(|(name, _)| name)
        .unwrap_or(export_name)
        .to_string()
}

/// How an exported function takes its arguments: the exports hold each
/// function's modes and its type together.
fn exported_signature(exports: &ModuleExports, export_name: &str) -> FunctionSignature {
    let modes = exports.function_ownership[export_name].clone();
    let formal_types = match exports.functions.get(export_name).map(|scheme| &scheme.ty) {
        Some(Ty::Fun(parameters, _)) => parameters.iter().cloned().map(Some).collect(),
        _ => vec![None; modes.len()],
    };
    FunctionSignature {
        modes,
        formal_types,
    }
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
    /// `Ok(key) as whole` (or `Ok(_) as whole`, a `_` owning what it
    /// stands for) would give one resource two owners.
    fn check_resource_patterns(&mut self, parse: &Parse) {
        for pattern in parse.syntax().descendants().filter_map(Pattern::cast) {
            let Pattern::As(as_pattern) = &pattern else {
                continue;
            };
            if as_pattern
                .pattern()
                .is_some_and(|inner| self.owns_resource(&inner))
            {
                self.errors.push(TypeError::ResourceViolation {
                    reason: "resource value cannot be bound both by `as` and inside its pattern"
                        .to_string(),
                    span: pattern.syntax().text_range(),
                });
            }
        }
    }

    /// Whether `pattern` binds a resource, by name or with a `_`.
    fn owns_resource(&self, pattern: &Pattern) -> bool {
        pattern
            .syntax()
            .descendants_with_tokens()
            .filter_map(|element| element.into_node())
            .filter_map(Pattern::cast)
            .any(|part| {
                let owns = match &part {
                    Pattern::Wildcard(_) => true,
                    Pattern::Ident(ident) => ident
                        .name()
                        .is_some_and(|name| !name.text().starts_with(char::is_uppercase)),
                    _ => false,
                };
                owns && self
                    .types
                    .get(&part.syntax().text_range())
                    .is_some_and(|ty| self.registry.is_resource_type(ty))
            })
    }

    fn check_top_level_binding(&mut self, binding: &LetBinding) {
        let ty = binding
            .initializer()
            .and_then(|initializer| self.known_expr_type(&initializer))
            .or_else(|| annotated_type(binding.type_annotation()));
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

        for (parameter, ty) in typed_params(self.types, actor.syntax(), actor.param_list()) {
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

        if let Some(body) = actor.body() {
            self.check_block(&body);
        }

        self.scopes.pop();
    }

    fn check_function(&mut self, function: &FnDef) {
        self.scopes.push(FxHashMap::default());

        let types = self.types;
        if let (Some(annotation), Some(Ty::Fun(_, return_type))) = (
            function.return_type(),
            types.get(&function.syntax().text_range()),
        ) {
            self.check_resource_holder(return_type, annotation.syntax().text_range());
        }
        for (parameter, ty) in typed_params(types, function.syntax(), function.param_list()) {
            let Some(ty) = ty else {
                continue;
            };
            self.check_resource_holder(&ty, parameter.syntax().text_range());
            let borrowed = parameter.ownership() == ParamOwnership::Borrow;
            if let Some(name) = parameter.name() {
                self.insert_binding(name.text().to_string(), ty, borrowed);
            } else if let Some(pattern) = parameter.pattern() {
                self.bind_pattern(&pattern);
                for name in pattern.binders() {
                    if let Some(binding) = self.lookup_mut(name.text()) {
                        binding.borrowed = borrowed;
                    }
                }
            }
        }
        if let Some(guard) = function.guard().and_then(|guard| guard.expr()) {
            self.check_guard(&guard);
        }

        if let Some(body) = function.body() {
            self.check_block(&body);
        } else if let Some(body) = function.expr_body() {
            self.check_expr(&body, Usage::Move);
        }

        self.scopes.pop();
    }

    /// A declared parameter or return type that holds a resource must keep
    /// it affine: no unrestricted collection holds one, nor a wrapper the
    /// checker cannot follow.
    fn check_resource_holder(&mut self, ty: &Ty, span: TextRange) {
        let reason = if is_unrestricted_collection_type(ty) && self.registry.is_resource_type(ty) {
            format!("resource-bearing type `{ty}` cannot be used as an unrestricted collection")
        } else if is_unsupported_resource_wrapper(self.registry, ty) {
            unsupported_wrapper_reason(ty)
        } else {
            return;
        };
        self.errors
            .push(TypeError::ResourceViolation { reason, span });
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
        // An unannotated list or map literal's resources are each reported
        // where they enter it.
        let unannotated_literal = matches!(initializer, Expr::ListLiteral(_) | Expr::MapLiteral(_))
            && binding.type_annotation().is_none();
        if let Some(ty) = initializer_ty.as_ref().filter(|_| !unannotated_literal) {
            self.check_resource_holder(ty, binding.syntax().text_range());
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
            Expr::PipeExpr(pipe) => self.check_pipe(pipe.lhs(), 0, pipe.rhs()),
            Expr::SlotPipeExpr(pipe) => self.check_pipe(
                pipe.lhs(),
                pipe.slot()
                    .map_or(0, |slot| (slot as usize).saturating_sub(1)),
                pipe.rhs(),
            ),
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
                        self.reject_resource(&inner, "cannot be interpolated or formatted");
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
                        if index == 1 {
                            self.reject_resource(
                                &argument,
                                "cannot cross an actor mailbox boundary",
                            );
                        }
                        self.check_expr(&argument, Usage::Read);
                    }
                }
            }
            Expr::SpawnExpr(spawn) => {
                if let Some(arguments) = spawn.arg_list() {
                    for (index, argument) in arguments.args().enumerate() {
                        if index > 0 {
                            self.reject_resource(
                                &argument,
                                "cannot be transferred into a spawned actor",
                            );
                        }
                        self.check_expr(&argument, Usage::Read);
                    }
                }
            }
            Expr::ListLiteral(list) => {
                for element in list.elements() {
                    self.reject_resource(&element, "cannot enter an unrestricted collection");
                    self.check_expr(&element, Usage::Read);
                }
            }
            Expr::MapLiteral(map) => {
                for entry in map.entries() {
                    let key = (!entry.is_keyword_entry()).then(|| entry.key()).flatten();
                    for element in key.into_iter().chain(entry.value()) {
                        self.reject_resource(&element, "cannot enter an unrestricted collection");
                        self.check_expr(&element, Usage::Read);
                    }
                }
            }
            Expr::JsonExpr(json) => {
                for value in json.fields().filter_map(|field| field.value()) {
                    self.reject_resource(&value, "cannot cross JSON or serialization boundaries");
                    self.check_expr(&value, Usage::Read);
                }
            }
            Expr::ClosureExpr(closure) => self.check_closure(closure),
            Expr::Block(block) => self.check_block(block),
            Expr::TupleExpr(tuple) => {
                for element in tuple.elements() {
                    self.check_expr(&element, self.usage_of(&element));
                }
            }
            Expr::ReturnExpr(return_expr) => {
                if let Some(value) = return_expr.value() {
                    self.check_expr(&value, self.usage_of(&value));
                }
            }
            Expr::TryExpr(try_expr) => {
                if let Some(operand) = try_expr.operand() {
                    self.check_expr(&operand, self.usage_of(&operand));
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

    /// Report `expr` if it is a resource, which `cannot` (as "cannot be
    /// interpolated or formatted") says it may not.
    fn reject_resource(&mut self, expr: &Expr, cannot: &str) {
        if self.expr_is_resource(expr) {
            self.errors.push(TypeError::ResourceViolation {
                reason: format!("resource `{}` {cannot}", self.expr_label(expr)),
                span: expr.syntax().text_range(),
            });
        }
    }

    /// How a value given up whole is used: a resource moves, anything else
    /// is read.
    fn usage_of(&self, expr: &Expr) -> Usage {
        if self.expr_is_resource(expr) {
            Usage::Move
        } else {
            Usage::Read
        }
    }

    fn check_call(&mut self, call: &CallExpr) {
        self.check_call_parts(call.syntax().text_range(), call.callee(), call.args());
    }

    /// `value |> f(a)` passes `value` as `f`'s first argument, and
    /// `value |2> f(a)` as its second; a bare `value |> f` calls `f` with it.
    fn check_pipe(&mut self, value: Option<Expr>, slot: usize, rhs: Option<Expr>) {
        if let Some(Expr::TryExpr(try_expr)) = rhs {
            return self.check_pipe(value, slot, try_expr.operand());
        }
        let (range, callee, mut args) = match rhs {
            Some(Expr::CallExpr(call)) => (call.syntax().text_range(), call.callee(), call.args()),
            Some(callee) => (callee.syntax().text_range(), Some(callee), Vec::new()),
            None => return,
        };
        if let Some(value) = value {
            args.insert(slot.min(args.len()), value);
        }
        self.check_call_parts(range, callee, args);
    }

    /// A call of `callee` with `args`; `range` is where the type checker
    /// recorded the call's type and the arity it runs.
    fn check_call_parts(&mut self, range: TextRange, callee: Option<Expr>, args: Vec<Expr>) {
        let callee_name = callee.as_ref().and_then(direct_callee_name).map(|name| {
            match self.call_targets.get(&range) {
                Some(arity) => match name.rsplit_once('.') {
                    Some((module, _)) => format!("{module}.{arity}"),
                    None => arity.clone(),
                },
                None => name,
            }
        });
        let transaction_api = match callee_name.as_deref() {
            Some("Pg.transaction" | "pg_transaction") => Some("Pg.transaction"),
            Some("Repo.transaction" | "repo_transaction") => Some("Repo.transaction"),
            _ => None,
        };
        if let Some(transaction_api) = transaction_api {
            if let Some(callback) = args.get(1) {
                let borrows_connection = match callback {
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
                .get(&range)
                .is_some_and(|ty| is_resource_sum_constructor(self.registry, ty, callee))
        });
        {
            for (index, argument) in args.into_iter().enumerate() {
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
        let values = update
            .override_fields()
            .into_iter()
            .filter_map(|field| field.value());
        for value in update.base_expr().into_iter().chain(values) {
            self.check_expr(&value, self.usage_of(&value));
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
            if let Some(guard) = arm.guard() {
                self.check_guard(&guard);
            }
            // A resource the arm leaves unmoved is destroyed where it ends.
            if let Some(body) = arm.body() {
                self.check_expr(&body, Usage::Move);
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
            if let Some(guard) = arm.guard() {
                self.check_guard(&guard);
            }
            if let Some(body) = arm.body() {
                self.check_expr(&body, Usage::Move);
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
            // The alternatives bind the same names.
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
            _ => {
                for child in pattern.sub_patterns() {
                    self.bind_pattern(&child);
                }
            }
        }
    }

    /// A failing guard passes the value on to the next arm, so it may not
    /// move a resource: the next arm would get one already moved.
    fn check_guard(&mut self, guard: &Expr) {
        let before = self.scopes.clone();
        self.check_expr(guard, Usage::Read);
        let mut moved: Vec<&String> = self
            .scopes
            .iter()
            .zip(&before)
            .flat_map(|(scope, before)| {
                scope.iter().filter(|(name, binding)| {
                    binding.moved && before.get(*name).is_some_and(|earlier| !earlier.moved)
                })
            })
            .map(|(name, _)| name)
            .collect();
        moved.sort();
        let errors: Vec<TypeError> = moved
            .into_iter()
            .map(|name| TypeError::ResourceViolation {
                reason: format!("a guard cannot move resource `{name}`"),
                span: guard.syntax().text_range(),
            })
            .collect();
        self.errors.extend(errors);
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
            }
        }
    }

    fn check_name(&mut self, name_ref: &NameRef, usage: Usage) {
        let Some(name) = name_ref.text() else {
            return;
        };
        // Only a binding of a resource is checked.
        let registry = self.registry;
        let Some(binding) = self
            .lookup_mut(&name)
            .filter(|binding| registry.is_resource_type(&binding.ty))
        else {
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

    /// How an error names a resource: by its name, or else by its type (a
    /// resource's is always known).
    fn expr_label(&self, expr: &Expr) -> String {
        match expr {
            Expr::NameRef(name) => name.text().unwrap_or_default(),
            _ => self
                .known_expr_type(expr)
                .map(|ty| ty.to_string())
                .unwrap_or_default(),
        }
    }
}

/// Whether `function` continues the clauses of the function defined right
/// before it: one with the same name and arity.
fn is_later_clause(function: &FnDef) -> bool {
    function
        .syntax()
        .prev_sibling()
        .and_then(FnDef::cast)
        .is_some_and(|previous| {
            previous.name().and_then(|name| name.text())
                == function.name().and_then(|name| name.text())
                && fn_arity(&previous) == fn_arity(function)
        })
}

fn fn_arity(function: &FnDef) -> usize {
    function
        .param_list()
        .map_or(0, |list| list.params().count())
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

/// Each parameter of a function or actor (`node`, with `params`) and its
/// type: the one the checker gave it, or else the type its annotation names.
fn typed_params(
    types: &FxHashMap<TextRange, Ty>,
    node: &SyntaxNode,
    params: Option<ParamList>,
) -> Vec<(Param, Option<Ty>)> {
    let formals = match types.get(&node.text_range()) {
        Some(Ty::Fun(formals, _)) => formals.as_slice(),
        _ => &[],
    };
    params
        .iter()
        .flat_map(|list| list.params())
        .enumerate()
        .map(|(index, param)| {
            let ty = formals
                .get(index)
                .cloned()
                .or_else(|| annotated_type(param.type_annotation()));
            (param, ty)
        })
        .collect()
}

/// The type an annotation names, by its name alone.
fn annotated_type(annotation: Option<TypeAnnotation>) -> Option<Ty> {
    Some(Ty::Con(crate::ty::TyCon::new(
        annotation?.type_name()?.text(),
    )))
}

/// How a function or actor (`node`, with `params`) takes each argument.
fn signature_of(
    types: &FxHashMap<TextRange, Ty>,
    node: &SyntaxNode,
    params: Option<ParamList>,
) -> FunctionSignature {
    let (modes, formal_types) = typed_params(types, node, params)
        .into_iter()
        .map(|(param, ty)| {
            // A method's receiver is borrowed, however the method is called:
            // `s.close()` only reads `s`, and `close(s)` moved it into a
            // method that never dropped it.
            let mode = if param.is_self() {
                ParamOwnership::Borrow
            } else {
                param.ownership()
            };
            (mode, ty)
        })
        .unzip();
    FunctionSignature {
        modes,
        formal_types,
    }
}

/// The module a definition is inside, if any.
fn enclosing_module(node: &SyntaxNode) -> Option<String> {
    node.ancestors()
        .skip(1)
        .find_map(ModuleDef::cast)
        .and_then(|module| module.name())
        .and_then(|name| name.text())
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
    matches!(ty.con_name(), Some("List" | "Map" | "Set"))
}

/// Whether `callee` is a variant of `ty`, a sum type that holds a resource.
fn is_resource_sum_constructor(registry: &TypeRegistry, ty: &Ty, callee: &str) -> bool {
    let variant_name = callee.rsplit('.').next().unwrap_or(callee);
    registry.is_resource_type(ty)
        && ty
            .con_name()
            .and_then(|name| registry.sum_type_defs.get(name))
            .is_some_and(|definition| {
                definition
                    .variants
                    .iter()
                    .any(|variant| variant.name == variant_name)
            })
}

fn is_unsupported_resource_wrapper(registry: &TypeRegistry, ty: &Ty) -> bool {
    let (Ty::App(_, arguments), Some(constructor)) = (ty, ty.con_name()) else {
        return false;
    };

    arguments
        .iter()
        .any(|argument| registry.is_resource_type(argument))
        && !matches!(
            constructor,
            "List" | "Map" | "Set" | "Pid" | "Option" | "Result"
        )
        && !registry.is_resource_name(constructor)
        && !registry.struct_defs.contains_key(constructor)
        && !registry.sum_type_defs.contains_key(constructor)
}

fn unsupported_wrapper_reason(ty: &Ty) -> String {
    format!("resource-bearing wrapper `{ty}` has no registered resource destructor")
}
