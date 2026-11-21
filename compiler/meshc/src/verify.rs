use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{ReportResult, ReportText};
use mesh_codegen::mir::{MirFunction, MirModule, MirSourceId, MirSourceSpan};
use mesh_common::module_graph::ModuleId;
use mesh_common::span::{LineIndex, Span};
use mesh_diagnostics::{Code, Diagnostic, Failed, Reporter, SourceFile};
use mesh_parser::ast::item::FnDef;
use mesh_parser::law::{LawDeclaration, LawFile, LawImport, LawProof, ProofTerm};
use mesh_parser::{AstNode, SyntaxNode};
use mesh_pkg::project::{laws, CheckedProject, ProjectData};
use mesh_typeck::ty::Ty;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::verify_proof;
use crate::verify_solver::{self, Verdict};

#[path = "verify_logic.rs"]
mod logic;

fn source_ids(
    project: &ProjectData,
) -> std::collections::HashMap<*const mesh_parser::Parse, MirSourceId> {
    project
        .module_parses
        .iter()
        .enumerate()
        .map(|(index, parse)| (std::ptr::from_ref(parse), MirSourceId(index)))
        .collect()
}

/// Lower the same project that will be built, retaining verifier metadata only
/// when a source module declares an embedded proof obligation.
/// `files` holds each module's file by module id.
pub(crate) fn lower_for_build(
    project: &ProjectData,
    checked: &mut CheckedProject,
    files: &[SourceFile],
    package: &str,
    reporter: &mut Reporter,
) -> Result<Vec<MirModule>, Failed> {
    let mut proofs = logic::Program::default();
    proofs
        .add_project_blocks(project, checked, files)
        .reported(reporter)?;
    let inventory = proofs
        .inventory(package)
        .reported_as(Code::LawBindingUnresolved, reporter)?;
    let sources = source_ids(project);
    let source_id = |parse: &mesh_parser::Parse| sources.get(&std::ptr::from_ref(parse)).copied();
    if inventory.is_empty()
        && !checked.typeck.iter().any(|module| {
            !module.proof_values.is_empty() || module.types.values().any(Ty::contains_proof)
        })
    {
        return crate::lower_checked_modules(project, checked, source_id, files, None, reporter)
            .reported(reporter);
    }

    let mut lowering = proofs.verification_lowering();
    let modules = crate::lower_checked_modules(
        project,
        checked,
        source_id,
        files,
        Some(&mut lowering),
        reporter,
    )
    .reported(reporter)?;
    if inventory.is_empty() {
        return Ok(modules);
    }
    proofs
        .verify(
            project,
            checked,
            &modules,
            &lowering.signatures,
            verify_proof::Settings {
                emit: None,
                verdict: false,
            },
        )
        .map_err(|failure| {
            let sources = project_sources(project, files);
            law_diagnostic(
                failure.code,
                failure.reason,
                failure.location.as_ref(),
                &sources,
            )
        })
        .reported(reporter)?;
    Ok(modules)
}

const INVENTORY_VERSION: u32 = 1;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Target {
    package: String,
    module: String,
    function: String,
    arity: usize,
    signature: String,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}::{}.{}/{} {}",
            self.package, self.module, self.function, self.arity, self.signature
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Location {
    path: String,
    line: u32,
    column: u32,
    start: u32,
    end: u32,
}

impl Location {
    fn new(path: &str, lines: &LineIndex, span: Span) -> Self {
        let (line, column) = lines.line_col(span.start);
        Self {
            path: path.to_owned(),
            line,
            column,
            start: span.start,
            end: span.end,
        }
    }

    fn span(&self) -> Span {
        Span::new(self.start, self.end)
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.path, self.line, self.column)
    }
}

#[derive(Serialize, Deserialize)]
struct InventoryEntry {
    target: Target,
    access: Access,
    requirement_digest: String,
    location: Location,
    declaration: Location,
}

#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Access {
    Public,
    Private,
    Theorem,
}

impl Access {
    fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Theorem => "theorem",
        }
    }
}

struct BoundLaw {
    entry: InventoryEntry,
    module_id: ModuleId,
    symbol: String,
    expected: mesh_parser::Parse,
    expected_types: mesh_typeck::TypeckResult,
    proof: Option<BoundProof>,
}

struct BoundProof {
    term: ProofTerm,
    location: Location,
    signature: Ty,
}

#[derive(Serialize)]
struct LawReport<'a> {
    #[serde(flatten)]
    entry: &'a InventoryEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    proof_location: Option<&'a Location>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    verdict: Option<Verdict>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unsupported_at: Option<Location>,
    #[serde(skip)]
    failure_code: Option<Code>,
}

#[derive(Deserialize)]
struct Baseline {
    schema_version: u32,
    laws: Vec<InventoryEntry>,
}

#[derive(Serialize)]
struct PreservationChange<'a> {
    target: &'a Target,
    change: &'static str,
    old_location: &'a Location,
    new_location: Option<&'a Location>,
}

pub(crate) fn run(
    project_dir: &Path,
    list: bool,
    json: bool,
    preserve: Option<&Path>,
    proof_settings: verify_proof::Settings<'_>,
    reporter: &mut Reporter,
) -> Result<(), Failed> {
    let project_dir = project_dir
        .canonicalize()
        .map_err(|error| {
            format!(
                "Failed to resolve project '{}': {error}",
                project_dir.display()
            )
        })
        .reported_as(Code::PathNotFound, reporter)?;
    let analysis = mesh_pkg::frontend::check(
        mesh_pkg::frontend::Target::Project { root: &project_dir },
        &mesh_pkg::frontend::CheckOptions::cli(),
    )
    .report(reporter)?;
    let files = laws::discover(&project_dir, analysis.manifest.as_ref())
        .reported_as(Code::PathNotFound, reporter)?;
    let sources = parse_law_files(&project_dir, &files.paths, reporter)?;
    verify_laws(
        &project_dir,
        analysis,
        &files.roots,
        sources,
        list,
        json,
        preserve,
        proof_settings,
        reporter,
    )
}

/// A discovered law file and what it parsed to.
struct LawSource {
    /// Its package-relative path with `/` separators.
    path: String,
    file: SourceFile,
    parsed: Parsed,
}

enum Parsed {
    Laws(LawFile),
    Logic(mesh_parser::logic::LogicFile),
}

/// Read and parse each law file. A syntax error is reported where it is in
/// its file (`P02xx`, or `P03xx` in a `logic` file).
fn parse_law_files(
    project_dir: &Path,
    paths: &[PathBuf],
    reporter: &mut Reporter,
) -> Result<Vec<LawSource>, Failed> {
    paths
        .iter()
        .map(|path| {
            let display = portable_path(path).reported_as(Code::PathNotFound, reporter)?;
            let full = project_dir.join(path);
            let text = mesh_pkg::project::read_file(&full).reported(reporter)?;
            let parsed = if mesh_parser::logic::is_logic_file(&text) {
                mesh_parser::logic::parse(&text).map(Parsed::Logic)
            } else {
                mesh_parser::law::parse(&text).map(Parsed::Laws)
            };
            let file = SourceFile::new(full, text);
            let parsed = parsed.map_err(|error| reporter.fail(error.to_diagnostic(&file)))?;
            Ok(LawSource {
                path: display,
                file,
                parsed,
            })
        })
        .collect()
}

fn verify_laws(
    project_dir: &Path,
    analysis: mesh_pkg::frontend::Analysis,
    law_roots: &[PathBuf],
    sources: Vec<LawSource>,
    list: bool,
    json: bool,
    preserve: Option<&Path>,
    proof_settings: verify_proof::Settings<'_>,
    reporter: &mut Reporter,
) -> Result<(), Failed> {
    let mesh_pkg::frontend::Analysis {
        manifest,
        project,
        mut checked,
        files: module_files,
        ..
    } = analysis;
    let package = manifest.as_ref().map_or_else(
        || {
            project_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(".")
        },
        |manifest| manifest.package.name.as_str(),
    );
    let mut bound = BTreeMap::<_, BoundLaw>::new();
    let mut logic = logic::Program::default();
    logic
        .add_project_blocks(&project, &checked, &module_files)
        .reported(reporter)?;
    let mut diagnostic_sources = project_sources(&project, &module_files);
    for source in &sources {
        diagnostic_sources.insert(source.path.clone(), source.file.clone());
    }
    for source in sources {
        let laws = match source.parsed {
            Parsed::Logic(file) => {
                logic
                    .add_file(&project, &checked, &source.path, &source.file, file)
                    .reported(reporter)?;
                continue;
            }
            Parsed::Laws(file) => file,
        };
        for law in bind_file(
            package,
            &source.path,
            &source.file,
            laws,
            &project,
            &checked,
        )
        .reported(reporter)?
        {
            let target = &law.entry.target;
            let key = (target.module.clone(), target.function.clone(), target.arity);
            if let Some(previous) = bound.get(&key) {
                return Err(reporter.fail(law_diagnostic(
                    Code::LawBindingUnresolved,
                    format!(
                        "Duplicate law for `{target}` at {} and {}",
                        previous.entry.location, law.entry.location
                    ),
                    Some(&law.entry.location),
                    &diagnostic_sources,
                )));
            }
            bound.insert(key, law);
        }
    }

    let theorems = logic
        .inventory(package)
        .reported_as(Code::LawBindingUnresolved, reporter)?;
    if bound.is_empty() && theorems.is_empty() && preserve.is_none() {
        return Err(reporter.fail(Diagnostic::new(Code::LawUnverified, "No laws found under the effective law roots")
            .with_repair("Declare a law under the effective law roots, or configure `[verify].law_roots` in the manifest.")));
    }
    let baseline = preserve.map(read_baseline).transpose().reported(reporter)?;
    let changes = baseline.as_ref().map(|base| {
        compare(
            base,
            bound
                .values()
                .map(|law| &law.entry)
                .chain(theorems.iter().map(|theorem| &theorem.entry)),
        )
    });
    let mut lowering = logic.verification_lowering();
    let program = if list || (bound.is_empty() && theorems.is_empty()) {
        None
    } else {
        let source_ids = source_ids(&project);
        Some(crate::lower_checked_modules(
            &project,
            &mut checked,
            |parse| source_ids.get(&std::ptr::from_ref(parse)).copied(),
            &module_files,
            Some(&mut lowering),
            reporter,
        ))
    };
    let mut reports = bound
        .values()
        .map(|law| {
            let mut unsupported_at = None;
            let mut failure_code = None;
            let verdict = program.as_ref().map(|program| {
                let result = match program {
                    Ok(modules) => prove(
                        law,
                        &project,
                        &checked,
                        modules,
                        &lowering.signatures,
                        proof_settings,
                    ),
                    Err(error) => Err(verify_solver::EncodingError::new(
                        Code::LawUnverified,
                        crate::diagnostic_reason(error),
                    )),
                };
                result.unwrap_or_else(|error| {
                    failure_code = Some(error.code);
                    unsupported_at = error
                        .location
                        .and_then(|location| expression_location(law, &project, location))
                        .or_else(|| law.proof.as_ref().map(|proof| proof.location.clone()));
                    Verdict::Unverified {
                        reason: error.reason,
                    }
                })
            });
            LawReport {
                entry: &law.entry,
                proof_location: law.proof.as_ref().map(|proof| &proof.location),
                verdict,
                unsupported_at,
                failure_code,
            }
        })
        .collect::<Vec<_>>();

    let logic_result = if theorems.is_empty() {
        None
    } else {
        program.as_ref().map(|program| match program {
            Ok(modules) => logic.verify(
                &project,
                &checked,
                modules,
                &lowering.signatures,
                proof_settings,
            ),
            Err(error) => Err(verify_solver::EncodingError::new(
                Code::LawUnverified,
                crate::diagnostic_reason(error),
            )
            .into()),
        })
    };
    for theorem in &theorems {
        let verdict = logic_result.as_ref().map(|result| match result {
            Ok(()) => Verdict::Verified,
            Err(failure) => Verdict::Unverified {
                reason: failure.reason.clone(),
            },
        });
        let unsupported_at = logic_result.as_ref().and_then(|result| {
            let failure = result.as_ref().err()?;
            failure
                .location
                .clone()
                .or_else(|| theorem.proof_location.clone())
                .or_else(|| Some(theorem.entry.location.clone()))
        });
        reports.push(LawReport {
            entry: &theorem.entry,
            proof_location: theorem.proof_location.as_ref(),
            verdict,
            unsupported_at,
            failure_code: logic_result
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .map(|failure| failure.code),
        });
    }

    let roots = law_roots
        .iter()
        .map(|path| portable_path(path))
        .collect::<Result<Vec<_>, _>>()
        .reported_as(Code::PathNotFound, reporter)?;
    if json {
        let report = serde_json::json!({
            "schema_version": INVENTORY_VERSION,
            "law_roots": roots,
            "laws": reports,
            "preservation": changes.as_ref().map(|changes| serde_json::json!({
                "preserved": changes.is_empty(),
                "changes": changes,
            })),
        });
        println!("{report}");
    } else {
        println!("Law roots: {}", roots.join(", "));
        for law in &reports {
            print_law(law);
        }
        if let Some(changes) = &changes {
            for change in changes {
                match change.new_location {
                    Some(new) => println!(
                        "Requirement changed: `{}` (base {}, candidate {new})",
                        change.target, change.old_location
                    ),
                    None => println!(
                        "Requirement missing: `{}` (base {})",
                        change.target, change.old_location
                    ),
                }
            }
            if changes.is_empty() {
                println!("All baseline requirements preserved.");
            }
        }
    }

    let mut failure = None;
    match &program {
        // The program did not lower: its own diagnostic says why every law
        // went unverified, once.
        Some(Err(error)) => failure = Some(reporter.fail(error.clone())),
        _ => {
            for law in &reports {
                if let Some(diagnostic) = verdict_diagnostic(law, &diagnostic_sources) {
                    failure = Some(reporter.fail(diagnostic));
                }
            }
        }
    }
    if changes.is_some_and(|changes| !changes.is_empty()) {
        failure = Some(reporter.fail(
            Diagnostic::new(Code::LawUnverified, "Law preservation failed").with_repair(
                "Restore the missing or changed requirements listed in the verification report.",
            ),
        ));
    }
    failure.map_or(Ok(()), Err)
}

fn project_sources(project: &ProjectData, files: &[SourceFile]) -> BTreeMap<String, SourceFile> {
    project
        .graph
        .modules
        .iter()
        .zip(files)
        .filter_map(|(module, file)| Some((portable_path(&module.path).ok()?, file.clone())))
        .collect()
}

fn law_diagnostic(
    code: Code,
    message: String,
    location: Option<&Location>,
    sources: &BTreeMap<String, SourceFile>,
) -> Diagnostic {
    if let Some((at, source)) = location.and_then(|at| Some((at, sources.get(&at.path)?))) {
        Diagnostic::in_file(code, source, message).at(at.span(), "")
    } else {
        Diagnostic::new(code, message)
    }
}

fn verdict_diagnostic(
    law: &LawReport<'_>,
    sources: &BTreeMap<String, SourceFile>,
) -> Option<Diagnostic> {
    let target = &law.entry.target;
    let location = &law.entry.location;
    match law.verdict.as_ref()? {
        Verdict::Verified => None,
        Verdict::Falsified {
            inputs,
            expected,
            actual,
        } => Some(
            law_diagnostic(
                Code::LawFalsified,
                format!("Law for `{target}` was falsified"),
                Some(location),
                sources,
            )
            .with_note(format!(
                "Counterexample: {}, expected {expected}, actual {actual}",
                serde_json::json!(inputs),
            )),
        ),
        Verdict::Unverified { reason } => {
            let code = law.failure_code.unwrap_or(Code::LawUnverified);
            let unsupported = law
                .unsupported_at
                .as_ref()
                .filter(|_| code == Code::LawUnsupported);
            let primary = unsupported
                .filter(|at| at.path == location.path)
                .unwrap_or(location);
            let mut diagnostic = law_diagnostic(
                code,
                format!("Law for `{target}` did not verify: {reason}"),
                Some(primary),
                sources,
            );
            if let Some(at) = unsupported.filter(|at| at.path != primary.path) {
                if let Some(file) = sources.get(&at.path) {
                    diagnostic = diagnostic.with_label_in(file, at.span(), "unsupported here");
                }
            }
            Some(diagnostic)
        }
    }
}

fn print_law(law: &LawReport<'_>) {
    let entry = law.entry;
    println!(
        "`{}` [{}] at {} (declared at {}; sha256 {})",
        entry.target,
        entry.access.as_str(),
        entry.location,
        entry.declaration,
        entry.requirement_digest,
    );
    match &law.verdict {
        Some(Verdict::Verified) => println!("  verified"),
        Some(Verdict::Falsified {
            inputs,
            expected,
            actual,
        }) => {
            let inputs = serde_json::json!(inputs);
            println!("  falsified: {inputs}, expected {expected}, actual {actual}");
        }
        Some(Verdict::Unverified { reason }) => match &law.unsupported_at {
            Some(location) => println!("  unverified at {location}: {reason}"),
            None => println!("  unverified: {reason}"),
        },
        None => {}
    }
}

fn bind_file(
    package: &str,
    path: &str,
    source: &SourceFile,
    file: LawFile,
    project: &ProjectData,
    checked: &CheckedProject,
) -> Result<Vec<BoundLaw>, Diagnostic> {
    let lines = LineIndex::new(source.text().expect("law source text"));
    let at = |span| Location::new(path, &lines, span);
    let binding = |span, message| {
        Diagnostic::in_file(Code::LawBindingUnresolved, source, message).at(span, "")
    };
    let LawFile {
        imports,
        laws,
        proofs,
    } = file;
    let mut proofs_by_name = BTreeMap::new();
    for proof in proofs {
        let span = proof.span;
        let name = proof.name.clone();
        if let Some(previous) = proofs_by_name.insert(name.clone(), proof) {
            return Err(binding(
                span,
                format!(
                    "Duplicate proof for `{name}` (previously at {})",
                    at(previous.span)
                ),
            ));
        }
    }
    let mut by_name = BTreeMap::<_, Vec<_>>::new();
    for import in &imports {
        by_name
            .entry(import.name.as_str())
            .or_default()
            .push(import);
    }
    let mut used = BTreeSet::new();
    let mut bound = Vec::new();
    for law in laws {
        let import = match by_name.get(law.name.as_str()).map(Vec::as_slice) {
            Some([import]) => *import,
            Some(candidates) => {
                let candidates = candidates
                    .iter()
                    .map(|import| {
                        format!("`{}.{}` at {}", import.module, import.name, at(import.span))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(binding(
                    law.span,
                    format!("Ambiguous target import for `{}`: {candidates}", law.name),
                ));
            }
            None => {
                return Err(binding(
                    law.span,
                    format!("Missing target import for `{}`", law.name),
                ))
            }
        };
        used.insert(import.name.as_str());
        let proof = proofs_by_name.remove(&law.name);
        bound.push(bind_law(
            package, project, checked, import, law, proof, &at, source,
        )?);
    }
    if let Some((name, proof)) = proofs_by_name.first_key_value() {
        return Err(binding(
            proof.span,
            format!("Proof `{name}` has no law in this file"),
        ));
    }
    if let Some(unused) = imports
        .iter()
        .find(|import| !used.contains(import.name.as_str()))
    {
        return Err(binding(
            unused.span,
            format!(
                "Target import `{}.{}` has no law",
                unused.module, unused.name
            ),
        ));
    }
    Ok(bound)
}

fn bind_law(
    package: &str,
    project: &ProjectData,
    checked: &CheckedProject,
    import: &LawImport,
    law: LawDeclaration,
    proof: Option<LawProof>,
    at: &impl Fn(Span) -> Location,
    source: &SourceFile,
) -> Result<BoundLaw, Diagnostic> {
    let location = at(law.span);
    let binding = |span, message| {
        Diagnostic::in_file(Code::LawBindingUnresolved, source, message).at(span, "")
    };
    let signature =
        |message| Diagnostic::in_file(Code::LawSignatureMismatch, source, message).at(law.span, "");
    let target_name = format!("{}.{}", import.module, import.name);
    let module_id = project.graph.resolve(&import.module).ok_or_else(|| {
        let candidates = project
            .graph
            .modules
            .iter()
            .filter(|module| module.name.rsplit('.').next() == import.module.rsplit('.').next())
            .map(|module| module.name.as_str())
            .collect::<Vec<_>>();
        binding(
            import.span,
            format!(
                "Unknown target module `{}`{}",
                import.module,
                candidate_list(&candidates)
            ),
        )
    })?;
    let module = project.graph.get(module_id);
    if module.path.is_absolute() {
        return Err(binding(import.span, format!(
            "Law target `{target_name}` must belong to the selected package `{package}`; found '{}'",
            module.path.display()
        )));
    }

    let expected_function = law
        .expected
        .tree()
        .fn_defs()
        .next()
        .ok_or_else(|| signature("Expected a law function".into()))?;
    let expected_params = expected_function
        .param_list()
        .ok_or_else(|| signature("Expected an explicit law parameter list".into()))?;
    let mut params = BTreeSet::new();
    for param in expected_params.params() {
        let Some(name) = param.name() else {
            return Err(signature("Law parameters must be named".into()));
        };
        if param.type_annotation().is_none() || param.ownership_modifier().is_some() {
            return Err(signature(format!(
                "Law parameter `{name}` needs a type annotation"
            )));
        }
        if !params.insert(name.text().to_owned()) {
            return Err(signature(format!("Duplicate law parameter `{name}`")));
        }
    }
    let arity = params.len();

    let idx = module_id.0 as usize;
    let tree = project.module_parses[idx].tree();
    let target_functions = tree.fn_defs().collect::<Vec<_>>();
    let actual = target_functions
        .iter()
        .find(|function| {
            function.name().and_then(|name| name.text()).as_deref() == Some(&import.name)
                && parameter_count(function) == arity
        })
        .ok_or_else(|| {
            let candidates = target_functions
                .iter()
                .filter_map(|function| {
                    let name = function.name()?.text()?;
                    Some(format!(
                        "{}.{}/{}",
                        module.name,
                        name,
                        parameter_count(function)
                    ))
                })
                .collect::<BTreeSet<_>>();
            let candidates = candidates.iter().map(String::as_str).collect::<Vec<_>>();
            let message = format!(
                "No checked function `{target_name}/{arity}`{}",
                candidate_list(&candidates)
            );
            if target_functions.iter().any(|function| {
                function.name().and_then(|name| name.text()).as_deref() == Some(&import.name)
            }) {
                signature(message)
            } else {
                binding(import.span, message)
            }
        })?;

    let source_symbol = if checked.typeck[idx]
        .overloaded_fn_names
        .contains(&import.name)
    {
        format!("{}__{arity}", import.name)
    } else {
        import.name.clone()
    };
    let public = checked.exports[idx]
        .as_ref()
        .is_some_and(|exports| exports.functions.contains_key(&source_symbol));
    let access = match (public, import.private) {
        (true, false) => Access::Public,
        (false, true) => Access::Private,
        (false, false) => {
            return Err(binding(
                import.span,
                format!(
            "`{target_name}/{arity}` is private; use `from {} import private {}` in this law file",
            import.module, import.name,
        ),
            ))
        }
        (true, true) => {
            return Err(binding(
                import.span,
                format!("`{target_name}/{arity}` is public; remove `private` from the import",),
            ))
        }
    };

    let actual_range = actual.syntax().text_range();
    let actual_type = checked.typeck[idx]
        .types
        .get(&actual_range)
        .ok_or_else(|| {
            signature(format!(
                "Missing checked signature for `{target_name}/{arity}`"
            ))
        })?;
    let expected_types = mesh_typeck::check(&law.expected);
    if let Some(error) = expected_types.errors.first() {
        let location = error.span().map_or(location.clone(), |range| {
            at(Span::new(
                law.span.start + u32::from(range.start()),
                law.span.start + u32::from(range.end()),
            ))
        });
        return Err(Diagnostic::in_file(
            error.code(),
            source,
            format!("Invalid law body for `{target_name}`: {error}"),
        )
        .at(location.span(), ""));
    }
    let expected_type = expected_types
        .types
        .get(&expected_function.syntax().text_range())
        .ok_or_else(|| signature(format!("Missing checked law signature for `{target_name}`")))?;
    if actual_type != expected_type
        || !matches!(actual_type, Ty::Fun(..))
        || actual_type.has_type_vars()
    {
        return Err(signature(format!(
            "Law signature {expected_type} does not match the concrete target `{target_name}` signature {actual_type}"
        )));
    }

    let declaration = Location::new(
        &portable_path(&module.path)
            .map_err(|message| Diagnostic::new(Code::PathNotFound, message))?,
        &LineIndex::new(&project.module_sources[idx]),
        Span::new(actual_range.start().into(), actual_range.end().into()),
    );
    let target = Target {
        package: package.into(),
        module: module.name.clone(),
        function: law.name,
        arity,
        signature: actual_type.to_string(),
    };
    let entry = InventoryEntry {
        requirement_digest: digest(&target, access, expected_function.syntax()),
        target,
        access,
        location,
        declaration,
    };
    let symbol = mesh_codegen::verification_function_symbol(&module.name, &source_symbol, public);
    Ok(BoundLaw {
        entry,
        module_id,
        symbol,
        expected: law.expected,
        expected_types,
        proof: proof.map(|proof| BoundProof {
            term: proof.term,
            location: at(proof.span),
            signature: actual_type.clone(),
        }),
    })
}

fn parameter_count(function: &FnDef) -> usize {
    function
        .param_list()
        .map_or(0, |params| params.params().count())
}

fn candidate_list(names: &[&str]) -> String {
    if names.is_empty() {
        String::new()
    } else {
        format!("; candidates: {}", names.join(", "))
    }
}

fn portable_path(path: &Path) -> Result<String, String> {
    path.components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| format!("Law path '{}' is not UTF-8", path.display()))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|components| components.join("/"))
}

fn digest(target: &Target, access: Access, function: &SyntaxNode) -> String {
    let identity = serde_json::to_vec(&(target, access)).expect("law identity is serializable");
    let mut hash = Sha256::new();
    hash.update(b"mesh-law-v1\0");
    hash.update(identity);
    hash_syntax(function, &mut hash);
    format!("{:x}", hash.finalize())
}

/// Hash CST structure as well as token spelling. A newline that changes how
/// Mesh parsed the function changes its tree even though trivia is skipped.
fn hash_syntax(node: &SyntaxNode, hash: &mut Sha256) {
    hash.update([0]);
    hash.update((node.kind() as u16).to_le_bytes());
    for child in node.children_with_tokens() {
        match child {
            mesh_parser::SyntaxElement::Node(node) => hash_syntax(&node, hash),
            mesh_parser::SyntaxElement::Token(token) if !token.kind().is_trivia() => {
                hash.update([1]);
                hash.update((token.kind() as u16).to_le_bytes());
                hash.update((token.text().len() as u64).to_le_bytes());
                hash.update(token.text().as_bytes());
            }
            mesh_parser::SyntaxElement::Token(_) => {}
        }
    }
    hash.update([2]);
}

fn prove(
    law: &BoundLaw,
    project: &ProjectData,
    checked: &CheckedProject,
    modules: &[MirModule],
    signatures: &[Vec<mesh_codegen::mir::lower::LoweredSignature>],
    proof_settings: verify_proof::Settings<'_>,
) -> Result<Verdict, verify_solver::EncodingError> {
    let idx = project
        .compilation_order
        .iter()
        .position(|id| *id == law.module_id)
        .ok_or_else(|| format!("No lowered module for `{}`", law.entry.target.module))?;
    let actual = lowered_function(&modules[idx], &law.symbol)?;
    let expected = mesh_codegen::mir::lower::lower_module_to_mir_with_sources(
        &law.expected,
        &law.expected_types,
        "",
        &Default::default(),
        &Default::default(),
        &[],
        |_| Some(MirSourceId(project.module_parses.len())),
    )?;
    let expected_symbol =
        mesh_codegen::verification_function_symbol("", &law.entry.target.function, false);
    let expected = lowered_function(&expected, &expected_symbol)?;
    match &law.proof {
        Some(proof) => {
            verify_proof::require_unambiguous_definition(modules, actual)?;
            let declared = verify_proof::nominal_declarations(project, checked);
            let callees = verify_proof::checked_callees(project, checked, modules, signatures);
            let mut session =
                verify_proof::ModelSession::new(modules, &declared).with_callees(callees);
            verify_proof::check(
                &mut session,
                (
                    actual,
                    &checked.typeck[law.module_id.0 as usize].type_registry,
                ),
                (expected, &law.expected_types.type_registry),
                &proof.signature,
                &proof.term,
                proof_settings,
            )
        }
        None => verify_solver::check(modules, actual, expected),
    }
}

fn expression_location(
    law: &BoundLaw,
    project: &ProjectData,
    location: MirSourceSpan,
) -> Option<Location> {
    let MirSourceSpan { file, span } = location;
    if file.0 == project.module_parses.len() {
        // The standalone law parse substitutes a same-width `fn ` for `law`.
        // Its ranges begin at the declaration, so rebase them into the file.
        let origin = &law.entry.location;
        let lines = LineIndex::new(&law.expected.syntax().text().to_string());
        let mut relative = Location::new(&origin.path, &lines, span);
        if relative.line == 1 {
            relative.column += origin.column - 1;
        }
        relative.line += origin.line - 1;
        relative.start += origin.start;
        relative.end += origin.start;
        return Some(relative);
    }
    project_expression_location(project, location)
}

fn project_expression_location(project: &ProjectData, location: MirSourceSpan) -> Option<Location> {
    let MirSourceSpan { file, span } = location;
    let source = project.module_sources.get(file.0)?;
    let module = project.graph.modules.get(file.0)?;
    let path = portable_path(&module.path).ok()?;
    Some(Location::new(&path, &LineIndex::new(source), span))
}

fn lowered_function<'a>(module: &'a MirModule, symbol: &str) -> Result<&'a MirFunction, String> {
    let mut functions = module
        .functions
        .iter()
        .filter(|function| function.name == symbol);
    let function = functions
        .next()
        .ok_or_else(|| format!("No typed MIR body for `{symbol}`"))?;
    if functions.next().is_some() {
        return Err(format!("Ambiguous typed MIR bodies for `{symbol}`"));
    }
    Ok(function)
}

fn read_baseline(path: &Path) -> Result<Baseline, Diagnostic> {
    let invalid = |reason: &str| {
        Diagnostic::new(
            Code::InvalidArguments,
            format!("Invalid baseline inventory '{}': {reason}", path.display()),
        )
    };
    let source = fs::read_to_string(path).map_err(|error| {
        Diagnostic::new(
            Code::PathNotFound,
            format!(
                "Failed to read baseline inventory '{}': {error}",
                path.display()
            ),
        )
    })?;
    let baseline: Baseline =
        serde_json::from_str(&source).map_err(|error| invalid(&error.to_string()))?;
    if baseline.schema_version != INVENTORY_VERSION || baseline.laws.is_empty() {
        return Err(invalid(
            "expected an inventory of the current version with at least one law",
        ));
    }
    let mut targets = BTreeSet::new();
    for entry in &baseline.laws {
        if !targets.insert(&entry.target) {
            return Err(invalid(&format!("duplicate target `{}`", entry.target)));
        }
        if entry.requirement_digest.len() != 64
            || !entry
                .requirement_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid(&format!("invalid digest for `{}`", entry.target)));
        }
    }
    Ok(baseline)
}

fn compare<'a>(
    baseline: &'a Baseline,
    candidate: impl Iterator<Item = &'a InventoryEntry>,
) -> Vec<PreservationChange<'a>> {
    let candidate: BTreeMap<_, _> = candidate.map(|entry| (&entry.target, entry)).collect();
    baseline
        .laws
        .iter()
        .filter_map(|old| {
            let (change, new_location) = match candidate.get(&old.target) {
                Some(new) if new.requirement_digest == old.requirement_digest => return None,
                Some(new) => ("changed", Some(&new.location)),
                None => ("missing", None),
            };
            Some(PreservationChange {
                target: &old.target,
                change,
                old_location: &old.location,
                new_location,
            })
        })
        .collect()
}
