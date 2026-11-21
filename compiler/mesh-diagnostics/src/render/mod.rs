//! The one writer of diagnostics in a CLI-like process.

mod human;
mod json;

use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::{Code, Diagnostic, Severity};

/// How a [`Reporter`] writes: Ariadne reports for a person, or one JSON
/// object per line for a tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Human { color: bool },
    Json,
}

impl Format {
    /// `meshc` and `meshpkg`: JSON if asked, otherwise colour only for a
    /// stderr terminal, with neither `NO_COLOR` set nor `--no-color` given.
    pub fn for_cli(json: bool, no_color: bool) -> Format {
        if json {
            Format::Json
        } else {
            Format::Human {
                color: !no_color && color_allowed(),
            }
        }
    }

    /// A compiled program: JSON when `MESH_DIAGNOSTICS=json` (as `meshc test
    /// --json` sets it), otherwise as [`for_cli`](Self::for_cli) without flags.
    pub fn for_runtime() -> Format {
        let json = std::env::var_os("MESH_DIAGNOSTICS").is_some_and(|value| value == "json");
        Format::for_cli(json, false)
    }
}

fn color_allowed() -> bool {
    std::env::var_os("NO_COLOR").is_none() && io::stderr().is_terminal()
}

/// Proof that an error diagnostic was reported. Only a [`Reporter`] makes
/// one, so a command cannot fail without saying why.
#[must_use]
#[derive(Debug)]
pub struct Failed(());

/// What a [`Reporter`] wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub errors: u32,
    pub warnings: u32,
}

impl Summary {
    /// `1` when an error was reported. Warnings alone do not fail a build.
    pub fn exit_code(&self) -> i32 {
        (self.errors > 0) as i32
    }
}

/// The only writer of diagnostics in a process: it owns the format, the
/// paths diagnostics name, the footer shown once per code, and the tally.
/// One owner passes it `&mut`; concurrent producers return diagnostics for
/// the owner to emit.
pub struct Reporter<W: Write = io::Stderr> {
    format: Format,
    out: W,
    paths: Paths,
    /// Each code reported, in first-seen order, with how often.
    codes: Vec<(Code, u32)>,
    summary: Summary,
}

impl Reporter<io::Stderr> {
    pub fn stderr(format: Format) -> Self {
        Reporter::new(format, io::stderr())
    }
}

impl<W: Write> Reporter<W> {
    /// A reporter writing to `out`; a test passes `&mut Vec<u8>`.
    pub fn new(format: Format, out: W) -> Self {
        Reporter {
            format,
            out,
            paths: Paths::default(),
            codes: Vec::new(),
            summary: Summary {
                errors: 0,
                warnings: 0,
            },
        }
    }

    /// Names `to` in place of `from` in every path written: a file, or a
    /// directory for the files under it. `meshc test` builds a copy of the
    /// project in a temporary directory, and names the user's files.
    pub fn show_path_as(&mut self, from: impl Into<PathBuf>, to: impl Into<PathBuf>) {
        self.paths.0.push((from.into(), to.into()));
    }

    pub fn emit(&mut self, diagnostic: Diagnostic) {
        match diagnostic.severity() {
            Severity::Error => self.summary.errors += 1,
            Severity::Warning => self.summary.warnings += 1,
        }
        let code = diagnostic.code();
        let first = match self.codes.iter_mut().find(|(seen, _)| *seen == code) {
            Some((_, count)) => {
                *count += 1;
                false
            }
            None => {
                self.codes.push((code, 1));
                true
            }
        };
        let text = match self.format {
            Format::Human { color } => {
                let mut text = human::render(&diagnostic, color, &self.paths);
                if first {
                    text.push_str(&human::footer(code, color));
                }
                text
            }
            Format::Json => json::render(&diagnostic, &self.paths),
        };
        self.write(&text);
    }

    /// Reports `diagnostic`, an error, and returns the proof that it was.
    pub fn fail(&mut self, diagnostic: Diagnostic) -> Failed {
        debug_assert_eq!(diagnostic.severity(), Severity::Error);
        self.emit(diagnostic);
        Failed(())
    }

    /// Reports every diagnostic; `Err` if any was an error.
    pub fn emit_all(
        &mut self,
        diagnostics: impl IntoIterator<Item = Diagnostic>,
    ) -> Result<(), Failed> {
        let mut failed = false;
        for diagnostic in diagnostics {
            failed |= diagnostic.severity() == Severity::Error;
            self.emit(diagnostic);
        }
        if failed {
            Err(Failed(()))
        } else {
            Ok(())
        }
    }

    /// A progress line for a person, as `  Compiled: target/app`. JSON
    /// output carries diagnostics only, so it writes nothing there.
    pub fn status(&mut self, line: impl fmt::Display) {
        if let Format::Human { .. } = self.format {
            self.write(&format!("{line}\n"));
        }
    }

    /// Ends the run: a person is shown the tally of what was reported.
    pub fn finish(mut self) -> Summary {
        if let Format::Human { color } = self.format {
            if let Some(tally) = human::tally(self.summary, &self.codes, color) {
                self.write(&tally);
            }
        }
        self.summary
    }

    /// One write per diagnostic keeps its lines together. A closed stderr
    /// leaves nowhere to say that writing failed.
    fn write(&mut self, text: &str) {
        let _ = self.out.write_all(text.as_bytes());
        let _ = self.out.flush();
    }
}

/// The `show_path_as` mappings, `(from, to)`.
#[derive(Default)]
struct Paths(Vec<(PathBuf, PathBuf)>);

impl Paths {
    fn display(&self, path: &Path) -> String {
        for (from, to) in &self.0 {
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
