//! Parser for Mesh law files headed by a standalone `logic` marker.
//!
//! `from Module import name` entries precede declarations. An import can
//! specialize a nominal type as `Name<T>` or specify a callable signature as
//! `name(T) -> U`. Imports accept an optional `as alias` suffix. Newlines or `;`
//! separate declarations, constructors, match and proof arms, and a let value
//! from its body. Type annotations accept `List<T>` and terms can apply the same type
//! with `List(T)`. Parentheses also enclose ascriptions `(value : Type)` and
//! pairs `(first, second)`. `=>` introduces an implication and `->` introduces
//! a function type.

use mesh_common::span::Span;
use mesh_common::token::{Token, TokenKind};
use mesh_lexer::Lexer;

use crate::ParseError;

#[derive(Clone, Debug, PartialEq)]
pub struct LogicFile {
    pub imports: Vec<LogicImport>,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogicImport {
    pub module: String,
    pub name: String,
    pub arguments: Vec<Term>,
    pub signature: Option<(Vec<Term>, Term)>,
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binder {
    pub name: String,
    pub ty: Term,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Constructor {
    pub name: String,
    pub fields: Vec<Binder>,
    pub result: Option<Term>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Declaration {
    Inductive {
        name: String,
        params: Vec<Binder>,
        kind: Option<Term>,
        constructors: Vec<Constructor>,
        span: Span,
    },
    Definition {
        name: String,
        params: Vec<Binder>,
        result: Term,
        body: Term,
        span: Span,
    },
    Theorem {
        name: String,
        public: bool,
        params: Vec<Binder>,
        proposition: Term,
        span: Span,
    },
    Proof {
        name: String,
        term: Proof,
        span: Span,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Name(Vec<String>),
    Nat(u64),
    Bool(bool),
    Call(Box<Term>, Vec<Term>),
    Binary(Binary, Box<Term>, Box<Term>),
    Forall(Box<Binder>, Box<Term>),
    Exists(Box<Binder>, Box<Term>),
    Lambda(Box<Binder>, Box<Term>),
    Annotate(Box<Term>, Box<Term>),
    Let(Box<Binder>, Box<Term>, Box<Term>),
    Pair(Box<Term>, Box<Term>),
    If(Box<Term>, Box<Term>, Box<Term>),
    Match(Box<Term>, Vec<Arm<Term>>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Binary {
    Eq,
    Ne,
    And,
    Or,
    Implies,
    Arrow,
    Add,
    Sub,
    Mul,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arm<T> {
    pub pattern: Pattern,
    pub body: T,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    Constructor(Vec<String>, Vec<String>),
    Wildcard,
    Bool(bool),
    Nat(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Proof {
    Refl,
    Goal,
    Exact(Term),
    Intro(String, Box<Proof>),
    Split(Box<Proof>, Box<Proof>),
    Witness(Term, Box<Proof>),
    Cases(String, Vec<Arm<Proof>>),
    Induction(String, Vec<Arm<Proof>>),
    Rewrite(Term, Box<Proof>),
    Branch(Term, String, Box<Proof>, String, Box<Proof>),
    Change(Term, Box<Proof>),
    Unfold(Vec<String>, Box<Proof>),
    Left(Box<Proof>),
    Right(Box<Proof>),
}

const MAX_NESTING: usize = 64;
const ARROW_PRECEDENCE: u8 = 1;

/// Check for `logic` as the only non-comment token on the first significant line.
pub fn is_logic_file(source: &str) -> bool {
    Parser::new(source).is_ok_and(|mut parser| parser.marker().is_ok())
}

/// Parse a complete logic law file, with spans relative to the original source.
pub fn parse(source: &str) -> Result<LogicFile, ParseError> {
    let mut parser = Parser::new(source)?;
    parser.marker()?;
    parse_declarations(parser)
}

/// Parse the contents of a `logic do ... end` block in a Mesh source file.
/// Byte ranges start at the beginning of the supplied block body.
pub fn parse_body(source: &str) -> Result<LogicFile, ParseError> {
    parse_declarations(Parser::new(source)?)
}

fn parse_declarations(mut parser: Parser<'_>) -> Result<LogicFile, ParseError> {
    parser.separators();
    let mut imports = Vec::new();
    while parser.at_word("from") {
        imports.push(parser.import()?);
        parser.statement_end(TokenKind::Eof)?;
    }

    let mut declarations = Vec::new();
    while !parser.at(TokenKind::Eof) {
        declarations.push(parser.declaration()?);
        parser.statement_end(TokenKind::Eof)?;
    }
    if declarations.is_empty() {
        return Err(parser.error("expected a logic declaration"));
    }
    Ok(LogicFile {
        imports,
        declarations,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TermContext {
    Value,
    Type,
    TypeArgument,
}

struct ParsedTerm {
    term: Term,
    height: usize,
}

impl ParsedTerm {
    fn leaf(term: Term) -> Self {
        Self { term, height: 1 }
    }

    fn above(term: Term, child_height: usize) -> Self {
        Self {
            term,
            height: child_height + 1,
        }
    }
}

struct Parser<'src> {
    lexer: Lexer<'src>,
    source: &'src str,
    current: Token,
    previous_end: u32,
}

impl<'src> Parser<'src> {
    fn new(source: &'src str) -> Result<Self, ParseError> {
        if u32::try_from(source.len()).is_err() {
            return Err(ParseError::new(
                "logic file exceeds the source size limit",
                Span::new(0, 0),
            ));
        }
        let mut lexer = Lexer::new(source);
        let current = Self::next_token(&mut lexer);
        Ok(Self {
            lexer,
            source,
            current,
            previous_end: 0,
        })
    }

    fn next_token(lexer: &mut Lexer<'src>) -> Token {
        lexer
            .find(|token| {
                !matches!(
                    token.kind,
                    TokenKind::Comment | TokenKind::DocComment | TokenKind::ModuleDocComment
                )
            })
            .expect("Mesh lexer emits an EOF token")
    }

    fn text(&self) -> &'src str {
        let span = self.current.span;
        &self.source[span.start as usize..span.end as usize]
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current.kind == kind
    }

    fn at_word(&self, word: &str) -> bool {
        self.at(TokenKind::Ident) && self.text() == word
    }

    fn advance(&mut self) {
        self.previous_end = self.current.span.end;
        if !self.at(TokenKind::Eof) {
            self.current = Self::next_token(&mut self.lexer);
        }
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError::new(message, self.current.span)
    }

    fn expect(&mut self, kind: TokenKind, spelling: &str) -> Result<(), ParseError> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.error(format!("expected `{spelling}`")))
        }
    }

    fn newlines(&mut self) {
        while self.eat(TokenKind::Newline) {}
    }

    fn separators(&mut self) {
        while self.eat(TokenKind::Newline) || self.eat(TokenKind::Semicolon) {}
    }

    fn statement_end(&mut self, closing: TokenKind) -> Result<(), ParseError> {
        if self.at(closing) {
            return Ok(());
        }
        if !matches!(self.current.kind, TokenKind::Newline | TokenKind::Semicolon) {
            return Err(self.error("expected a newline or `;` between entries"));
        }
        self.separators();
        Ok(())
    }

    fn marker(&mut self) -> Result<(), ParseError> {
        self.newlines();
        if !self.at_word("logic") {
            return Err(self.error("expected the `logic` file marker"));
        }
        self.advance();
        if !matches!(self.current.kind, TokenKind::Newline | TokenKind::Eof) {
            return Err(self.error("expected a newline after the `logic` file marker"));
        }
        Ok(())
    }

    fn ident(&mut self) -> Result<String, ParseError> {
        if !self.at(TokenKind::Ident) || matches!(self.text(), "_" | "forall" | "exists") {
            return Err(self.error("expected a name"));
        }
        let name = self.text().to_owned();
        self.advance();
        Ok(name)
    }

    fn path(&mut self) -> Result<Vec<String>, ParseError> {
        let mut path = vec![self.path_segment()?];
        while self.eat(TokenKind::Dot) {
            path.push(self.path_segment()?);
        }
        Ok(path)
    }

    fn path_segment(&mut self) -> Result<String, ParseError> {
        // Mesh lexes `nil` as a literal. In logic it names the list constructor.
        if self.eat(TokenKind::Nil) {
            Ok("nil".to_owned())
        } else {
            self.ident()
        }
    }

    fn nat(&mut self) -> Result<u64, ParseError> {
        let text = self.text();
        let (radix, digits) = match text.as_bytes() {
            [b'0', b'x' | b'X', ..] => (16, &text[2..]),
            [b'0', b'o' | b'O', ..] => (8, &text[2..]),
            [b'0', b'b' | b'B', ..] => (2, &text[2..]),
            _ => (10, text),
        };
        let number = u64::from_str_radix(&digits.replace('_', ""), radix)
            .map_err(|_| self.error("expected a valid natural number in the u64 range"))?;
        self.advance();
        Ok(number)
    }

    fn import(&mut self) -> Result<LogicImport, ParseError> {
        let start = self.current.span.start;
        self.advance(); // from
        let module = self.path()?.join(".");
        self.expect(TokenKind::Import, "import")?;
        let name = self.ident()?;
        let (arguments, signature) = match self.current.kind {
            TokenKind::Diamond => return Err(self.error("expected a type argument")),
            TokenKind::Lt => {
                self.advance();
                let arguments = self
                    .type_arguments(1)?
                    .into_iter()
                    .map(|argument| argument.term)
                    .collect();
                if self.at(TokenKind::LParen) {
                    return Err(self.error("a nominal import cannot specify a callable signature"));
                }
                (arguments, None)
            }
            TokenKind::LParen => {
                self.advance();
                let params = self.comma_list(TokenKind::RParen, ")", |parser| {
                    Ok(parser.term(0, 0, TermContext::Type)?.term)
                })?;
                self.expect(TokenKind::Arrow, "->")?;
                let result = self.term(0, 0, TermContext::Type)?.term;
                (Vec::new(), Some((params, result)))
            }
            _ => (Vec::new(), None),
        };
        let alias = if self.at_word("as") {
            self.advance();
            Some(self.ident()?)
        } else {
            None
        };
        Ok(LogicImport {
            module,
            name,
            arguments,
            signature,
            alias,
            span: Span::new(start, self.previous_end),
        })
    }

    fn declaration(&mut self) -> Result<Declaration, ParseError> {
        let start = self.current.span.start;
        let public = self.eat(TokenKind::Pub);
        if public && !self.at_word("theorem") {
            return Err(self.error("expected a theorem after `pub`"));
        }
        if self.at_word("inductive") {
            self.advance();
            let name = self.ident()?;
            let params = self.optional_binders()?;
            let kind = self.optional_arrow_type()?;
            self.open_block()?;
            let constructors = self.block_entries(true, Self::constructor)?;
            Ok(Declaration::Inductive {
                name,
                params,
                kind,
                constructors,
                span: Span::new(start, self.previous_end),
            })
        } else if self.eat(TokenKind::Fn) {
            let name = self.ident()?;
            let params = self.binders()?;
            self.expect(TokenKind::Arrow, "->")?;
            let result = self.term(0, 0, TermContext::Type)?.term;
            self.open_block()?;
            let body = self.term(0, 0, TermContext::Value)?.term;
            self.close_block()?;
            Ok(Declaration::Definition {
                name,
                params,
                result,
                body,
                span: Span::new(start, self.previous_end),
            })
        } else if self.at_word("theorem") {
            self.advance();
            let name = self.ident()?;
            let params = self.binders()?;
            self.open_block()?;
            let proposition = self.term(0, 0, TermContext::Value)?.term;
            self.close_block()?;
            Ok(Declaration::Theorem {
                name,
                public,
                params,
                proposition,
                span: Span::new(start, self.previous_end),
            })
        } else if self.at_word("proof") {
            self.advance();
            let name = self.ident()?;
            self.open_block()?;
            let term = self.proof(0)?;
            self.close_block()?;
            Ok(Declaration::Proof {
                name,
                term,
                span: Span::new(start, self.previous_end),
            })
        } else {
            Err(self.error("expected an `inductive`, `fn`, `theorem`, or `proof` declaration"))
        }
    }

    fn constructor(&mut self) -> Result<Constructor, ParseError> {
        let name = self.ident()?;
        let fields = self.optional_binders()?;
        let result = self.optional_arrow_type()?;
        Ok(Constructor {
            name,
            fields,
            result,
        })
    }

    fn optional_arrow_type(&mut self) -> Result<Option<Term>, ParseError> {
        if self.eat(TokenKind::Arrow) {
            Ok(Some(self.term(0, 0, TermContext::Type)?.term))
        } else {
            Ok(None)
        }
    }

    fn optional_binders(&mut self) -> Result<Vec<Binder>, ParseError> {
        if self.at(TokenKind::LParen) {
            self.binders()
        } else {
            Ok(Vec::new())
        }
    }

    fn binders(&mut self) -> Result<Vec<Binder>, ParseError> {
        self.expect(TokenKind::LParen, "(")?;
        self.comma_list(TokenKind::RParen, ")", |parser| {
            let (name, ty) = parser.typed_name(0, 0)?;
            Ok(Binder { name, ty: ty.term })
        })
    }

    fn typed_name(
        &mut self,
        depth: usize,
        precedence: u8,
    ) -> Result<(String, ParsedTerm), ParseError> {
        let name = self.ident()?;
        self.expect(TokenKind::ColonColon, "::")?;
        Ok((name, self.term(depth, precedence, TermContext::Type)?))
    }

    fn open_block(&mut self) -> Result<(), ParseError> {
        self.newlines();
        self.expect(TokenKind::Do, "do")
    }

    fn close_block(&mut self) -> Result<(), ParseError> {
        self.newlines();
        self.expect(TokenKind::End, "end")
    }

    fn block_entries<T>(
        &mut self,
        allow_empty: bool,
        mut entry: impl FnMut(&mut Self) -> Result<T, ParseError>,
    ) -> Result<Vec<T>, ParseError> {
        self.separators();
        if allow_empty && self.eat(TokenKind::End) {
            return Ok(Vec::new());
        }
        let mut entries = vec![entry(self)?];
        while !self.eat(TokenKind::End) {
            self.statement_end(TokenKind::End)?;
            if !self.at(TokenKind::End) {
                entries.push(entry(self)?);
            }
        }
        Ok(entries)
    }

    fn comma_list<T>(
        &mut self,
        closing: TokenKind,
        spelling: &str,
        mut item: impl FnMut(&mut Self) -> Result<T, ParseError>,
    ) -> Result<Vec<T>, ParseError> {
        let mut entries = Vec::new();
        self.newlines();
        while !self.at(closing.clone()) {
            entries.push(item(self)?);
            self.newlines();
            if !self.eat(TokenKind::Comma) {
                break;
            }
            self.newlines();
        }
        self.expect(closing, spelling)?;
        Ok(entries)
    }

    fn type_arguments(&mut self, depth: usize) -> Result<Vec<ParsedTerm>, ParseError> {
        self.newlines();
        if self.at(TokenKind::Gt) {
            return Err(self.error("expected a type argument"));
        }
        self.comma_list(TokenKind::Gt, ">", |parser| {
            parser.term(depth, 0, TermContext::TypeArgument)
        })
    }

    fn check_nesting(&self, depth: usize) -> Result<(), ParseError> {
        if depth >= MAX_NESTING {
            Err(self.error("logic nesting limit exceeded"))
        } else {
            Ok(())
        }
    }

    fn term(
        &mut self,
        depth: usize,
        precedence: u8,
        context: TermContext,
    ) -> Result<ParsedTerm, ParseError> {
        self.newlines();
        self.check_nesting(depth)?;
        let mut lhs = self.atom(depth + 1, context)?;
        self.check_nesting(depth + lhs.height - 1)?;
        loop {
            if self.eat(TokenKind::LParen) {
                let args = self.comma_list(TokenKind::RParen, ")", |parser| {
                    parser.term(depth + 1, 0, TermContext::Value)
                })?;
                lhs = Self::call(lhs, args);
            } else if let Some((operator, left, right)) = self.binary(context) {
                if left < precedence {
                    break;
                }
                self.advance();
                let rhs = self.term(depth + 1, right, context)?;
                let height = lhs.height.max(rhs.height);
                lhs = ParsedTerm::above(
                    Term::Binary(operator, Box::new(lhs.term), Box::new(rhs.term)),
                    height,
                );
            } else {
                break;
            }
            self.check_nesting(depth + lhs.height - 1)?;
        }
        Ok(lhs)
    }

    fn call(head: ParsedTerm, args: Vec<ParsedTerm>) -> ParsedTerm {
        let height = args
            .iter()
            .fold(head.height, |height, arg| height.max(arg.height));
        let args = args.into_iter().map(|arg| arg.term).collect();
        ParsedTerm::above(Term::Call(Box::new(head.term), args), height)
    }

    fn binary(&self, context: TermContext) -> Option<(Binary, u8, u8)> {
        let (op, precedence) = match self.current.kind {
            TokenKind::Arrow => return Some((Binary::Arrow, ARROW_PRECEDENCE, ARROW_PRECEDENCE)),
            TokenKind::FatArrow => return Some((Binary::Implies, 2, 2)),
            TokenKind::PipePipe => (Binary::Or, 3),
            TokenKind::AmpAmp => (Binary::And, 4),
            TokenKind::EqEq => (Binary::Eq, 5),
            TokenKind::NotEq => (Binary::Ne, 5),
            TokenKind::Lt => (Binary::Lt, 6),
            TokenKind::LtEq => (Binary::Le, 6),
            TokenKind::Gt if context != TermContext::TypeArgument => (Binary::Gt, 6),
            TokenKind::GtEq => (Binary::Ge, 6),
            TokenKind::Plus => (Binary::Add, 7),
            TokenKind::Minus => (Binary::Sub, 7),
            TokenKind::Star => (Binary::Mul, 8),
            _ => return None,
        };
        Some((op, precedence, precedence + 1))
    }

    fn atom(&mut self, depth: usize, context: TermContext) -> Result<ParsedTerm, ParseError> {
        match self.current.kind {
            TokenKind::IntLiteral => Ok(ParsedTerm::leaf(Term::Nat(self.nat()?))),
            TokenKind::True | TokenKind::False => {
                let value = self.at(TokenKind::True);
                self.advance();
                Ok(ParsedTerm::leaf(Term::Bool(value)))
            }
            TokenKind::LParen => self.parenthesized_term(depth, context),
            TokenKind::Let => self.let_term(depth, context),
            TokenKind::If => self.if_term(depth),
            TokenKind::Match | TokenKind::Case => self.match_term(depth),
            TokenKind::Fn => self.binder_term(depth, "fn"),
            TokenKind::Ident if matches!(self.text(), "forall" | "exists") => {
                self.binder_term(depth, self.text())
            }
            TokenKind::Ident | TokenKind::Nil => {
                let head = ParsedTerm::leaf(Term::Name(self.path()?));
                // Angles apply a type in annotations. Expression arguments can
                // use ordinary call syntax for the same application.
                if context != TermContext::Value
                    && self.current.span.start == self.previous_end
                    && self.eat(TokenKind::Lt)
                {
                    Ok(Self::call(head, self.type_arguments(depth)?))
                } else {
                    Ok(head)
                }
            }
            _ => Err(self.error("expected a logic term")),
        }
    }

    fn parenthesized_term(
        &mut self,
        depth: usize,
        context: TermContext,
    ) -> Result<ParsedTerm, ParseError> {
        self.advance();
        let inner_context = match context {
            TermContext::Value => TermContext::Value,
            _ => TermContext::Type,
        };
        let first = self.term(depth, 0, inner_context)?;
        self.newlines();
        let term = if self.eat(TokenKind::Colon) {
            let ty = self.term(depth, 0, TermContext::Type)?;
            let height = first.height.max(ty.height);
            ParsedTerm::above(
                Term::Annotate(Box::new(first.term), Box::new(ty.term)),
                height,
            )
        } else if self.eat(TokenKind::Comma) {
            let second = self.term(depth, 0, inner_context)?;
            let height = first.height.max(second.height);
            ParsedTerm::above(
                Term::Pair(Box::new(first.term), Box::new(second.term)),
                height,
            )
        } else {
            first
        };
        self.newlines();
        self.expect(TokenKind::RParen, ")")?;
        Ok(term)
    }

    fn let_term(&mut self, depth: usize, context: TermContext) -> Result<ParsedTerm, ParseError> {
        self.advance();
        let (name, ty) = self.typed_name(depth, 0)?;
        self.expect(TokenKind::Eq, "=")?;
        let value = self.term(depth, 0, TermContext::Value)?;
        if !matches!(self.current.kind, TokenKind::Newline | TokenKind::Semicolon) {
            return Err(self.error("expected a newline or `;` before the let body"));
        }
        self.separators();
        let body = self.term(depth, 0, context)?;
        let height = ty.height.max(value.height).max(body.height);
        let binder = Box::new(Binder { name, ty: ty.term });
        Ok(ParsedTerm::above(
            Term::Let(binder, Box::new(value.term), Box::new(body.term)),
            height,
        ))
    }

    fn binder_term(&mut self, depth: usize, keyword: &str) -> Result<ParsedTerm, ParseError> {
        self.advance();
        let is_lambda = keyword == "fn";
        let precedence = if is_lambda { ARROW_PRECEDENCE + 1 } else { 0 };
        let (name, ty) = self.typed_name(depth, precedence)?;
        if is_lambda {
            self.expect(TokenKind::Arrow, "->")?;
        } else {
            self.open_block()?;
        }
        let body = self.term(depth, 0, TermContext::Value)?;
        self.close_block()?;
        let height = ty.height.max(body.height);
        let binder = Box::new(Binder { name, ty: ty.term });
        let body = Box::new(body.term);
        let term = match keyword {
            "forall" => Term::Forall(binder, body),
            "exists" => Term::Exists(binder, body),
            _ => Term::Lambda(binder, body),
        };
        Ok(ParsedTerm::above(term, height))
    }

    fn if_term(&mut self, depth: usize) -> Result<ParsedTerm, ParseError> {
        self.advance();
        let condition = self.term(depth, 0, TermContext::Value)?;
        self.open_block()?;
        let yes = self.term(depth, 0, TermContext::Value)?;
        self.newlines();
        self.expect(TokenKind::Else, "else")?;
        let no = self.term(depth, 0, TermContext::Value)?;
        self.close_block()?;
        let height = condition.height.max(yes.height).max(no.height);
        Ok(ParsedTerm::above(
            Term::If(
                Box::new(condition.term),
                Box::new(yes.term),
                Box::new(no.term),
            ),
            height,
        ))
    }

    fn match_term(&mut self, depth: usize) -> Result<ParsedTerm, ParseError> {
        self.advance();
        let subject = self.term(depth, 0, TermContext::Value)?;
        self.open_block()?;
        let mut height = subject.height;
        let arms = self.block_entries(false, |parser| {
            parser.arm(|parser| {
                let body = parser.term(depth, 0, TermContext::Value)?;
                height = height.max(body.height);
                Ok(body.term)
            })
        })?;
        Ok(ParsedTerm::above(
            Term::Match(Box::new(subject.term), arms),
            height,
        ))
    }

    fn arm<T>(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<Arm<T>, ParseError> {
        let pattern = self.pattern()?;
        self.expect(TokenKind::Arrow, "->")?;
        Ok(Arm {
            pattern,
            body: body(self)?,
        })
    }

    fn pattern(&mut self) -> Result<Pattern, ParseError> {
        match self.current.kind {
            TokenKind::IntLiteral => self.nat().map(Pattern::Nat),
            TokenKind::True | TokenKind::False => {
                let value = self.at(TokenKind::True);
                self.advance();
                Ok(Pattern::Bool(value))
            }
            TokenKind::Ident if self.at_word("_") => {
                self.advance();
                Ok(Pattern::Wildcard)
            }
            TokenKind::Ident | TokenKind::Nil => {
                let constructor = self.path()?;
                let fields = if self.eat(TokenKind::LParen) {
                    self.comma_list(TokenKind::RParen, ")", |parser| {
                        if parser.at_word("_") {
                            parser.advance();
                            Ok("_".to_owned())
                        } else {
                            parser.ident()
                        }
                    })?
                } else {
                    Vec::new()
                };
                Ok(Pattern::Constructor(constructor, fields))
            }
            _ => Err(self.error("expected a logic pattern")),
        }
    }

    fn proof(&mut self, depth: usize) -> Result<Proof, ParseError> {
        self.newlines();
        self.check_nesting(depth)?;
        let span = self.current.span;
        let command = self.text();
        if !self.at(TokenKind::Ident) {
            return Err(self.error("expected a logic proof term"));
        }
        self.advance();
        let child = depth + 1;
        match command {
            "refl" => Ok(Proof::Refl),
            "goal" => Ok(Proof::Goal),
            "exact" => self
                .term(child, 0, TermContext::Value)
                .map(|term| Proof::Exact(term.term)),
            "intro" => {
                let name = self.ident()?;
                Ok(Proof::Intro(name, self.proof_block(child)?))
            }
            "split" => {
                self.open_block()?;
                let left = Box::new(self.proof(child)?);
                self.newlines();
                self.expect(TokenKind::Comma, ",")?;
                let right = Box::new(self.proof(child)?);
                self.close_block()?;
                Ok(Proof::Split(left, right))
            }
            "witness" | "rewrite" => {
                let term = self.term(child, 0, TermContext::Value)?.term;
                let body = self.proof_block(child)?;
                if command == "witness" {
                    Ok(Proof::Witness(term, body))
                } else {
                    Ok(Proof::Rewrite(term, body))
                }
            }
            "branch" => {
                let proposition = self.term(child, 0, TermContext::Value)?.term;
                self.open_block()?;
                self.separators();
                let (yes_name, yes_proof) = self.branch_arm("yes", child)?;
                self.statement_end(TokenKind::End)?;
                let (no_name, no_proof) = self.branch_arm("no", child)?;
                self.separators();
                self.expect(TokenKind::End, "end")?;
                Ok(Proof::Branch(
                    proposition,
                    yes_name,
                    yes_proof,
                    no_name,
                    no_proof,
                ))
            }
            "change" => {
                let proposition = self.term(child, 0, TermContext::Value)?.term;
                Ok(Proof::Change(proposition, self.proof_block(child)?))
            }
            "unfold" => {
                let path = self.path()?;
                Ok(Proof::Unfold(path, self.proof_block(child)?))
            }
            "cases" | "induction" => {
                let name = self.ident()?;
                self.open_block()?;
                let is_cases = command == "cases";
                let arms = self
                    .block_entries(is_cases, |parser| parser.arm(|parser| parser.proof(child)))?;
                if is_cases {
                    Ok(Proof::Cases(name, arms))
                } else {
                    Ok(Proof::Induction(name, arms))
                }
            }
            "left" => Ok(Proof::Left(self.proof_block(child)?)),
            "right" => Ok(Proof::Right(self.proof_block(child)?)),
            _ => Err(ParseError::new("expected a logic proof term", span)),
        }
    }

    fn branch_arm(
        &mut self,
        label: &str,
        depth: usize,
    ) -> Result<(String, Box<Proof>), ParseError> {
        if !self.at_word(label) {
            return Err(self.error(format!("expected `{label}`")));
        }
        self.advance();
        self.expect(TokenKind::LParen, "(")?;
        let name = self.ident()?;
        self.expect(TokenKind::RParen, ")")?;
        self.expect(TokenKind::Arrow, "->")?;
        Ok((name, Box::new(self.proof(depth)?)))
    }

    fn proof_block(&mut self, depth: usize) -> Result<Box<Proof>, ParseError> {
        self.open_block()?;
        let proof = self.proof(depth)?;
        self.close_block()?;
        Ok(Box::new(proof))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(path: &str) -> Term {
        Term::Name(path.split('.').map(str::to_owned).collect())
    }

    fn call(path: &str, args: Vec<Term>) -> Term {
        Term::Call(Box::new(name(path)), args)
    }

    fn binary(op: Binary, lhs: Term, rhs: Term) -> Term {
        Term::Binary(op, Box::new(lhs), Box::new(rhs))
    }

    fn binder(id: &str, ty: Term) -> Binder {
        Binder {
            name: id.to_owned(),
            ty,
        }
    }

    fn proposition(source: &str) -> Term {
        let declarations: [Declaration; 1] =
            parse(&format!("logic\ntheorem example() do {source} end"))
                .unwrap()
                .declarations
                .try_into()
                .unwrap();
        let [Declaration::Theorem { proposition, .. }] = declarations else {
            panic!("expected a theorem");
        };
        proposition
    }

    fn spanned(source: &str, span: Span) -> &str {
        &source[span.start as usize..span.end as usize]
    }

    #[test]
    fn recognizes_only_the_first_significant_line_as_the_marker() {
        assert!(is_logic_file(
            " \r\n##! Proofs\r\n  logic # marker\r\ntheorem t() do true end"
        ));
        assert!(is_logic_file("#= leading #= nested =# comment =#\nlogic"));
        for source in [
            "",
            "logical\ntheorem t() do true end",
            "logic theorem t() do true end",
            "logic;\ntheorem t() do true end",
            "# logic\nlaw example() -> Bool do true end",
            "from Policy import f\nlogic\ntheorem t() do true end",
        ] {
            assert!(!is_logic_file(source), "{source}");
            assert!(parse(source).is_err(), "{source}");
        }
    }

    #[test]
    fn parses_declarations_and_original_byte_ranges() {
        let source = "## Árbol
logic
from Domain.Trees import size # a program function
inductive Tree(T :: Type) do
  Leaf(value :: T)
  Branch(left :: Tree<T>, right :: Tree<T>)
end
fn singleton(value :: Nat,) -> Tree<Nat> do Tree.Leaf(value) end
theorem always(x :: Nat) do x == x end
proof always do refl end
";
        let parsed = parse(source).unwrap();
        let [import] = parsed.imports.as_slice() else {
            panic!("expected one import");
        };
        assert_eq!(import.module, "Domain.Trees");
        assert_eq!(import.name, "size");
        assert!(import.arguments.is_empty());
        assert_eq!(import.signature, None);
        assert_eq!(import.alias, None);
        assert_eq!(
            spanned(source, import.span),
            "from Domain.Trees import size"
        );
        let [Declaration::Inductive {
            name: type_name,
            params,
            kind,
            constructors,
            span,
        }, Declaration::Definition {
            name: def,
            params: arguments,
            result,
            body,
            span: def_span,
        }, Declaration::Theorem {
            proposition,
            span: theorem_span,
            ..
        }, Declaration::Proof {
            name: proof,
            term,
            span: proof_span,
        }] = parsed.declarations.as_slice()
        else {
            panic!("unexpected declarations: {:?}", parsed.declarations);
        };
        assert_eq!(type_name, "Tree");
        assert_eq!(*params, [binder("T", name("Type"))]);
        assert_eq!(*kind, None);
        assert_eq!(
            *constructors,
            [
                Constructor {
                    name: "Leaf".into(),
                    fields: vec![binder("value", name("T"))],
                    result: None,
                },
                Constructor {
                    name: "Branch".into(),
                    fields: vec![
                        binder("left", call("Tree", vec![name("T")])),
                        binder("right", call("Tree", vec![name("T")])),
                    ],
                    result: None,
                },
            ],
        );
        assert_eq!(def, "singleton");
        assert_eq!(*arguments, [binder("value", name("Nat"))]);
        assert_eq!(*result, call("Tree", vec![name("Nat")]));
        assert_eq!(*body, call("Tree.Leaf", vec![name("value")]));
        assert_eq!(*proposition, binary(Binary::Eq, name("x"), name("x")));
        assert_eq!(proof, "always");
        assert_eq!(*term, Proof::Refl);
        assert_eq!(
            spanned(source, *span),
            "inductive Tree(T :: Type) do\n  Leaf(value :: T)\n  Branch(left :: Tree<T>, right :: Tree<T>)\nend"
        );
        assert_eq!(
            spanned(source, *def_span),
            "fn singleton(value :: Nat,) -> Tree<Nat> do Tree.Leaf(value) end"
        );
        assert_eq!(
            spanned(source, *theorem_span),
            "theorem always(x :: Nat) do x == x end"
        );
        assert_eq!(spanned(source, *proof_span), "proof always do refl end");
    }

    #[test]
    fn parses_import_signatures_with_list_and_function_types() {
        let source = "logic
from Collections.Lists import copy(List<Int>) -> List<Int> # specialization
from Collections.Lists import map(Int -> Bool, List<List<Int>>,) -> List<Bool>
from Collections.Lists import empty() -> List<Int>
theorem t() do true end";
        let parsed = parse(source).unwrap();
        let [copy, map, empty] = parsed.imports.as_slice() else {
            panic!("expected three typed imports");
        };
        let integers = call("List", vec![name("Int")]);
        assert_eq!(copy.module, "Collections.Lists");
        assert_eq!(copy.name, "copy");
        assert!(copy.arguments.is_empty());
        assert_eq!(copy.alias, None);
        assert_eq!(
            copy.signature,
            Some((vec![integers.clone()], integers.clone()))
        );
        assert_eq!(
            spanned(source, copy.span),
            "from Collections.Lists import copy(List<Int>) -> List<Int>"
        );
        assert_eq!(
            map.signature,
            Some((
                vec![
                    binary(Binary::Arrow, name("Int"), name("Bool")),
                    call("List", vec![integers.clone()]),
                ],
                call("List", vec![name("Bool")]),
            ))
        );
        assert_eq!(empty.signature, Some((vec![], integers)));
    }

    #[test]
    fn parses_specialized_and_aliased_imports_with_original_spans() {
        let source = "# Árbol\r\nlogic\r\n  \
from Trees import Tree<List<Int>> as IntTree # nominal import\r\n\
from Trees import mirror(Tree<List<Int>>) -> Tree<List<Int>> as mirror_int; \
from Trees import Tree<List<Int>,\r\n  Int -> Bool,>\r\n\
from Trees import Leaf as TreeLeaf\r\n\
theorem t() do true end";
        let parsed = parse(source).unwrap();
        let [tree, mirror, unaliased, leaf] = parsed.imports.as_slice() else {
            panic!("expected four imports");
        };
        let integers = call("List", vec![name("Int")]);
        let tree_type = call("Tree", vec![integers.clone()]);
        assert_eq!(tree.module, "Trees");
        assert_eq!(tree.name, "Tree");
        assert_eq!(tree.arguments, [integers.clone()]);
        assert_eq!(tree.signature, None);
        assert_eq!(tree.alias.as_deref(), Some("IntTree"));
        assert!(mirror.arguments.is_empty());
        assert_eq!(mirror.signature, Some((vec![tree_type.clone()], tree_type)));
        assert_eq!(mirror.alias.as_deref(), Some("mirror_int"));
        assert_eq!(
            unaliased.arguments,
            [integers, binary(Binary::Arrow, name("Int"), name("Bool"))]
        );
        assert_eq!(unaliased.signature, None);
        assert_eq!(unaliased.alias, None);
        assert!(leaf.arguments.is_empty());
        assert_eq!(leaf.signature, None);
        assert_eq!(leaf.alias.as_deref(), Some("TreeLeaf"));
        for (import, expected) in [
            (tree, "from Trees import Tree<List<Int>> as IntTree"),
            (
                mirror,
                "from Trees import mirror(Tree<List<Int>>) -> Tree<List<Int>> as mirror_int",
            ),
            (
                unaliased,
                "from Trees import Tree<List<Int>,\r\n  Int -> Bool,>",
            ),
            (leaf, "from Trees import Leaf as TreeLeaf"),
        ] {
            assert_eq!(spanned(source, import.span), expected);
        }
    }

    #[test]
    fn rejects_invalid_import_suffixes_at_the_offending_token() {
        for (import, offending) in [
            ("Tree<Int>(Int) -> Int", "("),
            ("Tree<Int> -> Int", "->"),
            ("mirror(Int)<Int> -> Int", "<"),
            ("Tree<>", "<>"),
            ("Tree< >", ">"),
            ("Tree<Int", "theorem"),
            ("Tree<Int> as", "\n"),
            ("Tree<Int> as _", "_"),
            ("Tree<Int> asTree", "asTree"),
        ] {
            let source = format!("logic\nfrom Trees import {import}\ntheorem t() do true end");
            let error = parse(&source).expect_err(&source);
            assert_eq!(spanned(&source, error.span), offending, "{source}: {error}");
        }
    }

    #[test]
    fn parses_ascriptions_with_type_syntax_and_preserves_grouping() {
        assert_eq!(
            proposition("((transform : List<Int> -> List<Int>))(xs)"),
            Term::Call(
                Box::new(Term::Annotate(
                    Box::new(name("transform")),
                    Box::new(binary(
                        Binary::Arrow,
                        call("List", vec![name("Int")]),
                        call("List", vec![name("Int")]),
                    )),
                )),
                vec![name("xs")],
            )
        );
        assert_eq!(
            proposition("(x + y) * z"),
            binary(
                Binary::Mul,
                binary(Binary::Add, name("x"), name("y")),
                name("z")
            )
        );
    }

    #[test]
    fn parses_pairs_as_single_arguments_with_explicit_nesting() {
        assert_eq!(
            proposition("f((x,\n (y, z)), w)"),
            call(
                "f",
                vec![
                    Term::Pair(
                        Box::new(name("x")),
                        Box::new(Term::Pair(Box::new(name("y")), Box::new(name("z")))),
                    ),
                    name("w"),
                ],
            )
        );
    }

    #[test]
    fn parses_dependent_pair_types_as_calls_with_lambda_arguments() {
        for type_name in ["Sigma", "Subtype"] {
            assert_eq!(
                proposition(&format!(
                    "((x, proofValue) : {type_name}(fn n :: Nat -> U(n) end))"
                )),
                Term::Annotate(
                    Box::new(Term::Pair(
                        Box::new(name("x")),
                        Box::new(name("proofValue"))
                    )),
                    Box::new(call(
                        type_name,
                        vec![Term::Lambda(
                            Box::new(binder("n", name("Nat"))),
                            Box::new(call("U", vec![name("n")])),
                        )],
                    )),
                )
            );
        }
    }

    #[test]
    fn parses_let_scope_up_to_the_next_match_arm() {
        assert_eq!(
            proposition(
                "match flag do
                  true -> let n :: Nat = n + 1 # initializer
                    let next :: Nat = n + 1; (n, next)
                  false -> (0, 0)
                 end",
            ),
            Term::Match(
                Box::new(name("flag")),
                vec![
                    Arm {
                        pattern: Pattern::Bool(true),
                        body: Term::Let(
                            Box::new(binder("n", name("Nat"))),
                            Box::new(binary(Binary::Add, name("n"), Term::Nat(1))),
                            Box::new(Term::Let(
                                Box::new(binder("next", name("Nat"))),
                                Box::new(binary(Binary::Add, name("n"), Term::Nat(1))),
                                Box::new(Term::Pair(Box::new(name("n")), Box::new(name("next")))),
                            )),
                        ),
                    },
                    Arm {
                        pattern: Pattern::Bool(false),
                        body: Term::Pair(Box::new(Term::Nat(0)), Box::new(Term::Nat(0))),
                    },
                ],
            )
        );
    }

    #[test]
    fn parses_indexed_family_with_explicit_constructor_results() {
        let declaration = "inductive Vec(T :: Type) -> Nat -> Type do
  Zero -> Vec(T, 0)
  Cons(n :: Nat, head :: T, tail :: Vec<T,n>) -> Vec(T, n + 1)
end";
        let source = format!("# Vectör\r\nlogic\r\n{declaration} # indexed family");
        let parsed = parse(&source).unwrap();
        let [Declaration::Inductive {
            name: family,
            params,
            kind,
            constructors,
            span,
        }] = parsed.declarations.as_slice()
        else {
            panic!("expected one inductive family");
        };
        assert_eq!(family, "Vec");
        assert_eq!(*params, [binder("T", name("Type"))]);
        assert_eq!(
            *kind,
            Some(binary(Binary::Arrow, name("Nat"), name("Type")))
        );
        assert_eq!(
            *constructors,
            [
                Constructor {
                    name: "Zero".into(),
                    fields: vec![],
                    result: Some(call("Vec", vec![name("T"), Term::Nat(0)])),
                },
                Constructor {
                    name: "Cons".into(),
                    fields: vec![
                        binder("n", name("Nat")),
                        binder("head", name("T")),
                        binder("tail", call("Vec", vec![name("T"), name("n")])),
                    ],
                    result: Some(call(
                        "Vec",
                        vec![name("T"), binary(Binary::Add, name("n"), Term::Nat(1))],
                    )),
                },
            ]
        );
        assert_eq!(spanned(&source, *span), declaration);
    }

    #[test]
    fn parses_empty_families_and_nested_empty_cases() {
        let source = "logic
inductive Empty do end
inductive Impossible -> Nat -> Nat -> Prop do
  # no constructors
end
proof absurd do intro impossible do cases impossible do end end end";
        let parsed = parse(source).unwrap();
        let [Declaration::Inductive {
            kind: None,
            constructors: empty,
            span: empty_span,
            ..
        }, Declaration::Inductive {
            kind: Some(kind),
            constructors: impossible,
            ..
        }, Declaration::Proof { term, span, .. }] = parsed.declarations.as_slice()
        else {
            panic!("expected two inductives and a proof");
        };
        assert!(empty.is_empty());
        assert!(impossible.is_empty());
        assert_eq!(
            *kind,
            binary(
                Binary::Arrow,
                name("Nat"),
                binary(Binary::Arrow, name("Nat"), name("Prop")),
            )
        );
        assert_eq!(
            *term,
            Proof::Intro(
                "impossible".into(),
                Box::new(Proof::Cases("impossible".into(), vec![])),
            )
        );
        assert_eq!(spanned(source, *empty_span), "inductive Empty do end");
        assert_eq!(
            spanned(source, *span),
            "proof absurd do intro impossible do cases impossible do end end end"
        );
    }

    #[test]
    fn parses_precedence_and_right_associative_operators() {
        assert_eq!(
            proposition("a+2*b-1 < c && d != e || f == g => p => q"),
            binary(
                Binary::Implies,
                binary(
                    Binary::Or,
                    binary(
                        Binary::And,
                        binary(
                            Binary::Lt,
                            binary(
                                Binary::Sub,
                                binary(
                                    Binary::Add,
                                    name("a"),
                                    binary(Binary::Mul, Term::Nat(2), name("b"))
                                ),
                                Term::Nat(1),
                            ),
                            name("c"),
                        ),
                        binary(Binary::Ne, name("d"), name("e")),
                    ),
                    binary(Binary::Eq, name("f"), name("g")),
                ),
                binary(Binary::Implies, name("p"), name("q")),
            )
        );
        assert_eq!(
            proposition("A -> B -> C"),
            binary(
                Binary::Arrow,
                name("A"),
                binary(Binary::Arrow, name("B"), name("C"))
            )
        );
        assert_eq!(
            proposition("x<y || x>y || x<=y || x>=y"),
            binary(
                Binary::Or,
                binary(
                    Binary::Or,
                    binary(
                        Binary::Or,
                        binary(Binary::Lt, name("x"), name("y")),
                        binary(Binary::Gt, name("x"), name("y"))
                    ),
                    binary(Binary::Le, name("x"), name("y")),
                ),
                binary(Binary::Ge, name("x"), name("y")),
            )
        );
    }

    #[test]
    fn parses_quantifiers_lambdas_nested_types_and_calls() {
        let term = proposition(
            "forall xs :: Domain.List<List<Nat>> do
               exists f :: (Nat -> Nat) -> Nat do
                 f(fn x :: Nat -> x + 1 end) == 0xff_ff
               end
             end",
        );
        assert_eq!(
            term,
            Term::Forall(
                Box::new(binder(
                    "xs",
                    call("Domain.List", vec![call("List", vec![name("Nat")])])
                )),
                Box::new(Term::Exists(
                    Box::new(binder(
                        "f",
                        binary(
                            Binary::Arrow,
                            binary(Binary::Arrow, name("Nat"), name("Nat")),
                            name("Nat")
                        ),
                    )),
                    Box::new(binary(
                        Binary::Eq,
                        call(
                            "f",
                            vec![Term::Lambda(
                                Box::new(binder("x", name("Nat"))),
                                Box::new(binary(Binary::Add, name("x"), Term::Nat(1))),
                            )],
                        ),
                        Term::Nat(65_535),
                    )),
                )),
            )
        );
    }

    #[test]
    fn parses_mesh_conditionals_and_match_patterns() {
        assert_eq!(
            proposition(
                "if test(x) do
                match x do
                    List.nil() -> 0
                    List.cons(h, _) -> h
                    _ -> 3
                end
            else case y do true -> 1; false -> 2; 0 -> 4 end end"
            ),
            Term::If(
                Box::new(call("test", vec![name("x")])),
                Box::new(Term::Match(
                    Box::new(name("x")),
                    vec![
                        Arm {
                            pattern: Pattern::Constructor(
                                vec!["List".into(), "nil".into()],
                                vec![]
                            ),
                            body: Term::Nat(0)
                        },
                        Arm {
                            pattern: Pattern::Constructor(
                                vec!["List".into(), "cons".into()],
                                vec!["h".into(), "_".into()]
                            ),
                            body: name("h")
                        },
                        Arm {
                            pattern: Pattern::Wildcard,
                            body: Term::Nat(3)
                        },
                    ],
                )),
                Box::new(Term::Match(
                    Box::new(name("y")),
                    vec![
                        Arm {
                            pattern: Pattern::Bool(true),
                            body: Term::Nat(1)
                        },
                        Arm {
                            pattern: Pattern::Bool(false),
                            body: Term::Nat(2)
                        },
                        Arm {
                            pattern: Pattern::Nat(0),
                            body: Term::Nat(4)
                        },
                    ],
                )),
            )
        );
        assert_eq!(proposition("nil"), name("nil"));
    }

    #[test]
    fn parses_nested_proofs_with_lemma_calls_and_induction_hypotheses() {
        let file = parse(
            "logic\nproof example do
            intro xs do
              split do
                induction xs do
                  Nil() -> refl
                  Cons(h, t, ih) -> exact congr(List.cons(h), ih)
                end,
                cases flag do
                  false -> left do witness x do refl end end
                  true -> right do rewrite eq do exact earlier(x) end end
                end
              end
            end
          end",
        )
        .unwrap();
        let [Declaration::Proof { term, .. }] = file.declarations.as_slice() else {
            panic!("expected the example proof");
        };
        let introduce_witness = Proof::Witness(name("x"), Box::new(Proof::Refl));
        let apply_earlier = Proof::Exact(call("earlier", vec![name("x")]));
        let rewrite = Proof::Rewrite(name("eq"), Box::new(apply_earlier));
        assert_eq!(
            *term,
            Proof::Intro(
                "xs".into(),
                Box::new(Proof::Split(
                    Box::new(Proof::Induction(
                        "xs".into(),
                        vec![
                            Arm {
                                pattern: Pattern::Constructor(vec!["Nil".into()], vec![]),
                                body: Proof::Refl
                            },
                            Arm {
                                pattern: Pattern::Constructor(
                                    vec!["Cons".into()],
                                    vec!["h".into(), "t".into(), "ih".into()]
                                ),
                                body: Proof::Exact(call(
                                    "congr",
                                    vec![call("List.cons", vec![name("h")]), name("ih")]
                                )),
                            }
                        ],
                    )),
                    Box::new(Proof::Cases(
                        "flag".into(),
                        vec![
                            Arm {
                                pattern: Pattern::Bool(false),
                                body: Proof::Left(Box::new(introduce_witness)),
                            },
                            Arm {
                                pattern: Pattern::Bool(true),
                                body: Proof::Right(Box::new(rewrite)),
                            }
                        ],
                    )),
                )),
            )
        );
    }

    #[test]
    fn parses_branch_change_and_unfold_proofs() {
        let file = parse(
            "logic\nproof example do
                branch head <= pivot do
                    yes(le) -> change insert(head, xs) == xs do
                        unfold Sorting.Insertion.insert do exact le end
                    end
                    no(gt) -> unfold helper do exact gt end
                end
            end",
        )
        .unwrap();
        let [Declaration::Proof { term, .. }] = file.declarations.as_slice() else {
            panic!("expected the example proof");
        };
        assert_eq!(
            *term,
            Proof::Branch(
                binary(Binary::Le, name("head"), name("pivot")),
                "le".into(),
                Box::new(Proof::Change(
                    binary(
                        Binary::Eq,
                        call("insert", vec![name("head"), name("xs")]),
                        name("xs"),
                    ),
                    Box::new(Proof::Unfold(
                        vec!["Sorting".into(), "Insertion".into(), "insert".into()],
                        Box::new(Proof::Exact(name("le"))),
                    )),
                )),
                "gt".into(),
                Box::new(Proof::Unfold(
                    vec!["helper".into()],
                    Box::new(Proof::Exact(name("gt"))),
                )),
            )
        );
    }

    #[test]
    fn rejects_bad_syntax_at_the_offending_token() {
        for (statement, offending) in [
            ("theorem t(x Nat) do x end", "Nat"),
            ("theorem t() do x = y end", "="),
            (
                "theorem t() do 18446744073709551616 end",
                "18446744073709551616",
            ),
            ("theorem t() do 0b102 end", "0b102"),
            ("theorem t() do 1.5 end", "1.5"),
            ("theorem t() do a..b end", ".."),
            ("theorem t() do x.@bad end", "@"),
            ("theorem t() do f(a b) end", "b"),
            ("theorem t() do _ end", "_"),
            ("theorem t() do #= unterminated", "#= unterminated"),
            ("theorem t() do if b do 1 end end", "end"),
            ("theorem t() do case t do end end", "end"),
            ("theorem t() do x end theorem u() do y end", "theorem"),
            ("theorem t() do x end\nfrom A import f", "from"),
            ("from A import f(Int); theorem t() do x end", ";"),
            ("from A import f(Int) -> ; theorem t() do x end", ";"),
            ("theorem t() do (x : ) end", ")"),
            ("theorem t() do (x :: Nat) end", "::"),
            ("theorem t() do (x, ) end", ")"),
            ("theorem t() do (x, y, z) end", ","),
            ("theorem t() do () end", ")"),
            ("theorem t() do let n = 0; n end", "="),
            ("theorem t() do let n :: Nat = 0 n end", "n"),
            ("theorem t() do let n :: Nat = 0; end", "end"),
            ("theorem t(x :: List<Nat) do x end", ")"),
            ("inductive T do A() B() end", "B"),
            ("inductive T -> do end", "do"),
            ("inductive T -> Type do A() -> end", "end"),
            ("proof t do sorry end", "sorry"),
            ("proof t do split do refl refl end end", "refl"),
            ("proof t do cases x do false => refl end end", "=>"),
            ("proof t do induction x do end end", "end"),
            ("proof t do branch p do yes(h) -> refl end end", "end"),
            ("proof t do branch p do no(h) -> refl end end", "no"),
            ("proof t do branch p do yes -> refl end end", "->"),
            ("proof t do branch p do yes() -> refl end end", ")"),
            ("proof t do branch p do yes(_) -> refl end end", "_"),
            ("proof t do branch p do yes(h, j) -> refl end end", ","),
            ("proof t do branch p do yes(h) => refl end end", "=>"),
            (
                "proof t do branch p do yes(h) -> refl no(j) -> refl end end",
                "no",
            ),
            (
                "proof t do branch p do yes(h) -> refl; yes(j) -> refl end end",
                "yes",
            ),
            (
                "proof t do branch p do yes(h) -> refl; no(j) -> refl; no(k) -> refl end end",
                "no",
            ),
            ("proof t do change do refl end end", "do"),
            ("proof t do unfold Module.fn(x) do refl end end", "fn"),
            ("proof t do unfold Module.f(x) do refl end end", "("),
            ("proof t do unfold Module.f, Module.g do refl end end", ","),
            ("proof t do exact f(x) trailing end", "trailing"),
            ("proof t do intro x do refl end", ""),
        ] {
            let source = format!("logic\n{statement}");
            let error = parse(&source).expect_err(&source);
            assert_eq!(spanned(&source, error.span), offending, "{source}: {error}");
        }
    }

    #[test]
    fn bounds_syntax_recursion_and_left_associated_tree_height() {
        let deep = MAX_NESTING * 2;
        for statement in [
            format!(
                "theorem t() do {}0{} end",
                "(".repeat(deep),
                ")".repeat(deep)
            ),
            format!("theorem t() do {}0 end", "a => ".repeat(deep)),
            format!("theorem t() do 0{} end", " + 1".repeat(deep)),
            format!("theorem t() do f{} end", "()".repeat(deep)),
            format!("theorem t() do {}0 end", "let n :: Nat = 0; ".repeat(deep)),
            format!(
                "theorem t() do {}0{} end",
                "(0, ".repeat(deep),
                ")".repeat(deep)
            ),
            format!("inductive T -> {}Type do end", "Nat -> ".repeat(deep)),
            format!("inductive T do A -> T(0{}) end", " + 1".repeat(deep)),
            format!(
                "theorem t(x :: {}Nat{}) do x end",
                "List<".repeat(deep),
                ">".repeat(deep)
            ),
            format!(
                "from Trees import Tree<{}Int{}> as DeepTree\ntheorem t() do true end",
                "List<".repeat(deep),
                ">".repeat(deep)
            ),
            format!(
                "proof t do {}refl{} end",
                "intro x do ".repeat(deep),
                " end".repeat(deep)
            ),
            format!(
                "proof t do {}refl{} end",
                "change p do unfold f do branch p do yes(h) -> refl; no(h) -> ".repeat(deep),
                " end end end".repeat(deep)
            ),
        ] {
            let error = parse(&format!("logic\n{statement}")).unwrap_err();
            assert_eq!(error.message, "logic nesting limit exceeded", "{statement}");
        }

        let many_arguments = "0,".repeat(MAX_NESTING * 2);
        assert_eq!(
            proposition(&format!("f({many_arguments})")),
            call("f", vec![Term::Nat(0); MAX_NESTING * 2])
        );
    }
}
