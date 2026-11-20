use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;

use mesh_codegen::mir::{MirFunction, MirModule, MirSourceId, MirSourceSpan};
use mesh_common::module_graph::ModuleId;
use mesh_common::span::{LineIndex, Span};
use mesh_parser::ast::item::FnDef;
use mesh_parser::law::{LawDeclaration, LawFile, LawImport};
use mesh_parser::{AstNode, SyntaxNode};
use mesh_pkg::project::{laws, CheckedProject, ProjectData};
use mesh_typeck::diagnostics::DiagnosticOptions;
use mesh_typeck::ty::Ty;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::verify_solver::{self, Verdict};

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
}

impl Access {
    fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

struct BoundLaw {
    entry: InventoryEntry,
    module_id: ModuleId,
    symbol: String,
    expected: mesh_parser::Parse,
    expected_types: mesh_typeck::TypeckResult,
}

#[derive(Serialize)]
struct LawReport<'a> {
    #[serde(flatten)]
    entry: &'a InventoryEntry,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    verdict: Option<Verdict>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unsupported_at: Option<Location>,
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
) -> Result<(), String> {
    let project_dir = project_dir.canonicalize().map_err(|error| {
        format!(
            "Failed to resolve project '{}': {error}",
            project_dir.display()
        )
    })?;
    let opts = DiagnosticOptions {
        color: !json,
        json,
        ..Default::default()
    };
    let crate::CheckedBuild {
        manifest,
        project,
        mut checked,
        ..
    } = crate::load_checked_project(&project_dir, false, false, &opts)?;
    let files = laws::discover(&project_dir, manifest.as_ref())?;
    if files.paths.is_empty() && preserve.is_none() {
        return Err("No laws found under the effective law roots".into());
    }

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
    for path in &files.paths {
        let display = portable_path(path)?;
        let source = mesh_pkg::project::read_file(&project_dir.join(path))?;
        for law in bind_file(package, &display, &source, &project, &checked)? {
            let target = &law.entry.target;
            let key = (target.module.clone(), target.function.clone(), target.arity);
            if let Some(previous) = bound.get(&key) {
                return Err(format!(
                    "Duplicate law for `{target}` at {} and {}",
                    previous.entry.location, law.entry.location
                ));
            }
            bound.insert(key, law);
        }
    }

    let baseline = preserve.map(read_baseline).transpose()?;
    let changes = baseline.as_ref().map(|base| compare(base, bound.values()));
    let program = if list || bound.is_empty() {
        None
    } else {
        let source_ids = project
            .module_parses
            .iter()
            .enumerate()
            .map(|(index, parse)| (std::ptr::from_ref(parse), MirSourceId(index)))
            .collect::<std::collections::HashMap<_, _>>();
        Some(crate::lower_checked_modules(
            &project,
            &mut checked,
            |parse| source_ids.get(&std::ptr::from_ref(parse)).copied(),
        ))
    };
    let mut all_verified = true;
    let reports = bound
        .values()
        .map(|law| {
            let mut unsupported_at = None;
            let verdict = program.as_ref().map(|program| {
                let result = match program {
                    Ok(modules) => prove(law, &project, modules),
                    Err(error) => Err(error.clone().into()),
                };
                let verdict = result.unwrap_or_else(|error| {
                    unsupported_at = error
                        .location
                        .and_then(|location| expression_location(law, &project, location));
                    Verdict::Unverified {
                        reason: error.reason,
                    }
                });
                all_verified &= matches!(verdict, Verdict::Verified);
                verdict
            });
            LawReport {
                entry: &law.entry,
                verdict,
                unsupported_at,
            }
        })
        .collect::<Vec<_>>();

    let roots = files
        .roots
        .iter()
        .map(|path| portable_path(path))
        .collect::<Result<Vec<_>, _>>()?;
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
        for law in reports {
            print_law(&law)?;
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

    if changes.is_some_and(|changes| !changes.is_empty()) {
        return Err("Law preservation failed".into());
    }
    if !all_verified {
        return Err("Some laws did not verify".into());
    }
    Ok(())
}

fn print_law(law: &LawReport<'_>) -> Result<(), String> {
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
            let inputs = serde_json::to_string(inputs).map_err(|error| error.to_string())?;
            println!("  falsified: {inputs}, expected {expected}, actual {actual}");
        }
        Some(Verdict::Unverified { reason }) => match &law.unsupported_at {
            Some(location) => println!("  unverified at {location}: {reason}"),
            None => println!("  unverified: {reason}"),
        },
        None => {}
    }
    Ok(())
}

fn bind_file(
    package: &str,
    path: &str,
    source: &str,
    project: &ProjectData,
    checked: &CheckedProject,
) -> Result<Vec<BoundLaw>, String> {
    let lines = LineIndex::new(source);
    let at = |span| Location::new(path, &lines, span);
    let LawFile { imports, laws } = mesh_parser::law::parse(source)
        .map_err(|error| format!("{}: {}", at(error.span), error.message))?;
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
        let location = at(law.span);
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
                return Err(format!(
                    "{location}: Ambiguous target import for `{}`: {candidates}",
                    law.name
                ));
            }
            None => {
                return Err(format!(
                    "{location}: Missing target import for `{}`",
                    law.name
                ))
            }
        };
        used.insert(import.name.as_str());
        bound.push(bind_law(
            package, project, checked, import, law, location, &at,
        )?);
    }
    if let Some(unused) = imports
        .iter()
        .find(|import| !used.contains(import.name.as_str()))
    {
        return Err(format!(
            "{}: Target import `{}.{}` has no law",
            at(unused.span),
            unused.module,
            unused.name
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
    location: Location,
    at: &impl Fn(Span) -> Location,
) -> Result<BoundLaw, String> {
    let import_location = at(import.span);
    let target_name = format!("{}.{}", import.module, import.name);
    let module_id = project.graph.resolve(&import.module).ok_or_else(|| {
        let candidates = project
            .graph
            .modules
            .iter()
            .filter(|module| module.name.rsplit('.').next() == import.module.rsplit('.').next())
            .map(|module| module.name.as_str())
            .collect::<Vec<_>>();
        format!(
            "{import_location}: Unknown target module `{}`{}",
            import.module,
            candidate_list(&candidates)
        )
    })?;
    let module = project.graph.get(module_id);
    if module.path.is_absolute() {
        return Err(format!(
            "{import_location}: Law target `{target_name}` must belong to the selected package `{package}`; found '{}'",
            module.path.display()
        ));
    }

    let expected_function = law
        .expected
        .tree()
        .fn_defs()
        .next()
        .ok_or_else(|| format!("{location}: Expected a law function"))?;
    let expected_params = expected_function
        .param_list()
        .ok_or_else(|| format!("{location}: Expected an explicit law parameter list"))?;
    let mut params = BTreeSet::new();
    for param in expected_params.params() {
        let Some(name) = param.name() else {
            return Err(format!("{location}: Law parameters must be named"));
        };
        if param.type_annotation().is_none() || param.ownership_modifier().is_some() {
            return Err(format!(
                "{location}: Law parameter `{name}` needs a type annotation"
            ));
        }
        if !params.insert(name.text().to_owned()) {
            return Err(format!("{location}: Duplicate law parameter `{name}`"));
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
            format!(
                "{location}: No checked function `{target_name}/{arity}`{}",
                candidate_list(&candidates)
            )
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
        (false, false) => return Err(format!(
            "{import_location}: `{target_name}/{arity}` is private; use `from {} import private {}` in this law file",
            import.module, import.name,
        )),
        (true, true) => return Err(format!(
            "{import_location}: `{target_name}/{arity}` is public; remove `private` from the import",
        )),
    };

    let actual_range = actual.syntax().text_range();
    let actual_type = checked.typeck[idx]
        .types
        .get(&actual_range)
        .ok_or_else(|| {
            format!("{location}: Missing checked signature for `{target_name}/{arity}`")
        })?;
    let expected_types = mesh_typeck::check(&law.expected);
    if let Some(error) = expected_types.errors.first() {
        let location = error.span().map_or(location.clone(), |range| {
            at(Span::new(
                law.span.start + u32::from(range.start()),
                law.span.start + u32::from(range.end()),
            ))
        });
        return Err(format!(
            "{location}: Invalid law body for `{target_name}`: {error}"
        ));
    }
    let expected_type = expected_types
        .types
        .get(&expected_function.syntax().text_range())
        .ok_or_else(|| format!("{location}: Missing checked law signature for `{target_name}`"))?;
    if actual_type != expected_type
        || !matches!(actual_type, Ty::Fun(..))
        || crate::ty_contains_var(actual_type)
    {
        return Err(format!(
            "{location}: Law signature {expected_type} does not match the concrete target `{target_name}` signature {actual_type}"
        ));
    }

    let declaration = Location::new(
        &portable_path(&module.path)?,
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
        requirement_digest: digest(&target, access, expected_function.syntax())?,
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

fn digest(target: &Target, access: Access, function: &SyntaxNode) -> Result<String, String> {
    let identity = serde_json::to_vec(&(target, access)).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    hash.update(b"mesh-law-v1\0");
    hash.update(identity);
    hash_syntax(function, &mut hash);
    Ok(format!("{:x}", hash.finalize()))
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
    modules: &[MirModule],
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
    verify_solver::check(modules, actual, expected)
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

fn read_baseline(path: &Path) -> Result<Baseline, String> {
    let invalid =
        |reason: &str| format!("Invalid baseline inventory '{}': {reason}", path.display());
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed to read baseline inventory '{}': {error}",
            path.display()
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
    candidate: impl Iterator<Item = &'a BoundLaw>,
) -> Vec<PreservationChange<'a>> {
    let candidate: BTreeMap<_, _> = candidate
        .map(|law| (&law.entry.target, &law.entry))
        .collect();
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
