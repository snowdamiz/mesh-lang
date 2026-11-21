//! Event-based parser for Mesh.
//!
//! The parser consumes a token stream and produces events (Open/Close/Advance)
//! that are later converted into a rowan green tree. This decouples parsing
//! logic from tree construction.
//!
//! # Architecture
//!
//! The parser uses matklad's event-based approach (as in rust-analyzer):
//!
//! 1. Parse functions call `open()` to start a node, `advance()` to consume
//!    tokens, and `close()` to finish a node with its actual kind.
//! 2. Events are collected into a flat `Vec<Event>`.
//! 3. `build_tree()` converts events into a rowan `GreenNode`.
//!
//! The `open_before()` method enables wrapping a previously completed node
//! (e.g., turning `ident` into `call_expr(ident, arg_list)`) using the
//! "forward parent" technique.
//!
//! # Newline Significance
//!
//! Newlines are significant in Mesh (they act as statement terminators) UNLESS
//! the parser is inside delimiters (`()`, `[]`, `{}`). The `current()` and
//! `nth()` methods transparently skip insignificant newlines. Comments are
//! always skipped by lookahead. The `advance()` method emits Advance events
//! for all skipped trivia tokens so they appear in the CST.

pub(crate) mod expressions;
pub(crate) mod items;
pub(crate) mod patterns;

use mesh_common::span::Span;
use mesh_common::token::{Token, TokenKind};
use mesh_diagnostics::Code;
use mesh_lexer::{LexErrorKind, Lexer};

use crate::error::ParseError;
use crate::syntax_kind::SyntaxKind;

/// A parser event. Events are collected during parsing and later converted
/// into a rowan green tree by [`Parser::build_tree`].
#[derive(Debug)]
enum Event {
    /// Start a new CST node. The `kind` is initially TOMBSTONE and gets
    /// patched by `close()` with the real node kind.
    ///
    /// `forward_parent` is used by `open_before()` to indicate that this
    /// node should be opened before the node at the specified event index.
    Open {
        kind: SyntaxKind,
        forward_parent: Option<usize>,
    },
    /// Finish the current CST node.
    Close,
    /// Consume the current token, advancing the token position.
    Advance,
}

/// An opaque marker for a started but not-yet-closed CST node.
/// Contains the index into the events list where the corresponding
/// `Event::Open` was placed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MarkOpened {
    index: usize,
}

/// An opaque marker for a completed (opened and closed) CST node.
/// Used by `open_before()` to wrap a previously completed node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MarkClosed {
    index: usize,
    height: usize,
}

const MAX_PARSE_ERRORS: usize = 20;
const MAX_PARSE_DEPTH: usize = 64;
/// The message of the error at the depth limit, which a law file reports
/// as too large rather than as malformed.
pub(crate) const NESTED_TOO_DEEPLY: &str = "syntax nested too deeply";
const MAX_WRAPPED_NODE_HEIGHT: usize = 1024;

/// An aligned declaration seen inside a block may belong at the top level
/// if that block turns out to be missing its `end`.
#[derive(Default)]
enum Sibling {
    #[default]
    Unseen,
    Seen(usize),
    Rewind(usize),
    StopAt(usize),
}

struct ItemRecovery {
    start: usize,
    sibling: Sibling,
}

/// Event-based parser for Mesh source code.
///
/// The parser consumes a `Vec<Token>` (from the lexer) and source text,
/// producing events that are later converted into a rowan green tree.
///
/// # Usage
///
/// ```text
/// let mut parser = Parser::new(source);
/// // ... call parse methods ...
/// let (green, errors) = parser.build_tree();
/// ```
pub(crate) struct Parser<'src> {
    /// All tokens from the lexer (including Eof).
    tokens: Vec<Token>,
    /// Current position in the token stream.
    pos: usize,
    /// Collected parser events.
    events: Vec<Event>,
    /// Tallest child of each open node, including the child of a forward parent.
    child_heights: Vec<usize>,
    /// Original source text (for extracting token text via spans).
    source: &'src str,
    /// The `(`, `[`, and `{` consumed and not yet closed, innermost last: the
    /// token that closes each, and where it was opened. Inside any of them
    /// newlines are insignificant.
    open_delimiters: Vec<(SyntaxKind, Span)>,
    /// Why the lexer rejected each `Error` token, by token index.
    lex_errors: Vec<(usize, LexErrorKind)>,
    /// Collected parse errors.
    errors: Vec<ParseError>,
    /// Whether the current item has a syntax error that stops its grammar.
    has_error: bool,
    item: Option<ItemRecovery>,
    depth: usize,
    /// When true, the Pratt postfix loop will NOT treat `DO_KW` after a call
    /// as a trailing closure. Set by control-flow parsers (`if`, `while`,
    /// `case`, `for`) around their condition/scrutinee/iterable expression
    /// so that `if fn_call() do ... end` parses `do` as the block opener.
    suppress_trailing_closure: bool,
}

impl<'src> Parser<'src> {
    /// Lex `source` and start parsing at its first token.
    pub(crate) fn new(source: &'src str) -> Self {
        let (tokens, lex_errors) = Lexer::tokenize_with_errors(source);
        let lex_errors = tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| token.kind == TokenKind::Error)
            .map(|(index, _)| index)
            .zip(lex_errors.into_iter().map(|error| error.kind))
            .collect();
        Self {
            tokens,
            pos: 0,
            events: Vec::new(),
            child_heights: Vec::new(),
            source,
            open_delimiters: Vec::new(),
            lex_errors,
            errors: Vec::new(),
            has_error: false,
            item: None,
            depth: 0,
            suppress_trailing_closure: false,
        }
    }

    // ── Lookahead ──────────────────────────────────────────────────────

    /// Returns the SyntaxKind of the current significant token.
    ///
    /// Skips over trivia tokens (comments) and insignificant newlines
    /// (newlines inside delimiters). Returns `SyntaxKind::EOF` if past
    /// the end of the token stream.
    pub(crate) fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    /// Returns the SyntaxKind of the Nth significant token ahead.
    ///
    /// `nth(0)` is equivalent to `current()`. Skips trivia and
    /// insignificant newlines. Returns `SyntaxKind::EOF` if past end.
    pub(crate) fn nth(&self, n: usize) -> SyntaxKind {
        self.significant_kinds().nth(n).unwrap_or(SyntaxKind::EOF)
    }

    fn significant_kinds(&self) -> impl Iterator<Item = SyntaxKind> + '_ {
        self.tokens[self.pos..]
            .iter()
            .filter(|token| !self.should_skip(&token.kind))
            .map(|token| SyntaxKind::from(token.kind.clone()))
    }

    /// The index of the Nth significant token ahead, skipping trivia and
    /// insignificant newlines; `None` past the end.
    fn nth_index(&self, n: usize) -> Option<usize> {
        (self.pos..self.tokens.len())
            .filter(|&index| !self.should_skip(&self.tokens[index].kind))
            .nth(n)
    }

    fn nth_token(&self, n: usize) -> Option<&Token> {
        self.nth_index(n).map(|index| &self.tokens[index])
    }

    /// The name the last token before the current one that is not a comment
    /// or a newline spells, when it is an identifier.
    fn previous_ident(&self) -> Option<&str> {
        let token = self.tokens.get(..self.pos)?.iter().rev().find(|token| {
            !matches!(
                token.kind,
                TokenKind::Comment
                    | TokenKind::DocComment
                    | TokenKind::ModuleDocComment
                    | TokenKind::Newline
            )
        })?;
        (token.kind == TokenKind::Ident)
            .then(|| &self.source[token.span.start as usize..token.span.end as usize])
    }

    /// Returns the text of the current significant token.
    pub(crate) fn current_text(&self) -> &str {
        self.nth_text(0)
    }

    /// Returns the text of the Nth significant token ahead.
    pub(crate) fn nth_text(&self, n: usize) -> &str {
        self.nth_token(n).map_or("", |token| {
            &self.source[token.span.start as usize..token.span.end as usize]
        })
    }

    /// Returns the span of the Nth significant token ahead: past the end, the
    /// empty span at the end of the source.
    pub(crate) fn nth_span(&self, n: usize) -> Span {
        let end = self.source.len() as u32;
        self.nth_token(n)
            .map_or(Span::new(end, end), |token| token.span)
    }

    /// Returns the span of the current significant token.
    pub(crate) fn current_span(&self) -> Span {
        self.nth_span(0)
    }

    /// Check if the current significant token matches the given kind.
    pub(crate) fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    // ── Mutation: node management ──────────────────────────────────────

    /// Start a new CST node. Returns a marker that must be passed to
    /// `close()` to finish the node.
    ///
    /// The node kind is initially `TOMBSTONE` and gets patched by `close()`.
    pub(crate) fn open(&mut self) -> MarkOpened {
        // Trivia before the node's first token (a comment, a new line inside
        // brackets) belongs to the enclosing node: in the node it would come
        // before the node's own opening token (`[`, `{`, a name). The root
        // holds everything.
        if !self.events.is_empty() {
            self.skip_trivia();
        }
        self.child_heights.push(0);
        let mark = MarkOpened {
            index: self.events.len(),
        };
        self.events.push(Event::Open {
            kind: SyntaxKind::TOMBSTONE,
            forward_parent: None,
        });
        mark
    }

    /// Start a new CST node BEFORE a previously completed node.
    ///
    /// This enables wrapping: e.g., after parsing `ident`, we discover it's
    /// actually a function call `ident(args)`. We use `open_before(mark_closed)`
    /// to insert an Open event before the ident node, making it a child of
    /// the new call_expr node.
    ///
    /// Uses the "forward parent" technique: instead of physically inserting
    /// into the events vec (which would invalidate indices), we set a
    /// `forward_parent` link on the completed node's Open event.
    pub(crate) fn open_before(&mut self, completed: MarkClosed) -> MarkOpened {
        // Repeated wrapping, such as a long chain of additions, can build a
        // deep tree even without recursing in the grammar.
        if completed.height >= MAX_WRAPPED_NODE_HEIGHT {
            self.error(NESTED_TOO_DEEPLY);
        }
        self.child_heights.push(completed.height);
        let mark = MarkOpened {
            index: self.events.len(),
        };
        self.events.push(Event::Open {
            kind: SyntaxKind::TOMBSTONE,
            forward_parent: None,
        });
        // Link the completed node's Open event to point forward to this new event.
        if let Event::Open { forward_parent, .. } = &mut self.events[completed.index] {
            *forward_parent = Some(mark.index);
        }
        mark
    }

    /// Close a CST node, patching its Open event with the actual kind.
    /// Returns a `MarkClosed` that can be used with `open_before()`.
    pub(crate) fn close(&mut self, m: MarkOpened, kind: SyntaxKind) -> MarkClosed {
        if let Event::Open {
            kind: slot_kind, ..
        } = &mut self.events[m.index]
        {
            *slot_kind = kind;
        }
        self.events.push(Event::Close);
        let height = self.child_heights.pop().unwrap() + 1;
        if let Some(parent) = self.child_heights.last_mut() {
            *parent = (*parent).max(height);
        }
        MarkClosed {
            index: m.index,
            height,
        }
    }

    // ── Mutation: token consumption ────────────────────────────────────

    /// Consume the current token, emitting Advance events for all skipped
    /// trivia tokens and then for the significant token itself.
    ///
    /// Updates delimiter depth when consuming `(`, `)`, `[`, `]`, `{`, `}`.
    /// Emit Advance events for the trivia tokens before the next significant
    /// one.
    fn skip_trivia(&mut self) {
        while self.pos < self.tokens.len() && self.should_skip(&self.tokens[self.pos].kind) {
            self.events.push(Event::Advance);
            self.pos += 1;
        }
    }

    pub(crate) fn advance(&mut self) {
        self.skip_trivia();

        // Emit Advance for the significant token.
        if self.pos < self.tokens.len() {
            track_delimiter(&mut self.open_delimiters, &self.tokens[self.pos]);
            self.advance_raw();
        }
    }

    fn advance_raw(&mut self) {
        if !self.has_error && self.tokens[self.pos].kind == TokenKind::Error {
            if let Some(error) = self.lex_error(self.pos) {
                self.report(error);
            }
        }
        self.events.push(Event::Advance);
        self.pos += 1;
    }

    fn with_nesting<T: Default>(&mut self, parse: impl FnOnce(&mut Self) -> T) -> T {
        if self.depth == MAX_PARSE_DEPTH {
            self.error(NESTED_TOO_DEEPLY);
            return T::default();
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    /// If the current token matches `kind`, consume it and return true.
    /// Otherwise, emit an error and return false. Sets the error flag.
    ///
    /// A missing `)`, `]`, or `}` is reported as the delimiter left open.
    pub(crate) fn expect(&mut self, kind: SyntaxKind) -> bool {
        if self.eat(kind) {
            return true;
        }
        let opened = self
            .open_delimiters
            .iter()
            .rev()
            .find(|(closer, _)| *closer == kind)
            .map(|&(_, opened)| opened);
        match opened {
            Some(opened) => {
                self.rewind_at_unclosed_boundary(opened);
                self.unclosed(kind, opened);
            }
            None => {
                let expected = kind.describe();
                let found = self.found();
                let repair = match self.current() {
                    SyntaxKind::EOF => format!("Write {expected} before the end of the file."),
                    SyntaxKind::NEWLINE => format!("Write {expected} before the end of the line."),
                    _ => format!("Write {expected} before {found}, or in place of it."),
                };
                self.report_here(
                    ParseError::new(
                        Code::ExpectedToken,
                        format!("expected {expected}, found {found}"),
                        self.current_span(),
                    )
                    .with_repair(repair),
                );
            }
        }
        false
    }

    /// Report that `closer` (`)`, `]`, `}`, or the end of an interpolation)
    /// is missing here to close the delimiter opened at `opened`.
    pub(crate) fn unclosed(&mut self, closer: SyntaxKind, opened: Span) {
        let opener = match closer {
            SyntaxKind::INTERPOLATION_END => "the interpolation".to_string(),
            _ => format!(
                "`{}`",
                &self.source[opened.start as usize..opened.end as usize]
            ),
        };
        let closer = match closer {
            SyntaxKind::R_PAREN => "`)`",
            SyntaxKind::R_BRACKET => "`]`",
            _ => "`}`",
        };
        let found = self.found();
        self.report_here(
            ParseError::new(
                Code::UnclosedDelimiter,
                format!("expected {closer} to close {opener}, found {found}"),
                self.current_span(),
            )
            .with_related("opened here", opened),
        );
    }

    /// Consume the `end` that closes `what` (such as "function body"), the
    /// block opened at `opened`.
    pub(crate) fn expect_end(&mut self, what: &str, opened: Span) -> bool {
        if self.eat(SyntaxKind::END_KW) {
            return true;
        }
        let found = self.found();
        self.rewind_at_unclosed_boundary(opened);
        self.error_with_related(
            Code::MissingEnd,
            &format!("expected `end` to close {what}, found {found}"),
            opened,
            "block opened here",
        );
        false
    }

    /// The current token as a message names it: its text when it is a name
    /// or a literal.
    pub(crate) fn found(&self) -> String {
        match self.current() {
            SyntaxKind::IDENT
            | SyntaxKind::INT_LITERAL
            | SyntaxKind::FLOAT_LITERAL
            | SyntaxKind::ATOM_LITERAL => format!("`{}`", self.current_text()),
            kind => kind.describe(),
        }
    }

    /// If the current token matches `kind`, consume it and return true.
    /// Otherwise, return false (no error emitted).
    pub(crate) fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Whether the line ends here (a newline follows, significant or not) or
    /// the enclosing block does.
    /// The kinds of the tokens ahead with every new line, also those that
    /// are insignificant inside brackets; comments left out.
    pub(crate) fn raw_kinds_ahead(&self) -> impl Iterator<Item = SyntaxKind> + '_ {
        self.tokens
            .get(self.pos..)
            .into_iter()
            .flatten()
            .filter(|token| {
                !matches!(
                    token.kind,
                    TokenKind::Comment | TokenKind::DocComment | TokenKind::ModuleDocComment
                )
            })
            .map(|token| SyntaxKind::from(token.kind.clone()))
    }

    pub(crate) fn at_line_end(&self) -> bool {
        let next = self.tokens[self.pos..].iter().find(|token| {
            !matches!(
                token.kind,
                TokenKind::Comment | TokenKind::DocComment | TokenKind::ModuleDocComment
            )
        });
        matches!(
            next.map(|token| &token.kind),
            Some(TokenKind::Newline) | None
        ) || matches!(self.current(), SyntaxKind::END_KW | SyntaxKind::EOF)
    }

    /// Consume any significant newlines (used as statement separators).
    /// Only consumes newlines that are significant (at zero delimiter depth).
    /// Skip what separates statements: new lines and `;`s.
    pub(crate) fn eat_separators(&mut self) {
        self.eat_newlines();
        while self.eat(SyntaxKind::SEMICOLON) {
            self.eat_newlines();
        }
    }

    pub(crate) fn eat_newlines(&mut self) {
        while self.pos < self.tokens.len() {
            let kind = &self.tokens[self.pos].kind;
            // Skip comments (trivia)
            if matches!(
                kind,
                TokenKind::Comment | TokenKind::DocComment | TokenKind::ModuleDocComment
            ) {
                self.events.push(Event::Advance);
                self.pos += 1;
                continue;
            }
            // Only eat newlines when they are significant
            if *kind == TokenKind::Newline && !self.is_newline_insignificant() {
                self.events.push(Event::Advance);
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    // ── Error reporting ────────────────────────────────────────────────

    /// Record an unexpected-token error at the current token. Sets the
    /// error flag.
    pub(crate) fn error(&mut self, message: &str) {
        self.report_here(ParseError::new(
            Code::UnexpectedToken,
            message,
            self.current_span(),
        ));
    }

    /// Record `error`, found at the current token. Where the lexer rejected
    /// that token, the lexer's error says what is wrong instead.
    pub(crate) fn report_here(&mut self, error: ParseError) {
        let lex_error = self.nth_index(0).and_then(|index| self.lex_error(index));
        self.report(lex_error.unwrap_or(error));
    }

    /// Record `error` at its own span, unless an error came first. Sets the
    /// error flag.
    pub(crate) fn report(&mut self, error: ParseError) {
        if !self.has_error && self.errors.len() < MAX_PARSE_ERRORS {
            self.errors.push(error);
        }
        self.has_error = true;
    }

    /// Why the lexer rejected the current token, when it did.
    pub(crate) fn current_lex_kind(&self) -> Option<LexErrorKind> {
        self.nth_index(0).and_then(|index| self.lex_kind(index))
    }

    fn lex_kind(&self, index: usize) -> Option<LexErrorKind> {
        self.lex_errors
            .binary_search_by_key(&index, |&(at, _)| at)
            .ok()
            .map(|entry| self.lex_errors[entry].1)
    }

    /// The lexer's error for the token at `index`, when it rejected it.
    fn lex_error(&self, index: usize) -> Option<ParseError> {
        let kind = self.lex_kind(index)?;
        let span = self.tokens[index].span;
        let text = &self.source[span.start as usize..span.end as usize];
        Some(ParseError::new(kind.code(), kind.message(text), span))
    }

    /// Record an error in a token's text (an unknown escape) at `span`: the
    /// syntax around it is sound, so parsing goes on as if there were none.
    pub(crate) fn token_error_at(&mut self, message: &str, span: Span) {
        if !self.has_error && self.errors.len() < MAX_PARSE_ERRORS {
            self.errors
                .push(ParseError::new(Code::UnexpectedToken, message, span));
        }
    }

    /// Record an error at the current token with a related span for
    /// additional context.
    pub(crate) fn error_with_related(
        &mut self,
        code: Code,
        message: &str,
        related_span: Span,
        related_msg: &str,
    ) {
        self.report_here(
            ParseError::new(code, message, self.current_span())
                .with_related(related_msg, related_span),
        );
    }

    /// Whether the current item has encountered a syntax error.
    /// Parse functions should check this and bail early.
    pub(crate) fn has_error(&self) -> bool {
        self.has_error
    }

    /// Whether trailing closures (`foo() do ... end`) are currently
    /// suppressed. Control-flow parsers set this while parsing conditions
    /// so the Pratt postfix loop does not consume `do` as a closure.
    pub(crate) fn suppress_trailing_closure(&self) -> bool {
        self.suppress_trailing_closure
    }

    // ── Newline significance ───────────────────────────────────────────

    /// Whether newlines are currently insignificant (inside delimiters).
    pub(crate) fn is_newline_insignificant(&self) -> bool {
        !self.open_delimiters.is_empty()
    }

    /// Whether a token should be skipped by lookahead methods.
    ///
    /// Comments are always skipped. Newlines are skipped when inside
    /// delimiters (insignificant context).
    fn should_skip(&self, kind: &TokenKind) -> bool {
        match kind {
            TokenKind::Comment | TokenKind::DocComment | TokenKind::ModuleDocComment => true,
            TokenKind::Newline => self.is_newline_insignificant(),
            _ => false,
        }
    }

    /// Peek past newlines and comments at the top level (outside delimiters)
    /// to find the next significant non-newline token kind.
    ///
    /// This is used for multi-line pipe continuation: after parsing an expression
    /// and seeing a NEWLINE at the top level, we peek ahead to check if `|>`
    /// follows on the next line. Returns the SyntaxKind of the first
    /// non-newline, non-comment token after the current position.
    pub(crate) fn peek_past_newlines(&self) -> SyntaxKind {
        self.tokens
            .get(self.pos..)
            .into_iter()
            .flatten()
            .map(|token| &token.kind)
            .find(|kind| {
                !matches!(
                    kind,
                    TokenKind::Newline
                        | TokenKind::Comment
                        | TokenKind::DocComment
                        | TokenKind::ModuleDocComment
                )
            })
            .map_or(SyntaxKind::EOF, |kind| SyntaxKind::from(kind.clone()))
    }

    /// Skip newline and comment tokens at the current position, emitting
    /// Advance events for each so they appear in the CST.
    ///
    /// Used for multi-line pipe continuation: after confirming that `|>` follows
    /// newlines at the top level, consume the newlines so the Pratt parser can
    /// see the `|>` operator.
    pub(crate) fn skip_newlines_for_continuation(&mut self) {
        while self.pos < self.tokens.len() {
            match &self.tokens[self.pos].kind {
                TokenKind::Newline
                | TokenKind::Comment
                | TokenKind::DocComment
                | TokenKind::ModuleDocComment => {
                    self.events.push(Event::Advance);
                    self.pos += 1;
                }
                _ => break,
            }
        }
    }

    // ── Tree building ──────────────────────────────────────────────────

    /// Convert collected events into a rowan `GreenNode` and errors.
    ///
    /// This consumes the parser. The events are processed in order,
    /// with the "forward parent" technique handling `open_before()` links.
    ///
    /// Forward parents: when `open_before(completed)` is called, the completed
    /// node's Open event gets a `forward_parent` link pointing to the wrapping
    /// Open event. During tree building, when we encounter such an Open, we
    /// follow the chain, collect all kinds, and open nodes in reverse order
    /// (outermost wrapper first). The wrapper Open events are then marked as
    /// TOMBSTONE so they are skipped when encountered later.
    pub(crate) fn build_tree(mut self) -> (rowan::GreenNode, Vec<ParseError>) {
        // Fragment parsing can leave a tail unread.
        if !self.has_error && self.pos < self.tokens.len() {
            if let Some(&(index, _)) = self.lex_errors.first() {
                if let Some(error) = self.lex_error(index) {
                    self.report(error);
                }
            }
        }
        let mut builder = rowan::GreenNodeBuilder::new();
        let mut token_pos: usize = 0;
        // End of the last token placed in the tree. The lexer drops spaces
        // and tabs, so the gap up to the next token is emitted as a
        // WHITESPACE token before the next node opens or the next token is
        // placed: the tree's text is then the source text, every offset
        // taken from the tree is a source offset, and leading whitespace
        // stays outside the node it precedes.
        let mut last_end: usize = 0;
        let mut forward_parents: Vec<(usize, SyntaxKind)> = Vec::new();
        // The root must hold the whole text: a gap placed before it, from a
        // source that starts with a space or tab, would be a second top-level
        // element, which rowan rejects with a panic. The next event places it
        // inside the root instead.
        let mut root_open = false;

        let mut i = 0;
        while i < self.events.len() {
            match self.events[i] {
                Event::Open {
                    kind,
                    forward_parent,
                } => {
                    if root_open {
                        emit_gap(
                            &mut builder,
                            self.source,
                            &self.tokens,
                            token_pos,
                            &mut last_end,
                        );
                    }
                    root_open |= kind != SyntaxKind::TOMBSTONE || forward_parent.is_some();
                    if forward_parent.is_some() {
                        // Follow the forward_parent chain, collecting (index, kind) pairs.
                        forward_parents.clear();
                        // A forward parent is always an Open event.
                        let mut next = Some(i);
                        while let Some((
                            current,
                            Event::Open {
                                kind,
                                forward_parent,
                            },
                        )) = next.map(|current| (current, &self.events[current]))
                        {
                            forward_parents.push((current, *kind));
                            next = *forward_parent;
                        }

                        // Mark all forward parent Open events (except the first
                        // in the chain, which is at position i) as TOMBSTONE so
                        // they are skipped when we reach them later.
                        for &(fp_idx, _) in forward_parents.iter().skip(1) {
                            if let Event::Open {
                                ref mut kind,
                                ref mut forward_parent,
                            } = self.events[fp_idx]
                            {
                                *kind = SyntaxKind::TOMBSTONE;
                                *forward_parent = None;
                            }
                        }
                        // Clear the forward_parent on the first event too.
                        if let Event::Open {
                            ref mut forward_parent,
                            ..
                        } = self.events[i]
                        {
                            *forward_parent = None;
                        }

                        // Open nodes in reverse order: outermost wrapper first.
                        for &(_, fk) in forward_parents.iter().rev() {
                            if fk != SyntaxKind::TOMBSTONE {
                                builder.start_node(rowan::SyntaxKind(fk as u16));
                            }
                        }
                    } else if kind != SyntaxKind::TOMBSTONE {
                        builder.start_node(rowan::SyntaxKind(kind as u16));
                    }
                    // TOMBSTONE nodes are silently skipped.
                }
                Event::Close => {
                    builder.finish_node();
                }
                Event::Advance => {
                    emit_gap(
                        &mut builder,
                        self.source,
                        &self.tokens,
                        token_pos,
                        &mut last_end,
                    );
                    if token_pos < self.tokens.len() {
                        let token = &self.tokens[token_pos];
                        let syntax_kind = SyntaxKind::from(token.kind.clone());
                        let text = &self.source[token.span.start as usize..token.span.end as usize];
                        builder.token(rowan::SyntaxKind(syntax_kind as u16), text);
                        last_end = token.span.end as usize;
                        token_pos += 1;
                    }
                }
            }
            i += 1;
        }

        (builder.finish(), self.errors)
    }
}

/// Open or close a delimiter in `open` as its token is consumed. A closer
/// with no delimiter of its kind open closes nothing.
fn track_delimiter(open: &mut Vec<(SyntaxKind, Span)>, token: &Token) {
    let closer = match token.kind {
        TokenKind::LParen => SyntaxKind::R_PAREN,
        TokenKind::LBracket => SyntaxKind::R_BRACKET,
        TokenKind::LBrace => SyntaxKind::R_BRACE,
        TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
            let closer = SyntaxKind::from(token.kind.clone());
            if let Some(index) = open.iter().rposition(|&(open, _)| open == closer) {
                open.remove(index);
            }
            return;
        }
        _ => return,
    };
    open.push((closer, token.span));
}

/// Place the source text between the last emitted token and the token at
/// `token_pos` in the tree as a WHITESPACE token.
fn emit_gap(
    builder: &mut rowan::GreenNodeBuilder<'_>,
    source: &str,
    tokens: &[Token],
    token_pos: usize,
    last_end: &mut usize,
) {
    // The tokens end in EOF, so every event has a token at its position.
    let start = tokens[token_pos].span.start as usize;
    if start > *last_end {
        builder.token(
            rowan::SyntaxKind(SyntaxKind::WHITESPACE as u16),
            &source[*last_end..start],
        );
        *last_end = start;
    }
}

// ── Top-level parsing ──────────────────────────────────────────────────

/// Parse a complete source file.
///
/// Opens a SOURCE_FILE node, parses items and statements until EOF, and
/// closes the root node.
pub(crate) fn parse_source_file(p: &mut Parser) {
    let root = p.open();

    let mut balanced_groups = None;
    while p.errors.len() < MAX_PARSE_ERRORS {
        p.eat_separators();

        if p.at(SyntaxKind::EOF) {
            break;
        }

        let start = p.pos;
        let events = p.events.len();
        let errors = p.errors.len();
        p.item = Some(ItemRecovery {
            start,
            sibling: Sibling::Unseen,
        });
        parse_top_level_item(p);

        if p.has_error() {
            let groups = balanced_groups.get_or_insert_with(|| balanced_groups_in(&p.tokens));
            if let Some(ItemRecovery {
                sibling: Sibling::Rewind(index),
                ..
            }) = p.item
            {
                // If the candidate lies inside a balanced group, the parser
                // likely stopped early because of a syntax error in that group.
                let mut at = start;
                while at < index {
                    at = groups[at].unwrap_or(at) + 1;
                }
                if at == index {
                    p.pos = start;
                    p.events.truncate(events);
                    p.errors.truncate(errors);
                    p.reset_item();
                    p.item = Some(ItemRecovery {
                        start,
                        sibling: Sibling::StopAt(index),
                    });
                    parse_top_level_item(p);
                }
            }

            if p.errors.len() < MAX_PARSE_ERRORS {
                p.recover_to_item(start, groups);
            }
        }
        p.reset_item();
    }

    // Keep the CST lossless even when the diagnostic budget is spent.
    while p.pos < p.tokens.len() {
        p.events.push(Event::Advance);
        p.pos += 1;
    }

    p.close(root, SyntaxKind::SOURCE_FILE);
}

fn parse_top_level_item(p: &mut Parser) {
    if p.at(SyntaxKind::IDENT) && p.current_text() == "logic" && p.nth(1) == SyntaxKind::DO_KW {
        parse_logic_block(p);
    } else {
        parse_item_or_stmt(p);
    }
    if !p.has_error() {
        expect_statement_end(p);
    }
}

impl Parser<'_> {
    fn reset_item(&mut self) {
        self.open_delimiters.clear();
        self.has_error = false;
        self.suppress_trailing_closure = false;
        self.item = None;
    }

    fn rewind_at_unclosed_boundary(&mut self, opened: Span) {
        if let Some(item) = &mut self.item {
            if let Sibling::Seen(index) = item.sibling {
                if opened.start < self.tokens[index].span.start {
                    item.sibling = Sibling::Rewind(index);
                }
            }
        }
    }

    /// Record a possible sibling while parsing nested syntax. Only a retry
    /// after a missing closer can turn it into a hard stop.
    fn at_item_boundary(&mut self) -> bool {
        let Some(ItemRecovery { start, ref sibling }) = self.item else {
            return false;
        };
        let Some(index) = self.nth_index(0) else {
            return false;
        };
        if let Sibling::StopAt(stop) = *sibling {
            return index == stop;
        }
        if matches!(sibling, Sibling::Unseen)
            && !self.has_error
            && index > start
            && self.tokens[index - 1].kind == TokenKind::Newline
            && self.starts_item(index)
            && self.line_indent(index) <= self.line_indent(start)
        {
            self.item.as_mut().unwrap().sibling = Sibling::Seen(index);
        }
        false
    }

    fn starts_item(&self, index: usize) -> bool {
        let token = &self.tokens[index];
        match token.kind {
            TokenKind::Fn
            | TokenKind::Def
            | TokenKind::Pub
            | TokenKind::Struct
            | TokenKind::Type
            | TokenKind::Interface
            | TokenKind::Impl
            | TokenKind::Actor
            | TokenKind::Supervisor
            | TokenKind::Service
            | TokenKind::Module
            | TokenKind::Import
            | TokenKind::At => true,
            TokenKind::Ident => {
                let text = &self.source[token.span.start as usize..token.span.end as usize];
                let next = self.tokens.get(index + 1).map(|next| &next.kind);
                matches!(
                    (text, next),
                    ("from" | "law", _)
                        | ("logic", Some(TokenKind::Do))
                        | ("resource", Some(TokenKind::Ident))
                )
            }
            _ => false,
        }
    }

    fn line_indent(&self, index: usize) -> usize {
        let start = self.tokens[index].span.start as usize;
        self.source[..start]
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count()
    }

    /// Scan raw tokens so a stale delimiter cannot hide a newline. Skip
    /// balanced groups and keep indented members in an unclosed block
    /// until an aligned `end` or declaration gives us a place to resume.
    fn recover_to_item(&mut self, start: usize, groups: &[Option<usize>]) {
        let indent = self.line_indent(start);
        let mut inside_block = false;
        let mut index = start;
        while self.tokens[index].kind != TokenKind::Eof {
            if index > start
                && index >= self.pos
                && (self.tokens[index - 1].kind == TokenKind::Newline
                    || (!inside_block && self.tokens[index - 1].kind == TokenKind::Semicolon))
                && (self.starts_item(index)
                    || (!inside_block && matches!(self.tokens[index].kind, TokenKind::Let)))
                && (!inside_block || self.line_indent(index) <= indent)
            {
                break;
            }
            let end = groups[index].unwrap_or(index);
            match self.tokens[index].kind {
                // An arrow closure takes an `end` without a `do`. A deeply
                // indented match may be its closer rather than this block's.
                TokenKind::Do if !inside_block => {
                    inside_block = end == index || self.line_indent(end) > indent;
                }
                TokenKind::End
                    if inside_block
                        && self.tokens[index - 1].kind == TokenKind::Newline
                        && self.line_indent(index) <= indent =>
                {
                    inside_block = false;
                }
                _ => {}
            }
            index = end + 1;
        }
        while self.pos < index {
            self.events.push(Event::Advance);
            self.pos += 1;
        }
    }
}

/// Map punctuation, strings and `do` blocks to closing tokens for recovery.
/// An unmatched opener must not hide later items. An unterminated string
/// still owns its interpolation tokens through EOF.
fn balanced_groups_in(tokens: &[Token]) -> Vec<Option<usize>> {
    let mut groups = vec![None; tokens.len()];
    let mut stack = Vec::new();
    let mut string_depth = 0;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::StringStart => {
                string_depth += 1;
                groups[index] = Some(tokens.len() - 2);
                stack.push((index, TokenKind::StringEnd));
            }
            TokenKind::StringEnd if string_depth > 0 => {
                string_depth -= 1;
                if let Some((open, _)) = stack.pop() {
                    groups[open] = Some(index);
                }
            }
            _ if string_depth > 0 => {}
            TokenKind::LParen => stack.push((index, TokenKind::RParen)),
            TokenKind::LBracket => stack.push((index, TokenKind::RBracket)),
            TokenKind::LBrace => stack.push((index, TokenKind::RBrace)),
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                if stack
                    .last()
                    .is_some_and(|(_, closer)| *closer == token.kind)
                {
                    let (open, _) = stack.pop().unwrap();
                    groups[open] = Some(index);
                }
            }
            _ => {}
        }
    }
    // Within balanced punctuation and strings, block keywords belong to
    // expressions and cannot open or close the enclosing item.
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if let Some(end) = groups[index] {
            index = end + 1;
            continue;
        }
        match tokens[index].kind {
            TokenKind::Do => blocks.push(index),
            TokenKind::End => {
                if let Some(open) = blocks.pop() {
                    groups[open] = Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    groups
}

/// Keep the proof body opaque to Mesh, including its punctuation and trivia.
fn parse_logic_block(p: &mut Parser) {
    let block = p.open();
    p.advance(); // logic
    let do_span = p.current_span();
    p.advance(); // do

    let mut depth = 1;
    let mut string_depth = 0;
    // Consume raw tokens so punctuation inside the body does not affect Mesh's
    // delimiter state. Keywords inside a string or its interpolation are opaque.
    loop {
        match p.tokens[p.pos].kind {
            TokenKind::Eof => break,
            TokenKind::StringStart => string_depth += 1,
            TokenKind::StringEnd => string_depth -= 1,
            TokenKind::Do if string_depth == 0 => depth += 1,
            TokenKind::End if string_depth == 0 => depth -= 1,
            _ => {}
        }
        p.advance_raw();
        if depth == 0 {
            p.close(block, SyntaxKind::LOGIC_BLOCK);
            return;
        }
    }

    p.expect_end("logic block", do_span);
    p.close(block, SyntaxKind::ERROR_NODE);
}

/// After a statement or item, the line must end (or a `;` must separate the
/// next one). Without this, `assert_raises fn() do ... end` would silently
/// parse as two statements.
pub(crate) fn expect_statement_end(p: &mut Parser) {
    // `at_line_end` also sees the newlines that are insignificant inside
    // delimiters, such as the statements of a closure passed as an argument.
    if p.at(SyntaxKind::EQ) {
        // `x = 5` after `let x = ...`: a name is bound once.
        p.error("expected a newline or `;` after the statement: a variable cannot be assigned again; bind a new name with `let`");
    } else if !p.at_line_end()
        && !matches!(p.current(), SyntaxKind::SEMICOLON | SyntaxKind::ELSE_KW)
    {
        // `test "adds" do`: a call written without parentheses.
        let call = matches!(
            p.current(),
            SyntaxKind::STRING_START | SyntaxKind::INT_LITERAL | SyntaxKind::FLOAT_LITERAL
        )
        .then(|| p.previous_ident())
        .flatten()
        .map(str::to_string);
        match call {
            Some(name) => p.error(&format!(
                "expected a newline or `;` after the statement: to call `{name}`, \
                 put its arguments in parentheses, `{name}(...)`"
            )),
            None => p.error("expected a newline or `;` after the statement"),
        }
    }
}

/// Parse an item or statement.
///
/// Dispatches based on the current token to either an item parser (fn, module,
/// import, struct) or a statement/expression parser.
pub(crate) fn parse_item_or_stmt(p: &mut Parser) {
    p.with_nesting(parse_item_or_stmt_inner);
}

fn parse_item_or_stmt_inner(p: &mut Parser) {
    match p.current() {
        // pub -> peek ahead for item keyword
        SyntaxKind::PUB_KW => match p.nth(1) {
            SyntaxKind::FN_KW | SyntaxKind::DEF_KW => items::parse_fn_def(p),
            SyntaxKind::MODULE_KW => items::parse_module_def(p),
            SyntaxKind::STRUCT_KW => items::parse_struct_def(p),
            SyntaxKind::IDENT if p.nth_text(1) == "resource" => items::parse_resource_def(p),
            SyntaxKind::INTERFACE_KW => items::parse_interface_def(p),
            SyntaxKind::SUPERVISOR_KW => items::parse_supervisor_def(p),
            SyntaxKind::TYPE_KW => items::parse_type_decl(p, 1),
            _ => {
                p.error("expected `fn`, `module`, `struct`, `resource`, `interface`, `type`, or `supervisor` after `pub`");
            }
        },

        // fn/def: named function definition (`fn name(` / `fn name<`, and
        // `fn name do` with no parameters, as `def name do`) vs a closure
        // expression (`fn x -> ...`, `fn x, y do ...`, `fn(x) -> ...`): a
        // closure standing as a statement would never run. `def` never
        // starts a closure.
        SyntaxKind::FN_KW | SyntaxKind::DEF_KW => {
            if p.at(SyntaxKind::DEF_KW)
                || (p.nth(1) == SyntaxKind::IDENT
                    && matches!(
                        p.nth(2),
                        SyntaxKind::L_PAREN | SyntaxKind::LT | SyntaxKind::DO_KW
                    ))
            {
                items::parse_fn_def(p);
            } else {
                expressions::expr(p);
            }
        }

        SyntaxKind::MODULE_KW => items::parse_module_def(p),

        SyntaxKind::IMPORT_KW => items::parse_import_decl(p),

        // Source-first `@cluster` prefixes and fail-closed diagnostics for removed spellings.
        _ if items::starts_clustered_fn_def(p) => {
            items::parse_fn_def(p);
        }

        // "from" is an IDENT, not a keyword -- check text
        SyntaxKind::IDENT if p.current_text() == "from" => {
            items::parse_from_import_decl(p);
        }

        // `resource` is contextual so it remains available as an identifier
        // outside declaration position.
        _ if items::starts_resource_def(p) => {
            items::parse_resource_def(p);
        }

        SyntaxKind::STRUCT_KW => items::parse_struct_def(p),

        SyntaxKind::INTERFACE_KW => items::parse_interface_def(p),

        SyntaxKind::IMPL_KW => items::parse_impl_def(p),

        SyntaxKind::ACTOR_KW => items::parse_actor_def(p),

        SyntaxKind::SERVICE_KW => items::parse_service_def(p),

        SyntaxKind::SUPERVISOR_KW => items::parse_supervisor_def(p),

        // type at top level followed by IDENT -> sum type def or type alias
        SyntaxKind::TYPE_KW => items::parse_type_decl(p, 0),

        SyntaxKind::LET_KW => expressions::parse_let_binding(p),

        // `assert_receive PATTERN[, TIMEOUT]`, the assertion `meshc test`
        // expands into a `receive`: read so `meshc fmt` and `meshc lint`
        // take test files. Written as a call (`assert_receive(x)`, no space)
        // it stays one, as `meshc test` leaves it.
        SyntaxKind::IDENT
            if p.current_text() == "assert_receive"
                && !matches!(
                    p.nth(1),
                    SyntaxKind::NEWLINE | SyntaxKind::EOF | SyntaxKind::SEMICOLON
                )
                && p.nth_span(1).start > p.current_span().end =>
        {
            expressions::parse_assert_receive(p);
        }

        // `setup do ... end` (a test file's describe block): a statement that
        // is a name and a `do` block calls the name with it as a trailing
        // closure, as `setup() do ... end` does.
        SyntaxKind::IDENT if p.nth(1) == SyntaxKind::DO_KW => {
            expressions::parse_bare_trailing_closure_call(p);
        }

        SyntaxKind::RETURN_KW => {
            expressions::parse_return_expr(p);
        }

        _ => {
            expressions::expr(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AstNode;

    #[test]
    fn logic_blocks_keep_the_opaque_body_and_byte_offsets() {
        let prefix = "fn logic() = 1\n# λ\n\t";
        let header = "logic do";
        let body = "  # do end\n proof ordinary do\n  \"end ${end}\"\n end\n (\n\t";
        let block_text = format!("{header}{body}end");
        let source = format!("{prefix}{block_text}\nlogic do end\nlet answer = logic()\n");
        let parsed = crate::parse(&source);
        assert!(parsed.ok(), "{:?}", parsed.errors());
        assert_eq!(parsed.syntax().text().to_string(), source);

        let file = parsed.tree();
        let blocks: Vec<_> = file.logic_blocks().collect();
        let [block, second] = blocks.as_slice() else {
            panic!("expected two logic blocks");
        };
        assert_eq!(file.items().count(), 2);
        let function = file.fn_defs().next().unwrap();
        assert_eq!(function.name().unwrap().text().as_deref(), Some("logic"));
        assert_eq!(block.syntax().children().count(), 0);
        assert_eq!(block.syntax().text().to_string(), block_text);
        let range = block.body_range().unwrap();
        assert_eq!(usize::from(range.start()), prefix.len() + header.len());
        assert_eq!(
            usize::from(range.end()),
            prefix.len() + header.len() + body.len()
        );
        assert_eq!(&source[range], body);
        assert_eq!(&source[second.body_range().unwrap()], " ");
    }

    #[test]
    fn unterminated_logic_block_reports_the_outer_opener() {
        let source = "logic do\n proof sample do\n end";
        let parsed = crate::parse(source);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(parsed.tree().logic_blocks().count(), 0);
        let [error] = parsed.errors() else {
            panic!("expected a single error");
        };
        assert_eq!(error.code, Code::MissingEnd);
        assert_eq!(
            error.message,
            "expected `end` to close logic block, found the end of the file"
        );
        assert_eq!(
            error.span,
            Span::new(source.len() as u32, source.len() as u32)
        );
        assert_eq!(error.related.as_ref().unwrap().1, Span::new(6, 8));
    }

    #[test]
    fn smoke_test_parser_produces_green_node() {
        let source = "let x = 5";
        let mut parser = Parser::new(source);

        // Open a SOURCE_FILE node.
        let root = parser.open();

        // Open a LET_BINDING node.
        let binding = parser.open();

        // Consume: let, x, =, 5
        parser.advance(); // let
        parser.advance(); // x
        parser.advance(); // =
        parser.advance(); // 5

        parser.close(binding, SyntaxKind::LET_BINDING);

        // Consume EOF.
        parser.advance(); // Eof

        parser.close(root, SyntaxKind::SOURCE_FILE);

        let (green, errors) = parser.build_tree();
        assert!(errors.is_empty(), "expected no errors: {:?}", errors);

        // Verify the green node has the right kind.
        let root_node = crate::cst::SyntaxNode::new_root(green);
        assert_eq!(root_node.kind(), SyntaxKind::SOURCE_FILE);

        // The CST is lossless: whitespace the lexer skipped comes back as
        // WHITESPACE tokens, so the tree text is the source text.
        assert_eq!(root_node.text().to_string(), source);

        // Verify child structure.
        let children: Vec<_> = root_node.children().collect();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].kind(), SyntaxKind::LET_BINDING);
    }

    #[test]
    fn parser_newline_significance_inside_parens() {
        // Inside parentheses, newlines should be invisible to current()/nth().
        let source = "(\n42\n)";
        let mut parser = Parser::new(source);

        // Before consuming '(', newlines are significant.
        assert_eq!(parser.current(), SyntaxKind::L_PAREN);

        // Consume '(' -- now inside parens, newlines become insignificant.
        let root = parser.open();
        parser.advance(); // (

        // current() should skip the newline and see INT_LITERAL.
        assert_eq!(parser.current(), SyntaxKind::INT_LITERAL);

        parser.advance(); // 42

        // Should skip another newline and see ')'.
        assert_eq!(parser.current(), SyntaxKind::R_PAREN);

        parser.advance(); // )
        parser.advance(); // Eof
        parser.close(root, SyntaxKind::SOURCE_FILE);

        let (green, errors) = parser.build_tree();
        assert!(errors.is_empty());

        let root_node = crate::cst::SyntaxNode::new_root(green);
        assert_eq!(root_node.kind(), SyntaxKind::SOURCE_FILE);
    }

    #[test]
    fn parser_newline_significant_at_top_level() {
        // At top level (no delimiters), newlines are significant.
        let source = "x\ny";
        let parser = Parser::new(source);

        // First token is IDENT.
        assert_eq!(parser.current(), SyntaxKind::IDENT);

        // nth(1) should be NEWLINE (significant at top level).
        assert_eq!(parser.nth(1), SyntaxKind::NEWLINE);

        // nth(2) should be the second IDENT.
        assert_eq!(parser.nth(2), SyntaxKind::IDENT);
    }

    #[test]
    fn parser_comments_are_trivia() {
        // Comments should be invisible to current()/nth().
        let source = "x # this is a comment\ny";
        let parser = Parser::new(source);

        assert_eq!(parser.current(), SyntaxKind::IDENT);
        // nth(1) should skip the comment and see NEWLINE.
        assert_eq!(parser.nth(1), SyntaxKind::NEWLINE);
        // nth(2) should see the second IDENT.
        assert_eq!(parser.nth(2), SyntaxKind::IDENT);
    }

    #[test]
    fn parser_expect_success_and_failure() {
        let source = "let x";
        let mut parser = Parser::new(source);

        let root = parser.open();

        // Expecting LET_KW should succeed.
        assert!(parser.expect(SyntaxKind::LET_KW));
        assert!(!parser.has_error());

        // Expecting LET_KW again should fail (current is IDENT).
        assert!(!parser.expect(SyntaxKind::LET_KW));
        assert!(parser.has_error());
        assert_eq!(parser.errors.len(), 1);

        parser.advance(); // x
        parser.advance(); // Eof
        parser.close(root, SyntaxKind::SOURCE_FILE);

        let (_green, errors) = parser.build_tree();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn parser_eat_consumes_on_match() {
        let source = "let x";
        let mut parser = Parser::new(source);

        // eat LET_KW should succeed.
        assert!(parser.eat(SyntaxKind::LET_KW));

        // eat LET_KW should fail (no error emitted).
        assert!(!parser.eat(SyntaxKind::LET_KW));
        assert!(!parser.has_error());
    }

    #[test]
    fn parser_open_before_wraps_completed_node() {
        // Demonstrate wrapping: parse an ident, then wrap it in a call_expr.
        let source = "f(42)";
        let mut parser = Parser::new(source);

        let root = parser.open();

        // Parse the function name.
        let name = parser.open();
        parser.advance(); // f (Ident)
        let name_closed = parser.close(name, SyntaxKind::NAME_REF);

        // Now wrap it: open_before the name to create CALL_EXPR.
        let call = parser.open_before(name_closed);
        parser.advance(); // (
        parser.advance(); // 42
        parser.advance(); // )
        parser.close(call, SyntaxKind::CALL_EXPR);

        parser.advance(); // Eof
        parser.close(root, SyntaxKind::SOURCE_FILE);

        let (green, errors) = parser.build_tree();
        assert!(errors.is_empty());

        let root_node = crate::cst::SyntaxNode::new_root(green);
        assert_eq!(root_node.kind(), SyntaxKind::SOURCE_FILE);

        // The CALL_EXPR should contain NAME_REF as a child.
        let call_node = root_node.children().next().unwrap();
        assert_eq!(call_node.kind(), SyntaxKind::CALL_EXPR);

        let name_ref = call_node.children().next().unwrap();
        assert_eq!(name_ref.kind(), SyntaxKind::NAME_REF);
    }

    #[test]
    fn parser_current_text_returns_token_text() {
        let source = "hello 42";
        let parser = Parser::new(source);

        assert_eq!(parser.current_text(), "hello");
    }

    #[test]
    fn parser_eat_newlines_consumes_significant_newlines() {
        let source = "x\n\ny";
        let mut parser = Parser::new(source);

        parser.advance(); // x
        parser.eat_newlines(); // consume both newlines

        // After eating newlines, current should be IDENT (y).
        assert_eq!(parser.current(), SyntaxKind::IDENT);
    }

    #[test]
    fn parser_delimiter_depth_tracking() {
        let source = "([{x}])";
        let mut parser = Parser::new(source);
        let closers = |parser: &Parser| -> Vec<SyntaxKind> {
            parser
                .open_delimiters
                .iter()
                .map(|&(closer, _)| closer)
                .collect()
        };

        let root = parser.open();

        assert_eq!(closers(&parser), []);
        parser.advance(); // (
        assert_eq!(closers(&parser), [SyntaxKind::R_PAREN]);
        assert_eq!(parser.open_delimiters[0].1, Span::new(0, 1));

        parser.advance(); // [
        parser.advance(); // {
        assert_eq!(
            closers(&parser),
            [
                SyntaxKind::R_PAREN,
                SyntaxKind::R_BRACKET,
                SyntaxKind::R_BRACE
            ]
        );

        parser.advance(); // x

        parser.advance(); // }
        assert_eq!(
            closers(&parser),
            [SyntaxKind::R_PAREN, SyntaxKind::R_BRACKET]
        );

        parser.advance(); // ]
        parser.advance(); // )
        assert_eq!(closers(&parser), []);

        parser.advance(); // Eof
        parser.close(root, SyntaxKind::SOURCE_FILE);

        let (_green, _errors) = parser.build_tree();
    }
}
