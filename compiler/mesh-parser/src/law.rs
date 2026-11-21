//! The law-file grammar. Ordinary Mesh source continues to use the ordinary parser.

use mesh_common::span::Span;
use mesh_diagnostics::Code;

use crate::error::code_label;
use crate::parser::{expect_statement_end, expressions, items, Parser, NESTED_TOO_DEEPLY};
use crate::{Parse, ParseError, SyntaxKind};

pub struct LawFile {
    pub imports: Vec<LawImport>,
    pub laws: Vec<LawDeclaration>,
    pub proofs: Vec<LawProof>,
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

pub struct LawProof {
    pub name: String,
    pub span: Span,
    pub term: ProofTerm,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProofTerm {
    Refl,
    Exact(String),
    Congruence {
        head: String,
        hypothesis: String,
    },
    Induction {
        variable: String,
        nil: Box<ProofTerm>,
        head: String,
        tail: String,
        hypothesis: String,
        cons: Box<ProofTerm>,
    },
    Cases {
        variable: String,
        on_false: Box<ProofTerm>,
        on_true: Box<ProofTerm>,
    },
}

/// Parse target imports, result laws, and separately declared proofs. The
/// first syntax error is `P0201`, or `P0202` past a nesting limit.
pub fn parse(source: &str) -> Result<LawFile, ParseError> {
    parse_law_file(source).map_err(law_error)
}

/// A syntax error found by the Mesh grammar, as the law file's own: the
/// label and repair of a Mesh token error still say how to fix the token.
fn law_error(error: ParseError) -> ParseError {
    let ParseError {
        code,
        message,
        span,
        related,
        label,
        repair,
    } = error;
    if code == Code::LawTooLarge || message == NESTED_TOO_DEEPLY {
        return ParseError::new(Code::LawTooLarge, message, span);
    }
    let label = label.or_else(|| code_label(code).map(Into::into));
    let repair =
        repair.or_else(|| (code != Code::UnexpectedToken).then(|| code.info().repair.into()));
    ParseError {
        code: Code::LawSyntax,
        message,
        span,
        related,
        label,
        repair,
    }
}

fn parse_law_file(source: &str) -> Result<LawFile, ParseError> {
    let mut parser = Parser::new(source);
    let root = parser.open();
    let mut imports = Vec::new();
    let mut laws = Vec::new();
    let mut proofs = Vec::new();
    parser.eat_separators();
    while !parser.at(SyntaxKind::EOF) && !parser.has_error() {
        match parser.current_text() {
            "from" if laws.is_empty() => imports.push(parse_import(&mut parser)),
            "law" => laws.push(parse_declaration(&mut parser)),
            "proof" if !laws.is_empty() => {
                if let Some(proof) = parse_proof(&mut parser) {
                    proofs.push(proof);
                }
            }
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
                let shift = |at: Span| Span::new(span.start + at.start, span.start + at.end);
                let mut error = error.clone();
                error.span = shift(error.span);
                if let Some((_, related)) = &mut error.related {
                    *related = shift(*related);
                }
                return Err(error);
            }
            Ok(LawDeclaration {
                name,
                span,
                expected,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(LawFile {
        imports,
        laws,
        proofs,
    })
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
    let opened = parser.current_span();
    if parser.expect(SyntaxKind::DO_KW) && !parser.has_error() {
        expressions::parse_block_body(parser);
    }
    let end = parser.current_span().end;
    parser.expect_end("the law body", opened);
    (name, Span::new(start, end))
}

fn parse_proof(parser: &mut Parser<'_>) -> Option<LawProof> {
    let start = parser.current_span().start;
    parser.advance(); // proof
    let name = parse_proof_ident(parser)?;
    let opened = open_proof_block(parser)?;
    let term = parse_proof_term(parser, 0)?;
    parser.eat_newlines();
    let end = parser.current_span().end;
    parser.expect_end("the proof", opened).then_some(LawProof {
        name,
        span: Span::new(start, end),
        term,
    })
}

fn parse_proof_term(parser: &mut Parser<'_>, depth: usize) -> Option<ProofTerm> {
    parser.eat_newlines();
    if depth >= 128 {
        let span = parser.current_span();
        parser.report_here(ParseError::new(
            Code::LawTooLarge,
            "proof nesting limit exceeded",
            span,
        ));
        return None;
    }
    match parser.current_text() {
        "refl" => {
            parser.advance();
            Some(ProofTerm::Refl)
        }
        "exact" => {
            parser.advance();
            parse_proof_ident(parser).map(ProofTerm::Exact)
        }
        "congr" => {
            parser.advance();
            if parse_proof_ident(parser)? != "List" {
                parser.error("expected `List.cons` in a congruence proof");
                return None;
            }
            expect_proof_token(parser, SyntaxKind::DOT)?;
            if parse_proof_ident(parser)? != "cons" {
                parser.error("expected `List.cons` in a congruence proof");
                return None;
            }
            expect_proof_token(parser, SyntaxKind::L_PAREN)?;
            let head = parse_proof_ident(parser)?;
            expect_proof_token(parser, SyntaxKind::R_PAREN)?;
            expect_proof_token(parser, SyntaxKind::COMMA)?;
            let hypothesis = parse_proof_ident(parser)?;
            Some(ProofTerm::Congruence { head, hypothesis })
        }
        "induction" => parse_induction(parser, depth + 1),
        "cases" => parse_cases(parser, depth + 1),
        _ => {
            parser.error("expected `refl`, `exact`, `congr`, `induction`, or `cases`");
            None
        }
    }
}

fn parse_induction(parser: &mut Parser<'_>, depth: usize) -> Option<ProofTerm> {
    parser.advance(); // induction
    let variable = parse_proof_ident(parser)?;
    let opened = open_proof_block(parser)?;
    expect_proof_token(parser, SyntaxKind::L_BRACKET)?;
    expect_proof_token(parser, SyntaxKind::R_BRACKET)?;
    expect_proof_token(parser, SyntaxKind::ARROW)?;
    let nil = Box::new(parse_proof_term(parser, depth)?);
    let head = parse_proof_ident(parser)?;
    expect_proof_token(parser, SyntaxKind::COLON_COLON)?;
    let tail = parse_proof_ident(parser)?;
    expect_proof_token(parser, SyntaxKind::COMMA)?;
    let hypothesis = parse_proof_ident(parser)?;
    expect_proof_token(parser, SyntaxKind::ARROW)?;
    let cons = Box::new(parse_proof_term(parser, depth)?);
    close_proof_block(parser, "the induction", opened)?;
    Some(ProofTerm::Induction {
        variable,
        nil,
        head,
        tail,
        hypothesis,
        cons,
    })
}

fn parse_cases(parser: &mut Parser<'_>, depth: usize) -> Option<ProofTerm> {
    parser.advance(); // cases
    let variable = parse_proof_ident(parser)?;
    let opened = open_proof_block(parser)?;
    expect_proof_token(parser, SyntaxKind::FALSE_KW)?;
    expect_proof_token(parser, SyntaxKind::ARROW)?;
    let on_false = Box::new(parse_proof_term(parser, depth)?);
    expect_proof_token(parser, SyntaxKind::TRUE_KW)?;
    expect_proof_token(parser, SyntaxKind::ARROW)?;
    let on_true = Box::new(parse_proof_term(parser, depth)?);
    close_proof_block(parser, "the cases", opened)?;
    Some(ProofTerm::Cases {
        variable,
        on_false,
        on_true,
    })
}

fn parse_proof_ident(parser: &mut Parser<'_>) -> Option<String> {
    parser.eat_newlines();
    let name = parser.current_text().to_owned();
    parser.expect(SyntaxKind::IDENT).then_some(name)
}

fn expect_proof_token(parser: &mut Parser<'_>, kind: SyntaxKind) -> Option<()> {
    parser.eat_newlines();
    parser.expect(kind).then_some(())
}

/// Consume a proof block's `do`, and say where it is.
fn open_proof_block(parser: &mut Parser<'_>) -> Option<Span> {
    parser.eat_newlines();
    let opened = parser.current_span();
    parser.expect(SyntaxKind::DO_KW).then_some(opened)
}

fn close_proof_block(parser: &mut Parser<'_>, what: &str, opened: Span) -> Option<()> {
    parser.eat_newlines();
    parser.expect_end(what, opened).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAW: &str = "law count() -> Bool do true end\n";

    #[test]
    fn parses_law_without_proofs() {
        let source = format!("from Stats.List import private count\n{LAW}");
        let file = parse(&source).unwrap();
        assert_eq!(file.imports.len(), 1);
        assert_eq!(file.imports[0].module, "Stats.List");
        assert_eq!(file.imports[0].name, "count");
        assert!(file.imports[0].private);
        assert_eq!(file.laws.len(), 1);
        assert_eq!(file.laws[0].name, "count");
        assert!(file.proofs.is_empty());
    }

    #[test]
    fn parses_nested_proofs_with_comments_and_whitespace() {
        let nested = "proof count do
  induction xs do # split the list
    [] -> refl
    head :: tail, ih ->
      cases flag do
        false -> exact # use the hypothesis
          ih
        true -> refl
      end
  end
end";
        let source = format!("{LAW}{nested} # after proof\nproof other do refl end\n");
        let file = parse(&source).unwrap();
        assert_eq!(file.proofs.len(), 2);
        assert_eq!(file.proofs[0].name, "count");
        assert_eq!(
            file.proofs[0].span,
            Span::new(LAW.len() as u32, (LAW.len() + nested.len()) as u32)
        );
        assert_eq!(
            file.proofs[0].term,
            ProofTerm::Induction {
                variable: "xs".into(),
                nil: Box::new(ProofTerm::Refl),
                head: "head".into(),
                tail: "tail".into(),
                hypothesis: "ih".into(),
                cons: Box::new(ProofTerm::Cases {
                    variable: "flag".into(),
                    on_false: Box::new(ProofTerm::Exact("ih".into())),
                    on_true: Box::new(ProofTerm::Refl),
                }),
            }
        );
        assert_eq!(file.proofs[1].name, "other");
        assert_eq!(file.proofs[1].term, ProofTerm::Refl);
    }

    #[test]
    fn rejects_malformed_proofs_at_the_unexpected_token() {
        for (block, unexpected) in [
            ("proof count do unknown end", "unknown"),
            ("proof count do exact 1 end", "1"),
            ("proof count do cases flag do false -> refl end", "end"),
            ("proof count do cases flag do true -> refl end", "true"),
            ("proof count do induction xs do [] -> refl end", "end"),
            (
                "proof count do induction xs do h :: t, ih -> refl end",
                "h ::",
            ),
            ("proof count do cases flag do false => refl end", "=>"),
            ("proof count do refl junk end", "junk"),
            ("proof count do refl end junk", "junk"),
            ("proof count do refl", ""),
        ] {
            let source = format!("{LAW}{block}");
            let error = parse(&source).err().expect("malformed proof parsed");
            assert_eq!(error.code, Code::LawSyntax, "{block}: {error:?}");
            assert_eq!(
                error.span.start,
                source.rfind(unexpected).unwrap() as u32,
                "{block}: {error:?}"
            );
        }
    }

    #[test]
    fn an_unclosed_block_names_the_do_it_closes() {
        for (source, opened) in [
            ("law count() -> Bool do\n  true\n", "do\n  true"),
            (
                "law count() -> Bool do true end\nproof count do\n  refl\n",
                "do\n  refl",
            ),
            (
                "law count() -> Bool do true end\nproof count do\n  cases flag do\n    false -> refl\n    true -> refl\nend\n",
                "do\n  cases",
            ),
        ] {
            let error = parse(source).err().expect("unclosed block parsed");
            assert_eq!(error.code, Code::LawSyntax, "{source}: {error:?}");
            let start = source.find(opened).unwrap() as u32;
            assert_eq!(
                error.related,
                Some(("block opened here".into(), Span::new(start, start + 2))),
                "{source}"
            );
            assert_eq!(
                error.repair.as_deref(),
                Some(Code::MissingEnd.info().repair)
            );
            assert_eq!(error.label.as_deref(), Some("expected `end` here"));
        }
    }

    #[test]
    fn law_errors_keep_the_repair_of_the_token_error() {
        let source = "from Stats import count\nlaw count() -> Bool do count(true end\n";
        let error = parse(source).err().expect("an unclosed call parsed");
        assert_eq!(error.code, Code::LawSyntax);
        assert_eq!(
            error.related.as_ref().map(|(message, _)| message.as_str()),
            Some("opened here")
        );
        assert_eq!(
            error.repair.as_deref(),
            Some(Code::UnclosedDelimiter.info().repair)
        );
        assert_eq!(
            error.label.as_deref(),
            Some("closing delimiter expected here")
        );

        let source = "from Stats import count\nlaw count() -> Bool do \"open end\n";
        let error = parse(source).err().expect("an unterminated string parsed");
        assert_eq!(error.code, Code::LawSyntax);
        assert_eq!(
            error.repair.as_deref(),
            Some(Code::UnterminatedString.info().repair)
        );

        let error = parse("law count() -> Bool do true end\njunk\n")
            .err()
            .unwrap();
        assert_eq!(error.code, Code::LawSyntax);
        assert_eq!(error.repair, None, "the law grammar's repair");
    }

    #[test]
    fn nesting_past_a_limit_is_too_large() {
        let deep_proof = format!(
            "{LAW}proof count do {}refl{} end",
            "cases flag do false -> refl\ntrue -> ".repeat(130),
            " end".repeat(130)
        );
        let deep_body = format!(
            "law count() -> Bool do {}true{} end\n",
            "(".repeat(200),
            ")".repeat(200)
        );
        for source in [deep_proof, deep_body] {
            let error = parse(&source).err().expect("a deep law parsed");
            assert_eq!(error.code, Code::LawTooLarge, "{error:?}");
            assert_eq!(error.repair, None, "{error:?}");
        }
    }
}
