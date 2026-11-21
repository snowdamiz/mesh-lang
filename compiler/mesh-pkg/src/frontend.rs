//! The frontend every tool shares: load a project, parse and type-check its
//! modules, and run the checks that need the whole project. `meshc check`,
//! `meshc build`, `meshc test`, and the language server all call [`check`],
//! so an editor shows exactly what the compiler reports.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use mesh_common::module_graph::ModuleId;
use mesh_common::span::Span;
use mesh_diagnostics::{Code, Diagnostic, Failed, Reporter, Severity, SourceFile};
use mesh_parser::ast::item::Item;
use mesh_parser::ast::AstNode;
use mesh_parser::syntax_kind::SyntaxKind;

use crate::manifest::{
    plan_cluster_declarations, resolve_entrypoint, ClusteredDeclarationError,
    ClusteredExecutionMetadata, Manifest, DEFAULT_ENTRYPOINT,
};
use crate::native::ResolvedNativeBinding;
use crate::project::{self, CheckedProject, ProjectData};

/// What to check.
#[derive(Clone, Copy, Debug)]
pub enum Target<'a> {
    /// A project directory, or a lone `.mpl` file, as `meshc check` and
    /// `meshc build` take them.
    Project { root: &'a Path },
    /// A `*.test.mpl` file of the project at `root`: the program `meshc test`
    /// makes of it, checked after the project's modules and test helpers.
    TestFile { root: &'a Path, test: &'a Path },
    /// A document outside any project, by the path it is known by (which
    /// need not exist) and its text.
    Source { path: &'a Path, text: &'a str },
}

/// Where source files are read from.
#[derive(Clone, Copy, Debug)]
pub enum Read<'a> {
    Disk,
    /// An editor's open documents, by canonical path, read in place of
    /// their saved files.
    Overlays(&'a HashMap<PathBuf, String>),
}

#[derive(Clone, Copy, Debug)]
pub struct CheckOptions<'a> {
    pub read: Read<'a>,
    /// Check with `meshc test`'s builtins, and load the helper modules
    /// under the project's `tests/`.
    pub test_builtins: bool,
    /// A project without an entrypoint is an error (a build needs one)
    /// rather than a library.
    pub require_entrypoint: bool,
}

impl CheckOptions<'static> {
    /// `meshc check`: files from disk, a project without an entrypoint is
    /// a library.
    pub fn cli() -> Self {
        CheckOptions {
            read: Read::Disk,
            test_builtins: false,
            require_entrypoint: false,
        }
    }
}

impl<'a> CheckOptions<'a> {
    /// An editor: as [`cli`](CheckOptions::cli), reading open documents as
    /// the editor holds them.
    pub fn editor(overlays: &'a HashMap<PathBuf, String>) -> Self {
        CheckOptions {
            read: Read::Overlays(overlays),
            ..CheckOptions::cli()
        }
    }

    fn read(&self, path: &Path) -> Result<String, String> {
        if let Read::Overlays(overlays) = self.read {
            let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if let Some(text) = overlays.get(&canonical) {
                return Ok(text.clone());
            }
        }
        project::read_file(path)
    }
}

/// Everything the frontend found, and the analysis it finished.
pub struct Frontend {
    /// Ordered by file, in the order its first module is compiled, then by
    /// span start. Diagnostics about no file come first.
    pub diagnostics: Vec<Diagnostic>,
    /// `None` when the project could not be loaded; an error says why.
    pub analysis: Option<Analysis>,
}

/// A loaded and type-checked project.
pub struct Analysis {
    pub manifest: Option<Manifest>,
    /// The entry module's path, relative to the project root.
    pub entry: Option<PathBuf>,
    pub project: ProjectData,
    pub checked: CheckedProject,
    /// Each module's file, by module id. A `module` block shares its file's.
    pub files: Vec<SourceFile>,
    /// The `@cluster` functions. Empty when the project has errors.
    pub clustered_execution_plan: Vec<ClusteredExecutionMetadata>,
}

impl Frontend {
    pub fn has_errors(&self) -> bool {
        has_errors(&self.diagnostics)
    }

    /// Each file whose own module parsed without errors, with that parse:
    /// what a linter reads. A `module` block is linted with its file.
    pub fn lintable(&self) -> impl Iterator<Item = (&mesh_parser::Parse, &SourceFile)> + '_ {
        self.analysis.iter().flat_map(|analysis| {
            let mut seen = HashSet::new();
            analysis
                .project
                .module_parses
                .iter()
                .zip(&analysis.files)
                .filter(move |(parse, file)| seen.insert(file.path()) && parse.ok())
        })
    }

    /// Emits every diagnostic, and hands back the analysis when none was an
    /// error.
    pub fn report<W: Write>(self, reporter: &mut Reporter<W>) -> Result<Analysis, Failed> {
        reporter.emit_all(self.diagnostics)?;
        self.analysis.ok_or_else(|| {
            reporter.fail(Diagnostic::internal(
                "the project did not load, and no error says why",
            ))
        })
    }
}

/// A transitional string error as an uncategorized diagnostic (`C0001`),
/// until its producer gets a code. meshc and the language server convert
/// their string errors here too, so there is one such conversion.
pub fn uncategorized(message: impl Into<String>) -> Diagnostic {
    Diagnostic::uncategorized(message)
}

/// Checks `target`. One policy holds for every caller:
/// - A module with a parse error, or importing one that has, is not
///   type-checked for diagnostics, so its parse errors are not followed by
///   errors they caused.
/// - The checks that compare modules (`E0102`, `E0103`, `CFG0001`) run only
///   when the modules they compare are free of errors.
pub fn check(target: Target<'_>, options: &CheckOptions<'_>) -> Frontend {
    let loaded = match target {
        Target::Project { root } => load(root, options),
        Target::TestFile { root, test } => load_test(root, test, options),
        Target::Source { path, text } => load_source(path, text),
    };
    match loaded {
        Ok(loaded) => analyze(loaded, options),
        Err(message) => Frontend {
            diagnostics: vec![uncategorized(message)],
            analysis: None,
        },
    }
}

/// A project read and parsed, not yet checked.
struct Loaded {
    /// The directory module paths are relative to.
    root: PathBuf,
    manifest: Option<Manifest>,
    entry: Option<PathBuf>,
    native_bindings: Vec<ResolvedNativeBinding>,
    project: ProjectData,
}

fn load(root: &Path, options: &CheckOptions<'_>) -> Result<Loaded, String> {
    if !root.exists() {
        return Err(format!(
            "Project directory '{}' does not exist",
            root.display()
        ));
    }
    if root.is_file() && root.extension().is_some_and(|ext| ext == "mpl") {
        return load_source(root, &options.read(root)?);
    }
    if !root.is_dir() {
        return Err(format!(
            "'{}' is not a directory or a .mpl source file",
            root.display()
        ));
    }
    let manifest_path = root.join("mesh.toml");
    let manifest = manifest_path
        .exists()
        .then(|| Manifest::from_file(&manifest_path))
        .transpose()?;
    let native_bindings = if manifest.is_some() {
        crate::resolve_native_bindings(root)?
    } else {
        Vec::new()
    };
    let has_configured_entrypoint = manifest
        .as_ref()
        .is_some_and(|manifest| manifest.package.entrypoint.is_some());
    let entry = if options.require_entrypoint
        || has_configured_entrypoint
        || root.join(DEFAULT_ENTRYPOINT).exists()
    {
        Some(resolve_entrypoint(root, manifest.as_ref())?)
    } else {
        None
    };
    let project = project::load_modules(
        root,
        entry.as_deref(),
        &native_bindings,
        options.test_builtins,
        &|path| options.read(path),
    )?;
    Ok(Loaded {
        root: root.to_path_buf(),
        manifest,
        entry,
        native_bindings,
        project,
    })
}

fn load_source(path: &Path, text: &str) -> Result<Loaded, String> {
    let name = Path::new(path.file_name().unwrap_or(DEFAULT_ENTRYPOINT.as_ref()));
    Ok(Loaded {
        root: path.parent().unwrap_or(Path::new("")).to_path_buf(),
        manifest: None,
        entry: Some(name.to_path_buf()),
        native_bindings: Vec::new(),
        project: project::single_source_project_at(text, name)?,
    })
}

/// The project with its test helpers, and the test program as the module
/// `Main` (as `meshc test` names it), compiled last.
fn load_test(root: &Path, test: &Path, options: &CheckOptions<'_>) -> Result<Loaded, String> {
    let options = CheckOptions {
        test_builtins: true,
        require_entrypoint: false,
        ..*options
    };
    let mut loaded = load(root, &options)?;
    let program = mesh_parser::test_harness::preprocess_test_source(&options.read(test)?)
        .map_err(|error| format!("{}: {error}", test.display()))?;
    let parse = mesh_parser::parse(&program);
    let project = &mut loaded.project;
    let path = test.strip_prefix(root).unwrap_or(test).to_path_buf();
    let id = project.graph.add_module("Main".to_string(), path, false);
    for import in project::extract_imports(&parse.tree()) {
        if let Some(dependency) = project.graph.resolve(&import) {
            project.graph.add_dependency(id, dependency);
        }
    }
    project.compilation_order.push(id);
    project.module_sources.push(program);
    project.module_parses.push(parse);
    Ok(loaded)
}

fn analyze(loaded: Loaded, options: &CheckOptions<'_>) -> Frontend {
    let Loaded {
        root,
        manifest,
        entry,
        native_bindings,
        project,
    } = loaded;
    let files = project_files(&root, &project);
    let mut diagnostics = syntax_errors(&project, &files);
    let parsed = diagnostics.is_empty();
    // Types checked against a misplaced or ambiguous declaration mislead.
    let mut declarations = misplaced_native_declarations(&project, &files, &native_bindings);
    if parsed {
        declarations.extend(duplicate_type_names(&project, &files));
    }
    let declared = declarations.is_empty();
    diagnostics.extend(declarations);
    let checked = project::check_project(&project, options.test_builtins);
    if declared {
        let blocked = blocked_by_parse_errors(&project);
        for &id in &project.compilation_order {
            let idx = id.0 as usize;
            if !blocked[idx] {
                diagnostics.extend(checked.typeck[idx].diagnostics(&files[idx]));
            }
        }
    }
    if !has_errors(&diagnostics) {
        diagnostics.extend(duplicate_pub_functions(&project, &files, &checked.exports));
    }
    let mut clustered_execution_plan = Vec::new();
    if !has_errors(&diagnostics) {
        match plan_cluster_declarations(&project.graph, &project.module_parses) {
            Ok(plan) => clustered_execution_plan = plan,
            Err(issues) => diagnostics.extend(
                issues
                    .iter()
                    .map(|issue| cluster_declaration_error(&project, &files, issue)),
            ),
        }
    }
    sort(&mut diagnostics, &project, &files);
    Frontend {
        diagnostics,
        analysis: Some(Analysis {
            manifest,
            entry,
            project,
            checked,
            files,
            clustered_execution_plan,
        }),
    }
}

fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity() == Severity::Error)
}

/// Each module's file, by module id. A `module` block shares its file's,
/// as its spans index that file.
fn project_files(root: &Path, project: &ProjectData) -> Vec<SourceFile> {
    let mut by_path = HashMap::new();
    project
        .graph
        .modules
        .iter()
        .zip(&project.module_sources)
        .map(|(module, source)| {
            by_path
                .entry(&module.path)
                .or_insert_with(|| SourceFile::new(root.join(&module.path), source.as_str()))
                .clone()
        })
        .collect()
}

fn syntax_errors(project: &ProjectData, files: &[SourceFile]) -> Vec<Diagnostic> {
    project
        .compilation_order
        .iter()
        .flat_map(|id| {
            let idx = id.0 as usize;
            let file = &files[idx];
            project.module_parses[idx]
                .errors()
                .iter()
                .map(move |error| error.to_diagnostic(file))
        })
        .collect()
}

/// By module id: whether the module, or a module it imports, has a parse
/// error. Its exports, or those it imports, are then incomplete.
fn blocked_by_parse_errors(project: &ProjectData) -> Vec<bool> {
    let mut blocked = vec![false; project.graph.module_count()];
    // Dependencies come first in the compilation order.
    for &id in &project.compilation_order {
        let idx = id.0 as usize;
        blocked[idx] = !project.module_parses[idx].errors().is_empty()
            || project
                .graph
                .get(id)
                .dependencies
                .iter()
                .any(|dependency| blocked[dependency.0 as usize]);
    }
    blocked
}

/// A native declaration belongs in a binding the manifest declares: one
/// diagnostic per file that has one elsewhere.
fn misplaced_native_declarations(
    project: &ProjectData,
    files: &[SourceFile],
    native_bindings: &[ResolvedNativeBinding],
) -> Vec<Diagnostic> {
    let allowed = native_bindings
        .iter()
        .map(|binding| binding.path.as_path())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    project
        .compilation_order
        .iter()
        .map(|id| id.0 as usize)
        .filter(|&idx| {
            project.module_parses[idx]
                .tree()
                .fn_defs()
                .any(|function| function.native_decl().is_some())
        })
        .map(|idx| files[idx].path())
        .filter(|path| seen.insert(*path))
        .filter(|path| {
            !path
                .canonicalize()
                .is_ok_and(|path| allowed.contains(path.as_path()))
        })
        .map(|path| {
            uncategorized(format!(
                "Native declaration in '{}' is outside a manifest-declared native binding",
                path.display()
            ))
        })
        .collect()
}

/// A struct, sum type, interface, actor, service or supervisor name that
/// more than one module defines, public or not (`E0102`). These names are
/// one namespace across a project: the type checker took two `State`
/// structs for one type and codegen emitted one of them, so `Z.run_z()`
/// showed its `State` laid out as `A`'s, and a module's `spawn(worker)`
/// could start another module's `worker`.
fn duplicate_type_names(project: &ProjectData, files: &[SourceFile]) -> Vec<Diagnostic> {
    // A struct or sum type copied verbatim into several modules (a private
    // helper) is one layout, so its definitions are compared by their
    // tokens; code (actors, services) may call each module's own functions.
    let fingerprint = |item: &Item| -> Option<String> {
        if !matches!(item, Item::StructDef(_) | Item::SumTypeDef(_)) {
            return None;
        }
        // An indexed declaration retains its defining module in the checked
        // type. Even identical source declarations would collide after their
        // module identity and indices are projected to the native layout.
        if item.syntax().descendants().any(|node| {
            matches!(
                node.kind(),
                SyntaxKind::INDEX_PARAM | SyntaxKind::VARIANT_RESULT
            )
        }) {
            return None;
        }
        let tokens: Vec<String> = item
            .syntax()
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .filter(|token| !token.kind().is_trivia() && token.kind() != SyntaxKind::PUB_KW)
            .map(|token| token.text().to_string())
            .collect();
        Some(tokens.join(" "))
    };
    struct Definition {
        kind: &'static str,
        module: ModuleId,
        span: Span,
        fingerprint: Option<String>,
    }
    let mut owners = BTreeMap::<String, Vec<Definition>>::new();
    for &id in &project.compilation_order {
        for item in project.module_parses[id.0 as usize].tree().items() {
            let (kind, name) = match &item {
                Item::StructDef(def) => ("struct", def.name()),
                Item::SumTypeDef(def) => ("type", def.name()),
                Item::InterfaceDef(def) => ("interface", def.name()),
                Item::ActorDef(def) => ("actor", def.name()),
                Item::ServiceDef(def) => ("service", def.name()),
                Item::SupervisorDef(def) => ("supervisor", def.name()),
                _ => continue,
            };
            if let Some((text, range)) =
                name.and_then(|name| name.text().map(|text| (text, name.syntax().text_range())))
            {
                owners.entry(text).or_default().push(Definition {
                    kind,
                    module: id,
                    span: Span::new(range.start().into(), range.end().into()),
                    fingerprint: fingerprint(&item),
                });
            }
        }
    }

    owners
        .iter()
        .filter_map(|(name, defs)| {
            let first = &defs[0];
            if defs.iter().all(|def| def.module == first.module) {
                // One module (a duplicate there is the type checker's E0068).
                return None;
            }
            if first.fingerprint.is_some()
                && defs.iter().all(|def| def.fingerprint == first.fingerprint)
            {
                return None;
            }
            let label = |def: &Definition| {
                format!("{} in `{}`", def.kind, project.graph.get(def.module).name)
            };
            let mut diagnostic = Diagnostic::in_file(
                Code::DuplicateTypeAcrossModules,
                &files[first.module.0 as usize],
                format!("`{name}` is defined in more than one module"),
            )
            .at(first.span, label(first))
            .with_note(
                "Identical, non-indexed struct and sum type definitions can share a layout.",
            );
            for def in &defs[1..] {
                diagnostic =
                    diagnostic.with_label_in(&files[def.module.0 as usize], def.span, label(def));
            }
            Some(diagnostic)
        })
        .collect()
}

/// Two modules exporting a `pub fn` under the same symbol (`E0103`).
///
/// `Lowerer::qualify_name` deliberately leaves `pub` function names unqualified so
/// importing modules can call them by their bare name, and `merge_mir_modules` then
/// keys merged functions by that bare name, keeping the first one it sees. Two modules
/// exporting the same name therefore collapse into one symbol: the later module's body
/// is dropped, and every call site, including `from B import f`, binds to the earlier
/// module's body while the type checker went on believing it had `B`'s signature. That
/// mismatch is silent, and when the signatures differ it reinterprets values across
/// types (an `Int` returned where a `String` was expected becomes a wild pointer).
///
/// Until pub symbols are module-qualified, the ambiguous program is refused.
fn duplicate_pub_functions(
    project: &ProjectData,
    files: &[SourceFile],
    all_exports: &[Option<mesh_typeck::ExportedSymbols>],
) -> Vec<Diagnostic> {
    // Export keys are exactly the `pub_fns` handed to the lowerer, so a key collision
    // is a symbol collision. Same-name/different-arity pub fns inside one module are
    // already disambiguated by `collect_exports` as `name__<arity>`.
    let mut owners: BTreeMap<&str, Vec<ModuleId>> = BTreeMap::new();
    for &id in &project.compilation_order {
        let exports = all_exports[id.0 as usize].iter();
        for name in exports.flat_map(|exports| exports.functions.keys()) {
            owners.entry(name.as_str()).or_default().push(id);
        }
    }

    owners
        .iter()
        .filter(|(_, modules)| modules.len() > 1)
        .map(|(symbol, modules)| {
            // Strip the `__<arity>` suffix `collect_exports` adds to overloaded pub fns.
            let (display, arity) = symbol
                .rsplit_once("__")
                .and_then(|(base, arity)| {
                    arity.parse::<usize>().ok().map(|arity| (base, Some(arity)))
                })
                .unwrap_or((symbol, None));
            let mut declarations = modules.iter().filter_map(|&module| {
                let idx = module.0 as usize;
                project.module_parses[idx]
                    .tree()
                    .fn_defs()
                    .find_map(|function| {
                        let name = function.name()?;
                        let text = name.text()?;
                        let matches_overload = text == display
                            && arity.is_some_and(|arity| {
                                function
                                    .param_list()
                                    .map_or(0, |params| params.params().count())
                                    == arity
                            });
                        if function.visibility().is_none()
                            || (text != *symbol && !matches_overload)
                        {
                            return None;
                        }
                        let range = name.syntax().text_range();
                        Some((
                            &files[idx],
                            Span::new(range.start().into(), range.end().into()),
                            &project.graph.get(module).name,
                        ))
                    })
            });
            let message = format!("public function `{display}` is defined in more than one module");
            let mut diagnostic = if let Some((file, span, module)) = declarations.next() {
                Diagnostic::in_file(Code::DuplicatePubFnAcrossModules, file, message)
                    .at(span, format!("public definition in `{module}`"))
            } else {
                Diagnostic::new(Code::DuplicatePubFnAcrossModules, message)
            };
            for (file, span, module) in declarations {
                diagnostic =
                    diagnostic.with_label_in(file, span, format!("public definition in `{module}`"));
            }
            diagnostic.with_note(format!(
                "Public function names share one global symbol space, so a call to `{display}` would silently run only one of these definitions."
            ))
        })
        .collect()
}

fn cluster_declaration_error(
    project: &ProjectData,
    files: &[SourceFile],
    issue: &ClusteredDeclarationError,
) -> Diagnostic {
    let idx = project
        .graph
        .modules
        .iter()
        .position(|module| module.path == issue.file)
        .expect("an `@cluster` decorator belongs to a project file");
    Diagnostic::in_file(
        Code::ClusterDeclarationInvalid,
        &files[idx],
        issue.to_string(),
    )
    .at(issue.span, &issue.reason)
}

/// By file, in the order its first module is compiled, then by span
/// start. Diagnostics about no file, or about a whole file, come first.
fn sort(diagnostics: &mut [Diagnostic], project: &ProjectData, files: &[SourceFile]) {
    let mut rank = HashMap::new();
    for id in &project.compilation_order {
        let next = rank.len();
        rank.entry(files[id.0 as usize].path()).or_insert(next);
    }
    diagnostics.sort_by_key(|diagnostic| {
        (
            diagnostic
                .file()
                .map(|file| rank.get(file.path()).copied().unwrap_or(usize::MAX)),
            diagnostic.span().map(|span| span.start),
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n";

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mesh.toml"), MANIFEST).unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    /// The code and file name of each diagnostic, in order, once each.
    fn found(frontend: &Frontend) -> Vec<(&'static str, String)> {
        let mut found: Vec<_> = frontend
            .diagnostics
            .iter()
            .map(|d| {
                let file = d.file().map_or_else(String::new, |file| {
                    file.path()
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                });
                (d.code().id(), file)
            })
            .collect();
        found.dedup();
        found
    }

    fn found_in(root: &Path) -> Vec<(&'static str, String)> {
        found(&check(Target::Project { root }, &CheckOptions::cli()))
    }

    #[test]
    fn a_parse_error_is_not_followed_by_the_errors_it_causes() {
        let dir = project(&[
            (
                "main.mpl",
                "from Util import helper\n\nfn main() do\n  let n :: Int = helper()\nend\n",
            ),
            ("util.mpl", "pub fn helper( -> Int do\n  1\nend\n"),
            (
                "other.mpl",
                "pub fn other() -> Int do\n  \"not an Int\"\nend\n",
            ),
        ]);
        // `main` imports the module that does not parse, so it is not
        // checked; `other` does not, so its error is still reported.
        assert_eq!(
            found_in(dir.path()),
            [
                ("E0001", "other.mpl".to_string()),
                ("P0001", "util.mpl".to_string()),
            ]
        );
    }

    #[test]
    fn a_parse_error_keeps_its_related_span() {
        let text = "logic do\n  x\n";
        let frontend = check(
            Target::Source {
                path: Path::new("proof.mpl"),
                text,
            },
            &CheckOptions::cli(),
        );
        let [d] = frontend.diagnostics.as_slice() else {
            panic!("{:?}", frontend.diagnostics);
        };
        assert_eq!(d.code(), Code::MissingEnd);
        assert_eq!(d.labels()[0].message(), "block opened here");
        assert_eq!(d.labels()[0].span(), Span::new(6, 8));
    }

    #[test]
    fn diagnostics_are_ordered_by_file_in_compilation_order_then_position() {
        let dir = project(&[
            (
                "main.mpl",
                "from Util import helper\n\nfn main() do\n  let a :: Int = \"a\"\n  let b :: Int = helper()\nend\n",
            ),
            (
                "util.mpl",
                "pub fn helper() -> String do\n  \"h\"\nend\n\nfn unused() -> Int do\n  \"u\"\nend\n",
            ),
        ]);
        let frontend = check(Target::Project { root: dir.path() }, &CheckOptions::cli());
        let at: Vec<_> = frontend
            .diagnostics
            .iter()
            .map(|d| {
                (
                    d.file().unwrap().path().file_name().unwrap().to_owned(),
                    d.line(),
                )
            })
            .collect();
        assert_eq!(
            at,
            [
                ("util.mpl".into(), Some(6)),
                ("main.mpl".into(), Some(4)),
                ("main.mpl".into(), Some(5)),
            ]
        );
    }

    #[test]
    fn modules_are_compared_only_once_they_are_free_of_errors() {
        let duplicate = "module A do\n  pub fn count() do\n    1\n  end\nend\n\nmodule B do\n  pub fn count() do\n    2\n  end\nend\n";
        let source = |text| Target::Source {
            path: Path::new("main.mpl"),
            text,
        };
        let codes = |text| -> Vec<&str> {
            check(source(text), &CheckOptions::cli())
                .diagnostics
                .iter()
                .map(|d| d.code().id())
                .collect()
        };
        assert_eq!(codes(duplicate), ["E0103"]);
        let with_type_error = format!("{duplicate}\nfn main() do\n  let n :: Int = \"n\"\nend\n");
        assert_eq!(codes(&with_type_error), ["E0001"]);
    }

    #[test]
    fn a_project_that_does_not_load_says_why() {
        let dir = project(&[
            ("main.mpl", "fn main() do\nend\n"),
            ("a.mpl", "import B\n"),
            ("b.mpl", "import A\n"),
        ]);
        let frontend = check(Target::Project { root: dir.path() }, &CheckOptions::cli());
        assert!(frontend.analysis.is_none());
        let [d] = frontend.diagnostics.as_slice() else {
            panic!("{:?}", frontend.diagnostics);
        };
        assert!(
            d.message().contains("Circular dependency"),
            "{}",
            d.message()
        );

        let missing = dir.path().join("missing");
        let frontend = check(Target::Project { root: &missing }, &CheckOptions::cli());
        assert!(frontend.diagnostics[0].message().contains("does not exist"));
    }

    #[test]
    fn an_editor_reads_its_open_documents_in_place_of_their_files() {
        let dir = project(&[("main.mpl", "fn main() do\n  let n :: Int = \"n\"\nend\n")]);
        let root = dir.path().canonicalize().unwrap();
        assert_eq!(found_in(&root), [("E0001", "main.mpl".to_string())]);
        let overlays = HashMap::from([(
            root.join("main.mpl"),
            "fn main() do\n  let n :: Int = 1\nend\n".to_string(),
        )]);
        let frontend = check(
            Target::Project { root: &root },
            &CheckOptions::editor(&overlays),
        );
        assert_eq!(found(&frontend), []);
    }

    #[test]
    fn a_test_file_is_checked_as_meshc_test_builds_it() {
        let test =
            "from Greeting import message\n\ntest(\"greets\") do\n  assert_eq(message(), 1)\nend\n";
        let dir = project(&[
            ("main.mpl", "fn main() do\nend\n"),
            (
                "greeting.mpl",
                "pub fn message() -> String do\n  \"hi\"\nend\n",
            ),
            ("tests/greeting.test.mpl", test),
        ]);
        let path = dir.path().join("tests/greeting.test.mpl");
        let frontend = check(
            Target::TestFile {
                root: dir.path(),
                test: &path,
            },
            &CheckOptions::cli(),
        );
        let [d] = frontend.diagnostics.as_slice() else {
            panic!("{:?}", frontend.diagnostics);
        };
        assert_eq!(d.file().unwrap().path(), path);
        assert_eq!(d.line(), Some(4));
        let analysis = frontend.analysis.unwrap();
        let module = analysis.project.graph.modules.last().unwrap();
        assert_eq!(module.path, Path::new("tests/greeting.test.mpl"));
    }

    #[test]
    fn report_hands_back_the_analysis_unless_an_error_was_reported() {
        let mut out = Vec::new();
        let mut reporter = Reporter::new(mesh_diagnostics::Format::Json, &mut out);
        let warning = "fn main() do\n  let n = case 1 do\n    _ -> 0\n    1 -> 2\n  end\n  println(\"#{n}\")\nend\n";
        let checked = check(
            Target::Source {
                path: Path::new("main.mpl"),
                text: warning,
            },
            &CheckOptions::cli(),
        );
        assert!(!checked.has_errors());
        assert!(checked.report(&mut reporter).is_ok());
        let error = check(
            Target::Source {
                path: Path::new("main.mpl"),
                text: "fn main() do\n  let = 1\nend\n",
            },
            &CheckOptions::cli(),
        );
        assert!(error.has_errors());
        let errors = error.diagnostics.len() as u32;
        assert!(error.report(&mut reporter).is_err());
        assert_eq!(
            reporter.finish(),
            mesh_diagnostics::Summary {
                errors,
                warnings: 1
            }
        );
    }

    #[test]
    fn each_file_that_parsed_is_lintable_once() {
        let dir = project(&[
            (
                "main.mpl",
                "import Geo\n\nmodule Geo do\n  pub fn area() = 1\nend\n\nfn main() do\nend\n",
            ),
            ("broken.mpl", "fn broken( do\n"),
        ]);
        let frontend = check(Target::Project { root: dir.path() }, &CheckOptions::cli());
        let files: Vec<_> = frontend
            .lintable()
            .map(|(_, file)| file.path().file_name().unwrap().to_owned())
            .collect();
        assert_eq!(files, ["main.mpl"]);
    }
}
