//! Snow code formatter.
//!
//! This crate implements a canonical code formatter for Snow source code using
//! the Wadler-Lindig document IR approach. It works by:
//!
//! 1. Parsing source code to a CST (via `snow-parser`)
//! 2. Walking the CST to produce a `FormatIR` document tree
//! 3. Printing the IR to a string, respecting line width constraints
//!
//! The CST-based approach preserves comments and trivia while allowing the
//! formatter to rewrite whitespace and indentation canonically.

pub mod ir;
pub mod printer;
pub mod walker;

pub use printer::FormatConfig;

/// Format Snow source code according to the given configuration.
///
/// Parses the source, walks the CST to produce format IR, and prints the
/// result as a formatted string. Comments are preserved in their original
/// positions relative to code.
///
/// # Example
///
/// ```
/// use snow_fmt::{format_source, FormatConfig};
///
/// let source = "fn add(a, b) do\na + b\nend";
/// let formatted = format_source(source, &FormatConfig::default());
/// assert!(formatted.contains("add"));
/// ```
pub fn format_source(source: &str, config: &FormatConfig) -> String {
    let parse = snow_parser::parse(source);
    let root = parse.syntax();
    let doc = walker::walk_node(&root);
    printer::print(&doc, config)
}
