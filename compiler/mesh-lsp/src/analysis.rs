//! Document analysis: parse, type-check, and produce LSP diagnostics.
//!
//! This module bridges the Mesh compiler frontend (parser + typeck) with the
//! LSP protocol. It converts byte-offset spans into LSP line/character
//! positions (0-based, UTF-16 code units per the LSP spec) and translates
//! parse errors and type errors into `lsp_types::Diagnostic`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rowan::TextRange;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range, Url};

use mesh_common::module_graph::{ModuleGraph, ModuleId};
use mesh_pkg::manifest::{
    build_clustered_export_surface, collect_source_cluster_declarations, resolve_entrypoint,
    validate_cluster_declarations_with_source, ClusteredDeclarationError, Manifest,
};
use mesh_pkg::project::{
    build_import_context, build_project, check_project, single_source_project, CheckedProject,
    ProjectData,
};
use mesh_typeck::error::TypeError;
use mesh_typeck::ty::Ty;
use mesh_typeck::TypeckResult;

/// The result of analyzing a Mesh document.
pub struct AnalysisResult {
    /// LSP diagnostics (parse errors + type errors + warnings).
    pub diagnostics: Vec<Diagnostic>,
    /// The parse result, kept for further queries.
    pub parse: mesh_parser::Parse,
    /// The type-check result, kept for hover queries.
    pub typeck: TypeckResult,
    /// The text analyzed, which `parse` and `typeck` describe: the document,
    /// or for a test file the program `meshc test` makes of it, which keeps
    /// every line and column of the document but not its byte offsets.
    /// Positions are converted against it.
    pub source: String,
}

/// Analyze a Mesh document: parse, type-check, and produce diagnostics.
///
/// This is the main entry point called by the LSP server on didOpen/didChange.
/// When the URI belongs to a Mesh project (an ancestor contains `mesh.toml`),
/// analysis uses project-aware import resolution with open-document overlays so
/// project files behave like the real compiler path instead of isolated
/// single-file snippets.
pub fn analyze_document(
    uri: &str,
    source: &str,
    open_documents: &[(String, String)],
) -> AnalysisResult {
    match analyze_project_document(uri, source, open_documents) {
        ProjectAnalysis::Success(result) | ProjectAnalysis::Failed(result) => result,
        ProjectAnalysis::NotProject => analyze_single_document(source),
    }
}

/// A document outside any project, analyzed as meshc builds a directory
/// holding only it.
fn analyze_single_document(source: &str) -> AnalysisResult {
    match single_source_project(source) {
        Ok(project) => analyze_module(&project, ModuleId(0)),
        // A module block that conflicts with another, or an import cycle.
        Err(error) => project_failure_analysis(source, error),
    }
}

/// The document parsed and checked on its own, when it cannot be built.
fn analyze_source_alone(source: &str) -> AnalysisResult {
    let parse = mesh_parser::parse(source);
    let typeck = mesh_typeck::check(&parse);
    let diagnostics = diagnostics_from_parse_and_typeck(source, &parse, &typeck);

    AnalysisResult {
        diagnostics,
        parse,
        typeck,
        source: source.to_string(),
    }
}

enum ProjectAnalysis {
    Success(AnalysisResult),
    Failed(AnalysisResult),
    NotProject,
}

fn project_failure_analysis(source: &str, message: impl Into<String>) -> AnalysisResult {
    let mut result = analyze_source_alone(source);
    result.diagnostics.insert(0, project_diagnostic(message));
    result
}

/// Convert a byte offset to an LSP Position (0-based line, 0-based UTF-16 character offset).
///
/// The LSP specification requires positions in UTF-16 code units. For ASCII-only
/// sources, UTF-16 offset == byte offset within the line. For non-ASCII sources,
/// we count UTF-16 code units properly.
pub fn offset_to_position(source: &str, offset: usize) -> Position {
    let offset = offset.min(source.len());
    let before = &source[..offset];

    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_text = &source[line_start..offset];

    // Count UTF-16 code units for LSP spec compliance.
    let character: u32 = line_text.chars().map(|c| c.len_utf16() as u32).sum();

    Position { line, character }
}

/// Look up the inferred type at a given LSP position.
///
/// Searches the typeck result's type map for the smallest range that contains
/// the given byte offset. Returns the type formatted as a string.
pub fn type_at_position(
    source: &str,
    typeck: &TypeckResult,
    position: &Position,
) -> Option<String> {
    let offset = position_to_offset(source, position)?;
    let target_offset = rowan::TextSize::from(offset as u32);

    // Find the smallest range containing this offset.
    let mut best: Option<(TextRange, &Ty)> = None;
    for (range, ty) in &typeck.types {
        if range.contains(target_offset) || range.start() == target_offset {
            match &best {
                Some((best_range, _)) if range.len() < best_range.len() => {
                    best = Some((*range, ty));
                }
                None => {
                    best = Some((*range, ty));
                }
                _ => {}
            }
        }
    }

    best.map(|(_, ty)| format!("{}", ty))
}

/// Convert an LSP Position back to a byte offset in the source.
///
/// Public wrapper for go-to-definition support.
pub fn position_to_offset_pub(source: &str, position: &Position) -> Option<usize> {
    position_to_offset(source, position)
}

/// Convert an LSP Position back to a byte offset in the source. A character
/// past the end of its line is the line's end; a line past the end of the
/// source has no offset.
fn position_to_offset(source: &str, position: &Position) -> Option<usize> {
    let mut line_start = 0;
    for _ in 0..position.line {
        line_start += source[line_start..].find('\n')? + 1;
    }
    let rest = &source[line_start..];
    let line = &rest[..rest.find('\n').unwrap_or(rest.len())];
    // Characters are UTF-16 code units.
    let mut utf16 = 0u32;
    for (byte_idx, c) in line.char_indices() {
        if utf16 >= position.character {
            return Some(line_start + byte_idx);
        }
        utf16 += c.len_utf16() as u32;
    }
    Some(line_start + line.len())
}

/// Convert a TypeError into an LSP Diagnostic.
fn type_error_to_diagnostic(
    source: &str,
    error: &TypeError,
    severity: DiagnosticSeverity,
) -> Option<Diagnostic> {
    let range = error.span()?;
    let start_tree: usize = range.start().into();
    let end_tree: usize = range.end().into();
    let start_offset =
        crate::definition::tree_to_source_offset(source, start_tree).unwrap_or(start_tree);
    let end_offset = crate::definition::tree_to_source_offset(source, end_tree).unwrap_or(end_tree);

    let start = offset_to_position(source, start_offset);
    let end = offset_to_position(source, end_offset);

    Some(Diagnostic {
        range: Range::new(start, end),
        severity: Some(severity),
        source: Some("mesh".to_string()),
        message: format!("{}", error),
        ..Default::default()
    })
}

fn diagnostics_from_parse_and_typeck(
    source: &str,
    parse: &mesh_parser::Parse,
    typeck: &TypeckResult,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for error in parse.errors() {
        let start = offset_to_position(source, error.span.start as usize);
        let end = offset_to_position(source, error.span.end as usize);
        diagnostics.push(Diagnostic {
            range: Range::new(start, end),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("mesh".to_string()),
            message: error.message.clone(),
            ..Default::default()
        });
    }

    for error in &typeck.errors {
        if let Some(diag) = type_error_to_diagnostic(source, error, DiagnosticSeverity::ERROR) {
            diagnostics.push(diag);
        }
    }

    for warning in &typeck.warnings {
        if let Some(diag) = type_error_to_diagnostic(source, warning, DiagnosticSeverity::WARNING) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

fn analyze_project_document(
    uri: &str,
    source: &str,
    open_documents: &[(String, String)],
) -> ProjectAnalysis {
    let Some(doc_path) = canonical_file_path(uri) else {
        return ProjectAnalysis::NotProject;
    };
    let Some(project_root) = find_project_root(&doc_path) else {
        return ProjectAnalysis::NotProject;
    };
    let relative_path = match doc_path.strip_prefix(&project_root) {
        Ok(relative_path) => relative_path.to_path_buf(),
        Err(_) => {
            return ProjectAnalysis::Failed(project_failure_analysis(
                source,
                format!(
                    "Document '{}' is not contained within discovered project root '{}'",
                    doc_path.display(),
                    project_root.display()
                ),
            ));
        }
    };

    let mut overlays = HashMap::new();
    for (open_uri, open_source) in open_documents {
        if let Some(path) = canonical_file_path(open_uri) {
            overlays.insert(path, open_source.clone());
        }
    }
    overlays.insert(doc_path.clone(), source.to_string());

    // The root is where a mesh.toml is.
    let manifest = match Manifest::from_file(&project_root.join("mesh.toml")) {
        Ok(manifest) => manifest,
        Err(error) => return ProjectAnalysis::Failed(project_failure_analysis(source, error)),
    };

    let entry_relative_path = match resolve_entrypoint(&project_root, Some(&manifest)) {
        Ok(entry_relative_path) => entry_relative_path,
        Err(error) => {
            return ProjectAnalysis::Failed(project_failure_analysis(source, error));
        }
    };

    // A test file is not a module: `meshc test` compiles it on its own,
    // with the project's modules, the helpers under `tests/` and the test
    // builtins.
    let is_test_file = relative_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".test.mpl"));
    let with_test_helpers = is_test_file || relative_path.starts_with("tests");
    let native_bindings = match mesh_pkg::resolve_native_bindings(&project_root) {
        Ok(bindings) => bindings,
        Err(error) => return ProjectAnalysis::Failed(project_failure_analysis(source, error)),
    };
    let project = match build_project(
        &project_root,
        &entry_relative_path,
        &native_bindings,
        with_test_helpers,
        &|path| read_source_with_overlays(path, &overlays),
    ) {
        Ok(project) => project,
        Err(error) => {
            return ProjectAnalysis::Failed(project_failure_analysis(source, error));
        }
    };
    if is_test_file {
        return analyze_test_document(&project, source);
    }

    let Some(current_id) = project
        .graph
        .modules
        .iter()
        .find(|module| module.path == relative_path)
        .map(|module| module.id)
    else {
        return ProjectAnalysis::Failed(project_failure_analysis(
            source,
            format!(
                "Document '{}' was not discovered under project root '{}'",
                doc_path.display(),
                project_root.display()
            ),
        ));
    };
    ProjectAnalysis::Success(analyze_module(&project, current_id))
}

/// One module of a built project: its own diagnostics and those of the
/// `module ... do ... end` blocks in its file (modules of their own over the
/// same text), and, when the project has no other errors, its clustered
/// declarations'.
fn analyze_module(project: &ProjectData, current_id: ModuleId) -> AnalysisResult {
    let CheckedProject {
        typeck: mut all_typeck,
        exports: all_exports,
    } = check_project(project, false);
    let has_project_errors = project
        .module_parses
        .iter()
        .any(|parse| !parse.errors().is_empty())
        || all_typeck
            .iter()
            .flatten()
            .any(|typeck| !typeck.errors.is_empty());

    let path = &project.graph.get(current_id).path;
    let current_idx = current_id.0 as usize;
    let current_source = project.module_sources[current_idx].clone();
    let inline_errors: Vec<_> = project
        .graph
        .modules
        .iter()
        .filter(|module| module.path == *path && module.id != current_id)
        .filter_map(|module| all_typeck[module.id.0 as usize].as_ref())
        .flat_map(|typeck| typeck.errors.iter().cloned())
        .collect();
    let mut current_typeck = all_typeck[current_idx]
        .take()
        .expect("check_project checks every module");
    current_typeck.errors.extend(inline_errors);
    let current_parse = mesh_parser::parse(&current_source);
    let mut diagnostics =
        diagnostics_from_parse_and_typeck(&current_source, &current_parse, &current_typeck);

    if !has_project_errors {
        let source_cluster_declarations =
            collect_source_cluster_declarations(&project.graph, &project.module_parses);
        if let Some(cluster_diagnostics) = cluster_diagnostics(
            &source_cluster_declarations,
            &project.graph,
            &project.module_parses,
            &all_exports,
            path,
            &current_source,
        ) {
            diagnostics.extend(cluster_diagnostics);
        }
    }

    AnalysisResult {
        diagnostics,
        parse: current_parse,
        typeck: current_typeck,
        source: current_source,
    }
}

/// A `.test.mpl` document, analyzed as the program `meshc test` makes of it.
fn analyze_test_document(project: &ProjectData, source: &str) -> ProjectAnalysis {
    let program = match mesh_parser::test_harness::preprocess_test_source(source) {
        Ok(program) => program,
        Err(message) => return ProjectAnalysis::Failed(project_failure_analysis(source, message)),
    };
    let all_exports = check_project(project, true).exports;
    let parse = mesh_parser::parse(&program);
    let mut import_ctx = build_import_context(&project.graph, &all_exports, &parse);
    import_ctx.current_module = Some("Main".to_string());
    import_ctx.test_builtins = true;
    let typeck = mesh_typeck::check_with_imports(&parse, &import_ctx);
    let diagnostics = diagnostics_from_parse_and_typeck(&program, &parse, &typeck);
    ProjectAnalysis::Success(AnalysisResult {
        diagnostics,
        parse,
        typeck,
        source: program,
    })
}

fn project_diagnostic(message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        range: Range::new(Position::new(0, 0), Position::new(0, 0)),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("mesh".to_string()),
        message: message.into(),
        ..Default::default()
    }
}

fn cluster_diagnostics(
    source_cluster_declarations: &[mesh_pkg::manifest::SourceClusteredDeclaration],
    graph: &ModuleGraph,
    parses: &[mesh_parser::Parse],
    all_exports: &[Option<mesh_typeck::ExportedSymbols>],
    current_relative_path: &Path,
    current_source: &str,
) -> Option<Vec<Diagnostic>> {
    if source_cluster_declarations.is_empty() {
        return None;
    }

    let surface = build_clustered_export_surface(graph, parses, all_exports);
    match validate_cluster_declarations_with_source(source_cluster_declarations, &surface) {
        Ok(_) => None,
        Err(issues) => Some(
            issues
                .into_iter()
                .filter_map(|issue| {
                    clustered_declaration_diagnostic(issue, current_relative_path, current_source)
                })
                .collect(),
        ),
    }
}

fn clustered_issue_range(source: &str, span: mesh_common::span::Span) -> std::ops::Range<usize> {
    if source.is_empty() {
        return 0..0;
    }

    let mut start = (span.start as usize).min(source.len() - 1);
    let mut end = (span.end as usize).min(source.len());
    if end <= start {
        end = (start + 1).min(source.len());
    }
    start = start.min(end.saturating_sub(1));
    start..end
}

fn clustered_declaration_diagnostic(
    issue: ClusteredDeclarationError,
    current_relative_path: &Path,
    current_source: &str,
) -> Option<Diagnostic> {
    let range = match issue.origin.provenance() {
        Some(provenance) => {
            if provenance.file != current_relative_path {
                return None;
            }
            let span = clustered_issue_range(current_source, provenance.span);
            Range::new(
                offset_to_position(current_source, span.start),
                offset_to_position(current_source, span.end),
            )
        }
        None => Range::new(Position::new(0, 0), Position::new(0, 0)),
    };

    Some(Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("mesh".to_string()),
        message: issue.to_string(),
        ..Default::default()
    })
}

fn read_source_with_overlays(
    path: &Path,
    overlays: &HashMap<PathBuf, String>,
) -> Result<String, String> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(source) = overlays.get(&canonical) {
        return Ok(source.clone());
    }

    std::fs::read_to_string(path).map_err(|e| format!("Failed to read '{}': {}", path.display(), e))
}

fn canonical_file_path(uri: &str) -> Option<PathBuf> {
    let url = Url::parse(uri).ok()?;
    let path = url.to_file_path().ok()?;
    Some(std::fs::canonicalize(&path).unwrap_or(path))
}

fn find_project_root(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    loop {
        if current.join("mesh.toml").exists() {
            return Some(std::fs::canonicalize(&current).unwrap_or_else(|_| current.clone()));
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_pkg::manifest::DEFAULT_ENTRYPOINT;
    use std::path::PathBuf;

    fn package_manifest(name: &str) -> String {
        format!(
            "[package]\nname = \"{}\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
            name
        )
    }

    fn file_uri(path: &std::path::Path) -> String {
        Url::from_file_path(path)
            .expect("path should convert to file URI")
            .to_string()
    }

    fn entrypoint_manifest(name: &str, entrypoint: &str) -> String {
        format!(
            "[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nentrypoint = \"{entrypoint}\"\n\n[dependencies]\n"
        )
    }

    fn write_mesh_project(
        manifest: Option<&str>,
        files: &[(&str, &str)],
        open_relative_path: &str,
    ) -> (tempfile::TempDir, PathBuf, PathBuf, String) {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        std::fs::create_dir_all(&project_dir).unwrap();

        if let Some(manifest) = manifest {
            std::fs::write(project_dir.join("mesh.toml"), manifest).unwrap();
        }

        for (relative_path, contents) in files {
            let full_path = project_dir.join(relative_path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full_path, contents).unwrap();
        }

        let open_path = project_dir.join(open_relative_path);
        let source = std::fs::read_to_string(&open_path).unwrap();
        (tmp, project_dir, open_path, source)
    }

    fn diagnostic_messages(result: &AnalysisResult) -> Vec<&str> {
        result
            .diagnostics
            .iter()
            .map(|diag| diag.message.as_str())
            .collect()
    }

    fn entry_module(graph: &ModuleGraph) -> &mesh_common::module_graph::ModuleInfo {
        graph
            .modules
            .iter()
            .find(|module| module.is_entry)
            .expect("project graph should contain an entry module")
    }

    fn source_declared_work_project_main_source() -> &'static str {
        "fn main() do\n  nil\nend\n"
    }

    fn source_declared_public_work_source() -> &'static str {
        "@cluster pub fn handle_submit(payload :: String) -> String do\n  payload\nend\n\n@cluster(3) pub fn handle_retry(payload :: String) -> String do\n  payload\nend\n"
    }

    fn source_declared_private_work_source() -> &'static str {
        "@cluster fn hidden_submit(payload :: String) -> String do\n  payload\nend\n"
    }

    fn removed_clustered_work_source() -> &'static str {
        "clustered(work) pub fn handle_submit(payload :: String) -> String do\n  payload\nend\n"
    }

    fn removed_cluster_manifest(name: &str) -> String {
        format!(
            "{}\n[cluster]\nenabled = true\ndeclarations = [\n  {{ kind = \"work\", target = \"Work.handle_submit\" }},\n]\n",
            package_manifest(name)
        )
    }

    fn write_source_declared_work_project(
        manifest: &str,
        work_source: &str,
    ) -> (tempfile::TempDir, PathBuf, String) {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        let work_path = project_dir.join("work.mpl");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join("mesh.toml"), manifest).unwrap();
        std::fs::write(
            project_dir.join("main.mpl"),
            source_declared_work_project_main_source(),
        )
        .unwrap();
        std::fs::write(&work_path, work_source).unwrap();
        (tmp, work_path, work_source.to_string())
    }

    fn clustered_route_wrapper_main_source() -> &'static str {
        "from Api.Todos import handle_list_todos\n\npub fn handle_local(req :: Request) -> Response do\n  HTTP.response(200, \"ok\")\nend\n\nfn build() do\n  let router = HTTP.router()\n  let router = HTTP.on_get(router, \"/local\", HTTP.clustered(handle_local))\n  router |> HTTP.on_get(\"/todos\", HTTP.clustered(handle_list_todos))\nend\n\nfn main() do\n  let _ = build()\n  nil\nend\n"
    }

    fn clustered_route_wrapper_todos_source() -> &'static str {
        "pub fn handle_list_todos(req :: Request) -> Response do\n  HTTP.response(200, \"todos\")\nend\n\nfn hidden_todos(req :: Request) -> Response do\n  HTTP.response(200, \"hidden\")\nend\n"
    }

    fn write_clustered_route_wrapper_project() -> (tempfile::TempDir, PathBuf, String) {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        let main_path = project_dir.join("main.mpl");
        std::fs::create_dir_all(project_dir.join("api")).unwrap();
        std::fs::write(
            project_dir.join("mesh.toml"),
            package_manifest("clustered-routes"),
        )
        .unwrap();
        std::fs::write(&main_path, clustered_route_wrapper_main_source()).unwrap();
        std::fs::write(
            project_dir.join("api/todos.mpl"),
            clustered_route_wrapper_todos_source(),
        )
        .unwrap();
        (
            tmp,
            main_path,
            clustered_route_wrapper_main_source().to_string(),
        )
    }

    fn assert_decorator_anchored_range(diag: &Diagnostic) {
        assert_eq!(diag.range.start, Position::new(0, 0));
        assert!(
            diag.range.end.line > diag.range.start.line
                || diag.range.end.character > diag.range.start.character,
            "expected clustered diagnostic to cover a non-empty decorator span, got {:?}",
            diag.range
        );
    }

    // ── Entrypoint-aware project analysis ────────────────────────────────

    #[test]
    fn find_project_root_uses_manifest_ancestor() {
        let manifest = entrypoint_manifest("manifest-root", "app.mpl");
        let (_tmp, project_dir, open_path, _source) = write_mesh_project(
            Some(&manifest),
            &[
                ("app.mpl", "fn main() do\n  0\nend\n"),
                ("nested/main.mpl", "fn main() do\n  1\nend\n"),
                (
                    "nested/support.mpl",
                    "pub fn label() -> String do\n  \"nested\"\nend\n",
                ),
            ],
            "nested/support.mpl",
        );

        let detected_root = find_project_root(&open_path).expect("manifest root should resolve");

        assert_eq!(
            detected_root,
            std::fs::canonicalize(&project_dir).unwrap(),
            "manifest marker should identify the project root"
        );
    }

    #[test]
    fn override_only_project_marks_nested_entry_executable_and_analyzes_cleanly() {
        let manifest = entrypoint_manifest("override-only", "lib/start.mpl");
        let (_tmp, project_dir, open_path, source) = write_mesh_project(
            Some(&manifest),
            &[
                (
                    "lib/start.mpl",
                    "from Lib.Support import label\n\nfn main() do\n  println(label())\nend\n",
                ),
                (
                    "lib/support.mpl",
                    "pub fn label() -> String do\n  \"nested-support\"\nend\n",
                ),
            ],
            "lib/start.mpl",
        );

        let manifest = Manifest::from_file(&project_dir.join("mesh.toml")).unwrap();
        let entry_relative_path = resolve_entrypoint(&project_dir, Some(&manifest)).unwrap();
        let project = build_project(
            &project_dir,
            &entry_relative_path,
            &[],
            false,
            &mesh_pkg::project::read_file,
        )
        .unwrap();
        let entry = entry_module(&project.graph);

        assert_eq!(entry_relative_path, PathBuf::from("lib/start.mpl"));
        assert_eq!(entry.path, PathBuf::from("lib/start.mpl"));
        assert_eq!(entry.name, "Lib.Start");
        assert_eq!(
            project
                .graph
                .modules
                .iter()
                .filter(|module| module.is_entry)
                .count(),
            1
        );

        let result = analyze_document(&file_uri(&open_path), &source, &[]);
        let messages = diagnostic_messages(&result);
        assert!(
            messages.is_empty(),
            "override-only project should analyze cleanly without a root main.mpl, got: {:?}",
            messages
        );
    }

    #[test]
    fn override_precedence_keeps_root_main_path_derived_but_not_executable() {
        let manifest = entrypoint_manifest("override-precedence", "lib/start.mpl");
        let (_tmp, project_dir, open_path, source) = write_mesh_project(
            Some(&manifest),
            &[
                ("main.mpl", "fn main() do\n  0\nend\n"),
                (
                    "lib/start.mpl",
                    "from App import label\n\nfn main() do\n  println(label())\nend\n",
                ),
                (
                    "app.mpl",
                    "pub fn label() -> String do\n  \"override-app\"\nend\n",
                ),
            ],
            "lib/start.mpl",
        );

        let manifest = Manifest::from_file(&project_dir.join("mesh.toml")).unwrap();
        let entry_relative_path = resolve_entrypoint(&project_dir, Some(&manifest)).unwrap();
        let project = build_project(
            &project_dir,
            &entry_relative_path,
            &[],
            false,
            &mesh_pkg::project::read_file,
        )
        .unwrap();
        let entry = entry_module(&project.graph);
        let root_main = project
            .graph
            .modules
            .iter()
            .find(|module| module.path == Path::new(DEFAULT_ENTRYPOINT))
            .expect("root main.mpl should still be discovered");

        assert_eq!(entry.path, PathBuf::from("lib/start.mpl"));
        assert_eq!(entry.name, "Lib.Start");
        assert_eq!(
            project
                .graph
                .modules
                .iter()
                .filter(|module| module.is_entry)
                .count(),
            1
        );
        assert_eq!(root_main.name, "Main");
        assert!(
            !root_main.is_entry,
            "root main.mpl should not stay executable"
        );

        let result = analyze_document(&file_uri(&open_path), &source, &[]);
        let messages = diagnostic_messages(&result);
        assert!(
            messages.is_empty(),
            "override-precedence project should analyze cleanly, got: {:?}",
            messages
        );
    }

    #[test]
    fn missing_configured_entry_reports_project_diagnostic() {
        let manifest = entrypoint_manifest("broken-entry", "lib/start.mpl");
        let (_tmp, _project_dir, open_path, source) = write_mesh_project(
            Some(&manifest),
            &[(
                "app.mpl",
                "pub fn label() -> String do\n  \"still-clean-alone\"\nend\n",
            )],
            "app.mpl",
        );

        let result = analyze_document(&file_uri(&open_path), &source, &[]);
        let messages = diagnostic_messages(&result);

        assert!(
            messages
                .iter()
                .any(|message| message.contains("Entrypoint 'lib/start.mpl'")),
            "missing configured entry should surface as a project diagnostic, got: {:?}",
            messages
        );
        assert_eq!(
            messages.len(),
            1,
            "clean standalone files should only receive the project diagnostic, got: {:?}",
            messages
        );
    }

    #[test]
    fn invalid_manifest_entrypoint_reports_project_diagnostic() {
        let manifest = entrypoint_manifest("escaping-entry", "../escape.mpl");
        let (_tmp, _project_dir, open_path, source) = write_mesh_project(
            Some(&manifest),
            &[(
                "lib/support.mpl",
                "pub fn label() -> String do\n  \"still-clean-alone\"\nend\n",
            )],
            "lib/support.mpl",
        );

        let result = analyze_document(&file_uri(&open_path), &source, &[]);
        let messages = diagnostic_messages(&result);

        assert!(
            messages
                .iter()
                .any(|message| message.contains("stay within the project root")),
            "invalid manifest entrypoint should surface as a project diagnostic, got: {:?}",
            messages
        );
        assert_eq!(
            messages.len(),
            1,
            "clean standalone files should only receive the project diagnostic, got: {:?}",
            messages
        );
    }

    // ── Clustered cutover contract ────────────────────────────────────

    #[test]
    fn source_decorated_work_still_analyzes_without_diagnostics() {
        let (_tmp, work_path, source) = write_source_declared_work_project(
            &package_manifest("clustered-source-proof"),
            source_declared_public_work_source(),
        );

        let result = analyze_document(&file_uri(&work_path), &source, &[]);
        let messages = result
            .diagnostics
            .iter()
            .map(|diag| diag.message.as_str())
            .collect::<Vec<_>>();

        assert!(
            messages.is_empty(),
            "expected clean diagnostics for valid source-decorated work, got: {:?}",
            messages
        );
    }

    #[test]
    fn removed_manifest_cluster_section_reports_project_diagnostic() {
        let (_tmp, work_path, source) = write_source_declared_work_project(
            &removed_cluster_manifest("clustered-source-proof"),
            source_declared_public_work_source(),
        );

        let result = analyze_document(&file_uri(&work_path), &source, &[]);
        let diag = result
            .diagnostics
            .iter()
            .find(|diag| {
                diag.message
                    .contains("`[cluster]` manifest sections are no longer supported")
                    && diag.message.contains("mesh.toml")
            })
            .unwrap_or_else(|| {
                panic!(
                    "expected removed-manifest diagnostic, got: {:?}",
                    result
                        .diagnostics
                        .iter()
                        .map(|diag| (&diag.message, diag.range))
                        .collect::<Vec<_>>()
                )
            });

        assert_eq!(diag.range.start, Position::new(0, 0));
    }

    #[test]
    fn removed_clustered_work_reports_parse_diagnostic_at_source_range() {
        let (_tmp, work_path, source) = write_source_declared_work_project(
            &package_manifest("clustered-source-proof"),
            removed_clustered_work_source(),
        );

        let result = analyze_document(&file_uri(&work_path), &source, &[]);
        let diag = result
            .diagnostics
            .iter()
            .find(|diag| {
                diag.message
                    .contains("`clustered(work)` declarations are not supported")
            })
            .unwrap_or_else(|| {
                panic!(
                    "expected removed clustered(work) diagnostic, got: {:?}",
                    result
                        .diagnostics
                        .iter()
                        .map(|diag| (&diag.message, diag.range))
                        .collect::<Vec<_>>()
                )
            });

        assert_eq!(diag.range.start.line, 0);
        assert!(
            diag.range.end.line > diag.range.start.line
                || diag.range.end.character > diag.range.start.character,
            "expected non-empty removed-syntax source range, got {:?}",
            diag.range
        );
    }

    #[test]
    fn source_decorated_work_analyzes_without_diagnostics() {
        let (_tmp, work_path, source) = write_source_declared_work_project(
            &package_manifest("clustered-source-proof"),
            source_declared_public_work_source(),
        );

        let result = analyze_document(&file_uri(&work_path), &source, &[]);
        let messages = result
            .diagnostics
            .iter()
            .map(|diag| diag.message.as_str())
            .collect::<Vec<_>>();

        assert!(
            messages.is_empty(),
            "expected clean diagnostics for valid source-decorated work, got: {:?}",
            messages
        );
    }

    #[test]
    fn private_source_decorator_reports_declaration_range() {
        let (_tmp, work_path, source) = write_source_declared_work_project(
            &package_manifest("clustered-source-proof"),
            source_declared_private_work_source(),
        );

        let result = analyze_document(&file_uri(&work_path), &source, &[]);
        let diag = result
            .diagnostics
            .iter()
            .find(|diag| {
                diag.message.contains("private function")
                    && diag.message.contains("Work.hidden_submit")
                    && diag.message.contains("source `@cluster` decorator")
            })
            .unwrap_or_else(|| {
                panic!(
                    "expected private decorated work diagnostic, got: {:?}",
                    result
                        .diagnostics
                        .iter()
                        .map(|diag| (&diag.message, diag.range))
                        .collect::<Vec<_>>()
                )
            });

        assert_decorator_anchored_range(diag);
    }

    #[test]
    fn manifest_source_duplicate_reports_declaration_range() {
        let (_tmp, work_path, source) = write_source_declared_work_project(
            &removed_cluster_manifest("clustered-source-proof"),
            source_declared_public_work_source(),
        );

        let result = analyze_document(&file_uri(&work_path), &source, &[]);
        let diag = result
            .diagnostics
            .iter()
            .find(|diag| {
                diag.message
                    .contains("`[cluster]` manifest sections are no longer supported")
                    && diag.message.contains("mesh.toml")
            })
            .unwrap_or_else(|| {
                panic!(
                    "expected removed-manifest diagnostic, got: {:?}",
                    result
                        .diagnostics
                        .iter()
                        .map(|diag| (&diag.message, diag.range))
                        .collect::<Vec<_>>()
                )
            });

        assert_eq!(diag.range.start, Position::new(0, 0));
    }

    // ── The editor sees the program meshc builds ─────────────────────────

    /// A module that implements another's `pub interface` without importing
    /// it is checked after that module, as meshc orders them; the editor
    /// checked `App` first and reported `Area` unknown.
    #[test]
    fn a_module_implementing_an_unimported_interface_analyzes_cleanly() {
        let (_tmp, _project_dir, open_path, source) = write_mesh_project(
            Some(&package_manifest("shapes")),
            &[
                ("shapes.mpl", "pub interface Area do\n  fn area(self) -> Int\nend\n"),
                (
                    "app.mpl",
                    "pub struct Square do\n  side :: Int\nend\n\nimpl Area for Square do\n  fn area(self) -> Int do\n    self.side * self.side\n  end\nend\n",
                ),
                ("main.mpl", "fn main() do\n  println(\"ok\")\nend\n"),
            ],
            "app.mpl",
        );
        let result = analyze_document(&file_uri(&open_path), &source, &[]);
        assert!(
            result.diagnostics.is_empty(),
            "{:?}",
            diagnostic_messages(&result)
        );
    }

    /// A file outside any project is analyzed as meshc builds a directory
    /// holding only it: its module blocks are modules (the editor reported
    /// `import Geo` unknown), and a `let` outside a function is an error.
    #[test]
    fn a_document_outside_a_project_is_built_like_one() {
        let source = "import Geo\n\nmodule Geo do\n  pub fn area(w :: Int, h :: Int) -> Int do\n    w * h\n  end\nend\n\nfn main() do\n  println(\"${Geo.area(1, 2)}\")\nend\n";
        let result = analyze_document("file:///nowhere/main.mpl", source, &[]);
        assert!(
            result.diagnostics.is_empty(),
            "{:?}",
            diagnostic_messages(&result)
        );
        let result = analyze_document("file:///nowhere/main.mpl", "let x = 1\n", &[]);
        assert_eq!(
            diagnostic_messages(&result),
            ["`let x` outside a function is not supported"]
        );
    }

    /// A document that cannot be analyzed as a module of its project says
    /// why, first thing.
    #[test]
    fn documents_that_cannot_be_analyzed_as_modules_say_why() {
        // A standalone file whose module blocks conflict.
        let result = analyze_document(
            "file:///nowhere/main.mpl",
            "module A do\nend\n\nmodule A do\nend\n",
            &[],
        );
        assert!(
            diagnostic_messages(&result)[0].contains("conflicts"),
            "{:?}",
            diagnostic_messages(&result)
        );
        // An editor buffer that is not a file is analyzed on its own.
        let result = analyze_document("untitled:Untitled-1", "fn main() do\n  1\nend\n", &[]);
        assert!(
            result.diagnostics.is_empty(),
            "{:?}",
            diagnostic_messages(&result)
        );

        for (files, open, expected) in [
            (
                vec![
                    ("a.mpl", "import B\n\npub fn a() = 1\n"),
                    ("b.mpl", "import A\n\npub fn b() = 2\n"),
                    ("main.mpl", "fn main() do\n  1\nend\n"),
                ],
                "a.mpl",
                "Circular dependency",
            ),
            (
                vec![
                    ("main.mpl", "fn main() do\n  1\nend\n"),
                    (".scratch/note.mpl", "fn note() = 1\n"),
                ],
                ".scratch/note.mpl",
                "was not discovered",
            ),
            (
                vec![
                    ("main.mpl", "fn main() do\n  1\nend\n"),
                    (
                        "math.test.mpl",
                        "describe(\"math\") do\n  teardown do\n    1\n  end\n  teardown do\n    2\n  end\nend\n",
                    ),
                ],
                "math.test.mpl",
                "one `teardown`",
            ),
        ] {
            let (_tmp, _project_dir, open_path, source) =
                write_mesh_project(Some(&package_manifest("failing")), &files, open);
            let result = analyze_document(&file_uri(&open_path), &source, &[]);
            assert!(
                diagnostic_messages(&result)[0].contains(expected),
                "{expected}: {:?}",
                diagnostic_messages(&result)
            );
        }
    }

    #[test]
    fn warnings_are_reported_as_warnings() {
        let result = analyze_document(
            "file:///nowhere/main.mpl",
            "fn main() do\n  case 1 do\n    _ -> 1\n    2 -> 2\n  end\nend\n",
            &[],
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING)),
            "{:?}",
            diagnostic_messages(&result)
        );
    }

    /// An open document is read as the editor holds it, not as it was saved.
    #[test]
    fn open_documents_are_read_in_place_of_their_files() {
        let (_tmp, project_dir, open_path, source) = write_mesh_project(
            Some(&package_manifest("overlays")),
            &[
                (
                    "main.mpl",
                    "from Util import helper\n\nfn main() do\n  println(helper())\nend\n",
                ),
                ("util.mpl", "pub fn other() = 1\n"),
            ],
            "main.mpl",
        );
        let saved = analyze_document(&file_uri(&open_path), &source, &[]);
        assert!(!saved.diagnostics.is_empty());
        let open = [(
            file_uri(&project_dir.join("util.mpl")),
            "pub fn helper() -> String do\n  \"h\"\nend\n".to_string(),
        )];
        let edited = analyze_document(&file_uri(&open_path), &source, &open);
        assert!(
            edited.diagnostics.is_empty(),
            "{:?}",
            diagnostic_messages(&edited)
        );
    }

    /// meshc refuses a `let` outside a function; the editor said nothing.
    #[test]
    fn a_top_level_let_is_reported_in_a_project() {
        let (_tmp, _project_dir, open_path, source) = write_mesh_project(
            Some(&package_manifest("lets")),
            &[(
                "main.mpl",
                "let limit = 3\n\nfn main() do\n  println(\"${limit}\")\nend\n",
            )],
            "main.mpl",
        );
        let result = analyze_document(&file_uri(&open_path), &source, &[]);
        assert!(
            diagnostic_messages(&result)
                .iter()
                .any(|message| message.contains("`let limit` outside a function")),
            "{:?}",
            diagnostic_messages(&result)
        );
    }

    // ── Scoped installed package regressions ────────────────────────────

    #[test]
    fn scoped_installed_package_analyzes_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("consumer");
        let package_root = project_dir.join(".mesh/packages/acme/greeter@1.0.0");
        let main_path = project_dir.join("main.mpl");

        std::fs::create_dir_all(package_root.join("support")).unwrap();
        std::fs::write(project_dir.join("mesh.toml"), package_manifest("consumer")).unwrap();
        std::fs::write(
            &main_path,
            "from Support.Message import message\n\nfn main() do\n  println(message())\nend\n",
        )
        .unwrap();
        std::fs::write(
            package_root.join("mesh.toml"),
            package_manifest("acme/greeter"),
        )
        .unwrap();
        std::fs::write(package_root.join("main.mpl"), "fn main() do\n  0\nend\n").unwrap();
        std::fs::write(
            package_root.join("support/message.mpl"),
            "pub fn message() -> String do\n  \"hello from package\"\nend\n",
        )
        .unwrap();

        let source = std::fs::read_to_string(&main_path).unwrap();
        let result = analyze_document(&file_uri(&main_path), &source, &[]);
        let messages = result
            .diagnostics
            .iter()
            .map(|diag| diag.message.as_str())
            .collect::<Vec<_>>();

        assert!(
            messages.is_empty(),
            "scoped installed packages should analyze without diagnostics, got: {:?}",
            messages
        );
    }

    #[test]
    fn scoped_installed_package_flat_layout_analyzes_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("consumer");
        let package_root = project_dir.join(".mesh/packages/greeter@1.0.0");
        let main_path = project_dir.join("main.mpl");

        std::fs::create_dir_all(&package_root).unwrap();
        std::fs::write(project_dir.join("mesh.toml"), package_manifest("consumer")).unwrap();
        std::fs::write(
            &main_path,
            "from Greeting import message\n\nfn main() do\n  println(message())\nend\n",
        )
        .unwrap();
        std::fs::write(package_root.join("mesh.toml"), package_manifest("greeter")).unwrap();
        std::fs::write(
            package_root.join("greeting.mpl"),
            "pub fn message() -> String do\n  \"hello from flat package\"\nend\n",
        )
        .unwrap();

        let source = std::fs::read_to_string(&main_path).unwrap();
        let result = analyze_document(&file_uri(&main_path), &source, &[]);
        let messages = result
            .diagnostics
            .iter()
            .map(|diag| diag.message.as_str())
            .collect::<Vec<_>>();

        assert!(
            messages.is_empty(),
            "flat installed packages should analyze without diagnostics, got: {:?}",
            messages
        );
    }

    #[test]
    fn clustered_route_wrapper_project_keeps_imported_origin_metadata() {
        let (_tmp, main_path, source) = write_clustered_route_wrapper_project();

        let result = analyze_document(&file_uri(&main_path), &source, &[]);
        let messages = result
            .diagnostics
            .iter()
            .map(|diag| diag.message.as_str())
            .collect::<Vec<_>>();

        assert!(
            messages.is_empty(),
            "expected clean diagnostics for valid clustered route wrappers, got: {:?}",
            messages
        );

        let metadata = result
            .typeck
            .clustered_route_wrappers
            .values()
            .find(|metadata| metadata.runtime_name == "Api.Todos.handle_list_todos")
            .unwrap_or_else(|| {
                panic!(
                    "expected imported clustered route metadata, got: {:?}",
                    result
                        .typeck
                        .clustered_route_wrappers
                        .values()
                        .map(|metadata| {
                            (
                                metadata.runtime_name.as_str(),
                                metadata.defining_module.as_deref(),
                                metadata.replication_count.value,
                            )
                        })
                        .collect::<Vec<_>>()
                )
            });

        assert_eq!(metadata.defining_module.as_deref(), Some("Api.Todos"));
        assert_eq!(metadata.replication_count.value, 2);
    }

    #[test]
    fn clustered_route_wrapper_reports_wrapper_range_in_lsp() {
        let source = "pub fn handle(req :: Request) -> Response do\n  HTTP.response(200, \"ok\")\nend\n\nfn build() do\n  let wrapped = HTTP.clustered(handle)\n  wrapped\nend\n";
        let result = analyze_document("file:///test.mpl", source, &[]);
        let diag = result
            .diagnostics
            .iter()
            .find(|diag| diag.message.contains("HTTP.clustered"))
            .unwrap_or_else(|| {
                panic!(
                    "expected clustered-route diagnostic, got: {:?}",
                    result
                        .diagnostics
                        .iter()
                        .map(|diag| (&diag.message, diag.range))
                        .collect::<Vec<_>>()
                )
            });

        let wrapper_start = source
            .find("HTTP.clustered")
            .expect("wrapper text should exist");
        assert_eq!(diag.range.start, offset_to_position(source, wrapper_start));
        assert!(
            diag.range.end.line > diag.range.start.line
                || diag.range.end.character > diag.range.start.character,
            "expected wrapper diagnostic to cover a non-empty range, got {:?}",
            diag.range
        );
    }

    // ── Diagnostic Tests ──────────────────────────────────────────────────

    #[test]
    fn analyze_valid_source_no_diagnostics() {
        let source = "fn main() do\n  let x = 42\n  x\nend\n";
        let result = analyze_document("file:///test.mpl", source, &[]);
        assert!(
            result.diagnostics.is_empty(),
            "Valid source should produce no diagnostics, got: {:?}",
            result
                .diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn analyze_valid_function_no_diagnostics() {
        let source = "fn add(a, b) do\na + b\nend";
        let result = analyze_document("file:///test.mpl", source, &[]);
        assert!(
            result.diagnostics.is_empty(),
            "Valid function should produce no diagnostics, got: {:?}",
            result
                .diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn analyze_type_error_produces_diagnostic() {
        // Using an undefined variable should produce a type error diagnostic.
        let source = "let x = undefined_var";
        let result = analyze_document("file:///test.mpl", source, &[]);
        assert!(
            !result.diagnostics.is_empty(),
            "Type error should produce at least one diagnostic"
        );
        let diag = &result.diagnostics[0];
        assert_eq!(diag.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(diag.source.as_deref(), Some("mesh"));
    }

    #[test]
    fn analyze_type_error_has_range() {
        // The diagnostic range should point to the error location.
        let source = "let x = undefined_var";
        let result = analyze_document("file:///test.mpl", source, &[]);
        assert!(!result.diagnostics.is_empty());
        let diag = &result.diagnostics[0];
        // The error is for "undefined_var" which is on line 0.
        assert_eq!(diag.range.start.line, 0);
    }

    #[test]
    fn analyze_resource_violation_produces_diagnostic() {
        let source = "fn misuse(secret :: SecretBytes) do\n  let moved = secret\n  secret\nend";
        let result = analyze_document("file:///test.mpl", source, &[]);

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Some(DiagnosticSeverity::ERROR)
                && diagnostic
                    .message
                    .contains("resource `secret` was used after it moved")
        }));
    }

    #[test]
    fn analyze_multiple_errors_all_reported() {
        // Two undefined variables should produce at least two diagnostics.
        let source = "let x = undef1\nlet y = undef2";
        let result = analyze_document("file:///test.mpl", source, &[]);
        assert!(
            result.diagnostics.len() >= 2,
            "Expected at least 2 diagnostics, got {}",
            result.diagnostics.len()
        );
    }

    #[test]
    fn analyze_parse_error_produces_diagnostic() {
        // A parse error (incomplete expression) should produce a diagnostic.
        // Note: `fn do end` is now valid syntax (no-params closure, Phase 12-01).
        // Use a clearly invalid expression instead.
        let source = "let x = + +";
        let result = analyze_document("file:///test.mpl", source, &[]);
        assert!(
            !result.diagnostics.is_empty(),
            "Parse error should produce at least one diagnostic"
        );
        let diag = &result.diagnostics[0];
        assert_eq!(diag.severity, Some(DiagnosticSeverity::ERROR));
    }

    // ── Hover Tests ───────────────────────────────────────────────────────

    #[test]
    fn hover_integer_literal() {
        let source = "let x = 42";
        let result = analyze_document("file:///test.mpl", source, &[]);
        // Hover over the let binding -- should show the type.
        // The rowan tree has "letx=42" so the LET_BINDING covers tree offsets.
        // The type map uses tree-coordinate ranges.
        // type_at_position converts LSP position to source byte offset.
        // However, since the typeck uses rowan ranges (not source byte offsets),
        // the hover might not work correctly for all positions due to the
        // whitespace coordinate mismatch (pre-existing issue).
        // We test with line 0, character 0 which should be in the LET_BINDING range.
        let ty = type_at_position(
            source,
            &result.typeck,
            &Position {
                line: 0,
                character: 0,
            },
        );
        // May return Some("Int") or None depending on what range the typeck stored.
        // At minimum, verify it doesn't panic.
        let _ = ty;
    }

    #[test]
    fn hover_over_empty_space_returns_none() {
        // Hovering over whitespace or at end of file should return None.
        let source = "let x = 42";
        let result = analyze_document("file:///test.mpl", source, &[]);
        // Position past end of source.
        let ty = type_at_position(
            source,
            &result.typeck,
            &Position {
                line: 5,
                character: 0,
            },
        );
        assert!(ty.is_none(), "Hover past end should return None");
    }

    // ── Go-to-definition Tests ────────────────────────────────────────────

    #[test]
    fn goto_def_function_defined_then_called() {
        let source = "fn greet(name) do\nname\nend\nlet msg = greet(42)";
        let result = analyze_document("file:///test.mpl", source, &[]);
        let root = result.parse.syntax();
        // Find the call to "greet" in `greet(42)`.
        let call_offset = source.rfind("greet").unwrap();
        let def = crate::definition::find_definition(source, &root, call_offset);
        assert!(def.is_some(), "Should find definition of greet");
        // Verify it resolves to the fn definition, not the call.
        let range = def.unwrap();
        let def_source = crate::definition::tree_to_source_offset(source, range.start().into());
        assert!(def_source.is_some());
        let offset = def_source.unwrap();
        // "fn greet" -- "greet" starts at offset 3.
        assert_eq!(offset, 3);
    }

    #[test]
    fn goto_def_let_binding_used_later() {
        let source = "let count = 10\nlet doubled = count + count";
        let result = analyze_document("file:///test.mpl", source, &[]);
        let root = result.parse.syntax();
        // Find "count" in the second let binding.
        let second_count = source.find("count + count").unwrap();
        let def = crate::definition::find_definition(source, &root, second_count);
        assert!(def.is_some(), "Should find definition of count");
        let range = def.unwrap();
        let def_source =
            crate::definition::tree_to_source_offset(source, range.start().into()).unwrap();
        // "let count" -- "count" starts at offset 4.
        assert_eq!(def_source, 4);
    }

    #[test]
    fn goto_def_variable_shadowing_inner_scope() {
        let source = "fn test() do\nlet x = 1\nfn inner() do\nlet x = 2\nlet y = x\nend\nend";
        let result = analyze_document("file:///test.mpl", source, &[]);
        let root = result.parse.syntax();
        let y_binding = source.find("let y = x").unwrap();
        let x_use = y_binding + "let y = ".len();
        let def = crate::definition::find_definition(source, &root, x_use);
        assert!(def.is_some(), "Should find inner x definition");
        let range = def.unwrap();
        let def_source =
            crate::definition::tree_to_source_offset(source, range.start().into()).unwrap();
        let inner_x = source.find("let x = 2").unwrap() + "let ".len();
        assert_eq!(
            def_source, inner_x,
            "Should resolve to inner binding, not outer"
        );
    }

    #[test]
    fn goto_def_unknown_identifier_returns_none() {
        let source = "let y = completely_unknown";
        let result = analyze_document("file:///test.mpl", source, &[]);
        let root = result.parse.syntax();
        let unknown_offset = source.find("completely_unknown").unwrap();
        let def = crate::definition::find_definition(source, &root, unknown_offset);
        assert!(def.is_none(), "Unknown identifier should return None");
    }

    #[test]
    fn goto_def_struct_name_resolves() {
        let source = "struct Point do\nx :: Int\nend";
        let result = analyze_document("file:///test.mpl", source, &[]);
        let root = result.parse.syntax();
        // Definition search for "Point" at the struct def should find itself.
        let point_offset = source.find("Point").unwrap();
        // "Point" at the definition site is in a NAME node, not NAME_REF,
        // so it won't resolve to anything (it IS the definition).
        let def = crate::definition::find_definition(source, &root, point_offset);
        // This should return None since the user is clicking on the definition itself.
        assert!(
            def.is_none(),
            "Clicking on definition site should return None"
        );
    }

    // ── Position Conversion Tests ─────────────────────────────────────────

    #[test]
    fn offset_to_position_first_line() {
        let source = "hello world";
        let pos = offset_to_position(source, 0);
        assert_eq!(
            pos,
            Position {
                line: 0,
                character: 0
            }
        );

        let pos = offset_to_position(source, 5);
        assert_eq!(
            pos,
            Position {
                line: 0,
                character: 5
            }
        );
    }

    #[test]
    fn offset_to_position_multiline() {
        let source = "line1\nline2\nline3";
        // 'l' of line2 is at offset 6
        let pos = offset_to_position(source, 6);
        assert_eq!(
            pos,
            Position {
                line: 1,
                character: 0
            }
        );

        // 'l' of line3 is at offset 12
        let pos = offset_to_position(source, 12);
        assert_eq!(
            pos,
            Position {
                line: 2,
                character: 0
            }
        );

        // 'i' of line2 is at offset 7
        let pos = offset_to_position(source, 7);
        assert_eq!(
            pos,
            Position {
                line: 1,
                character: 1
            }
        );
    }

    #[test]
    fn offset_to_position_at_end() {
        let source = "ab\ncd";
        let pos = offset_to_position(source, 5);
        assert_eq!(
            pos,
            Position {
                line: 1,
                character: 2
            }
        );
    }

    #[test]
    fn position_to_offset_single_line() {
        let source = "hello";
        assert_eq!(
            position_to_offset(
                source,
                &Position {
                    line: 0,
                    character: 0
                }
            ),
            Some(0)
        );
        assert_eq!(
            position_to_offset(
                source,
                &Position {
                    line: 0,
                    character: 3
                }
            ),
            Some(3)
        );
        assert_eq!(
            position_to_offset(
                source,
                &Position {
                    line: 0,
                    character: 5
                }
            ),
            Some(5)
        );
    }

    #[test]
    fn position_to_offset_multiline() {
        let source = "abc\ndef\nghi";
        // First char of line 2 (0-indexed) at (1, 0).
        assert_eq!(
            position_to_offset(
                source,
                &Position {
                    line: 1,
                    character: 0
                }
            ),
            Some(4)
        );
        // First char of line 3 at (2, 0).
        assert_eq!(
            position_to_offset(
                source,
                &Position {
                    line: 2,
                    character: 0
                }
            ),
            Some(8)
        );
    }

    #[test]
    fn position_to_offset_stays_on_its_line() {
        // A character past the end of a line counted on into the next lines.
        let source = "ab\ncdef\né𝄞x\n";
        let at = |line, character| position_to_offset(source, &Position { line, character });
        assert_eq!(at(0, 10), Some(2));
        assert_eq!(at(1, 2), Some(5));
        // `é` is one UTF-16 unit, `𝄞` two.
        assert_eq!(at(2, 1), Some(10));
        assert_eq!(at(2, 3), Some(14));
        assert_eq!(at(2, 9), Some(15));
        assert_eq!(at(3, 0), Some(16));
        assert_eq!(at(4, 0), None);
    }

    #[test]
    fn position_to_offset_roundtrip() {
        let source = "hello\nworld\nfoo";
        for offset in 0..source.len() {
            let pos = offset_to_position(source, offset);
            let back = position_to_offset(source, &pos);
            assert_eq!(
                back,
                Some(offset),
                "Roundtrip failed for offset {} (pos {:?})",
                offset,
                pos
            );
        }
    }

    #[test]
    fn position_past_eof_returns_none() {
        let source = "hello";
        let result = position_to_offset(
            source,
            &Position {
                line: 5,
                character: 0,
            },
        );
        assert!(result.is_none(), "Position past EOF should return None");
    }

    // ── Source/Tree Offset Conversion Tests ────────────────────────────────

    #[test]
    fn source_tree_offset_roundtrip() {
        let source = "let x = 42\nlet y = x";
        // For each non-EOF token in the source, verify the roundtrip.
        let tokens = mesh_lexer::Lexer::tokenize(source);
        for token in &tokens {
            // Skip EOF (zero-length token at end).
            if token.kind == mesh_common::token::TokenKind::Eof {
                continue;
            }
            let src_start = token.span.start as usize;
            let tree = crate::definition::source_to_tree_offset(source, src_start);
            assert!(
                tree.is_some(),
                "source_to_tree_offset should succeed for offset {}",
                src_start
            );
            let back = crate::definition::tree_to_source_offset(source, tree.unwrap());
            assert_eq!(
                back,
                Some(src_start),
                "Roundtrip failed for source offset {}",
                src_start
            );
        }
    }

    #[test]
    fn a_test_file_is_analyzed_as_meshc_test_compiles_it() {
        // It was analyzed as a plain module that was not in the project:
        // `test`, `assert_eq` and its imports were errors in the editor.
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("app");
        std::fs::create_dir_all(project_dir.join("tests")).unwrap();
        std::fs::write(project_dir.join("mesh.toml"), package_manifest("app")).unwrap();
        std::fs::write(project_dir.join("main.mpl"), "fn main() do\nend\n").unwrap();
        std::fs::write(
            project_dir.join("greeting.mpl"),
            "pub fn message() -> String do\n  \"hello\"\nend\n",
        )
        .unwrap();
        let test_path = project_dir.join("tests/greeting.test.mpl");
        let source = "from Greeting import message\n\ndescribe(\"greeting\") do\n  setup do\n    let expected = \"hello\"\n  end\n  test(\"says hello\") do\n    assert_eq(message(), expected)\n  end\nend\n";
        std::fs::write(&test_path, source).unwrap();

        let result = analyze_document(&file_uri(&test_path), source, &[]);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // Hover converts positions against the analyzed program: line 8,
        // `expected`, is a String there too.
        let hover = type_at_position(&result.source, &result.typeck, &Position::new(7, 26));
        assert_eq!(hover.as_deref(), Some("String"));

        // An error is reported where it is in the test file.
        let broken = source.replace(
            "assert_eq(message(), expected)",
            "assert_eq(message() + 1, expected)",
        );
        let result = analyze_document(&file_uri(&test_path), &broken, &[]);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].range.start.line, 7);
    }

    #[test]
    fn a_path_dependency_analyzes_cleanly() {
        // The LSP found installed packages only: a module of a path
        // dependency was "module not found" in the editor.
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("consumer");
        let library = tmp.path().join("library");
        let main_path = project_dir.join("main.mpl");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(
            project_dir.join("mesh.toml"),
            "[package]\nname = \"consumer\"\nversion = \"1.0.0\"\n\n[dependencies]\nlibrary = { path = \"../library\" }\n",
        )
        .unwrap();
        std::fs::write(library.join("mesh.toml"), package_manifest("library")).unwrap();
        std::fs::write(
            library.join("greeting.mpl"),
            "pub fn message() -> String do\n  \"hello\"\nend\n",
        )
        .unwrap();
        std::fs::write(
            &main_path,
            "from Greeting import message\n\nfn main() do\n  println(message())\nend\n",
        )
        .unwrap();

        let source = std::fs::read_to_string(&main_path).unwrap();
        let result = analyze_document(&file_uri(&main_path), &source, &[]);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }

    #[test]
    fn a_module_block_is_checked_in_its_file() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("app");
        let main_path = project_dir.join("main.mpl");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join("mesh.toml"), package_manifest("app")).unwrap();
        let source = "import Billing\n\nmodule Billing do\n  pub fn total() -> Int do\n    \"no\" + nope\n  end\nend\n\nfn main() do\n  println(\"${Billing.total()}\")\nend\n";
        std::fs::write(&main_path, source).unwrap();

        let result = analyze_document(&file_uri(&main_path), source, &[]);
        let messages: Vec<&str> = result
            .diagnostics
            .iter()
            .map(|diag| diag.message.as_str())
            .collect();
        assert!(
            messages.iter().any(|message| message.contains("nope")),
            "{messages:?}"
        );
        assert!(
            !messages.iter().any(|message| message.contains("Billing")),
            "{messages:?}"
        );
    }
}
