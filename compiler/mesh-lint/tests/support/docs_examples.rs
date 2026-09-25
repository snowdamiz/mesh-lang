//! The Mesh examples on the documentation site, for the tests that check
//! them (the linter's and the formatter's).

use std::path::{Path, PathBuf};

fn markdown_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("docs directory") {
        let path = entry.expect("docs entry").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_dir() && !name.starts_with('.') && name != "node_modules" {
            markdown_files(&path, files);
        } else if name.ends_with(".md") {
            files.push(path);
        }
    }
}

/// `(first line, source)` of each ```mesh block.
fn mesh_blocks(markdown: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for (index, line) in markdown.lines().enumerate() {
        match open.as_mut() {
            None if line.starts_with("```mesh") => open = Some((index + 2, String::new())),
            Some(_) if line.starts_with("```") => blocks.extend(open.take()),
            Some((_, source)) => {
                source.push_str(line);
                source.push('\n');
            }
            None => {}
        }
    }
    blocks
}

/// Snippets that are not programs: the test runner's `assert_receive` lines
/// and the cheatsheet's list of bare patterns.
fn is_fragment(source: &str) -> bool {
    source.contains("assert_receive") || source.starts_with("# Wildcard and name binding")
}

/// Every example that is a program: `(file relative to the docs, first line,
/// source)`.
pub fn examples() -> Vec<(String, usize, String)> {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../website/docs");
    let mut files = Vec::new();
    markdown_files(&docs, &mut files);
    files.sort();
    let mut examples = Vec::new();
    for file in &files {
        let markdown = std::fs::read_to_string(file).expect("markdown");
        let relative = file
            .strip_prefix(&docs)
            .unwrap_or(file)
            .display()
            .to_string();
        for (line, source) in mesh_blocks(&markdown) {
            if !is_fragment(&source) {
                examples.push((relative.clone(), line, source));
            }
        }
    }
    assert!(
        examples.len() > 200,
        "found only {} examples under {}",
        examples.len(),
        docs.display()
    );
    examples
}
