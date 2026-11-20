//! The law-file grammar. Ordinary Mesh source continues to use the ordinary parser.

use mesh_common::span::Span;

use crate::parser::{expect_statement_end, expressions, items, Parser};
use crate::{Parse, ParseError, SyntaxKind};

pub struct LawFile {
    pub imports: Vec<LawImport>,
    pub laws: Vec<LawDeclaration>,
}

pub struct LawImport {
    pub module: String,
    pub name: String,
    pub private: bool,
    pub span: Span,
}

pub struct LawDeclaration {
    pub name: String,
    pub span: Span,
    /// The equivalent standalone Mesh function, with offsets relative to `span.start`.
    pub expected: Parse,
}

/// Parse explicit target imports followed by one or more complete result laws.
pub fn parse(source: &str) -> Result<LawFile, ParseError> {
    let mut parser = Parser::new(mesh_lexer::Lexer::tokenize(source), source);
    let root = parser.open();
    let mut imports = Vec::new();
    let mut laws = Vec::new();
    parser.eat_separators();
    while !parser.at(SyntaxKind::EOF) && !parser.has_error() {
        match parser.current_text() {
            "from" if laws.is_empty() => imports.push(parse_import(&mut parser)),
            "law" => laws.push(parse_declaration(&mut parser)),
            _ => parser.error("expected a target import or a `law` declaration"),
        }
        if !parser.has_error() {
            expect_statement_end(&mut parser);
            parser.eat_separators();
        }
    }
    if laws.is_empty() && !parser.has_error() {
        parser.error("expected at least one law declaration");
    }
    while !parser.at(SyntaxKind::EOF) {
        parser.advance();
    }
    parser.advance();
    parser.close(root, SyntaxKind::SOURCE_FILE);
    let (_, errors) = parser.build_tree();
    if let Some(error) = errors.into_iter().next() {
        return Err(error);
    }

    let laws = laws
        .into_iter()
        .map(|(name, span)| {
            // Keep the exact byte offsets within the declaration.
            let expected = crate::parse(&format!(
                "fn {}",
                &source[span.start as usize + "law".len()..span.end as usize]
            ));
            if let Some(error) = expected.errors().first() {
                return Err(ParseError::new(
                    &error.message,
                    Span::new(span.start + error.span.start, span.start + error.span.end),
                ));
            }
            Ok(LawDeclaration {
                name,
                span,
                expected,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(LawFile { imports, laws })
}

fn parse_import(parser: &mut Parser<'_>) -> LawImport {
    let start = parser.current_span().start;
    parser.advance(); // from
    let mut module = parser.current_text().to_owned();
    parser.expect(SyntaxKind::IDENT);
    while parser.eat(SyntaxKind::DOT) && !parser.has_error() {
        module.push('.');
        module.push_str(parser.current_text());
        parser.expect(SyntaxKind::IDENT);
    }
    parser.expect(SyntaxKind::IMPORT_KW);
    let private = parser.current_text() == "private" && parser.nth(1) == SyntaxKind::IDENT;
    if private {
        parser.advance();
    }
    let name = parser.current_text().to_owned();
    let end = parser.current_span().end;
    parser.expect(SyntaxKind::IDENT);
    LawImport {
        module,
        name,
        private,
        span: Span::new(start, end),
    }
}

fn parse_declaration(parser: &mut Parser<'_>) -> (String, Span) {
    let start = parser.current_span().start;
    parser.advance(); // law
    let name = parser.current_text().to_owned();
    if parser.expect(SyntaxKind::IDENT) {
        if parser.at(SyntaxKind::L_PAREN) {
            expressions::parse_param_list(parser);
        } else {
            parser.error("expected a law parameter list");
        }
    }
    if parser.expect(SyntaxKind::ARROW) {
        items::parse_type(parser);
    }
    if parser.expect(SyntaxKind::DO_KW) && !parser.has_error() {
        expressions::parse_block_body(parser);
    }
    let end = parser.current_span().end;
    parser.expect(SyntaxKind::END_KW);
    (name, Span::new(start, end))
}
