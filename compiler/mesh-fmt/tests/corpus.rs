//! Every Mesh source in the repository formats without the formatter
//! refusing it (a refusal is a formatter bug: the output would have changed
//! the program), and formatting is idempotent.

use std::path::{Path, PathBuf};

use mesh_fmt::{format_source, try_format, FormatConfig};

#[path = "../../mesh-lint/tests/support/docs_examples.rs"]
mod docs_examples;

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            sources(&path, out);
        } else if name.ends_with(".mpl") {
            out.push(path);
        }
    }
}

#[test]
fn every_repository_source_formats_losslessly_and_idempotently() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in [
        "tests",
        "examples",
        "benchmarks",
        "packages",
        "mesh-slug",
        "scripts",
    ] {
        sources(&root.join(dir), &mut files);
    }
    assert!(files.len() > 200, "found only {} sources", files.len());
    let config = FormatConfig::default();
    let mut problems = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        if !mesh_parser::parse(&source).errors().is_empty() {
            continue; // a fixture of a syntax error
        }
        match try_format(&source, &config) {
            Err(reason) => problems.push(format!("{}: {reason}", file.display())),
            Ok(formatted) => {
                if format_source(&formatted, &config) != formatted {
                    problems.push(format!("{}: formatting again changes it", file.display()));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Every Mesh example in the docs formats losslessly and idempotently too:
/// they use constructs no repository source does.
#[test]
fn every_docs_example_formats_losslessly_and_idempotently() {
    let config = FormatConfig::default();
    let mut problems = Vec::new();
    for (file, line, source) in docs_examples::examples() {
        match try_format(&source, &config) {
            Err(reason) => problems.push(format!("{file}:{line}: {reason}")),
            Ok(formatted) => {
                if format_source(&formatted, &config) != formatted {
                    problems.push(format!("{file}:{line}: formatting again changes it"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Statements continued across lines by an operator or `=` format as one
/// statement, and formatting them again changes nothing.
#[test]
fn continued_statements_format_losslessly() {
    let config = FormatConfig::default();
    for source in [
        "fn f(a, b) do\n  let x =\n    a\n  let y = a +\n    b\n  let z = a\n    + b\n  let w = a and\n    b\n  x\nend\n",
        "fn f(a) =\n  a * 2\n",
    ] {
        let formatted = try_format(source, &config).unwrap_or_else(|e| panic!("{e}\n{source}"));
        assert_eq!(format_source(&formatted, &config), formatted, "{formatted}");
        println!("{formatted}");
    }
}

/// Every source and docs example with a comment at the end of each line
/// formats losslessly and idempotently: a line may end inside any construct,
/// and its comment has to stay there.
#[test]
fn trailing_comments_on_every_line_format_losslessly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["tests", "examples", "packages", "mesh-slug"] {
        sources(&root.join(dir), &mut files);
    }
    let mut programs: Vec<(String, String)> = files
        .iter()
        .map(|file| {
            (
                file.display().to_string(),
                std::fs::read_to_string(file).unwrap(),
            )
        })
        .collect();
    programs.extend(
        docs_examples::examples()
            .into_iter()
            .map(|(file, line, source)| (format!("{file}:{line}"), source)),
    );
    let config = FormatConfig::default();
    let (mut checked, mut problems) = (0, Vec::new());
    for (name, source) in programs {
        let commented: String = source
            .lines()
            .map(|line| {
                if line.trim().is_empty() {
                    "\n".to_string()
                } else {
                    format!("{line} # c\n")
                }
            })
            .collect();
        // A line inside a string or a heredoc changes the program; skip what
        // no longer parses.
        if !mesh_parser::parse(&source).errors().is_empty()
            || !mesh_parser::parse(&commented).errors().is_empty()
        {
            continue;
        }
        checked += 1;
        match try_format(&commented, &config) {
            Err(reason) => problems.push(format!("{name}: {reason}")),
            Ok(formatted) => {
                if format_source(&formatted, &config) != formatted {
                    problems.push(format!("{name}: formatting again changes it"));
                }
            }
        }
    }
    assert!(checked > 300, "checked only {checked}");
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// A comment may sit after any token of any construct; the formatter keeps
/// every one (it refuses output that would drop or move one into code).
#[test]
fn comments_everywhere_format_losslessly() {
    let source = include_str!("comments.mpl");
    let parse = mesh_parser::parse(source);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    let config = FormatConfig::default();
    let formatted = try_format(source, &config).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(format_source(&formatted, &config), formatted, "{formatted}");
}
