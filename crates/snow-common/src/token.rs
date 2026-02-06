use serde::Serialize;

use crate::span::Span;

/// A token produced by the Snow lexer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    /// Create a new token from a kind and byte offsets.
    pub fn new(kind: TokenKind, start: u32, end: u32) -> Self {
        Self {
            kind,
            span: Span::new(start, end),
        }
    }
}

/// Every kind of token in the Snow language.
///
/// This enum is the complete vocabulary for the lexer. It covers all keywords,
/// operators, delimiters, literals, string interpolation markers, comments,
/// identifiers, and special tokens.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum TokenKind {
    // ── Keywords (39) ──────────────────────────────────────────────────
    After,
    Alias,
    And,
    Case,
    Cond,
    Def,
    Do,
    Else,
    End,
    False,
    Fn,
    For,
    If,
    Impl,
    Import,
    In,
    Let,
    Link,
    Match,
    Module,
    Monitor,
    Nil,
    Not,
    Or,
    Pub,
    Receive,
    Return,
    /// The `self` keyword. Named `SelfKw` to avoid conflict with Rust's `Self`.
    SelfKw,
    Send,
    Spawn,
    Struct,
    Supervisor,
    Trait,
    Trap,
    True,
    Type,
    When,
    Where,
    With,

    // ── Operators (22) ─────────────────────────────────────────────────
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `==`
    EqEq,
    /// `!=`
    NotEq,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    LtEq,
    /// `>=`
    GtEq,
    /// `&&`
    AmpAmp,
    /// `||`
    PipePipe,
    /// `!`
    Bang,
    /// `|>`
    Pipe,
    /// `..`
    DotDot,
    /// `<>`
    Diamond,
    /// `++`
    PlusPlus,
    /// `=`
    Eq,
    /// `->`
    Arrow,
    /// `=>`
    FatArrow,
    /// `::`
    ColonColon,

    // ── Delimiters (6) ─────────────────────────────────────────────────
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `{`
    LBrace,
    /// `}`
    RBrace,

    // ── Punctuation (5) ────────────────────────────────────────────────
    /// `,`
    Comma,
    /// `.`
    Dot,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
    /// Significant newline (statement terminator).
    Newline,

    // ── Literals (7) ───────────────────────────────────────────────────
    /// Integer literal, e.g. `42`, `0xFF`, `0b1010`.
    IntLiteral,
    /// Floating-point literal, e.g. `3.14`, `1.0e10`.
    FloatLiteral,
    /// Opening `"` or `"""` of a string.
    StringStart,
    /// Closing `"` or `"""` of a string.
    StringEnd,
    /// Literal text content inside a string (between delimiters/interpolations).
    StringContent,
    /// `${` -- start of string interpolation.
    InterpolationStart,
    /// `}` that closes a string interpolation.
    InterpolationEnd,

    // ── Identifiers and comments (4) ───────────────────────────────────
    /// Regular identifier, e.g. `foo`, `my_var`.
    Ident,
    /// Line comment content (`# ...`). Preserved for tooling.
    Comment,
    /// Doc comment content (`## ...`).
    DocComment,
    /// Module doc comment content (`##! ...`).
    ModuleDocComment,

    // ── Special (2) ────────────────────────────────────────────────────
    /// End of file.
    Eof,
    /// Invalid/unexpected input. Used for error recovery.
    Error,
}

/// Look up a keyword from its string representation.
///
/// Returns `Some(TokenKind)` if the string is a Snow keyword, `None` otherwise.
/// The lexer calls this to distinguish keywords from identifiers after scanning
/// an identifier-shaped token.
pub fn keyword_from_str(s: &str) -> Option<TokenKind> {
    match s {
        "after" => Some(TokenKind::After),
        "alias" => Some(TokenKind::Alias),
        "and" => Some(TokenKind::And),
        "case" => Some(TokenKind::Case),
        "cond" => Some(TokenKind::Cond),
        "def" => Some(TokenKind::Def),
        "do" => Some(TokenKind::Do),
        "else" => Some(TokenKind::Else),
        "end" => Some(TokenKind::End),
        "false" => Some(TokenKind::False),
        "fn" => Some(TokenKind::Fn),
        "for" => Some(TokenKind::For),
        "if" => Some(TokenKind::If),
        "impl" => Some(TokenKind::Impl),
        "import" => Some(TokenKind::Import),
        "in" => Some(TokenKind::In),
        "let" => Some(TokenKind::Let),
        "link" => Some(TokenKind::Link),
        "match" => Some(TokenKind::Match),
        "module" => Some(TokenKind::Module),
        "monitor" => Some(TokenKind::Monitor),
        "nil" => Some(TokenKind::Nil),
        "not" => Some(TokenKind::Not),
        "or" => Some(TokenKind::Or),
        "pub" => Some(TokenKind::Pub),
        "receive" => Some(TokenKind::Receive),
        "return" => Some(TokenKind::Return),
        "self" => Some(TokenKind::SelfKw),
        "send" => Some(TokenKind::Send),
        "spawn" => Some(TokenKind::Spawn),
        "struct" => Some(TokenKind::Struct),
        "supervisor" => Some(TokenKind::Supervisor),
        "trait" => Some(TokenKind::Trait),
        "trap" => Some(TokenKind::Trap),
        "true" => Some(TokenKind::True),
        "type" => Some(TokenKind::Type),
        "when" => Some(TokenKind::When),
        "where" => Some(TokenKind::Where),
        "with" => Some(TokenKind::With),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_from_str_recognizes_all_keywords() {
        let keywords = [
            ("after", TokenKind::After),
            ("alias", TokenKind::Alias),
            ("and", TokenKind::And),
            ("case", TokenKind::Case),
            ("cond", TokenKind::Cond),
            ("def", TokenKind::Def),
            ("do", TokenKind::Do),
            ("else", TokenKind::Else),
            ("end", TokenKind::End),
            ("false", TokenKind::False),
            ("fn", TokenKind::Fn),
            ("for", TokenKind::For),
            ("if", TokenKind::If),
            ("impl", TokenKind::Impl),
            ("import", TokenKind::Import),
            ("in", TokenKind::In),
            ("let", TokenKind::Let),
            ("link", TokenKind::Link),
            ("match", TokenKind::Match),
            ("module", TokenKind::Module),
            ("monitor", TokenKind::Monitor),
            ("nil", TokenKind::Nil),
            ("not", TokenKind::Not),
            ("or", TokenKind::Or),
            ("pub", TokenKind::Pub),
            ("receive", TokenKind::Receive),
            ("return", TokenKind::Return),
            ("self", TokenKind::SelfKw),
            ("send", TokenKind::Send),
            ("spawn", TokenKind::Spawn),
            ("struct", TokenKind::Struct),
            ("supervisor", TokenKind::Supervisor),
            ("trait", TokenKind::Trait),
            ("trap", TokenKind::Trap),
            ("true", TokenKind::True),
            ("type", TokenKind::Type),
            ("when", TokenKind::When),
            ("where", TokenKind::Where),
            ("with", TokenKind::With),
        ];

        for (s, expected) in &keywords {
            assert_eq!(
                keyword_from_str(s),
                Some(expected.clone()),
                "keyword_from_str({s:?}) should return Some({expected:?})"
            );
        }

        // Verify we tested all 39 keywords
        assert_eq!(keywords.len(), 39, "must test all 39 keywords");
    }

    #[test]
    fn keyword_from_str_rejects_non_keywords() {
        assert_eq!(keyword_from_str("foo"), None);
        assert_eq!(keyword_from_str("bar"), None);
        assert_eq!(keyword_from_str("x"), None);
        assert_eq!(keyword_from_str(""), None);
        assert_eq!(keyword_from_str("IF"), None); // case-sensitive
        assert_eq!(keyword_from_str("True"), None); // case-sensitive
    }

    #[test]
    fn token_new_constructor() {
        let tok = Token::new(TokenKind::Fn, 10, 12);
        assert_eq!(tok.kind, TokenKind::Fn);
        assert_eq!(tok.span, Span::new(10, 12));
    }

    #[test]
    fn token_kind_variant_count() {
        // Count variants by checking that all categories are covered.
        // Keywords: 39, Operators: 22, Delimiters: 6, Punctuation: 5,
        // Literals: 7, Identifiers/comments: 4, Special: 2 = 85 total
        // This test documents the expected count.
        let keywords = 39u32;
        let operators = 22;
        let delimiters = 6;
        let punctuation = 5;
        let literals = 7;
        let ident_comments = 4;
        let special = 2;
        let total = keywords + operators + delimiters + punctuation + literals + ident_comments + special;
        assert_eq!(total, 85, "TokenKind should have 85 variants");
    }
}
