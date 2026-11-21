//! How a `meshc` command reports diagnostics: type errors through a
//! `Reporter`, and parse and `@cluster` errors, which are not `Diagnostic`s
//! yet, in the same format.

use std::path::{Path, PathBuf};

use mesh_diagnostics::{Format, Reporter};

pub(crate) struct DiagnosticOptions {
    pub(crate) format: Format,
    /// Paths to name in place of others, `(from, to)`: a file, or a
    /// directory for the files under it. `meshc test` builds a copy of the
    /// project in a temporary directory, and names the user's files.
    pub(crate) display_paths: Vec<(PathBuf, PathBuf)>,
}

impl DiagnosticOptions {
    pub(crate) fn new(format: Format) -> Self {
        DiagnosticOptions {
            format,
            display_paths: Vec::new(),
        }
    }

    pub(crate) fn json(&self) -> bool {
        self.format == Format::Json
    }

    /// A reporter to stderr that names paths as these options do.
    pub(crate) fn reporter(&self) -> Reporter {
        let mut reporter = Reporter::stderr(self.format);
        for (from, to) in &self.display_paths {
            reporter.show_path_as(from, to);
        }
        reporter
    }

    /// Settings for the reports drawn here. Spans are byte offsets
    /// (ariadne's default is chars, which moves every report after a
    /// non-ASCII character).
    pub(crate) fn report_config(&self) -> ariadne::Config {
        ariadne::Config::default()
            .with_color(self.format == Format::Human { color: true })
            .with_index_type(ariadne::IndexType::Byte)
    }

    /// `path` as diagnostics name it (see `display_paths`).
    pub(crate) fn display_path(&self, path: &Path) -> String {
        for (from, to) in &self.display_paths {
            if let Ok(rest) = path.strip_prefix(from) {
                // `join("")` would add a separator.
                let to = if rest.as_os_str().is_empty() {
                    to.clone()
                } else {
                    to.join(rest)
                };
                return to.display().to_string();
            }
        }
        path.display().to_string()
    }
}
