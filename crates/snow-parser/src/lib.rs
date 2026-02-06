//! Snow parser: recursive descent parser producing a rowan-based CST.
//!
//! This crate transforms the token stream from `snow-lexer` into a lossless
//! concrete syntax tree (CST) using the `rowan` library. The CST preserves
//! all tokens including whitespace and comments, enabling future tooling
//! (formatter, LSP) to work from the same tree.

pub mod cst;
pub mod error;
mod parser;
pub mod syntax_kind;

pub use cst::{SyntaxElement, SyntaxNode, SyntaxToken};
pub use error::ParseError;
pub use syntax_kind::SyntaxKind;

/// Result of parsing a Snow source file.
///
/// Contains the green tree (the immutable, cheap-to-clone CST) and any
/// parse errors encountered. With the current first-error-only strategy,
/// `errors` will contain at most one error.
pub struct Parse {
    green: rowan::GreenNode,
    errors: Vec<ParseError>,
}

impl Parse {
    /// Build the syntax tree root from the green node.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Parse errors encountered during parsing.
    pub fn errors(&self) -> &[ParseError] {
        &self.errors
    }

    /// Whether parsing completed without errors.
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Parse a Snow source file into a CST.
///
/// This is the main entry point for the parser. It lexes the source,
/// parses the token stream, and returns a [`Parse`] result containing
/// the syntax tree and any errors.
pub fn parse(_source: &str) -> Parse {
    todo!("parse() will be implemented once the Parser struct is complete")
}
