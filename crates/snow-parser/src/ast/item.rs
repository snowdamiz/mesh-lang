//! Typed AST nodes for declarations and items.
//!
//! Covers: SourceFile, FnDef, ParamList, Param, TypeAnnotation, ModuleDef,
//! ImportDecl, FromImportDecl, ImportList, StructDef, StructField, LetBinding,
//! Visibility, Block, Name, NameRef, Path.

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
