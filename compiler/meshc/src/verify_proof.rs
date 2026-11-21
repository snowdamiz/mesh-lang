//! Check Mesh proof terms against definitions generated from the checked program.

#[path = "verify_proof_model.rs"]
mod model;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use mesh_codegen::mir::{lower::LoweredSignature, MirFunction, MirModule};
use mesh_parser::ast::item::Item;
use mesh_parser::law::ProofTerm;
use mesh_pkg::project::{CheckedProject, ProjectData};
use mesh_typeck::ty::Ty;
use mesh_typeck::TypeRegistry;
use sha2::{Digest, Sha256};

use crate::verify_process::{self, RunError};
use crate::verify_solver::{EncodingError, Verdict};

pub(crate) use model::{CheckedCallee, ModelSession};

const LEAN_VERSION: &str = "4.36.0";
const LEAN_COMMIT: &str = "611c117bfaf7b7ae1cfcf246ce6fba31e8f509e9";
pub(crate) const PRELUDE: &str = "prelude\nimport Init\nset_option autoImplicit false\n\n";

#[derive(Clone, Copy)]
pub(crate) struct Settings<'a> {
    pub(crate) emit: Option<&'a Path>,
    pub(crate) verdict: bool,
}

pub(crate) fn check(
    session: &mut ModelSession<'_>,
    actual: (&MirFunction, &TypeRegistry),
    expected: (&MirFunction, &TypeRegistry),
    signature: &Ty,
    proof: &ProofTerm,
    settings: Settings<'_>,
) -> Result<Verdict, EncodingError> {
    let model = session.translate(actual.0, actual.1, expected.0, expected.1, signature)?;
    let source = render(model, expected.0, proof)?;
    check_artifact(source, &["mesh_law".to_owned()], settings)?;
    Ok(Verdict::Verified)
}

pub(crate) fn nominal_declarations<'a>(
    project: &ProjectData,
    checked: &'a CheckedProject,
) -> Vec<model::NominalDeclaration<'a>> {
    let mut declarations = Vec::new();
    for &id in &project.compilation_order {
        let idx = id.0 as usize;
        let registry = &checked.typeck[idx].type_registry;
        for item in project.module_parses[idx].tree().items() {
            let name = match item {
                Item::StructDef(record) => record.name(),
                Item::SumTypeDef(sum) => sum.name(),
                _ => continue,
            };
            if let Some(name) = name.and_then(|name| name.text()) {
                declarations.push(model::NominalDeclaration { name, registry });
            }
        }
    }
    declarations
}

pub(crate) fn checked_callees<'a>(
    project: &ProjectData,
    checked: &'a CheckedProject,
    modules: &'a [MirModule],
    signatures: &'a [Vec<LoweredSignature>],
) -> BTreeMap<&'a str, CheckedCallee<'a>> {
    let mut symbols = BTreeMap::new();
    for (index, module) in modules.iter().enumerate() {
        for function in &module.functions {
            symbols
                .entry(function.name.as_str())
                .and_modify(|definition| *definition = None)
                .or_insert(Some((index, function)));
        }
        for native in &module.native_functions {
            symbols.insert(native.name.as_str(), None);
            symbols.insert(native.symbol.as_str(), None);
        }
    }

    let mut checked_definitions = BTreeMap::new();
    for (index, (id, entries)) in project.compilation_order.iter().zip(signatures).enumerate() {
        let registry = &checked.typeck[id.0 as usize].type_registry;
        for entry in entries {
            if entry.signature.has_type_vars() {
                continue;
            }
            let Some(Some((owner, function))) = symbols.get(entry.symbol.as_str()) else {
                continue;
            };
            if index == *owner {
                checked_definitions
                    .entry(entry.symbol.as_str())
                    .and_modify(|definition| *definition = None)
                    .or_insert_with(|| {
                        Some(CheckedCallee {
                            function,
                            signature: &entry.signature,
                            registry,
                            lowering: Some(entry),
                        })
                    });
            }
        }
    }
    checked_definitions
        .into_iter()
        .filter_map(|(symbol, definition)| definition.map(|definition| (symbol, definition)))
        .collect()
}

pub(crate) fn require_unambiguous_definition(
    modules: &[MirModule],
    actual: &MirFunction,
) -> Result<(), EncodingError> {
    let mut definitions = modules
        .iter()
        .flat_map(|module| &module.functions)
        .filter(|function| function.name == actual.name);
    let native_alias = modules
        .iter()
        .flat_map(|module| &module.native_functions)
        .any(|function| function.name == actual.name || function.symbol == actual.name);
    if !matches!(definitions.next(), Some(definition) if std::ptr::eq(definition, actual))
        || definitions.next().is_some()
        || native_alias
    {
        return Err(format!(
            "ambiguous kernel model implementation symbol `{}`",
            actual.name
        )
        .into());
    }
    require_unshadowed_runtime(modules)
}

pub(crate) fn require_unshadowed_runtime(modules: &[MirModule]) -> Result<(), EncodingError> {
    // Constructors and matches rely on the native ABI for these symbols.
    let mut symbols = modules.iter().flat_map(|module| {
        module
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .chain(
                module
                    .native_functions
                    .iter()
                    .flat_map(|function| [function.name.as_str(), function.symbol.as_str()]),
            )
    });
    if let Some(shadowed) = symbols.find(|symbol| model::TRUSTED_BUILTINS.contains(symbol)) {
        return Err(
            format!("kernel model cannot trust a shadowed runtime symbol `{shadowed}`").into(),
        );
    }
    Ok(())
}

pub(crate) fn check_artifact(
    mut source: String,
    theorem_symbols: &[String],
    settings: Settings<'_>,
) -> Result<(), String> {
    if theorem_symbols.is_empty() {
        return Err("proof artifact has no theorems".into());
    }
    for symbol in theorem_symbols {
        writeln!(source, "\n#print axioms {symbol}").unwrap();
    }
    let export = check_kernel(&source, theorem_symbols, settings.verdict)?;
    if let Some(directory) = settings.emit {
        save_proof(directory, &source, "lean")
            .and_then(|()| {
                if let Some(export) = &export {
                    save_proof(directory, export, "ndjson")
                } else {
                    Ok(())
                }
            })
            .map_err(|error| {
                format!(
                    "could not export checked proof to {}: {error}",
                    directory.display()
                )
            })?;
    }
    Ok(())
}

fn save_proof(directory: &Path, source: &str, extension: &str) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let hash = Sha256::digest(source.as_bytes());
    let path = directory.join(format!("Proof_{hash:x}.{extension}"));
    let mut artifact = tempfile::NamedTempFile::new_in(directory)?;
    artifact.write_all(source.as_bytes())?;
    match artifact.persist_noclobber(&path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.is_file()
                && metadata.len() == source.len() as u64
                && std::fs::read(&path)? == source.as_bytes()
            {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("proof artifact differs at {}", path.display()),
                ))
            }
        }
        Err(error) => Err(error.error),
    }
}

fn render(
    model: model::KernelModel,
    expected: &MirFunction,
    proof: &ProofTerm,
) -> Result<String, String> {
    if model.parameter_types.len() != expected.params.len() {
        return Err("Proof model and law disagree on the parameter count".into());
    }
    let mut source = PRELUDE.to_owned();
    source.push_str(&model.definitions);
    source.push_str("\n\ntheorem mesh_law");
    let mut scope = BTreeMap::new();
    let mut arguments = String::new();
    for (index, ((name, _), ty)) in expected
        .params
        .iter()
        .zip(model.parameter_types)
        .enumerate()
    {
        let binder = format!("arg{index}");
        write!(source, " ({binder} : {ty})").unwrap();
        write!(arguments, " {binder}").unwrap();
        scope.insert(name.as_str(), binder);
    }
    writeln!(
        source,
        " : mesh_actual{arguments} = mesh_expected{arguments} := by"
    )
    .unwrap();
    ProofRenderer {
        source: &mut source,
        next_binder: 0,
    }
    .term(proof, &scope, 2)?;
    Ok(source)
}

struct ProofRenderer<'a> {
    source: &'a mut String,
    next_binder: usize,
}

impl ProofRenderer<'_> {
    fn line(&mut self, indent: usize, line: impl std::fmt::Display) {
        writeln!(self.source, "{:indent$}{line}", "").unwrap();
    }

    fn fresh_binder(&mut self) -> String {
        let binder = format!("p{}", self.next_binder);
        self.next_binder += 1;
        binder
    }

    fn term<'a>(
        &mut self,
        term: &'a ProofTerm,
        scope: &BTreeMap<&'a str, String>,
        indent: usize,
    ) -> Result<(), String> {
        let binding = |name: &str| {
            scope
                .get(name)
                .ok_or_else(|| format!("Unknown proof binding `{name}`"))
        };
        match term {
            ProofTerm::Refl => self.line(indent, "rfl"),
            ProofTerm::Exact(name) => self.line(indent, format_args!("exact {}", binding(name)?)),
            ProofTerm::Congruence { head, hypothesis } => self.line(
                indent,
                format_args!(
                    "exact congrArg (List.cons {}) {}",
                    binding(head)?,
                    binding(hypothesis)?
                ),
            ),
            ProofTerm::Induction {
                variable,
                nil,
                head,
                tail,
                hypothesis,
                cons,
            } => {
                if head == tail || head == hypothesis || tail == hypothesis {
                    return Err("List induction bindings must be distinct".into());
                }
                self.line(
                    indent,
                    format_args!("induction {} with", binding(variable)?),
                );
                let mut branch = scope.clone();
                branch.remove(variable.as_str());
                self.line(indent, "| nil =>");
                self.term(nil, &branch, indent + 2)?;
                let head_binder = self.fresh_binder();
                let tail_binder = self.fresh_binder();
                let hypothesis_binder = self.fresh_binder();
                self.line(
                    indent,
                    format_args!("| cons {head_binder} {tail_binder} {hypothesis_binder} =>"),
                );
                branch.insert(head, head_binder);
                branch.insert(tail, tail_binder);
                branch.insert(hypothesis, hypothesis_binder);
                self.term(cons, &branch, indent + 2)?;
            }
            ProofTerm::Cases {
                variable,
                on_false,
                on_true,
            } => {
                self.line(indent, format_args!("cases {} with", binding(variable)?));
                let mut branch = scope.clone();
                branch.remove(variable.as_str());
                self.line(indent, "| false =>");
                self.term(on_false, &branch, indent + 2)?;
                self.line(indent, "| true =>");
                self.term(on_true, &branch, indent + 2)?;
            }
        }
        Ok(())
    }
}

fn check_kernel(
    source: &str,
    theorem_symbols: &[String],
    verdict: bool,
) -> Result<Option<String>, String> {
    let deadline =
        Instant::now() + Duration::from_millis(u64::from(verify_process::timeout_ms(30_000)?));
    let executable = std::env::var_os("MESH_VERIFY_LEAN").unwrap_or_else(|| "lean".into());
    let path = Path::new(&executable);
    let executable = if path.components().count() > 1 {
        std::path::absolute(path)
            .map_err(|error| format!("Lean executable path: {error}"))?
            .into_os_string()
    } else {
        executable
    };
    let directory =
        tempfile::tempdir().map_err(|error| format!("Lean proof directory: {error}"))?;
    let make_command = |executable: &std::ffi::OsStr| {
        let mut command = Command::new(&executable);
        command.current_dir(directory.path());
        for var in ["LEAN_PATH", "LEAN_SRC_PATH", "LEAN_SYSROOT"] {
            command.env_remove(var);
        }
        command
    };
    let run = |command: &mut Command, limit| {
        let tool = command.get_program().to_string_lossy().into_owned();
        verify_process::run_with_limit(command, "", deadline, limit).map_err(|error| match error {
            RunError::Start(error) => format!("could not start Lean tool {tool}: {error}; set MESH_VERIFY_LEAN to the trusted Lean {LEAN_VERSION} executable"),
            RunError::Io(error) => format!("Lean I/O: {error}"),
            RunError::Timeout => "proof time budget exhausted".to_owned(),
        })
    };
    let run_lean = |arguments: &[&str]| {
        run(
            make_command(&executable).args(arguments),
            verify_process::DIAGNOSTIC_LIMIT,
        )
    };
    let version = run_lean(&["--version"])?;
    if !version.status.success()
        || !version.stderr.trim().is_empty()
        || version
            .stdout
            .strip_prefix("Lean (version ")
            .and_then(|line| line.split_once(','))
            .map(|(version, _)| version)
            != Some(LEAN_VERSION)
    {
        return Err(format!(
            "mesh verify requires Lean {LEAN_VERSION}; found {}",
            version.stdout.trim()
        ));
    }

    std::fs::write(directory.path().join("Proof.lean"), source)
        .map_err(|error| format!("could not write generated proof: {error}"))?;
    let output = if verdict {
        run_lean(&["--trust=0", "-o", "Proof.olean", "Proof.lean"])?
    } else {
        run_lean(&["--trust=0", "Proof.lean"])?
    };
    if !output.status.success()
        || !output.stderr.trim().is_empty()
        || !check_axiom_report(&output.stdout, theorem_symbols)
    {
        let detail = format!("{} {}", output.stdout.trim(), output.stderr.trim());
        let detail: String = detail.trim().chars().take(500).collect();
        return Err(format!(
            "proof did not check with the permitted axioms: {detail}"
        ));
    }
    if !verdict {
        return Ok(None);
    }

    let prefix = checked_output(run_lean(&["--print-prefix"])?, "Lean toolchain lookup")?;
    let prefix = PathBuf::from(prefix.trim());
    if !prefix.is_absolute() {
        return Err("Lean did not report an absolute toolchain prefix".into());
    }
    let bin = prefix.join("bin");
    let tool = |name: &str| {
        let path = bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let mut command = make_command(path.as_os_str());
        command.env("PATH", &bin).env("LEAN_PATH", directory.path());
        command
    };
    let export = checked_output(
        run(
            tool("leanexport")
                .args(["Proof", "--"])
                .args(theorem_symbols),
            32 * 1024 * 1024,
        )?,
        "Lean proof export",
    )?;
    check_export(&export, theorem_symbols)?;
    std::fs::write(directory.path().join("Proof.ndjson"), &export)
        .map_err(|error| format!("could not write proof export: {error}"))?;
    let report = checked_output(
        run(
            tool("con-leche").args(["--verified", "--jobs=1", "Proof.ndjson"]),
            verify_process::DIAGNOSTIC_LIMIT,
        )?,
        "independent ConLeche check",
    )?;
    let checked = report
        .trim()
        .strip_prefix("con-leche: accepted ")
        .and_then(|count| count.strip_suffix(" declarations (--verified)"))
        .and_then(|count| count.parse::<usize>().ok());
    if !checked.is_some_and(|count| count >= theorem_symbols.len()) {
        return Err(format!(
            "independent checker did not confirm the proof export: {}",
            report.trim()
        ));
    }
    Ok(Some(export))
}

fn check_axiom_report(report: &str, theorems: &[String]) -> bool {
    let mut lines = report.trim().lines();
    for symbol in theorems {
        let Some(report) = lines.next() else {
            return false;
        };
        let prefix = format!("'{symbol}' ");
        if !matches!(
            report.strip_prefix(&prefix),
            Some("does not depend on any axioms" | "depends on axioms: [propext]")
        ) {
            return false;
        }
    }
    lines.next().is_none()
}

fn check_export(export: &str, theorems: &[String]) -> Result<(), String> {
    let mut lines = export.lines();
    let parse = |line| {
        serde_json::from_str::<serde_json::Value>(line)
            .map_err(|error| format!("Invalid Lean proof export: {error}"))
    };
    let metadata = parse(lines.next().ok_or("Lean exported no proof declarations")?)?;
    if metadata["meta"]["format"]["version"] != "3.1.0"
        || metadata["meta"]["lean"]["version"] != LEAN_VERSION
        || metadata["meta"]["lean"]["githash"] != LEAN_COMMIT
    {
        return Err("Lean proof export does not match the pinned format and toolchain".into());
    }
    let names: BTreeSet<_> = theorems.iter().map(String::as_str).collect();
    let mut indices = BTreeMap::new();
    let mut included = BTreeSet::new();
    for line in lines {
        let row = parse(line)?;
        if let (Some(index), Some(name), Some(0)) = (
            row["in"].as_u64(),
            row["str"]["str"].as_str(),
            row["str"]["pre"].as_u64(),
        ) {
            if let Some(required) = names.get(name) {
                if indices.insert(index, *required).is_some() {
                    return Err(format!(
                        "Duplicate theorem name index in proof export: {name}"
                    ));
                }
            }
        }
        if let Some(name) = row["thm"]["name"]
            .as_u64()
            .and_then(|index| indices.get(&index))
        {
            if !included.insert(*name) {
                return Err(format!("Duplicate theorem in proof export: {name}"));
            }
        }
    }
    if let Some(missing) = names.difference(&included).next() {
        return Err(format!("Proof export is missing theorem `{missing}`"));
    }
    Ok(())
}

fn checked_output(output: verify_process::Output, step: &str) -> Result<String, String> {
    if output.status.success() && output.stderr.trim().is_empty() {
        Ok(output.stdout)
    } else {
        let detail = format!("{} {}", output.stdout.trim(), output.stderr.trim());
        let detail: String = detail.trim().chars().take(500).collect();
        Err(format!("{step} failed: {detail}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_exported_theorems_by_name() {
        let required = ["mesh_law".to_owned()];
        let meta = format!(
            r#"{{"meta":{{"format":{{"version":"3.1.0"}},"lean":{{"version":"{LEAN_VERSION}","githash":"{LEAN_COMMIT}"}}}}}}"#
        );
        let names = r#"{"in":1,"str":{"pre":0,"str":"mesh_law"}}"#;
        let declaration = r#"{"thm":{"name":1,"type":20,"value":21}}"#;
        assert!(check_export(&format!("{meta}\n{names}\n{declaration}"), &required).is_ok());
        assert!(check_export(&format!("{meta}\n{names}"), &required)
            .unwrap_err()
            .contains("missing theorem"));
        assert!(check_export(
            &format!("{meta}\n{names}\n{declaration}\n{declaration}"),
            &required
        )
        .unwrap_err()
        .contains("Duplicate theorem"));
        assert!(check_export(&format!("{meta}\n{names}\n{{"), &required)
            .unwrap_err()
            .contains("Invalid Lean proof export"));
    }

    #[test]
    fn permits_only_propositional_extensionality() {
        let theorems = ["first".to_owned(), "second".to_owned()];
        assert!(check_axiom_report(
            "'first' depends on axioms: [propext]\n'second' does not depend on any axioms\n",
            &theorems,
        ));
        assert!(!check_axiom_report(
            "'first' does not depend on any axioms\n'second' depends on axioms: [propext, sorryAx]\n",
            &theorems,
        ));
        assert!(!check_axiom_report(
            "'first' does not depend on any axioms\n",
            &theorems,
        ));
    }
}
