//! The law-file grammar. Ordinary Mesh source continues to use the ordinary parser.

use mesh_common::span::Span;

use crate::parser::{expect_statement_end, expressions, items, Parser};
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

/// Parse target imports, result laws, and separately declared proofs.
pub fn parse(source: &str) -> Result<LawFile, ParseError> {
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
    if parser.expect(SyntaxKind::DO_KW) && !parser.has_error() {
        expressions::parse_block_body(parser);
    }
    let end = parser.current_span().end;
    parser.expect(SyntaxKind::END_KW);
    (name, Span::new(start, end))
}

fn parse_proof(parser: &mut Parser<'_>) -> Option<LawProof> {
    let start = parser.current_span().start;
    parser.advance(); // proof
    let name = parse_proof_ident(parser)?;
    expect_proof_token(parser, SyntaxKind::DO_KW)?;
    let term = parse_proof_term(parser, 0)?;
    parser.eat_newlines();
    let end = parser.current_span().end;
    parser.expect(SyntaxKind::END_KW).then_some(LawProof {
        name,
        span: Span::new(start, end),
        term,
    })
}

fn parse_proof_term(parser: &mut Parser<'_>, depth: usize) -> Option<ProofTerm> {
    parser.eat_newlines();
    if depth >= 128 {
        parser.error("proof nesting limit exceeded");
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
    expect_proof_token(parser, SyntaxKind::DO_KW)?;
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
    expect_proof_token(parser, SyntaxKind::END_KW)?;
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
    expect_proof_token(parser, SyntaxKind::DO_KW)?;
    expect_proof_token(parser, SyntaxKind::FALSE_KW)?;
    expect_proof_token(parser, SyntaxKind::ARROW)?;
    let on_false = Box::new(parse_proof_term(parser, depth)?);
    expect_proof_token(parser, SyntaxKind::TRUE_KW)?;
    expect_proof_token(parser, SyntaxKind::ARROW)?;
    let on_true = Box::new(parse_proof_term(parser, depth)?);
    expect_proof_token(parser, SyntaxKind::END_KW)?;
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
            assert_eq!(
                error.span.start,
                source.rfind(unexpected).unwrap() as u32,
                "{block}: {error:?}"
            );
        }
    }
}
