//! Mesh's diagnostic catalog and the diagnostic value every tool reports.
//!
//! **Adding a diagnostic.** Add one row to `src/catalog.rs`: a code in its
//! family, a kebab-case name, a severity, a title, a one-sentence summary, a
//! concrete default repair, a guide topic, and for P/E/W/L codes a minimal
//! bad example with its fix. Build the diagnostic where the failure is found
//! with `Diagnostic::in_file(Code::YourName, file, observation).at(span,
//! label)`, and add `.with_repair(..)` when the instance allows a more
//! specific repair than the catalog's. Library crates return diagnostics;
//! they never print them.

#[macro_use]
mod catalog_macro;
mod catalog;
mod code;
mod diagnostic;
pub mod docs;
mod render;
mod source;

pub use catalog::Code;
pub use code::{closest, CodeInfo, Example, Family, Lookup, Severity, Status, DOCS_BASE};
pub use diagnostic::{Applicability, Diagnostic, Edit, Label};
pub use render::{Failed, Format, Reporter, Summary};
pub use source::{LineCol, SourceFile};
