//! Item/declaration parsers for Snow.
//!
//! Parses top-level and nested declarations: function definitions, module
//! definitions, import declarations, and struct definitions. Handles visibility
//! (pub keyword) and type annotations.

use crate::syntax_kind::SyntaxKind;

use super::expressions::parse_param_list;
use super::Parser;

// ── Visibility ───────────────────────────────────────────────────────────

/// If the current token is `pub`, parse it as a VISIBILITY node.
fn parse_optional_visibility(p: &mut Parser) {
    if p.at(SyntaxKind::PUB_KW) {
        let m = p.open();
        p.advance(); // pub
        p.close(m, SyntaxKind::VISIBILITY);
    }
}

// ── Function Definition ──────────────────────────────────────────────────

/// Parse a function definition: `[pub] fn|def name(params) [-> ReturnType] do body end`
pub(crate) fn parse_fn_def(p: &mut Parser) {
    let m = p.open();

    // Optional visibility.
    parse_optional_visibility(p);

    // Consume fn or def keyword.
    p.advance(); // FN_KW or DEF_KW

    // Function name.
    if p.at(SyntaxKind::IDENT) {
        let name = p.open();
        p.advance();
        p.close(name, SyntaxKind::NAME);
    } else {
        p.error("expected function name");
        p.close(m, SyntaxKind::FN_DEF);
        return;
    }

    // Parameter list.
    if p.at(SyntaxKind::L_PAREN) {
        parse_param_list(p);
    }

    // Optional return type: -> Type
    if p.at(SyntaxKind::ARROW) {
        let ann = p.open();
        p.advance(); // ->
        parse_type(p);
        p.close(ann, SyntaxKind::TYPE_ANNOTATION);
    }

    // Expect `do`.
    let do_span = p.current_span();
    p.expect(SyntaxKind::DO_KW);

    // Parse body.
    if !p.has_error() {
        parse_item_block_body(p);
    }

    // Expect `end`.
    if !p.at(SyntaxKind::END_KW) {
        p.error_with_related(
            "expected `end` to close function body",
            do_span,
            "`do` block started here",
        );
    } else {
        p.advance(); // END_KW
    }

    p.close(m, SyntaxKind::FN_DEF);
}

// ── Module Definition ────────────────────────────────────────────────────

/// Parse a module definition: `[pub] module Name do items end`
pub(crate) fn parse_module_def(p: &mut Parser) {
    let m = p.open();

    // Optional visibility.
    parse_optional_visibility(p);

    p.advance(); // MODULE_KW

    // Module name.
    if p.at(SyntaxKind::IDENT) {
        let name = p.open();
        p.advance();
        p.close(name, SyntaxKind::NAME);
    } else {
        p.error("expected module name");
        p.close(m, SyntaxKind::MODULE_DEF);
        return;
    }

    // Expect `do`.
    let do_span = p.current_span();
    p.expect(SyntaxKind::DO_KW);

    // Parse module items.
    if !p.has_error() {
        parse_item_block_body(p);
    }

    // Expect `end`.
    if !p.at(SyntaxKind::END_KW) {
        p.error_with_related(
            "expected `end` to close module body",
            do_span,
            "`do` block started here",
        );
    } else {
        p.advance(); // END_KW
    }

    p.close(m, SyntaxKind::MODULE_DEF);
}

// ── Import Declarations ──────────────────────────────────────────────────

/// Parse an import declaration: `import ModulePath`
///
/// Module path is dot-separated: `import Foo.Bar.Baz`
pub(crate) fn parse_import_decl(p: &mut Parser) {
    let m = p.open();
    p.advance(); // IMPORT_KW

    // Parse module path.
    parse_module_path(p);

    p.close(m, SyntaxKind::IMPORT_DECL);
}

/// Parse a from-import declaration: `from ModulePath import name1, name2`
///
/// Glob imports (`from Module import *`) are rejected.
pub(crate) fn parse_from_import_decl(p: &mut Parser) {
    let m = p.open();

    // "from" is an identifier (not a keyword), advance it.
    p.advance(); // "from" IDENT

    // Parse module path.
    parse_module_path(p);

    // Expect `import`.
    p.expect(SyntaxKind::IMPORT_KW);

    // Parse import list.
    if !p.has_error() {
        let list = p.open();

        // Reject glob import: `from Module import *`
        if p.at(SyntaxKind::STAR) {
            p.error("glob imports are not allowed; import names explicitly");
            p.close(list, SyntaxKind::IMPORT_LIST);
            p.close(m, SyntaxKind::FROM_IMPORT_DECL);
            return;
        }

        if p.at(SyntaxKind::IDENT) {
            let name = p.open();
            p.advance();
            p.close(name, SyntaxKind::NAME);

            while p.eat(SyntaxKind::COMMA) {
                if p.at(SyntaxKind::NEWLINE) || p.at(SyntaxKind::EOF) {
                    break;
                }
                let name = p.open();
                p.expect(SyntaxKind::IDENT);
                p.close(name, SyntaxKind::NAME);
            }
        } else {
            p.error("expected import name");
        }

        p.close(list, SyntaxKind::IMPORT_LIST);
    }

    p.close(m, SyntaxKind::FROM_IMPORT_DECL);
}

/// Parse a dot-separated module path: `Foo.Bar.Baz`
fn parse_module_path(p: &mut Parser) {
    let m = p.open();

    if p.at(SyntaxKind::IDENT) {
        p.advance(); // first segment

        while p.at(SyntaxKind::DOT) {
            p.advance(); // .
            p.expect(SyntaxKind::IDENT);
        }
    } else {
        p.error("expected module name");
    }

    p.close(m, SyntaxKind::PATH);
}

// ── Struct Definition ────────────────────────────────────────────────────

/// Parse a struct definition: `[pub] struct Name [TypeParams] do fields end`
pub(crate) fn parse_struct_def(p: &mut Parser) {
    let m = p.open();

    // Optional visibility.
    parse_optional_visibility(p);

    p.advance(); // STRUCT_KW

    // Struct name.
    if p.at(SyntaxKind::IDENT) {
        let name = p.open();
        p.advance();
        p.close(name, SyntaxKind::NAME);
    } else {
        p.error("expected struct name");
        p.close(m, SyntaxKind::STRUCT_DEF);
        return;
    }

    // Optional type parameters: [A, B]
    if p.at(SyntaxKind::L_BRACKET) {
        parse_type_param_list(p);
    }

    // Expect `do`.
    let do_span = p.current_span();
    p.expect(SyntaxKind::DO_KW);

    // Parse fields.
    if !p.has_error() {
        loop {
            p.eat_newlines();

            if p.at(SyntaxKind::END_KW) || p.at(SyntaxKind::EOF) {
                break;
            }

            parse_struct_field(p);

            if p.has_error() {
                break;
            }
        }
    }

    // Expect `end`.
    if !p.at(SyntaxKind::END_KW) {
        p.error_with_related(
            "expected `end` to close struct body",
            do_span,
            "`do` block started here",
        );
    } else {
        p.advance(); // END_KW
    }

    p.close(m, SyntaxKind::STRUCT_DEF);
}

/// Parse a struct field: `name :: Type`
fn parse_struct_field(p: &mut Parser) {
    let m = p.open();

    // Field name.
    if p.at(SyntaxKind::IDENT) {
        let name = p.open();
        p.advance();
        p.close(name, SyntaxKind::NAME);
    } else {
        p.error("expected field name");
        p.close(m, SyntaxKind::STRUCT_FIELD);
        return;
    }

    // Type annotation: :: Type
    if p.at(SyntaxKind::COLON_COLON) {
        let ann = p.open();
        p.advance(); // ::
        parse_type(p);
        p.close(ann, SyntaxKind::TYPE_ANNOTATION);
    }

    p.close(m, SyntaxKind::STRUCT_FIELD);
}

/// Parse a type parameter list: `[A, B, C]`
fn parse_type_param_list(p: &mut Parser) {
    let m = p.open();
    p.advance(); // [

    if !p.at(SyntaxKind::R_BRACKET) {
        p.expect(SyntaxKind::IDENT);
        while p.eat(SyntaxKind::COMMA) {
            if p.at(SyntaxKind::R_BRACKET) {
                break;
            }
            p.expect(SyntaxKind::IDENT);
        }
    }

    p.expect(SyntaxKind::R_BRACKET);
    p.close(m, SyntaxKind::TYPE_PARAM_LIST);
}

// ── Type Parsing ─────────────────────────────────────────────────────────

/// Parse a type expression: `Ident`, `Ident[A, B]`, `Mod.Type`
pub(crate) fn parse_type(p: &mut Parser) {
    if p.at(SyntaxKind::IDENT) {
        p.advance(); // type name

        // Optional dot-separated path: Foo.Bar
        while p.at(SyntaxKind::DOT) {
            p.advance(); // .
            p.expect(SyntaxKind::IDENT);
        }

        // Optional type parameters: [A, B]
        if p.at(SyntaxKind::L_BRACKET) {
            let params = p.open();
            p.advance(); // [
            if !p.at(SyntaxKind::R_BRACKET) {
                parse_type(p);
                while p.eat(SyntaxKind::COMMA) {
                    if p.at(SyntaxKind::R_BRACKET) {
                        break;
                    }
                    parse_type(p);
                }
            }
            p.expect(SyntaxKind::R_BRACKET);
            p.close(params, SyntaxKind::TYPE_PARAM_LIST);
        }
    } else {
        p.error("expected type name");
    }
}

// ── Item Block Body ──────────────────────────────────────────────────────

/// Parse a block body that can contain items (fn, module, struct) as well
/// as statements/expressions.
///
/// This is used for module bodies and function bodies.
fn parse_item_block_body(p: &mut Parser) {
    let m = p.open();

    loop {
        // Eat leading newlines/semicolons between statements.
        p.eat_newlines();
        while p.eat(SyntaxKind::SEMICOLON) {
            p.eat_newlines();
        }

        // Check if we've reached a block terminator.
        match p.current() {
            SyntaxKind::END_KW | SyntaxKind::ELSE_KW | SyntaxKind::EOF => break,
            _ => {}
        }

        // Parse an item or statement.
        super::parse_item_or_stmt(p);

        if p.has_error() {
            break;
        }

        // After a statement, handle separators.
        match p.current() {
            SyntaxKind::NEWLINE => {
                p.eat_newlines();
            }
            SyntaxKind::SEMICOLON => {
                // Will be eaten at top of loop.
            }
            SyntaxKind::END_KW | SyntaxKind::ELSE_KW | SyntaxKind::EOF => {
                // Block terminator -- stop.
            }
            _ => {}
        }
    }

    p.close(m, SyntaxKind::BLOCK);
}
