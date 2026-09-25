use mesh_common::{module_graph::ModuleGraph, span::Span};
use mesh_parser::ast::item::Item;
use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::autonomous::AutonomousClusterConfig;

/// Represents a parsed mesh.toml manifest file.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub package: Package,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
    #[serde(default)]
    pub native: Option<NativePackage>,
    /// Runtime-owned deployment policy parsed from the modern `[cluster]` table.
    #[serde(skip)]
    pub autonomous_cluster: Option<AutonomousClusterConfig>,
}

/// Package metadata from the [package] section of mesh.toml.
#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default, deserialize_with = "deserialize_entrypoint")]
    pub entrypoint: Option<PathBuf>,
}

/// Stable package-owned native ABI contract.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NativePackage {
    pub abi: u32,
    #[serde(default)]
    pub bindings: Vec<PathBuf>,
    #[serde(default)]
    pub libraries: Vec<NativeLibrary>,
}

/// One checksummed static archive for an exact LLVM target triple.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NativeLibrary {
    pub target: String,
    pub path: PathBuf,
    pub sha256: String,
}

impl NativePackage {
    fn validate(&mut self) -> Result<(), String> {
        if self.abi != 1 {
            return Err(format!(
                "unsupported native ABI {}; this compiler supports only ABI 1",
                self.abi
            ));
        }

        for binding in &mut self.bindings {
            let normalized = normalize_native_path(binding, "mpl", "native binding")?;
            if normalized
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.ends_with(".test.mpl") || name.ends_with(".test-support.mpl")
                })
            {
                return Err(format!(
                    "native binding path must not name a test-only Mesh source file, got `{}`",
                    normalized.display()
                ));
            }
            *binding = normalized;
        }

        let mut targets = BTreeSet::new();
        for library in &mut self.libraries {
            if library.target.is_empty()
                || !library
                    .target
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(format!(
                    "native library target must be an exact target triple, got `{}`",
                    library.target
                ));
            }
            if !targets.insert(library.target.clone()) {
                return Err(format!(
                    "native package declares more than one library for target `{}`",
                    library.target
                ));
            }

            let extension = if library.target.contains("windows-msvc") {
                "lib"
            } else {
                "a"
            };
            library.path =
                normalize_native_path(&library.path, extension, "native static archive")?;

            if library.sha256.len() != 64
                || !library
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(format!(
                    "native library `{}` must declare a lowercase 64-character SHA-256",
                    library.path.display()
                ));
            }
        }

        Ok(())
    }
}

pub const DEFAULT_ENTRYPOINT: &str = "main.mpl";

pub const DEFAULT_CLUSTER_REPLICATION_COUNT: u32 = 2;

/// Whether a clustered declaration used the default replication count or an explicit source value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusteredReplicationCountSource {
    Default,
    Explicit,
}

impl fmt::Display for ClusteredReplicationCountSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusteredReplicationCountSource::Default => f.write_str("default"),
            ClusteredReplicationCountSource::Explicit => f.write_str("explicit"),
        }
    }
}

/// Resolved replication count for a clustered declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusteredReplicationCount {
    pub value: u32,
    pub source: ClusteredReplicationCountSource,
}

impl ClusteredReplicationCount {
    pub fn defaulted() -> Self {
        Self {
            value: DEFAULT_CLUSTER_REPLICATION_COUNT,
            source: ClusteredReplicationCountSource::Default,
        }
    }

    pub fn explicit(value: u32) -> Self {
        Self {
            value,
            source: ClusteredReplicationCountSource::Explicit,
        }
    }
}

impl fmt::Display for ClusteredReplicationCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.value, self.source)
    }
}

/// A clustered function to run on the cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusteredExecutionMetadata {
    /// `Module.function`.
    pub runtime_registration_name: String,
    pub executable_symbol: String,
    pub replication_count: ClusteredReplicationCount,
}

/// An `@cluster` decorator on a function that cannot run on the cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusteredDeclarationError {
    /// The decorated function's file, relative to the project root.
    pub file: PathBuf,
    /// The decorator's span.
    pub span: Span,
    pub target: String,
    pub replication_count: ClusteredReplicationCount,
    pub reason: String,
}

impl fmt::Display for ClusteredDeclarationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the source `@cluster` decorator on `{}` with replication count {} is invalid: {}",
            self.target, self.replication_count, self.reason
        )
    }
}

/// A dependency specification -- registry, git-based, or path-based.
///
/// Serde uses `untagged` deserialization, so variants are tried in declaration
/// order. RegistryShorthand MUST be first so a bare string "1.0.0" matches it
/// before Git or Path are attempted.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    /// Bare string shorthand: `foo = "1.0.0"`
    RegistryShorthand(String),
    /// Table form: `foo = { version = "1.0.0" }`
    Registry { version: String },
    /// Git source: `foo = { git = "https://...", ... }`
    Git {
        git: String,
        #[serde(default)]
        rev: Option<String>,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        tag: Option<String>,
    },
    /// Local path: `foo = { path = "../foo" }`
    Path { path: String },
}

impl Dependency {
    /// Returns the version string if this is a registry dependency.
    pub fn registry_version(&self) -> Option<&str> {
        match self {
            Dependency::RegistryShorthand(v) => Some(v),
            Dependency::Registry { version } => Some(version),
            _ => None,
        }
    }
}

fn deserialize_entrypoint<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    raw.map(|value| normalize_entrypoint(&value).map_err(serde::de::Error::custom))
        .transpose()
}

fn normalize_entrypoint(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("`[package].entrypoint` must not be blank".to_string());
    }

    let normalized = root_relative_path(trimmed, "`[package].entrypoint`", "project")?;
    if normalized.as_os_str().is_empty() {
        return Err("`[package].entrypoint` must not be blank".to_string());
    }

    if normalized.extension().and_then(|ext| ext.to_str()) != Some("mpl") {
        return Err(format!(
            "`[package].entrypoint` must end with `.mpl`, got `{trimmed}`"
        ));
    }

    Ok(normalized)
}

/// `path` relative to a `root` ("project" or "package") with its `.`
/// segments dropped, refusing one that is absolute or leaves the root.
fn root_relative_path(path: &str, what: &str, root: &str) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir => {
                return Err(format!(
                    "{what} must stay within the {root} root, got `{path}`"
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{what} must be {root}-root-relative, got `{path}`"));
            }
        }
    }
    Ok(normalized)
}

fn normalize_native_path(path: &Path, extension: &str, label: &str) -> Result<PathBuf, String> {
    let raw = path
        .to_str()
        .ok_or_else(|| format!("{label} path must be valid UTF-8"))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} path must not be blank"));
    }
    if trimmed.starts_with('-') {
        return Err(format!(
            "{label} path must name a package file, not a linker flag: `{trimmed}`"
        ));
    }

    let normalized = root_relative_path(trimmed, &format!("{label} path"), "package")?;

    if normalized.extension().and_then(|value| value.to_str()) != Some(extension) {
        return Err(format!(
            "{label} path must name a `.{extension}` file, got `{trimmed}`"
        ));
    }

    if normalized.starts_with(Path::new("tests")) {
        return Err(format!(
            "{label} path must not be under the top-level `tests/` tree, got `{trimmed}`"
        ));
    }

    Ok(normalized)
}

/// The roots of a project's path and git dependencies, and of theirs, each
/// once. A path dependency is read where it is; a git dependency from its
/// `meshc deps` checkout under `<project>/.mesh/deps/<name>`. Registry
/// packages live under `.mesh/packages` and are found separately.
pub fn source_dependency_roots(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let manifest_path = project_root.join("mesh.toml");
    if !manifest_path.is_file() {
        return Ok(Vec::new());
    }
    let mut roots = Vec::new();
    let mut visited = BTreeSet::new();
    collect_source_dependency_roots(
        project_root,
        project_root,
        &Manifest::from_file(&manifest_path)?,
        &mut visited,
        &mut roots,
    )?;
    roots.sort();
    Ok(roots)
}

fn collect_source_dependency_roots(
    project_root: &Path,
    package_root: &Path,
    manifest: &Manifest,
    visited: &mut BTreeSet<PathBuf>,
    roots: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for (name, dependency) in &manifest.dependencies {
        let root = match dependency {
            Dependency::Path { path } => {
                package_root.join(path).canonicalize().map_err(|error| {
                    format!("Failed to resolve path dependency `{name}` ({path}): {error}")
                })?
            }
            Dependency::Git { .. } => {
                let checkout = project_root.join(".mesh").join("deps").join(name);
                if !checkout.join("mesh.toml").is_file() {
                    return Err(format!(
                        "Git dependency `{name}` is not fetched; run `meshc deps`"
                    ));
                }
                checkout
            }
            Dependency::RegistryShorthand(_) | Dependency::Registry { .. } => continue,
        };
        if !visited.insert(root.clone()) {
            continue;
        }
        let dependency_manifest = Manifest::from_file(&root.join("mesh.toml"))?;
        roots.push(root.clone());
        collect_source_dependency_roots(project_root, &root, &dependency_manifest, visited, roots)?;
    }
    Ok(())
}

pub fn resolve_entrypoint(
    project_root: &Path,
    manifest: Option<&Manifest>,
) -> Result<PathBuf, String> {
    let entrypoint = manifest
        .and_then(|manifest| manifest.package.entrypoint.clone())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ENTRYPOINT));
    let entry_full_path = project_root.join(&entrypoint);

    if !entry_full_path.exists() {
        return Err(format!(
            "Entrypoint '{}' was not found in project '{}'",
            entrypoint.display(),
            project_root.display()
        ));
    }

    if !entry_full_path.is_file() {
        return Err(format!(
            "Entrypoint '{}' in project '{}' is not a file",
            entrypoint.display(),
            project_root.display()
        ));
    }

    Ok(entrypoint)
}

/// The manifest `meshc test` builds a test program with, in a scratch
/// directory: its entrypoint the default one, and its relative path
/// dependencies made absolute from `project_root`.
pub fn rewrite_test_manifest_source(
    manifest_source: &str,
    project_root: &Path,
) -> Result<String, String> {
    let mut manifest_value: toml::Value = toml::from_str(manifest_source)
        .map_err(|error| format!("Failed to parse manifest for rewrite: {}", error))?;
    let manifest_table = manifest_value
        .as_table_mut()
        .expect("a parsed TOML document is a table");
    let package_table = manifest_table
        .get_mut("package")
        .and_then(toml::Value::as_table_mut)
        .ok_or_else(|| {
            "Manifest must contain a [package] table to rewrite entrypoint".to_string()
        })?;
    package_table.insert(
        "entrypoint".to_string(),
        toml::Value::String(DEFAULT_ENTRYPOINT.to_string()),
    );

    let dependencies = manifest_table
        .get_mut("dependencies")
        .and_then(toml::Value::as_table_mut);
    for (name, dependency) in dependencies.into_iter().flatten() {
        let Some(path_value) = dependency
            .as_table_mut()
            .and_then(|table| table.get_mut("path"))
        else {
            continue;
        };
        let Some(path) = path_value.as_str().map(str::to_owned) else {
            continue;
        };
        if Path::new(&path).is_absolute() {
            continue;
        }
        let absolute = project_root.join(&path).canonicalize().map_err(|error| {
            format!(
                "Failed to resolve path dependency `{name}` ({path}) for test manifest: {error}"
            )
        })?;
        *path_value = toml::Value::String(absolute.to_string_lossy().into_owned());
    }

    toml::to_string_pretty(&manifest_value)
        .map_err(|error| format!("Failed to serialize manifest rewrite: {}", error))
}

/// Why a manifest failed to parse, naming its file when it has one.
fn parse_failure(source_path: Option<&Path>, message: impl fmt::Display) -> String {
    match source_path {
        Some(path) => format!("Failed to parse {}: {}", path.display(), message),
        None => format!("Failed to parse manifest: {}", message),
    }
}

/// Parse a mesh.toml manifest from a string.
impl std::str::FromStr for Manifest {
    type Err = String;

    fn from_str(content: &str) -> Result<Manifest, String> {
        Self::parse(content, None)
    }
}

impl Manifest {
    /// Read and parse a mesh.toml manifest from a file path.
    pub fn from_file(path: &Path) -> Result<Manifest, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        Self::parse(content.as_str(), Some(path))
    }

    fn parse(content: &str, source_path: Option<&Path>) -> Result<Manifest, String> {
        let failure = |message: &dyn fmt::Display| parse_failure(source_path, message);
        let mut value: toml::Value = toml::from_str(content).map_err(|error| failure(&error))?;

        let autonomous_cluster = if let Some(cluster_value) = value
            .as_table_mut()
            .and_then(|table| table.remove("cluster"))
        {
            let is_removed_shape = cluster_value.as_table().is_some_and(|cluster| {
                cluster.contains_key("enabled") || cluster.contains_key("declarations")
            });
            if is_removed_shape {
                return Err(failure(&"`[cluster]` manifest sections are no longer supported; move clustered declarations into source with `@cluster` or `@cluster(N)`"));
            }
            let config: AutonomousClusterConfig =
                cluster_value.try_into().map_err(|error| failure(&error))?;
            config
                .validate()
                .map_err(|errors| failure(&errors.join("; ")))?;
            Some(config)
        } else {
            None
        };

        let mut manifest: Manifest = value.try_into().map_err(|error| failure(&error))?;
        if let Some(native) = &mut manifest.native {
            native.validate().map_err(|message| failure(&message))?;
        }
        manifest.autonomous_cluster = autonomous_cluster;
        Ok(manifest)
    }
}

/// Plan the project's `@cluster` functions. Each must be a public function
/// its module defines once, and be declared once.
pub fn plan_cluster_declarations(
    graph: &ModuleGraph,
    parses: &[mesh_parser::Parse],
) -> Result<Vec<ClusteredExecutionMetadata>, Vec<ClusteredDeclarationError>> {
    let mut plan = Vec::new();
    let mut issues = Vec::new();

    for (module_info, parse) in graph.modules.iter().zip(parses) {
        let functions: Vec<_> = parse
            .tree()
            .items()
            .filter_map(|item| match item {
                Item::FnDef(fn_def) => {
                    let name = fn_def.name()?.text()?;
                    Some((fn_def, name))
                }
                _ => None,
            })
            .collect();
        let mut declared = BTreeSet::new();

        for (fn_def, name) in &functions {
            let Some(decl) = fn_def.clustered_decl() else {
                continue;
            };
            let target = format!("{}.{}", module_info.name, name);
            let replication_count = decl.explicit_replica_count().map_or_else(
                ClusteredReplicationCount::defaulted,
                ClusteredReplicationCount::explicit,
            );
            let public_definitions = functions
                .iter()
                .filter(|(other, other_name)| other_name == name && other.visibility().is_some())
                .count();
            let reason = if !declared.insert(name) {
                "target is declared more than once via source clustered declarations"
            } else if fn_def.visibility().is_none() {
                "target resolves to a private function; declare a `pub fn` clustered work entrypoint"
            } else if public_definitions > 1 {
                "target resolves to multiple public functions with the same source name; overloaded clustered work entrypoints are unsupported"
            } else {
                plan.push(ClusteredExecutionMetadata {
                    runtime_registration_name: target,
                    executable_symbol: name.clone(),
                    replication_count,
                });
                continue;
            };
            issues.push(ClusteredDeclarationError {
                file: module_info.path.clone(),
                span: decl.declaration_span(),
                target,
                replication_count,
                reason: reason.to_string(),
            });
        }
    }

    if issues.is_empty() {
        Ok(plan)
    } else {
        Err(issues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::{fs, path::PathBuf};

    /// Plans the `@cluster` functions of one module, `Work`.
    fn plan_work(
        source: &str,
    ) -> Result<Vec<ClusteredExecutionMetadata>, Vec<ClusteredDeclarationError>> {
        let mut graph = ModuleGraph::new();
        graph.add_module("Work".to_string(), "work.mpl".into(), false);
        let parse = mesh_parser::parse(source);
        assert!(parse.errors().is_empty());
        plan_cluster_declarations(&graph, &[parse])
    }

    #[test]
    fn cluster_decorators_plan_public_functions_with_their_counts() {
        let plan = plan_work(
            "@cluster pub fn handle_submit() -> Int do\n  1\nend\n\n@cluster(3) pub fn handle_retry() -> Int do\n  2\nend\n\npub fn local_only() -> Int do\n  3\nend\n",
        )
        .expect("public clustered work should plan");

        assert_eq!(
            plan,
            [
                ClusteredExecutionMetadata {
                    runtime_registration_name: "Work.handle_submit".to_string(),
                    executable_symbol: "handle_submit".to_string(),
                    replication_count: ClusteredReplicationCount::defaulted(),
                },
                ClusteredExecutionMetadata {
                    runtime_registration_name: "Work.handle_retry".to_string(),
                    executable_symbol: "handle_retry".to_string(),
                    replication_count: ClusteredReplicationCount::explicit(3),
                },
            ]
        );
        assert_eq!(plan_work("fn main() do\n  1\nend\n"), Ok(Vec::new()));
    }

    #[test]
    fn cluster_decorators_on_functions_that_cannot_run_are_refused_where_they_are() {
        let source = "@cluster(3) fn hidden() -> Int do\n  1\nend\n\n@cluster pub fn twice() -> Int do\n  1\nend\n\n@cluster pub fn twice() -> Int do\n  2\nend\n";
        let issues = plan_work(source).expect_err("these cannot run on the cluster");

        let reasons: Vec<_> = issues
            .iter()
            .map(|issue| (issue.target.as_str(), issue.reason.as_str()))
            .collect();
        assert_eq!(reasons.len(), 3, "{reasons:?}");
        assert_eq!(reasons[0].0, "Work.hidden");
        assert!(reasons[0].1.contains("private function"));
        assert_eq!(reasons[1].0, "Work.twice");
        assert!(reasons[1]
            .1
            .contains("overloaded clustered work entrypoints"));
        assert!(reasons[2].1.contains("more than once"));

        let hidden = &issues[0];
        assert_eq!(hidden.file, PathBuf::from("work.mpl"));
        assert_eq!(
            &source[hidden.span.start as usize..hidden.span.end as usize],
            "@cluster(3)"
        );
        assert_eq!(
            hidden.replication_count,
            ClusteredReplicationCount::explicit(3)
        );
        assert!(
            hidden.to_string().starts_with(
                "the source `@cluster` decorator on `Work.hidden` with replication count 3 (explicit) is invalid:"
            ),
            "{hidden}"
        );
    }

    #[test]
    fn parse_full_manifest() {
        let toml = r#"
[package]
name = "my-project"
version = "0.1.0"
description = "A test project"
authors = ["Alice", "Bob"]

[dependencies]
json-lib = { git = "https://github.com/example/json-lib.git", tag = "v1.0" }
math-utils = { git = "https://github.com/example/math-utils.git", branch = "main" }
local-dep = { path = "../local-dep" }
"#;
        let manifest = Manifest::from_str(toml).unwrap();
        assert_eq!(manifest.package.name, "my-project");
        assert_eq!(manifest.package.version, "0.1.0");
        assert_eq!(
            manifest.package.description.as_deref(),
            Some("A test project")
        );
        assert_eq!(manifest.package.authors, vec!["Alice", "Bob"]);
        assert_eq!(manifest.dependencies.len(), 3);

        // BTreeMap is sorted by key
        let keys: Vec<&String> = manifest.dependencies.keys().collect();
        assert_eq!(keys, vec!["json-lib", "local-dep", "math-utils"]);

        match &manifest.dependencies["json-lib"] {
            Dependency::Git { git, tag, .. } => {
                assert_eq!(git, "https://github.com/example/json-lib.git");
                assert_eq!(tag.as_deref(), Some("v1.0"));
            }
            _ => panic!("Expected git dependency"),
        }

        match &manifest.dependencies["local-dep"] {
            Dependency::Path { path } => {
                assert_eq!(path, "../local-dep");
            }
            _ => panic!("Expected path dependency"),
        }

        match &manifest.dependencies["math-utils"] {
            Dependency::Git { git, branch, .. } => {
                assert_eq!(git, "https://github.com/example/math-utils.git");
                assert_eq!(branch.as_deref(), Some("main"));
            }
            _ => panic!("Expected git dependency"),
        }
    }

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
[package]
name = "minimal"
version = "0.0.1"
"#;
        let manifest = Manifest::from_str(toml).unwrap();
        assert_eq!(manifest.package.name, "minimal");
        assert_eq!(manifest.package.version, "0.0.1");
        assert!(manifest.package.description.is_none());
        assert!(manifest.package.authors.is_empty());
        assert!(manifest.package.entrypoint.is_none());
        assert!(manifest.dependencies.is_empty());
        assert!(manifest.native.is_none());
    }

    #[test]
    fn parse_native_package_contract() {
        let manifest = Manifest::from_str(
            r#"
[package]
name = "native-math"
version = "0.1.0"

[native]
abi = 1
bindings = ["bindings/math.mpl"]

[[native.libraries]]
target = "aarch64-apple-darwin"
path = "native/aarch64-apple-darwin/libnative_math.a"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#,
        )
        .unwrap();

        let native = manifest.native.expect("native contract");
        assert_eq!(native.abi, 1);
        assert_eq!(native.bindings, vec![PathBuf::from("bindings/math.mpl")]);
        assert_eq!(native.libraries.len(), 1);
        assert_eq!(native.libraries[0].target, "aarch64-apple-darwin");
        assert_eq!(
            native.libraries[0].path,
            PathBuf::from("native/aarch64-apple-darwin/libnative_math.a")
        );
    }

    /// Each library a native package declares is an exact target, once,
    /// with a checksum and an archive path inside the package.
    #[test]
    fn native_libraries_are_refused_for_what_is_wrong_with_them() {
        const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let library = |target: &str, path: &str, sha: &str| {
            format!("[[native.libraries]]\ntarget = \"{target}\"\npath = \"{path}\"\nsha256 = \"{sha}\"\n")
        };
        let manifest = |libraries: &str| {
            Manifest::from_str(&format!(
                "[package]\nname = \"n\"\nversion = \"0.1.0\"\n\n[native]\nabi = 1\n\n{libraries}"
            ))
        };
        for (libraries, expected) in [
            (library("", "lib.a", SHA), "must be an exact target triple"),
            (
                library("x86 64", "lib.a", SHA),
                "must be an exact target triple",
            ),
            (
                library("aarch64-apple-darwin", "a.a", SHA)
                    + &library("aarch64-apple-darwin", "b.a", SHA),
                "more than one library for target `aarch64-apple-darwin`",
            ),
            (
                library("aarch64-apple-darwin", "lib.a", "ABC"),
                "lowercase 64-character SHA-256",
            ),
            (
                library("aarch64-apple-darwin", "lib.a", &SHA.to_uppercase()),
                "lowercase 64-character SHA-256",
            ),
            (
                library("aarch64-apple-darwin", " ", SHA),
                "path must not be blank",
            ),
            (
                library("aarch64-apple-darwin", "/abs/lib.a", SHA),
                "package-root-relative",
            ),
            (
                library("aarch64-apple-darwin", "lib.lib", SHA),
                "must name a `.a` file",
            ),
            (
                library("x86_64-pc-windows-msvc", "lib.a", SHA),
                "must name a `.lib` file",
            ),
        ] {
            let error = manifest(&libraries).unwrap_err();
            assert!(error.contains(expected), "{expected}: {error}");
        }
        let native = manifest(&library("x86_64-pc-windows-msvc", "./native/m.lib", SHA))
            .unwrap()
            .native
            .unwrap();
        assert_eq!(native.libraries[0].path, PathBuf::from("native/m.lib"));
    }

    /// Path dependencies are read where they are and git ones from their
    /// checkout, each once however often they are reached.
    #[test]
    fn source_dependency_roots_follow_path_and_git_dependencies_once() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let package = |dir: &Path, dependencies: &str| {
            fs::create_dir_all(dir).unwrap();
            fs::write(
                dir.join("mesh.toml"),
                format!("[package]\nname = \"p\"\nversion = \"0.1.0\"\n\n[dependencies]\n{dependencies}"),
            )
            .unwrap();
        };
        let app = root.join("app");
        package(
            &app,
            "a = { path = \"../a\" }\nremote = { git = \"https://example.com/r.git\" }\nreg = \"1.0.0\"\n",
        );
        // `a` and `b` depend on each other.
        package(&root.join("a"), "b = { path = \"../b\" }\n");
        package(&root.join("b"), "a = { path = \"../a\" }\n");

        let error = source_dependency_roots(&app).unwrap_err();
        assert!(error.contains("`remote` is not fetched"), "{error}");

        package(&app.join(".mesh/deps/remote"), "");
        assert_eq!(
            source_dependency_roots(&app).unwrap(),
            [
                root.join("a"),
                app.join(".mesh/deps/remote"),
                root.join("b")
            ]
        );
        assert_eq!(
            source_dependency_roots(&root).unwrap(),
            Vec::<PathBuf>::new()
        );

        package(&app, "gone = { path = \"../gone\" }\n");
        let error = source_dependency_roots(&app).unwrap_err();
        assert!(
            error.contains("Failed to resolve path dependency `gone`"),
            "{error}"
        );
    }

    #[test]
    fn an_entrypoint_that_is_a_directory_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("main.mpl")).unwrap();
        let error = resolve_entrypoint(tmp.path(), None).unwrap_err();
        assert!(error.contains("is not a file"), "{error}");
    }

    #[test]
    fn an_entrypoint_that_is_only_dots_is_blank() {
        let error = Manifest::from_str(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nentrypoint = \"./.\"\n",
        )
        .unwrap_err();
        assert!(error.contains("must not be blank"), "{error}");
    }

    #[test]
    fn reject_test_only_native_members() {
        for (binding, expected) in [
            (
                "bindings/math.test.mpl",
                "must not name a test-only Mesh source file",
            ),
            (
                "bindings/math.test-support.mpl",
                "must not name a test-only Mesh source file",
            ),
            (
                "tests/math.mpl",
                "must not be under the top-level `tests/` tree",
            ),
        ] {
            let source = format!(
                "[package]\nname = \"native-math\"\nversion = \"0.1.0\"\n\n[native]\nabi = 1\nbindings = [\"{binding}\"]\n"
            );

            let error = Manifest::from_str(&source).unwrap_err();

            assert!(
                error.contains(expected) && error.contains(binding),
                "unexpected error: {error}"
            );
        }

        let error = Manifest::from_str(
            r#"
[package]
name = "test-native"
version = "0.1.0"

[native]
abi = 1

[[native.libraries]]
target = "aarch64-apple-darwin"
path = "tests/libtest.a"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#,
        )
        .unwrap_err();
        assert!(
            error.contains("native static archive") && error.contains("tests/libtest.a"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn reject_unstable_native_abi_and_unsafe_artifact_paths() {
        for (native, expected) in [
            ("abi = 2\nbindings = []", "unsupported native ABI"),
            (
                "abi = 1\nbindings = [\"../escape.mpl\"]",
                "must stay within",
            ),
        ] {
            let source =
                format!("[package]\nname = \"bad\"\nversion = \"0.1.0\"\n\n[native]\n{native}\n");
            let error = Manifest::from_str(&source).unwrap_err();
            assert!(error.contains(expected), "unexpected error: {error}");
        }

        let error = Manifest::from_str(
            r#"
[package]
name = "flags-are-not-paths"
version = "0.1.0"

[native]
abi = 1

[[native.libraries]]
target = "x86_64-unknown-linux-gnu"
path = "-Wl,--whole-archive"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#,
        )
        .unwrap_err();
        assert!(
            error.contains("static archive"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parse_manifest_entrypoint_override() {
        let toml = r#"
[package]
name = "custom-entry"
version = "0.1.0"
entrypoint = "lib/start.mpl"
"#;

        let manifest = Manifest::from_str(toml).unwrap();

        assert_eq!(
            manifest.package.entrypoint,
            Some(PathBuf::from("lib/start.mpl"))
        );
    }

    #[test]
    fn reject_blank_entrypoint() {
        let toml = r#"
[package]
name = "custom-entry"
version = "0.1.0"
entrypoint = "   "
"#;

        let err = Manifest::from_str(toml).unwrap_err();
        assert!(err.contains("must not be blank"), "unexpected error: {err}");
    }

    #[test]
    fn reject_absolute_entrypoint() {
        let toml = r#"
[package]
name = "custom-entry"
version = "0.1.0"
entrypoint = "/tmp/start.mpl"
"#;

        let err = Manifest::from_str(toml).unwrap_err();
        assert!(
            err.contains("project-root-relative"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reject_escaping_entrypoint() {
        let toml = r#"
[package]
name = "custom-entry"
version = "0.1.0"
entrypoint = "../escape.mpl"
"#;

        let err = Manifest::from_str(toml).unwrap_err();
        assert!(
            err.contains("stay within the project root"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reject_non_mpl_entrypoint() {
        let toml = r#"
[package]
name = "custom-entry"
version = "0.1.0"
entrypoint = "lib/start.txt"
"#;

        let err = Manifest::from_str(toml).unwrap_err();
        assert!(
            err.contains("must end with `.mpl`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_entrypoint_defaults_to_root_main() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("main.mpl"), "fn main() do\n  0\nend\n").unwrap();

        let entrypoint = resolve_entrypoint(temp.path(), None).unwrap();

        assert_eq!(entrypoint, PathBuf::from(DEFAULT_ENTRYPOINT));
    }

    #[test]
    fn resolve_entrypoint_prefers_manifest_override_when_both_entry_files_exist() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("lib")).unwrap();
        fs::write(temp.path().join("main.mpl"), "fn main() do\n  0\nend\n").unwrap();
        fs::write(
            temp.path().join("lib/start.mpl"),
            "fn main() do\n  1\nend\n",
        )
        .unwrap();
        let manifest = Manifest::from_str(
            r#"
[package]
name = "custom-entry"
version = "0.1.0"
entrypoint = "lib/start.mpl"
"#,
        )
        .unwrap();

        let entrypoint = resolve_entrypoint(temp.path(), Some(&manifest)).unwrap();

        assert_eq!(entrypoint, PathBuf::from("lib/start.mpl"));
    }

    #[test]
    fn resolve_entrypoint_rejects_missing_configured_file() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = Manifest::from_str(
            r#"
[package]
name = "custom-entry"
version = "0.1.0"
entrypoint = "lib/start.mpl"
"#,
        )
        .unwrap();

        let err = resolve_entrypoint(temp.path(), Some(&manifest)).unwrap_err();

        assert!(err.contains("lib/start.mpl"), "unexpected error: {err}");
    }

    #[test]
    fn a_test_manifest_builds_the_default_entrypoint_with_absolute_path_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("app");
        fs::create_dir_all(project.join("../shared")).unwrap();
        fs::create_dir_all(&project).unwrap();
        let shared = fs::canonicalize(tmp.path().join("shared")).unwrap();
        let rewritten = rewrite_test_manifest_source(
            r#"
[package]
name = "custom-entry"
version = "0.1.0"
entrypoint = "lib/start.mpl"

[dependencies]
shared = { path = "../shared" }
pinned = { path = "/opt/pinned" }
remote = { git = "https://example.com/remote.git" }
registry = "1.0.0"
"#,
            &project,
        )
        .expect("manifest rewrite should succeed");

        let manifest = Manifest::from_str(&rewritten).expect("rewritten manifest should parse");
        assert_eq!(
            manifest.package.entrypoint,
            Some(PathBuf::from(DEFAULT_ENTRYPOINT))
        );
        let path_of = |name: &str| match &manifest.dependencies[name] {
            Dependency::Path { path } => path.clone(),
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(path_of("shared"), shared.to_string_lossy());
        assert_eq!(path_of("pinned"), "/opt/pinned");
        assert!(matches!(
            manifest.dependencies["remote"],
            Dependency::Git { .. }
        ));

        for (source, expected) in [
            ("[package", "Failed to parse manifest for rewrite"),
            ("name = \"x\"", "must contain a [package] table"),
            (
                "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\ngone = { path = \"missing\" }\n",
                "Failed to resolve path dependency `gone`",
            ),
        ] {
            let error = rewrite_test_manifest_source(source, &project).unwrap_err();
            assert!(error.contains(expected), "{expected}: {error}");
        }
    }

    #[test]
    fn reject_missing_package_section() {
        let toml = r#"
[dependencies]
foo = { path = "./foo" }
"#;
        let result = Manifest::from_str(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Failed to parse manifest"), "Error: {}", err);
    }

    #[test]
    fn reject_missing_name() {
        let toml = r#"
[package]
version = "1.0.0"
"#;
        let result = Manifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn reject_missing_version() {
        let toml = r#"
[package]
name = "no-version"
"#;
        let result = Manifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_git_dep_with_rev() {
        let toml = r#"
[package]
name = "rev-test"
version = "1.0.0"

[dependencies]
pinned = { git = "https://example.com/pinned.git", rev = "abc123" }
"#;
        let manifest = Manifest::from_str(toml).unwrap();
        match &manifest.dependencies["pinned"] {
            Dependency::Git { git, rev, .. } => {
                assert_eq!(git, "https://example.com/pinned.git");
                assert_eq!(rev.as_deref(), Some("abc123"));
            }
            _ => panic!("Expected git dependency"),
        }
    }

    #[test]
    fn parse_git_dep_bare() {
        let toml = r#"
[package]
name = "bare-git"
version = "1.0.0"

[dependencies]
lib = { git = "https://example.com/lib.git" }
"#;
        let manifest = Manifest::from_str(toml).unwrap();
        match &manifest.dependencies["lib"] {
            Dependency::Git {
                git,
                rev,
                branch,
                tag,
            } => {
                assert_eq!(git, "https://example.com/lib.git");
                assert!(rev.is_none());
                assert!(branch.is_none());
                assert!(tag.is_none());
            }
            _ => panic!("Expected git dependency"),
        }
    }

    #[test]
    fn parse_registry_shorthand() {
        let toml = r#"
[package]
name = "uses-registry"
version = "0.1.0"

[dependencies]
foo = "1.0.0"
"#;
        let manifest = Manifest::from_str(toml).unwrap();
        match &manifest.dependencies["foo"] {
            Dependency::RegistryShorthand(v) => {
                assert_eq!(v, "1.0.0");
            }
            other => panic!("Expected RegistryShorthand, got: {:?}", other),
        }
        assert_eq!(
            manifest.dependencies["foo"].registry_version(),
            Some("1.0.0")
        );
    }

    #[test]
    fn parse_registry_table_form() {
        let toml = r#"
[package]
name = "uses-registry-table"
version = "0.1.0"

[dependencies]
foo = { version = "1.0.0" }
"#;
        let manifest = Manifest::from_str(toml).unwrap();
        match &manifest.dependencies["foo"] {
            Dependency::Registry { version } => {
                assert_eq!(version, "1.0.0");
            }
            other => panic!("Expected Registry, got: {:?}", other),
        }
        assert_eq!(
            manifest.dependencies["foo"].registry_version(),
            Some("1.0.0")
        );
    }

    #[test]
    fn parse_mixed_dependency_types() {
        let toml = r#"
[package]
name = "mixed-deps"
version = "1.0.0"

[dependencies]
registry-short = "2.3.4"
registry-table = { version = "1.0.0" }
git-dep = { git = "https://github.com/example/lib.git", tag = "v1.0" }
path-dep = { path = "../path-dep" }
"#;
        let manifest = Manifest::from_str(toml).unwrap();
        assert_eq!(manifest.dependencies.len(), 4);

        match &manifest.dependencies["registry-short"] {
            Dependency::RegistryShorthand(v) => assert_eq!(v, "2.3.4"),
            other => panic!("Expected RegistryShorthand, got: {:?}", other),
        }

        match &manifest.dependencies["registry-table"] {
            Dependency::Registry { version } => assert_eq!(version, "1.0.0"),
            other => panic!("Expected Registry, got: {:?}", other),
        }

        match &manifest.dependencies["git-dep"] {
            Dependency::Git { git, tag, .. } => {
                assert_eq!(git, "https://github.com/example/lib.git");
                assert_eq!(tag.as_deref(), Some("v1.0"));
            }
            other => panic!("Expected Git, got: {:?}", other),
        }

        match &manifest.dependencies["path-dep"] {
            Dependency::Path { path } => assert_eq!(path, "../path-dep"),
            other => panic!("Expected Path, got: {:?}", other),
        }
    }

    #[test]
    fn parse_license_field() {
        let toml_with_license = r#"
[package]
name = "licensed"
version = "1.0.0"
license = "MIT"
"#;
        let manifest = Manifest::from_str(toml_with_license).unwrap();
        assert_eq!(manifest.package.license.as_deref(), Some("MIT"));

        let toml_no_license = r#"
[package]
name = "unlicensed"
version = "1.0.0"
"#;
        let manifest = Manifest::from_str(toml_no_license).unwrap();
        assert!(manifest.package.license.is_none());
    }

    #[test]
    fn manifest_rejects_removed_cluster_section_with_migration_guidance() {
        let cases = [
            r#"
[package]
name = "clustered"
version = "1.0.0"

[cluster]
enabled = true
declarations = [
  { kind = "service_call", target = "Services.Jobs.submit" },
  { kind = "service_cast", target = "Services.Jobs.reset" },
  { kind = "work", target = "Work.handle_submit" },
]
"#,
            r#"
[package]
name = "clustered"
version = "1.0.0"

[cluster]
enabled = false
declarations = [{ kind = "work", target = "Work.handle_submit" }]
"#,
            r#"
[package]
name = "clustered"
version = "1.0.0"

[cluster]
enabled = true
declarations = []
"#,
            r#"
[package]
name = "clustered"
version = "1.0.0"

[cluster]
enabled = true
declarations = [{ kind = "service", target = "Services.Jobs.submit" }]
"#,
        ];

        for toml in cases {
            let err = Manifest::from_str(toml).unwrap_err();
            assert!(
                err.contains("`[cluster]` manifest sections are no longer supported")
                    && err.contains("`@cluster`")
                    && err.contains("`@cluster(N)`"),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn manifest_without_removed_cluster_section_still_parses() {
        let toml = r#"
[package]
name = "clustered"
version = "1.0.0"

[dependencies]
foo = { path = "../foo" }
"#;

        let manifest = Manifest::from_str(toml).unwrap();
        assert_eq!(manifest.package.name, "clustered");
    }
}
