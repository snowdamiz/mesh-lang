// Snow lexer -- tokenizer for the Snow programming language.

mod cursor;

use cursor::Cursor;
use snow_common::token::{keyword_from_str, Token, TokenKind};

/// Tracks whether the lexer is inside a string literal.
#[derive(Debug, Clone, Copy)]
enum StringMode {
    /// Not inside a string.
    None,
    /// Inside a single-quoted string (after StringStart emitted).
    Single,
    /// Inside a triple-quoted string (after StringStart emitted).
    Triple,
}

/// The Snow lexer. Converts source text into a stream of tokens.
///
/// Wraps a [`Cursor`] for byte-level iteration and implements
/// `Iterator<Item = Token>` so callers can consume tokens lazily
/// or collect them into a `Vec`.
pub struct Lexer<'src> {
    cursor: Cursor<'src>,
    source: &'src str,
    /// Whether we have already emitted the `Eof` token.
    emitted_eof: bool,
    /// A token to emit on the next call to `next()` before resuming normal lexing.
    pending_token: Option<Token>,
    /// Tracks whether we are inside a string and need to lex content next.
    string_mode: StringMode,
}

impl<'src> Lexer<'src> {
    /// Create a new lexer for the given source text.
    pub fn new(source: &'src str) -> Self {
        Self {
            cursor: Cursor::new(source),
            source,
            emitted_eof: false,
            pending_token: None,
            string_mode: StringMode::None,
        }
    }

    /// Convenience: tokenize the entire source into a `Vec<Token>`.
    ///
    /// The returned vector includes the final `Eof` token.
    pub fn tokenize(source: &str) -> Vec<Token> {
        Lexer::new(source).collect()
    }

    /// Produce the next token from the source (normal mode, not inside a string).
    fn next_token(&mut self) -> Token {
        self.skip_whitespace();

        let start = self.cursor.pos();

        let Some(c) = self.cursor.peek() else {
            // EOF
            return Token::new(TokenKind::Eof, start, start);
        };

        match c {
            // ── Single-character delimiters ───────────────────────────────
            '(' => self.single_char_token(TokenKind::LParen, start),
            ')' => self.single_char_token(TokenKind::RParen, start),
            '[' => self.single_char_token(TokenKind::LBracket, start),
            ']' => self.single_char_token(TokenKind::RBracket, start),
            '{' => self.single_char_token(TokenKind::LBrace, start),
            '}' => self.single_char_token(TokenKind::RBrace, start),
            ',' => self.single_char_token(TokenKind::Comma, start),
            ';' => self.single_char_token(TokenKind::Semicolon, start),

            // ── Multi-character operators ─────────────────────────────────
            '=' => self.lex_eq(start),
            '!' => self.lex_bang(start),
            '<' => self.lex_lt(start),
            '>' => self.lex_gt(start),
            '&' => self.lex_amp(start),
            '|' => self.lex_pipe(start),
            '+' => self.lex_plus(start),
            '-' => self.lex_minus(start),
            ':' => self.lex_colon(start),
            '.' => self.lex_dot(start),
            '*' => self.single_char_token(TokenKind::Star, start),
            '/' => self.single_char_token(TokenKind::Slash, start),
            '%' => self.single_char_token(TokenKind::Percent, start),

            // ── Comments ─────────────────────────────────────────────────
            '#' => self.lex_comment(start),

            // ── Number literals ──────────────────────────────────────────
            '0'..='9' => self.lex_number(start),

            // ── String literals ──────────────────────────────────────────
            '"' => self.lex_string_start(start),

            // ── Identifiers and keywords ─────────────────────────────────
            c if is_ident_start(c) => self.lex_ident(start),

            // ── Unknown character (error recovery) ───────────────────────
            _ => {
                self.cursor.advance();
                Token::new(TokenKind::Error, start, self.cursor.pos())
            }
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    /// Skip whitespace characters (spaces, tabs, carriage returns, newlines).
    ///
    /// Note: In this plan, newlines are skipped as whitespace. Plan 03 will
    /// add newline-as-terminator logic.
    fn skip_whitespace(&mut self) {
        self.cursor
            .eat_while(|c| c == ' ' || c == '\t' || c == '\r' || c == '\n');
    }

    /// Consume one character and return a token of the given kind.
    fn single_char_token(&mut self, kind: TokenKind, start: u32) -> Token {
        self.cursor.advance();
        Token::new(kind, start, self.cursor.pos())
    }

    // ── Operator lexing ──────────────────────────────────────────────────

    /// `=` -> `Eq`, `==` -> `EqEq`, `=>` -> `FatArrow`
    fn lex_eq(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '='
        match self.cursor.peek() {
            Some('=') => {
                self.cursor.advance();
                Token::new(TokenKind::EqEq, start, self.cursor.pos())
            }
            Some('>') => {
                self.cursor.advance();
                Token::new(TokenKind::FatArrow, start, self.cursor.pos())
            }
            _ => Token::new(TokenKind::Eq, start, self.cursor.pos()),
        }
    }

    /// `!` -> `Bang`, `!=` -> `NotEq`
    fn lex_bang(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '!'
        if self.cursor.peek() == Some('=') {
            self.cursor.advance();
            Token::new(TokenKind::NotEq, start, self.cursor.pos())
        } else {
            Token::new(TokenKind::Bang, start, self.cursor.pos())
        }
    }

    /// `<` -> `Lt`, `<=` -> `LtEq`, `<>` -> `Diamond`
    fn lex_lt(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '<'
        match self.cursor.peek() {
            Some('=') => {
                self.cursor.advance();
                Token::new(TokenKind::LtEq, start, self.cursor.pos())
            }
            Some('>') => {
                self.cursor.advance();
                Token::new(TokenKind::Diamond, start, self.cursor.pos())
            }
            _ => Token::new(TokenKind::Lt, start, self.cursor.pos()),
        }
    }

    /// `>` -> `Gt`, `>=` -> `GtEq`
    fn lex_gt(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '>'
        if self.cursor.peek() == Some('=') {
            self.cursor.advance();
            Token::new(TokenKind::GtEq, start, self.cursor.pos())
        } else {
            Token::new(TokenKind::Gt, start, self.cursor.pos())
        }
    }

    /// `&&` -> `AmpAmp`, single `&` -> `Error`
    fn lex_amp(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '&'
        if self.cursor.peek() == Some('&') {
            self.cursor.advance();
            Token::new(TokenKind::AmpAmp, start, self.cursor.pos())
        } else {
            Token::new(TokenKind::Error, start, self.cursor.pos())
        }
    }

    /// `||` -> `PipePipe`, `|>` -> `Pipe`, single `|` -> `Error`
    fn lex_pipe(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '|'
        match self.cursor.peek() {
            Some('|') => {
                self.cursor.advance();
                Token::new(TokenKind::PipePipe, start, self.cursor.pos())
            }
            Some('>') => {
                self.cursor.advance();
                Token::new(TokenKind::Pipe, start, self.cursor.pos())
            }
            _ => Token::new(TokenKind::Error, start, self.cursor.pos()),
        }
    }

    /// `+` -> `Plus`, `++` -> `PlusPlus`
    fn lex_plus(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '+'
        if self.cursor.peek() == Some('+') {
            self.cursor.advance();
            Token::new(TokenKind::PlusPlus, start, self.cursor.pos())
        } else {
            Token::new(TokenKind::Plus, start, self.cursor.pos())
        }
    }

    /// `-` -> `Minus`, `->` -> `Arrow`
    fn lex_minus(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '-'
        if self.cursor.peek() == Some('>') {
            self.cursor.advance();
            Token::new(TokenKind::Arrow, start, self.cursor.pos())
        } else {
            Token::new(TokenKind::Minus, start, self.cursor.pos())
        }
    }

    /// `:` -> `Colon`, `::` -> `ColonColon`
    fn lex_colon(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume ':'
        if self.cursor.peek() == Some(':') {
            self.cursor.advance();
            Token::new(TokenKind::ColonColon, start, self.cursor.pos())
        } else {
            Token::new(TokenKind::Colon, start, self.cursor.pos())
        }
    }

    /// `.` -> `Dot`, `..` -> `DotDot`
    fn lex_dot(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '.'
        if self.cursor.peek() == Some('.') {
            self.cursor.advance();
            Token::new(TokenKind::DotDot, start, self.cursor.pos())
        } else {
            Token::new(TokenKind::Dot, start, self.cursor.pos())
        }
    }

    // ── Comments ─────────────────────────────────────────────────────────

    /// Lex a comment starting with `#`.
    ///
    /// - `##!` -> `ModuleDocComment`
    /// - `##`  -> `DocComment`
    /// - `#=`  -> placeholder for block comments (Plan 03)
    /// - `#`   -> `Comment`
    fn lex_comment(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume '#'

        if self.cursor.peek() == Some('#') {
            // Could be doc comment or module doc comment
            self.cursor.advance(); // consume second '#'

            if self.cursor.peek() == Some('!') {
                // Module doc comment: ##!
                self.cursor.advance(); // consume '!'
                // Skip optional leading space
                if self.cursor.peek() == Some(' ') {
                    self.cursor.advance();
                }
                self.cursor.eat_while(|c| c != '\n');
                Token::new(TokenKind::ModuleDocComment, start, self.cursor.pos())
            } else {
                // Doc comment: ##
                // Skip optional leading space
                if self.cursor.peek() == Some(' ') {
                    self.cursor.advance();
                }
                self.cursor.eat_while(|c| c != '\n');
                Token::new(TokenKind::DocComment, start, self.cursor.pos())
            }
        } else if self.cursor.peek() == Some('=') {
            // Block comment placeholder: #= ... =#
            // For now, scan to end of line (Plan 03 adds nestable block comments)
            self.cursor.eat_while(|c| c != '\n');
            Token::new(TokenKind::Comment, start, self.cursor.pos())
        } else {
            // Regular line comment: #
            // Skip optional leading space
            if self.cursor.peek() == Some(' ') {
                self.cursor.advance();
            }
            self.cursor.eat_while(|c| c != '\n');
            Token::new(TokenKind::Comment, start, self.cursor.pos())
        }
    }

    // ── Number literals ──────────────────────────────────────────────────

    /// Lex a number literal starting with a digit.
    ///
    /// Handles decimal, hex (0x), binary (0b), octal (0o), floats, and
    /// scientific notation. Underscore separators are allowed.
    fn lex_number(&mut self, start: u32) -> Token {
        let first = self.cursor.advance().unwrap(); // consume first digit

        if first == '0' {
            match self.cursor.peek() {
                Some('x' | 'X') => return self.lex_hex(start),
                Some('b' | 'B') => return self.lex_binary(start),
                Some('o' | 'O') => return self.lex_octal(start),
                _ => {}
            }
        }

        // Decimal: eat digits and underscores
        self.cursor.eat_while(|c| c.is_ascii_digit() || c == '_');

        // Check for float: `.` followed by a digit (not `..` range)
        if self.cursor.peek() == Some('.')
            && self
                .cursor
                .peek_next()
                .is_some_and(|c| c.is_ascii_digit())
        {
            self.cursor.advance(); // consume '.'
            self.cursor.eat_while(|c| c.is_ascii_digit() || c == '_');

            // Check for scientific notation
            if matches!(self.cursor.peek(), Some('e' | 'E')) {
                self.lex_exponent();
            }

            return Token::new(TokenKind::FloatLiteral, start, self.cursor.pos());
        }

        // Check for scientific notation on integer (makes it a float)
        if matches!(self.cursor.peek(), Some('e' | 'E')) {
            self.lex_exponent();
            return Token::new(TokenKind::FloatLiteral, start, self.cursor.pos());
        }

        Token::new(TokenKind::IntLiteral, start, self.cursor.pos())
    }

    /// Lex hex digits after `0x`/`0X`.
    fn lex_hex(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume 'x'/'X'
        self.cursor
            .eat_while(|c| c.is_ascii_hexdigit() || c == '_');
        Token::new(TokenKind::IntLiteral, start, self.cursor.pos())
    }

    /// Lex binary digits after `0b`/`0B`.
    fn lex_binary(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume 'b'/'B'
        self.cursor
            .eat_while(|c| c == '0' || c == '1' || c == '_');
        Token::new(TokenKind::IntLiteral, start, self.cursor.pos())
    }

    /// Lex octal digits after `0o`/`0O`.
    fn lex_octal(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume 'o'/'O'
        self.cursor
            .eat_while(|c| matches!(c, '0'..='7' | '_'));
        Token::new(TokenKind::IntLiteral, start, self.cursor.pos())
    }

    /// Lex the exponent part of a float literal: `e`/`E` followed by optional
    /// sign and digits.
    fn lex_exponent(&mut self) {
        self.cursor.advance(); // consume 'e'/'E'
        if matches!(self.cursor.peek(), Some('+' | '-')) {
            self.cursor.advance(); // consume sign
        }
        self.cursor.eat_while(|c| c.is_ascii_digit() || c == '_');
    }

    // ── String literals ──────────────────────────────────────────────────

    /// Lex the opening of a string literal (`"` or `"""`).
    ///
    /// Emits `StringStart` and sets the string mode so the next call to
    /// `next()` will lex the string content.
    fn lex_string_start(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume first '"'

        // Check for triple-quote: two more '"' follow
        if self.cursor.peek() == Some('"') && self.cursor.peek_next() == Some('"') {
            self.cursor.advance(); // consume second '"'
            self.cursor.advance(); // consume third '"'
            self.string_mode = StringMode::Triple;
            Token::new(TokenKind::StringStart, start, self.cursor.pos())
        } else {
            self.string_mode = StringMode::Single;
            Token::new(TokenKind::StringStart, start, self.cursor.pos())
        }
    }

    /// Lex the content and closing of a single-quoted string.
    ///
    /// Returns `StringContent`, stores `StringEnd` as pending.
    /// On unterminated string, returns `Error`.
    fn lex_single_string_content(&mut self) -> Token {
        let start = self.cursor.pos();

        loop {
            match self.cursor.peek() {
                None => {
                    // Unterminated string
                    self.string_mode = StringMode::None;
                    return Token::new(TokenKind::Error, start, self.cursor.pos());
                }
                Some('"') => {
                    let content_end = self.cursor.pos();
                    self.cursor.advance(); // consume closing '"'
                    self.string_mode = StringMode::None;
                    self.pending_token = Some(Token::new(
                        TokenKind::StringEnd,
                        content_end,
                        self.cursor.pos(),
                    ));
                    return Token::new(TokenKind::StringContent, start, content_end);
                }
                Some('\\') => {
                    self.cursor.advance(); // consume '\'
                    self.cursor.advance(); // consume escaped char
                }
                Some(_) => {
                    self.cursor.advance();
                }
            }
        }
    }

    /// Lex the content and closing of a triple-quoted string.
    ///
    /// Returns `StringContent`, stores `StringEnd` as pending.
    /// On unterminated string, returns `Error`.
    fn lex_triple_string_content(&mut self) -> Token {
        let start = self.cursor.pos();

        loop {
            match self.cursor.peek() {
                None => {
                    // Unterminated triple-quote string
                    self.string_mode = StringMode::None;
                    return Token::new(TokenKind::Error, start, self.cursor.pos());
                }
                Some('"') => {
                    // Check for closing """
                    if self.cursor.peek_next() == Some('"') {
                        let saved_pos = self.cursor.pos();
                        self.cursor.advance(); // first '"'
                        self.cursor.advance(); // second '"'
                        if self.cursor.peek() == Some('"') {
                            // Found closing """
                            self.cursor.advance(); // third '"'
                            self.string_mode = StringMode::None;
                            self.pending_token = Some(Token::new(
                                TokenKind::StringEnd,
                                saved_pos,
                                self.cursor.pos(),
                            ));
                            return Token::new(TokenKind::StringContent, start, saved_pos);
                        }
                        // Only two quotes -- keep scanning (they're part of content)
                        continue;
                    }
                    // Single quote inside triple-quoted string is content
                    self.cursor.advance();
                }
                Some('\\') => {
                    self.cursor.advance(); // consume '\'
                    self.cursor.advance(); // consume escaped char
                }
                Some(_) => {
                    self.cursor.advance();
                }
            }
        }
    }

    // ── Identifiers and keywords ─────────────────────────────────────────

    /// Lex an identifier or keyword.
    fn lex_ident(&mut self, start: u32) -> Token {
        self.cursor.advance(); // consume first char
        self.cursor.eat_while(is_ident_continue);
        let text = self.cursor.slice(start, self.cursor.pos());

        let kind = keyword_from_str(text).unwrap_or(TokenKind::Ident);
        Token::new(kind, start, self.cursor.pos())
    }
}

impl<'src> Iterator for Lexer<'src> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        if self.emitted_eof {
            return None;
        }

        // Check for pending token first (e.g. StringEnd after StringContent).
        if let Some(token) = self.pending_token.take() {
            if token.kind == TokenKind::Eof {
                self.emitted_eof = true;
            }
            return Some(token);
        }

        // If we just emitted StringStart, lex the string body next.
        match self.string_mode {
            StringMode::Single => {
                let token = self.lex_single_string_content();
                return Some(token);
            }
            StringMode::Triple => {
                let token = self.lex_triple_string_content();
                return Some(token);
            }
            StringMode::None => {}
        }

        let token = self.next_token();
        if token.kind == TokenKind::Eof {
            self.emitted_eof = true;
        }
        Some(token)
    }
}

/// Whether a character can start an identifier.
fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// Whether a character can continue an identifier.
fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lex_simple_expression() {
        let tokens = Lexer::tokenize("let x = 42");
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Let,
                &TokenKind::Ident,
                &TokenKind::Eq,
                &TokenKind::IntLiteral,
                &TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_simple_string() {
        let tokens = Lexer::tokenize(r#""hello""#);
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::StringStart,
                &TokenKind::StringContent,
                &TokenKind::StringEnd,
                &TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_spans_accurate() {
        let tokens = Lexer::tokenize("let x = 42");
        // let: 0-3
        assert_eq!(tokens[0].span.start, 0);
        assert_eq!(tokens[0].span.end, 3);
        // x: 4-5
        assert_eq!(tokens[1].span.start, 4);
        assert_eq!(tokens[1].span.end, 5);
        // =: 6-7
        assert_eq!(tokens[2].span.start, 6);
        assert_eq!(tokens[2].span.end, 7);
        // 42: 8-10
        assert_eq!(tokens[3].span.start, 8);
        assert_eq!(tokens[3].span.end, 10);
    }
}
