//! The plaintext build report: every `declassify` call, with its reason,
//! and every `@display` export of a project, as deterministic JSON.
//!
//! `meshc build --plaintext-report PATH` writes it with a build, and
//! `meshc plaintext-report DIR [--output PATH] [--check COMMITTED]` prints,
//! writes or checks it. `--check` fails when a site was added, removed or
//! changed (a moved line is no change), so a project's CI can refuse
//! declassifications nobody reviewed. See `docs/security/plaintext-types.md`.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) const FORMAT: &str = "mesh-plaintext-report/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Report {
    pub format: String,
    pub package: String,
    pub declassify: Vec<Declassify>,
    pub display: Vec<Display>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct Declassify {
    pub file: String,
    pub line: u64,
    pub function: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct Display {
    pub file: String,
    pub line: u64,
    pub function: String,
    pub symbol: String,
}

/// The report of a checked project: its modules' sites, by file and line.
pub(crate) fn collect(
    dir: &Path,
    package: &str,
    project: &mesh_pkg::project::ProjectData,
    typeck: &[mesh_typeck::TypeckResult],
) -> Report {
    let mut report = Report {
        format: FORMAT.to_string(),
        package: package.to_string(),
        declassify: Vec::new(),
        display: Vec::new(),
    };
    for &id in &project.compilation_order {
        let index = id.0 as usize;
        let source = &project.module_sources[index];
        let file = display_path(dir, &project.graph.get(id).path);
        let facts = &typeck[index].plaintext;
        report
            .declassify
            .extend(facts.declassify_sites.iter().map(|site| Declassify {
                file: file.clone(),
                line: line_of(source, u32::from(site.span.start()) as usize),
                function: site.function.clone(),
                reason: site.reason.clone(),
            }));
        report
            .display
            .extend(facts.display_exports.iter().map(|export| Display {
                file: file.clone(),
                line: line_of(source, u32::from(export.span.start()) as usize),
                function: export.function.clone(),
                symbol: export.symbol.clone(),
            }));
    }
    report.declassify.sort();
    report.display.sort();
    report
}

/// The report as it is written: pretty JSON and a final newline.
pub(crate) fn render(report: &Report) -> String {
    serde_json::to_string_pretty(report).expect("the report is serializable") + "\n"
}

pub(crate) fn write(report: &Report, path: &Path) -> Result<(), String> {
    std::fs::write(path, render(report))
        .map_err(|error| format!("Failed to write '{}': {error}", path.display()))
}

/// What differs between `committed` and `current`, line numbers aside: a
/// `+` for a site only the build has, a `-` for one it no longer has.
pub(crate) fn changes(committed: &Report, current: &Report) -> Vec<String> {
    let declassify = |report: &Report| -> Vec<String> {
        report
            .declassify
            .iter()
            .map(|site| {
                format!(
                    "declassify in `{}` ({}): {}",
                    site.function, site.file, site.reason
                )
            })
            .collect()
    };
    let display = |report: &Report| -> Vec<String> {
        report
            .display
            .iter()
            .map(|export| {
                format!(
                    "display export `{}` ({}) as {}",
                    export.function, export.file, export.symbol
                )
            })
            .collect()
    };
    let mut changed = Vec::new();
    for (before, after) in [
        (declassify(committed), declassify(current)),
        (display(committed), display(current)),
    ] {
        changed.extend(
            only_in(&after, &before)
                .into_iter()
                .map(|site| format!("+ {site}")),
        );
        changed.extend(
            only_in(&before, &after)
                .into_iter()
                .map(|site| format!("- {site}")),
        );
    }
    changed
}

/// The entries of `these` that `those` lacks, counting repeats.
fn only_in<'a>(these: &'a [String], those: &[String]) -> Vec<&'a String> {
    let mut unmatched: Vec<&String> = those.iter().collect();
    these
        .iter()
        .filter(
            |entry| match unmatched.iter().position(|other| other == entry) {
                Some(found) => {
                    unmatched.swap_remove(found);
                    false
                }
                None => true,
            },
        )
        .collect()
}

/// `meshc plaintext-report`: print the report, write it, or check it
/// against a committed one.
pub(crate) fn run(
    report: &Report,
    output: Option<&Path>,
    check: Option<&Path>,
) -> Result<(), String> {
    if let Some(path) = output {
        write(report, path)?;
    }
    let Some(committed_path) = check else {
        if output.is_none() {
            print!("{}", render(report));
        }
        return Ok(());
    };
    let text = std::fs::read_to_string(committed_path)
        .map_err(|error| format!("Failed to read '{}': {error}", committed_path.display()))?;
    let committed: Report = serde_json::from_str(&text).map_err(|error| {
        format!(
            "'{}' is not a plaintext report: {error}",
            committed_path.display()
        )
    })?;
    let changed = changes(&committed, report);
    if changed.is_empty() {
        eprintln!(
            "plaintext report matches {} ({} declassify sites, {} display exports)",
            committed_path.display(),
            report.declassify.len(),
            report.display.len()
        );
        if committed != *report {
            eprintln!("note: sites moved to other lines; write the report again to update them");
        }
        return Ok(());
    }
    for change in &changed {
        eprintln!("{change}");
    }
    Err(format!(
        "the plaintext report differs from {}: review the changes above, then write it with \
         `meshc plaintext-report <dir> --output {}`",
        committed_path.display(),
        committed_path.display()
    ))
}

/// A module's file as the report names it: relative to the project, with
/// `/` between its parts, wherever the project is checked out.
fn display_path(dir: &Path, path: &Path) -> String {
    let relative = if path.is_absolute() {
        let base = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        relative_path(&base, &target)
    } else {
        path.to_path_buf()
    };
    relative
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// `target` from `base`, both absolute: `../lib/a.mpl`.
fn relative_path(base: &Path, target: &Path) -> PathBuf {
    let base: Vec<_> = base.components().collect();
    let target: Vec<_> = target.components().collect();
    let shared = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut relative = PathBuf::new();
    for _ in shared..base.len() {
        relative.push("..");
    }
    for component in &target[shared..] {
        relative.push(component.as_os_str());
    }
    relative
}

/// The 1-based line of byte `offset` in `source`.
fn line_of(source: &str, offset: usize) -> u64 {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count() as u64
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(file: &str, line: u64, function: &str, reason: &str) -> Declassify {
        Declassify {
            file: file.to_string(),
            line,
            function: function.to_string(),
            reason: reason.to_string(),
        }
    }

    fn report(declassify: Vec<Declassify>) -> Report {
        Report {
            format: FORMAT.to_string(),
            package: "p".to_string(),
            declassify,
            display: Vec::new(),
        }
    }

    /// A site said twice is two sites: removing one is a change.
    #[test]
    fn repeated_sites_count_and_lines_do_not() {
        let twice = report(vec![
            site("a.mpl", 1, "f", "why"),
            site("a.mpl", 9, "f", "why"),
        ]);
        let once_moved = report(vec![site("a.mpl", 4, "f", "why")]);
        assert_eq!(
            changes(&twice, &once_moved),
            ["- declassify in `f` (a.mpl): why"]
        );
        assert!(changes(&once_moved, &report(vec![site("a.mpl", 7, "f", "why")])).is_empty());
    }

    #[test]
    fn a_dependency_outside_the_project_is_named_relative_to_it() {
        assert_eq!(
            relative_path(Path::new("/w/app/core"), Path::new("/w/app/lib/x.mpl")),
            PathBuf::from("../lib/x.mpl")
        );
    }
}
