//! Typed AST nodes for declarations and items.
//!
//! Covers: SourceFile, FnDef, ParamList, Param, TypeAnnotation, ModuleDef,
//! ImportDecl, FromImportDecl, ImportList, StructDef, StructField, LetBinding,
//! Visibility, Block, Name, NameRef, Path, SumTypeDef, VariantDef, VariantField.

use crate::ast::{ast_node, child_node, child_nodes, child_token, AstNode};
use crate::cst::{SyntaxNode, SyntaxToken};
use crate::syntax_kind::SyntaxKind;

// ── Source File ──────────────────────────────────────────────────────────

ast_node!(SourceFile, SOURCE_FILE);

impl SourceFile {
    /// All top-level items in the source file.
    pub fn items(&self) -> impl Iterator<Item = Item> + '_ {
        self.syntax.children().filter_map(Item::cast)
    }

    /// All top-level function definitions.
    pub fn fn_defs(&self) -> impl Iterator<Item = FnDef> + '_ {
        child_nodes(&self.syntax)
    }

    /// All top-level module definitions.
    pub fn modules(&self) -> impl Iterator<Item = ModuleDef> + '_ {
        child_nodes(&self.syntax)
    }
}

// ── Item enum ────────────────────────────────────────────────────────────

/// Any top-level or nested item declaration.
#[derive(Debug, Clone)]
pub enum Item {
    FnDef(FnDef),
    ModuleDef(ModuleDef),
    ImportDecl(ImportDecl),
    FromImportDecl(FromImportDecl),
    StructDef(StructDef),
    LetBinding(LetBinding),
    InterfaceDef(InterfaceDef),
    ImplDef(ImplDef),
    TypeAliasDef(TypeAliasDef),
    SumTypeDef(SumTypeDef),
    ActorDef(ActorDef),
    ServiceDef(ServiceDef),
    SupervisorDef(SupervisorDef),
}

impl Item {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        match node.kind() {
            SyntaxKind::FN_DEF => Some(Item::FnDef(FnDef { syntax: node })),
            SyntaxKind::MODULE_DEF => Some(Item::ModuleDef(ModuleDef { syntax: node })),
            SyntaxKind::IMPORT_DECL => Some(Item::ImportDecl(ImportDecl { syntax: node })),
            SyntaxKind::FROM_IMPORT_DECL => {
                Some(Item::FromImportDecl(FromImportDecl { syntax: node }))
            }
            SyntaxKind::STRUCT_DEF => Some(Item::StructDef(StructDef { syntax: node })),
            SyntaxKind::LET_BINDING => Some(Item::LetBinding(LetBinding { syntax: node })),
            SyntaxKind::INTERFACE_DEF => {
                Some(Item::InterfaceDef(InterfaceDef { syntax: node }))
            }
            SyntaxKind::IMPL_DEF => Some(Item::ImplDef(ImplDef { syntax: node })),
            SyntaxKind::TYPE_ALIAS_DEF => {
                Some(Item::TypeAliasDef(TypeAliasDef { syntax: node }))
            }
            SyntaxKind::SUM_TYPE_DEF => {
                Some(Item::SumTypeDef(SumTypeDef { syntax: node }))
            }
            SyntaxKind::ACTOR_DEF => Some(Item::ActorDef(ActorDef { syntax: node })),
            SyntaxKind::SERVICE_DEF => {
                Some(Item::ServiceDef(ServiceDef { syntax: node }))
            }
            SyntaxKind::SUPERVISOR_DEF => {
                Some(Item::SupervisorDef(SupervisorDef { syntax: node }))
            }
            _ => None,
        }
    }
}

// ── Function Definition ──────────────────────────────────────────────────

ast_node!(FnDef, FN_DEF);

impl FnDef {
    /// The visibility modifier (`pub`), if present.
    pub fn visibility(&self) -> Option<Visibility> {
        child_node(&self.syntax)
    }

    /// The function name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The parameter list.
    pub fn param_list(&self) -> Option<ParamList> {
        child_node(&self.syntax)
    }

    /// The return type annotation (`-> Type`), if present.
    pub fn return_type(&self) -> Option<TypeAnnotation> {
        child_node(&self.syntax)
    }

    /// The function body block.
    pub fn body(&self) -> Option<Block> {
        child_node(&self.syntax)
    }
}

// ── Parameter List ───────────────────────────────────────────────────────

ast_node!(ParamList, PARAM_LIST);

impl ParamList {
    /// All parameters in the list.
    pub fn params(&self) -> impl Iterator<Item = Param> + '_ {
        child_nodes(&self.syntax)
    }
}

// ── Parameter ────────────────────────────────────────────────────────────

ast_node!(Param, PARAM);

impl Param {
    /// The parameter name token (IDENT).
    pub fn name(&self) -> Option<SyntaxToken> {
        child_token(&self.syntax, SyntaxKind::IDENT)
    }

    /// The type annotation, if present.
    pub fn type_annotation(&self) -> Option<TypeAnnotation> {
        child_node(&self.syntax)
    }
}

// ── Type Annotation ──────────────────────────────────────────────────────

ast_node!(TypeAnnotation, TYPE_ANNOTATION);

impl TypeAnnotation {
    /// The type name token(s). Returns the first IDENT in the annotation.
    pub fn type_name(&self) -> Option<SyntaxToken> {
        child_token(&self.syntax, SyntaxKind::IDENT)
    }
}

// ── Module Definition ────────────────────────────────────────────────────

ast_node!(ModuleDef, MODULE_DEF);

impl ModuleDef {
    /// The visibility modifier, if present.
    pub fn visibility(&self) -> Option<Visibility> {
        child_node(&self.syntax)
    }

    /// The module name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The items in the module body.
    pub fn items(&self) -> impl Iterator<Item = Item> + '_ {
        // Items are inside the BLOCK child.
        self.syntax
            .children()
            .filter(|n| n.kind() == SyntaxKind::BLOCK)
            .flat_map(|block| block.children().filter_map(Item::cast))
    }
}

// ── Import Declarations ──────────────────────────────────────────────────

ast_node!(ImportDecl, IMPORT_DECL);

impl ImportDecl {
    /// The module path being imported.
    pub fn module_path(&self) -> Option<Path> {
        child_node(&self.syntax)
    }
}

ast_node!(FromImportDecl, FROM_IMPORT_DECL);

impl FromImportDecl {
    /// The module path being imported from.
    pub fn module_path(&self) -> Option<Path> {
        child_node(&self.syntax)
    }

    /// The list of imported names.
    pub fn import_list(&self) -> Option<ImportList> {
        child_node(&self.syntax)
    }
}

ast_node!(ImportList, IMPORT_LIST);

impl ImportList {
    /// All imported names.
    pub fn names(&self) -> impl Iterator<Item = Name> + '_ {
        child_nodes(&self.syntax)
    }
}

// ── Struct Definition ────────────────────────────────────────────────────

ast_node!(StructDef, STRUCT_DEF);

impl StructDef {
    /// The visibility modifier, if present.
    pub fn visibility(&self) -> Option<Visibility> {
        child_node(&self.syntax)
    }

    /// The struct name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The struct fields.
    pub fn fields(&self) -> impl Iterator<Item = StructField> + '_ {
        child_nodes(&self.syntax)
    }
}

ast_node!(StructField, STRUCT_FIELD);

impl StructField {
    /// The field name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The type annotation.
    pub fn type_annotation(&self) -> Option<TypeAnnotation> {
        child_node(&self.syntax)
    }
}

// ── Let Binding ──────────────────────────────────────────────────────────

ast_node!(LetBinding, LET_BINDING);

impl LetBinding {
    /// The binding name (for simple `let x = ...`).
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The pattern, if the binding uses destructuring.
    pub fn pattern(&self) -> Option<super::pat::Pattern> {
        self.syntax
            .children()
            .find_map(super::pat::Pattern::cast)
    }

    /// The type annotation, if present.
    pub fn type_annotation(&self) -> Option<TypeAnnotation> {
        child_node(&self.syntax)
    }

    /// The initializer expression.
    pub fn initializer(&self) -> Option<super::expr::Expr> {
        // The initializer is the expression child after the = token.
        // We find the first expression-like child node.
        self.syntax
            .children()
            .find_map(super::expr::Expr::cast)
    }
}

// ── Visibility ───────────────────────────────────────────────────────────

ast_node!(Visibility, VISIBILITY);

impl Visibility {
    /// The `pub` keyword token.
    pub fn pub_kw(&self) -> Option<SyntaxToken> {
        child_token(&self.syntax, SyntaxKind::PUB_KW)
    }
}

// ── Block ────────────────────────────────────────────────────────────────

ast_node!(Block, BLOCK);

impl Block {
    /// Statements and expressions in the block.
    pub fn stmts(&self) -> impl Iterator<Item = Item> + '_ {
        self.syntax.children().filter_map(Item::cast)
    }

    /// The tail expression (last expression that is the block's value).
    /// This is the last child that can be cast to an Expr.
    pub fn tail_expr(&self) -> Option<super::expr::Expr> {
        self.syntax
            .children()
            .filter_map(super::expr::Expr::cast)
            .last()
    }
}

// ── Name and NameRef ─────────────────────────────────────────────────────

ast_node!(Name, NAME);

impl Name {
    /// The identifier text.
    pub fn text(&self) -> Option<String> {
        child_token(&self.syntax, SyntaxKind::IDENT).map(|t| t.text().to_string())
    }
}

ast_node!(NameRef, NAME_REF);

impl NameRef {
    /// The identifier text.
    pub fn text(&self) -> Option<String> {
        child_token(&self.syntax, SyntaxKind::IDENT).map(|t| t.text().to_string())
    }
}

// ── Path ─────────────────────────────────────────────────────────────────

ast_node!(Path, PATH);

impl Path {
    /// All segment identifiers in the path.
    pub fn segments(&self) -> Vec<String> {
        self.syntax
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .filter(|t| t.kind() == SyntaxKind::IDENT)
            .map(|t| t.text().to_string())
            .collect()
    }
}

// ── Interface Definition ────────────────────────────────────────────────

ast_node!(InterfaceDef, INTERFACE_DEF);

impl InterfaceDef {
    /// The visibility modifier, if present.
    pub fn visibility(&self) -> Option<Visibility> {
        child_node(&self.syntax)
    }

    /// The interface name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The method signatures in the interface.
    pub fn methods(&self) -> impl Iterator<Item = InterfaceMethod> + '_ {
        child_nodes(&self.syntax)
    }
}

ast_node!(InterfaceMethod, INTERFACE_METHOD);

impl InterfaceMethod {
    /// The method name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The parameter list.
    pub fn param_list(&self) -> Option<ParamList> {
        child_node(&self.syntax)
    }

    /// The return type annotation, if present.
    pub fn return_type(&self) -> Option<TypeAnnotation> {
        child_node(&self.syntax)
    }
}

// ── Impl Definition ─────────────────────────────────────────────────────

ast_node!(ImplDef, IMPL_DEF);

impl ImplDef {
    /// The trait path being implemented.
    pub fn trait_path(&self) -> Option<Path> {
        child_node(&self.syntax)
    }

    /// The function definitions in the impl block.
    pub fn methods(&self) -> impl Iterator<Item = FnDef> + '_ {
        // Methods are inside the BLOCK child.
        self.syntax
            .children()
            .filter(|n| n.kind() == SyntaxKind::BLOCK)
            .flat_map(|block| block.children().filter_map(FnDef::cast))
    }
}

// ── Type Alias ──────────────────────────────────────────────────────────

ast_node!(TypeAliasDef, TYPE_ALIAS_DEF);

impl TypeAliasDef {
    /// The alias name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }
}

// ── Sum Type Definition ──────────────────────────────────────────────────

ast_node!(SumTypeDef, SUM_TYPE_DEF);

impl SumTypeDef {
    /// The visibility modifier, if present.
    pub fn visibility(&self) -> Option<Visibility> {
        child_node(&self.syntax)
    }

    /// The sum type name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The variant definitions in the sum type.
    pub fn variants(&self) -> impl Iterator<Item = VariantDef> + '_ {
        child_nodes(&self.syntax)
    }
}

// ── Variant Definition ──────────────────────────────────────────────────

ast_node!(VariantDef, VARIANT_DEF);

impl VariantDef {
    /// The variant name IDENT token.
    pub fn name(&self) -> Option<SyntaxToken> {
        child_token(&self.syntax, SyntaxKind::IDENT)
    }

    /// Named fields (VARIANT_FIELD children) in the variant.
    ///
    /// For `Rectangle(width :: Float, height :: Float)`, this yields the named fields.
    pub fn fields(&self) -> impl Iterator<Item = VariantField> + '_ {
        child_nodes(&self.syntax)
    }

    /// Positional type annotations (TYPE_ANNOTATION children) in the variant.
    ///
    /// For `Circle(Float)` or `Pair(Int, Int)`, this yields the positional types.
    pub fn positional_types(&self) -> impl Iterator<Item = TypeAnnotation> + '_ {
        child_nodes(&self.syntax)
    }
}

// ── Variant Field ───────────────────────────────────────────────────────

ast_node!(VariantField, VARIANT_FIELD);

impl VariantField {
    /// The field name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The field type annotation.
    pub fn type_annotation(&self) -> Option<TypeAnnotation> {
        child_node(&self.syntax)
    }
}

// ── Actor Definition ────────────────────────────────────────────────────

ast_node!(ActorDef, ACTOR_DEF);

impl ActorDef {
    /// The actor name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The parameter list (state arguments), if present.
    pub fn param_list(&self) -> Option<ParamList> {
        child_node(&self.syntax)
    }

    /// The actor body block.
    pub fn body(&self) -> Option<Block> {
        child_node(&self.syntax)
    }

    /// The optional terminate clause for cleanup logic.
    pub fn terminate_clause(&self) -> Option<TerminateClause> {
        // The terminate clause is inside the BLOCK child of the actor body.
        self.syntax
            .children()
            .filter(|n| n.kind() == SyntaxKind::BLOCK)
            .flat_map(|block| block.children())
            .find_map(TerminateClause::cast)
    }
}

// ── Terminate Clause ─────────────────────────────────────────────────────

ast_node!(TerminateClause, TERMINATE_CLAUSE);

impl TerminateClause {
    /// The body block of the terminate clause.
    pub fn body(&self) -> Option<Block> {
        child_node(&self.syntax)
    }
}

// ── Supervisor Definition ──────────────────────────────────────────────

ast_node!(SupervisorDef, SUPERVISOR_DEF);

impl SupervisorDef {
    /// The supervisor name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The strategy clause node, if present.
    pub fn strategy(&self) -> Option<SyntaxNode> {
        self.syntax
            .children()
            .flat_map(|n| {
                if n.kind() == SyntaxKind::BLOCK {
                    n.children().collect::<Vec<_>>()
                } else {
                    vec![n]
                }
            })
            .find(|c| c.kind() == SyntaxKind::STRATEGY_CLAUSE)
    }

    /// The max_restarts clause node, if present.
    pub fn max_restarts(&self) -> Option<SyntaxNode> {
        self.syntax
            .children()
            .flat_map(|n| {
                if n.kind() == SyntaxKind::BLOCK {
                    n.children().collect::<Vec<_>>()
                } else {
                    vec![n]
                }
            })
            .find(|c| c.kind() == SyntaxKind::RESTART_LIMIT)
    }

    /// The max_seconds clause node, if present.
    pub fn max_seconds(&self) -> Option<SyntaxNode> {
        self.syntax
            .children()
            .flat_map(|n| {
                if n.kind() == SyntaxKind::BLOCK {
                    n.children().collect::<Vec<_>>()
                } else {
                    vec![n]
                }
            })
            .find(|c| c.kind() == SyntaxKind::SECONDS_LIMIT)
    }

    /// The child spec nodes inside the supervisor body.
    pub fn child_specs(&self) -> Vec<SyntaxNode> {
        self.syntax
            .children()
            .flat_map(|n| {
                if n.kind() == SyntaxKind::BLOCK {
                    n.children().collect::<Vec<_>>()
                } else {
                    vec![n]
                }
            })
            .filter(|c| c.kind() == SyntaxKind::CHILD_SPEC_DEF)
            .collect()
    }
}

// ── Service Definition ──────────────────────────────────────────────

ast_node!(ServiceDef, SERVICE_DEF);

impl ServiceDef {
    /// The service name.
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The init function definition (fn init(...) ... end), if present.
    pub fn init_fn(&self) -> Option<FnDef> {
        self.syntax
            .children()
            .filter(|n| n.kind() == SyntaxKind::BLOCK)
            .flat_map(|block| block.children())
            .find_map(FnDef::cast)
    }

    /// All call handlers in the service body.
    pub fn call_handlers(&self) -> Vec<CallHandler> {
        self.syntax
            .children()
            .filter(|n| n.kind() == SyntaxKind::BLOCK)
            .flat_map(|block| block.children())
            .filter_map(CallHandler::cast)
            .collect()
    }

    /// All cast handlers in the service body.
    pub fn cast_handlers(&self) -> Vec<CastHandler> {
        self.syntax
            .children()
            .filter(|n| n.kind() == SyntaxKind::BLOCK)
            .flat_map(|block| block.children())
            .filter_map(CastHandler::cast)
            .collect()
    }
}

// ── Call Handler ─────────────────────────────────────────────────────

ast_node!(CallHandler, CALL_HANDLER);

impl CallHandler {
    /// The call handler variant name (e.g., "GetCount").
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The parameter list, if present.
    pub fn params(&self) -> Option<ParamList> {
        child_node(&self.syntax)
    }

    /// The return type annotation (:: Type), if present.
    pub fn return_type(&self) -> Option<TypeAnnotation> {
        child_node(&self.syntax)
    }

    /// The state parameter name from the |state| pattern.
    pub fn state_param_name(&self) -> Option<String> {
        // First NAME is the handler name, second NAME (before BLOCK) is state param.
        let names: Vec<_> = self.syntax
            .children()
            .filter(|n| n.kind() == SyntaxKind::NAME)
            .collect();
        if names.len() >= 2 {
            return Name::cast(names[1].clone()).and_then(|n| n.text());
        }
        None
    }

    /// The body block of the call handler.
    pub fn body(&self) -> Option<Block> {
        child_node(&self.syntax)
    }
}

// ── Cast Handler ─────────────────────────────────────────────────────

ast_node!(CastHandler, CAST_HANDLER);

impl CastHandler {
    /// The cast handler variant name (e.g., "Reset").
    pub fn name(&self) -> Option<Name> {
        child_node(&self.syntax)
    }

    /// The parameter list, if present.
    pub fn params(&self) -> Option<ParamList> {
        child_node(&self.syntax)
    }

    /// The state parameter name from the |state| pattern.
    pub fn state_param_name(&self) -> Option<String> {
        // First NAME is the handler name, second NAME (before BLOCK) is state param.
        let names: Vec<_> = self.syntax
            .children()
            .filter(|n| n.kind() == SyntaxKind::NAME)
            .collect();
        if names.len() >= 2 {
            return Name::cast(names[1].clone()).and_then(|n| n.text());
        }
        None
    }

    /// The body block of the cast handler.
    pub fn body(&self) -> Option<Block> {
        child_node(&self.syntax)
    }
}
