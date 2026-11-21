//! What the lexer rejects: one [`LexError`] per `TokenKind::Error` token.

use mesh_common::span::Span;
use mesh_diagnostics::Code;

/// Why the lexer could not read a token. Each kind is one catalog code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexErrorKind {
    UnknownCharacter,
    UnterminatedString,
    UnterminatedInterpolation,
    UnterminatedBlockComment,
    InvalidSlotPipe(SlotPipeError),
    InvalidRegex(RegexError),
    SingleAmpersand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotPipeError {
    /// `|0>`: arguments are numbered from 1.
    Zero,
    /// `|1>`: `|>` is the pipe into the first argument.
    One,
    /// `|2` with no `>`.
    Unclosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegexError {
    /// `~` not followed by `r/`.
    Start,
    /// No closing `/` before the end of the file.
    Unterminated,
    /// A flag other than `i`, `m`, or `s`.
    Flag,
}

/// A token the lexer rejected: the `TokenKind::Error` token at `span`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LexError {
    pub kind: LexErrorKind,
    pub span: Span,
}

impl LexErrorKind {
    pub fn code(self) -> Code {
        match self {
            Self::UnknownCharacter => Code::UnknownCharacter,
            Self::UnterminatedString => Code::UnterminatedString,
            Self::UnterminatedInterpolation => Code::UnterminatedInterpolation,
            Self::UnterminatedBlockComment => Code::UnterminatedBlockComment,
            Self::InvalidSlotPipe(_) => Code::InvalidSlotPipe,
            Self::InvalidRegex(_) => Code::InvalidRegexLiteral,
            Self::SingleAmpersand => Code::SingleAmpersand,
        }
    }

    /// What is wrong with `text`, the rejected token's source text.
    pub fn message(self, text: &str) -> String {
        match self {
            Self::UnknownCharacter => format!("unknown character `{text}`"),
            Self::UnterminatedString => "unterminated string: no closing `\"`".to_string(),
            Self::UnterminatedInterpolation => {
                "unterminated interpolation: no closing `}` before the end of the file".to_string()
            }
            Self::UnterminatedBlockComment => {
                "unterminated block comment: no closing `=#` before the end of the file".to_string()
            }
            Self::InvalidSlotPipe(SlotPipeError::Zero) => {
                format!("`{text}` is not a pipe: arguments are numbered from 1")
            }
            Self::InvalidSlotPipe(SlotPipeError::One) => {
                format!("`{text}` is not a pipe: `|>` pipes into the first argument")
            }
            Self::InvalidSlotPipe(SlotPipeError::Unclosed) => {
                format!("`{text}` is not a pipe: a slot pipe ends with `>`, as in `{text}>`")
            }
            Self::InvalidRegex(RegexError::Start) => {
                format!("`{text}` does not start a regex literal: write `~r/pattern/`")
            }
            Self::InvalidRegex(RegexError::Unterminated) => {
                "unterminated regex literal: no closing `/` before the end of the file".to_string()
            }
            Self::InvalidRegex(RegexError::Flag) => {
                let flag = text.chars().next_back().unwrap_or_default();
                format!("unknown regex flag `{flag}`: the flags are `i`, `m`, and `s`")
            }
            Self::SingleAmpersand => {
                "`&` is not an operator: logical and is `&&` or `and`".to_string()
            }
        }
    }
}
