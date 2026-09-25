//! A Mesh project's modules: its files, its dependencies' and installed
//! packages' files, its native bindings and its `module` blocks, parsed and
//! ordered so each comes after the modules it uses. `meshc` builds a project
//! and the language server analyses one through this one path, so an editor
//! sees the program the compiler builds.

use std::path::{Component, Path, PathBuf};

use mesh_common::module_graph::{self, CycleError, ModuleGraph, ModuleId};
use mesh_parser::ast::item::{Item, SourceFile};
use mesh_parser::syntax_kind::SyntaxKind;

use crate::manifest::DEFAULT_ENTRYPOINT;
use crate::native::ResolvedNativeBinding;

/// Convert a snake_case string to PascalCase.
///
/// Splits on `_`, capitalizes the first character of each non-empty part,
/// and joins them together.
///
/// # Examples
///
/// - `"vector"` -> `"Vector"`
/// - `"linear_algebra"` -> `"LinearAlgebra"`
/// - `"my_cool_lib"` -> `"MyCoolLib"`
pub fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter_map(|part| {
            let mut chars = part.chars();
            let first = chars.next()?;
            Some(first.to_uppercase().chain(chars).collect::<String>())
        })
        .collect()
}

/// Convert a relative file path to a PascalCase module name.
///
/// Returns `None` for `main.mpl` in the project root (the entry point), and
/// for a path with a name that is not UTF-8.
///
/// # Convention
///
/// - `math/vector.mpl` -> `Some("Math.Vector")`
/// - `utils.mpl` -> `Some("Utils")`
/// - `math/linear_algebra.mpl` -> `Some("Math.LinearAlgebra")`
/// - `a/b/c/d.mpl` -> `Some("A.B.C.D")`
/// - `main.mpl` -> `None`
pub fn path_to_module_name(relative_path: &Path) -> Option<String> {
    let stem = relative_path.file_stem()?.to_str()?;
    // A directory whose name is not UTF-8 names no module.
    let directories = relative_path
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_str()),
            _ => None,
        })
        .collect::<Option<Vec<&str>>>()?;
    if stem == "main" && directories.is_empty() {
        return None;
    }
    let parts: Vec<String> = directories
        .into_iter()
        .chain([stem])
        .map(to_pascal_case)
        .collect();
    Some(parts.join("."))
}

/// Recursively discover all `.mpl` files in a project directory.
///
/// Returns paths relative to `project_root`, sorted alphabetically for
/// determinism. Hidden directories (names starting with `.`) and the top-level
/// `tests/` tree are skipped.
pub fn discover_mesh_files(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    discover_mesh_files_with_test_helpers(project_root, false)
}

fn discover_mesh_files_with_test_helpers(
    project_root: &Path,
    include_test_helpers: bool,
) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    discover_recursive(project_root, project_root, &mut files, include_test_helpers).map_err(
        |e| {
            format!(
                "Failed to walk directory '{}': {}",
                project_root.display(),
                e
            )
        },
    )?;
    files.sort();
    Ok(files)
}

/// Internal recursive walker that collects `.mpl` files as relative paths.
fn discover_recursive(
    root: &Path,
    dir: &Path,
    files: &mut Vec<PathBuf>,
    include_test_helpers: bool,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let entry_path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        // Skip hidden directories and files
        if name_str.starts_with('.') {
            continue;
        }

        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Mesh project path '{}' must not be a symbolic link",
                    entry_path.display()
                ),
            ));
        }

        if file_type.is_dir() {
            if !include_test_helpers && dir == root && name_str == "tests" {
                continue;
            }
            discover_recursive(root, &entry_path, files, include_test_helpers)?;
        } else if file_type.is_file()
            && entry_path.extension().and_then(|e| e.to_str()) == Some("mpl")
        {
            // Test DSL files and support fragments belong only to `meshc test`.
            if name_str.ends_with(".test.mpl") || name_str.ends_with(".test-support.mpl") {
                continue;
            }
            // Store path relative to root
            let relative = entry_path
                .strip_prefix(root)
                .unwrap_or(&entry_path)
                .to_path_buf();
            files.push(relative);
        }
    }
    Ok(())
}

fn discover_installed_package_roots(packages_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut package_roots = Vec::new();
    discover_installed_package_roots_recursive(packages_dir, &mut package_roots).map_err(|e| {
        format!(
            "Failed to walk installed packages under '{}': {}",
            packages_dir.display(),
            e
        )
    })?;
    package_roots.sort();
    Ok(package_roots)
}

fn discover_installed_package_roots_recursive(
    dir: &Path,
    package_roots: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    let mut child_dirs = Vec::new();
    let mut has_manifest = false;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if name.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            child_dirs.push(path);
        } else if name == "mesh.toml" {
            has_manifest = true;
        }
    }

    if has_manifest {
        package_roots.push(dir.to_path_buf());
        return Ok(());
    }

    child_dirs.sort();
    for child_dir in child_dirs {
        discover_installed_package_roots_recursive(&child_dir, package_roots)?;
    }

    Ok(())
}

/// Whether module `from` depends on `to`, directly or through others.
fn module_reaches(graph: &ModuleGraph, from: ModuleId, to: ModuleId) -> bool {
    let mut stack = vec![from];
    let mut seen = std::collections::HashSet::new();
    while let Some(module) = stack.pop() {
        if module == to {
            return true;
        }
        if seen.insert(module) {
            stack.extend(graph.get(module).dependencies.iter().copied());
        }
    }
    false
}

/// Extract import module paths from a parsed source file.
///
/// Walks the top-level items and collects module paths from both
/// `import Foo.Bar` and `from Foo.Bar import { ... }` declarations.
/// Returns PascalCase dot-separated module names.
pub fn extract_imports(source_file: &SourceFile) -> Vec<String> {
    let mut imports = Vec::new();
    for item in source_file.items() {
        match item {
            Item::ImportDecl(decl) => {
                if let Some(path) = decl.module_path() {
                    let segments = path.segments();
                    if !segments.is_empty() {
                        imports.push(segments.join("."));
                    }
                }
            }
            Item::FromImportDecl(decl) => {
                if let Some(path) = decl.module_path() {
                    let segments = path.segments();
                    if !segments.is_empty() {
                        imports.push(segments.join("."));
                    }
                }
            }
            _ => {}
        }
    }
    imports
}

/// Complete project data after discovery, parsing, and graph construction.
///
/// All Vecs are indexed by ModuleId.0 -- the i-th entry corresponds to
/// the module with ModuleId(i).
pub struct ProjectData {
    /// The module dependency graph.
    pub graph: ModuleGraph,
    /// Modules in compilation order (dependencies before dependents).
    pub compilation_order: Vec<ModuleId>,
    /// Source code for each module (indexed by ModuleId.0).
    pub module_sources: Vec<String>,
    /// Parsed AST for each module (indexed by ModuleId.0).
    pub module_parses: Vec<mesh_parser::Parse>,
}

/// Read a project file from disk.
pub fn read_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("Failed to read '{}': {}", path.display(), e))
}

/// Build a complete project: discover files, parse all, build dependency graph.
///
/// Pipeline:
/// 1. Discover all `.mpl` files in the project (with the helpers under the
///    top-level `tests/` when `include_test_helpers`, as `meshc test` builds).
/// 2. Register each file as a module, read it with `read_source` and parse it,
///    and the same for dependencies, installed packages and native bindings.
/// 3. Extract imports from parsed ASTs to build dependency edges.
/// 4. Run topological sort to get compilation order.
///
/// `read_source` is [`read_file`] for a build; an editor passes one that
/// returns the text of open documents instead.
///
/// Unknown imports (stdlib, typos) are silently skipped.
/// Self-imports produce a specific error.
/// Circular dependencies produce an error with the cycle path.
pub fn build_project(
    project_root: &Path,
    entry_relative_path: &Path,
    native_bindings: &[ResolvedNativeBinding],
    include_test_helpers: bool,
    read_source: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<ProjectData, String> {
    if entry_relative_path.as_os_str().is_empty()
        || entry_relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "Resolved entrypoint '{}' must stay within project '{}'",
            entry_relative_path.display(),
            project_root.display()
        ));
    }

    // Phase 1: Discover files, register modules, read and parse source.
    let files = discover_mesh_files_with_test_helpers(project_root, include_test_helpers)?;
    if !files
        .iter()
        .any(|relative_path| relative_path == entry_relative_path)
    {
        return Err(format!(
            "Resolved entrypoint '{}' was not found under project '{}'",
            entry_relative_path.display(),
            project_root.display()
        ));
    }
    let mut graph = ModuleGraph::new();
    let mut module_sources = Vec::new();
    let mut module_parses = Vec::new();

    for relative_path in &files {
        let source = read_source(&project_root.join(relative_path))?;
        let is_entry = relative_path == entry_relative_path;
        let name = if relative_path == Path::new(DEFAULT_ENTRYPOINT) {
            "Main".to_string()
        } else {
            path_to_module_name(relative_path).ok_or_else(|| {
                format!(
                    "Cannot determine module name for '{}'",
                    relative_path.display()
                )
            })?
        };

        let parse = mesh_parser::parse(&source);
        let _id = graph.add_module(name, relative_path.clone(), is_entry);

        module_sources.push(source);
        module_parses.push(parse);
    }

    // Phase 1b: Discover declared path and git dependencies and installed
    // package modules under .mesh/packages.
    let mut package_roots = crate::manifest::source_dependency_roots(project_root)?;
    let packages_dir = project_root.join(".mesh").join("packages");
    if packages_dir.exists() {
        package_roots.extend(discover_installed_package_roots(&packages_dir)?);
    }
    package_roots.sort();
    package_roots.dedup();
    for package_root in package_roots {
        let pkg_files = discover_mesh_files(&package_root)?;
        for relative_path in &pkg_files {
            let name = match path_to_module_name(relative_path) {
                Some(n) => n,
                None => continue, // skip package-root main.mpl
            };
            let full_path = package_root.join(relative_path);
            let source = read_source(&full_path)?;
            let parse = mesh_parser::parse(&source);
            let _id = graph.add_module(name, full_path, false);
            module_sources.push(source);
            module_parses.push(parse);
        }
    }

    for extra in native_bindings {
        let name = path_to_module_name(&extra.relative_path).ok_or_else(|| {
            format!(
                "Cannot determine native binding module name for '{}'",
                extra.relative_path.display()
            )
        })?;
        if let Some(existing) = graph.resolve(&name) {
            let existing_path = &graph.get(existing).path;
            let existing_full_path = if existing_path.is_absolute() {
                existing_path.clone()
            } else {
                project_root.join(existing_path)
            };
            if existing_full_path.canonicalize().ok() == extra.path.canonicalize().ok() {
                continue;
            }
            return Err(format!(
                "Native binding module `{name}` from '{}' conflicts with '{}'",
                extra.path.display(),
                existing_full_path.display()
            ));
        }
        let source = read_source(&extra.path)?;
        let parse = mesh_parser::parse(&source);
        graph.add_module(name, extra.path.clone(), false);
        module_sources.push(source);
        module_parses.push(parse);
    }

    link(graph, module_sources, module_parses)
}

/// A file outside any project, built as meshc builds a directory holding
/// only it: the entry module `Main`, with its module blocks and imports.
pub fn single_source_project(source: &str) -> Result<ProjectData, String> {
    let mut graph = ModuleGraph::new();
    graph.add_module("Main".to_string(), PathBuf::from(DEFAULT_ENTRYPOINT), true);
    link(
        graph,
        vec![source.to_string()],
        vec![mesh_parser::parse(source)],
    )
}

/// Make each `module` block a module of its own, connect every module to
/// those it imports or whose interfaces it uses, and order them.
fn link(
    mut graph: ModuleGraph,
    mut module_sources: Vec<String>,
    mut module_parses: Vec<mesh_parser::Parse>,
) -> Result<ProjectData, String> {
    // Phase 1c: a `module Name do ... end` block is a module of its own,
    // which files import like any other (the file holding it too). Its
    // source is the file with the rest blanked, so diagnostics point into
    // the file. A block without `pub` is private to its file.
    let mut private_inline: std::collections::HashMap<ModuleId, ModuleId> =
        std::collections::HashMap::new();
    let mut next = 0;
    while next < graph.module_count() {
        let owner = ModuleId(next as u32);
        // A file that does not parse is reported as it is.
        let inline = if module_parses[next].ok() {
            mesh_parser::inline_modules(&module_sources[next], &module_parses[next])
        } else {
            Vec::new()
        };
        for module in inline {
            let path = graph.get(owner).path.clone();
            if let Some(existing) = graph.resolve(&module.name) {
                return Err(format!(
                    "Module `{}` declared in '{}' conflicts with the module of '{}'",
                    module.name,
                    path.display(),
                    graph.get(existing).path.display()
                ));
            }
            let parse = mesh_parser::parse(&module.source);
            let id = graph.add_module(module.name, path, false);
            if !module.public {
                private_inline.insert(id, owner);
            }
            module_sources.push(module.source);
            module_parses.push(parse);
        }
        next += 1;
    }

    // Phase 2: Build dependency edges from import declarations.
    for (index, parse) in module_parses.iter().enumerate() {
        let id = ModuleId(index as u32);
        let imports = extract_imports(&parse.tree());
        let module_name = graph.get(id).name.clone();

        for import_name in imports {
            match graph.resolve(&import_name) {
                None => {
                    // Unknown import (stdlib or typo) -- skip silently.
                }
                Some(dep_id) if dep_id == id => {
                    return Err(format!("Module '{}' cannot import itself", module_name));
                }
                Some(dep_id) => {
                    if let Some(&owner) = private_inline.get(&dep_id) {
                        if graph.get(owner).path != graph.get(id).path {
                            return Err(format!(
                                "Module `{import_name}` is private to '{}': declare it \
                                 `pub module` to import it from '{}'",
                                graph.get(owner).path.display(),
                                graph.get(id).path.display()
                            ));
                        }
                    }
                    graph.add_dependency(id, dep_id);
                }
            }
        }
    }

    // Phase 2b: interfaces are visible in the modules checked after the
    // module declaring them, so one that implements another module's
    // `pub interface` (or bounds or calls through it) without importing
    // that module depends on it too. An edge that would close a cycle is
    // left out.
    let mut interface_homes: std::collections::HashMap<String, Vec<ModuleId>> =
        std::collections::HashMap::new();
    for (index, parse) in module_parses.iter().enumerate() {
        for item in parse.tree().items() {
            if let Item::InterfaceDef(interface) = item {
                if interface.visibility().is_some() {
                    if let Some(name) = interface.name().and_then(|name| name.text()) {
                        interface_homes
                            .entry(name)
                            .or_default()
                            .push(ModuleId(index as u32));
                    }
                }
            }
        }
    }
    for (index, parse) in module_parses.iter().enumerate() {
        let id = ModuleId(index as u32);
        let root = parse.syntax();
        let used = root.descendants_with_tokens().filter_map(|element| {
            let token = element.into_token()?;
            let in_use = token.parent_ancestors().any(|node| {
                matches!(
                    node.kind(),
                    SyntaxKind::PATH | SyntaxKind::WHERE_CLAUSE | SyntaxKind::NAME_REF
                )
            });
            (token.kind() == SyntaxKind::IDENT && in_use).then(|| token.text().to_string())
        });
        let mut homes: Vec<ModuleId> = Vec::new();
        for name in used {
            if let Some([home]) = interface_homes.get(&name).map(|homes| homes.as_slice()) {
                if *home != id && !homes.contains(home) {
                    homes.push(*home);
                }
            }
        }
        for home in homes {
            if !module_reaches(&graph, home, id) {
                graph.add_dependency(id, home);
            }
        }
    }

    // Phase 3: Topological sort.
    let compilation_order = module_graph::topological_sort(&graph)
        .map_err(|e: CycleError| format!("Circular dependency: {}", e))?;

    Ok(ProjectData {
        graph,
        compilation_order,
        module_sources,
        module_parses,
    })
}

/// Each module's type-check result and exports, indexed by `ModuleId.0`
/// (every entry is `Some`; the shape is what the export surface takes).
pub struct CheckedProject {
    pub typeck: Vec<Option<mesh_typeck::TypeckResult>>,
    pub exports: Vec<Option<mesh_typeck::ExportedSymbols>>,
}

/// Type-check every module in compilation order, each with the exports of
/// the modules it imports. `test_builtins` gives `meshc test`'s builtins.
///
/// A `let` outside a function is an error here, not in the type checker: the
/// REPL runs its bindings inside each evaluation, but in a project nothing
/// evaluates one, and a function naming it failed with "Undefined variable".
pub fn check_project(project: &ProjectData, test_builtins: bool) -> CheckedProject {
    let module_count = project.graph.module_count();
    let mut exports: Vec<Option<mesh_typeck::ExportedSymbols>> = vec![None; module_count];
    let mut typeck: Vec<Option<mesh_typeck::TypeckResult>> =
        (0..module_count).map(|_| None).collect();
    for &id in &project.compilation_order {
        let idx = id.0 as usize;
        let parse = &project.module_parses[idx];
        let mut import_ctx = build_import_context(&project.graph, &exports, parse);
        // Clustered route handlers are named with the module's name.
        import_ctx.current_module = Some(project.graph.get(id).name.clone());
        import_ctx.test_builtins = test_builtins;
        let mut result = mesh_typeck::check_with_imports(parse, &import_ctx);
        result.errors.extend(top_level_lets(parse));
        exports[idx] = Some(mesh_typeck::collect_exports(parse, &result));
        typeck[idx] = Some(result);
    }
    CheckedProject { typeck, exports }
}

/// The `let`s outside every function, actor, service and closure.
fn top_level_lets(parse: &mesh_parser::Parse) -> Vec<mesh_typeck::error::TypeError> {
    parse
        .syntax()
        .descendants()
        .filter(|node| {
            node.kind() == SyntaxKind::LET_BINDING
                && !node.ancestors().any(|ancestor| {
                    matches!(
                        ancestor.kind(),
                        SyntaxKind::FN_DEF
                            | SyntaxKind::ACTOR_DEF
                            | SyntaxKind::SERVICE_DEF
                            | SyntaxKind::SUPERVISOR_DEF
                            | SyntaxKind::IMPL_DEF
                            | SyntaxKind::INTERFACE_DEF
                            | SyntaxKind::CLOSURE_EXPR
                            | SyntaxKind::TRAILING_CLOSURE
                    )
                })
        })
        .map(|let_| mesh_typeck::error::TypeError::TopLevelLet {
            name: let_
                .children()
                .find(|child| child.kind() == SyntaxKind::NAME)
                .map_or_else(|| "_".to_string(), |name| name.text().to_string()),
            span: let_.text_range(),
        })
        .collect()
}

/// A module's import context: the exports of the modules it imports, and the
/// interfaces and implementations of every module checked before it (those
/// are visible everywhere).
pub fn build_import_context(
    graph: &ModuleGraph,
    all_exports: &[Option<mesh_typeck::ExportedSymbols>],
    parse: &mesh_parser::Parse,
) -> mesh_typeck::ImportContext {
    let mut ctx = mesh_typeck::ImportContext::empty();
    // `Main`, the entry module, is not importable.
    ctx.project_modules = graph
        .modules
        .iter()
        .map(|module| module.name.clone())
        .filter(|name| name != "Main")
        .collect();
    for exports in all_exports.iter().flatten() {
        ctx.all_trait_defs
            .extend(exports.trait_defs.iter().cloned());
        ctx.all_trait_impls
            .extend(exports.trait_impls.iter().cloned());
    }
    for module_name in extract_imports(&parse.tree()) {
        // A module not in the graph is reported by the type checker.
        let Some(Some(exports)) = graph
            .resolve(&module_name)
            .and_then(|dep_id| all_exports.get(dep_id.0 as usize))
        else {
            continue;
        };
        let last_segment = module_name
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_string();
        ctx.module_exports.insert(
            last_segment,
            mesh_typeck::ModuleExports::new(module_name, exports),
        );
    }
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn build_project_with_entrypoint(root: &Path, entry: &Path) -> Result<ProjectData, String> {
        super::build_project(root, entry, &[], false, &read_file)
    }

    fn build_project(root: &Path) -> Result<ProjectData, String> {
        build_project_with_entrypoint(root, Path::new(DEFAULT_ENTRYPOINT))
    }

    fn build_module_graph(root: &Path) -> Result<(ModuleGraph, Vec<ModuleId>), String> {
        let project = build_project(root)?;
        Ok((project.graph, project.compilation_order))
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("vector"), "Vector");
        assert_eq!(to_pascal_case("linear_algebra"), "LinearAlgebra");
        assert_eq!(to_pascal_case("a"), "A");
        assert_eq!(to_pascal_case("already_long_name"), "AlreadyLongName");
    }

    #[test]
    fn test_path_to_module_name_simple() {
        let path = Path::new("utils.mpl");
        assert_eq!(path_to_module_name(path), Some("Utils".to_string()));
    }

    #[test]
    fn test_path_to_module_name_nested() {
        let path = Path::new("math/vector.mpl");
        assert_eq!(path_to_module_name(path), Some("Math.Vector".to_string()));
    }

    #[test]
    fn test_path_to_module_name_snake_case() {
        let path = Path::new("math/linear_algebra.mpl");
        assert_eq!(
            path_to_module_name(path),
            Some("Math.LinearAlgebra".to_string())
        );
    }

    #[test]
    fn test_path_to_module_name_deeply_nested() {
        let path = Path::new("a/b/c/d.mpl");
        assert_eq!(path_to_module_name(path), Some("A.B.C.D".to_string()));
    }

    #[test]
    fn test_path_to_module_name_main() {
        let path = Path::new("main.mpl");
        assert_eq!(path_to_module_name(path), None);
    }

    #[test]
    fn test_discover_mesh_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create test files
        fs::write(root.join("main.mpl"), "").unwrap();
        fs::create_dir_all(root.join("math")).unwrap();
        fs::write(root.join("math/vector.mpl"), "").unwrap();
        fs::write(root.join("utils.mpl"), "").unwrap();
        fs::write(root.join("utils.test-support.mpl"), "").unwrap();
        fs::create_dir_all(root.join("tests/fixtures")).unwrap();
        fs::write(root.join("tests/support.mpl"), "").unwrap();
        fs::write(root.join("tests/fixtures/account.mpl"), "").unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join(".hidden/secret.mpl"), "").unwrap();

        let files = discover_mesh_files(root).unwrap();
        let file_strs: Vec<&str> = files.iter().map(|p| p.to_str().unwrap()).collect();

        assert_eq!(file_strs, vec!["main.mpl", "math/vector.mpl", "utils.mpl"]);

        let test_files = discover_mesh_files_with_test_helpers(root, true).unwrap();
        let test_file_strs: Vec<&str> = test_files
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect();
        assert_eq!(
            test_file_strs,
            vec![
                "main.mpl",
                "math/vector.mpl",
                "tests/fixtures/account.mpl",
                "tests/support.mpl",
                "utils.mpl",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_discover_mesh_files_rejects_visible_symlink_aliases() {
        use std::os::unix::fs::symlink;

        for target_name in ["tests", "outside"] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("project");
            fs::create_dir(&root).unwrap();
            fs::write(root.join("main.mpl"), "").unwrap();
            fs::create_dir(root.join("tests")).unwrap();
            fs::write(root.join("tests/support.mpl"), "").unwrap();
            fs::create_dir(tmp.path().join("outside")).unwrap();
            fs::write(tmp.path().join("outside/support.mpl"), "").unwrap();
            let target = if target_name == "tests" {
                root.join("tests")
            } else {
                tmp.path().join("outside")
            };
            let alias = root.join(format!("{target_name}_alias"));
            symlink(target, &alias).unwrap();

            let error = discover_mesh_files(&root).unwrap_err();
            assert!(
                error.contains("symbolic link")
                    && error.contains(alias.file_name().unwrap().to_str().unwrap()),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn test_discover_installed_package_roots_scoped_and_flat() {
        let tmp = tempfile::tempdir().unwrap();
        let packages_dir = tmp.path().join(".mesh/packages");

        fs::create_dir_all(packages_dir.join("acme/greeter@1.0.0")).unwrap();
        fs::write(
            packages_dir.join("acme/greeter@1.0.0/mesh.toml"),
            "[package]\nname = \"acme/greeter\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
        )
        .unwrap();
        fs::write(packages_dir.join("acme/greeter@1.0.0/main.mpl"), "").unwrap();

        fs::create_dir_all(packages_dir.join("flat@1.0.0")).unwrap();
        fs::write(
            packages_dir.join("flat@1.0.0/mesh.toml"),
            "[package]\nname = \"flat\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
        )
        .unwrap();

        fs::create_dir_all(packages_dir.join("owner-only")).unwrap();
        fs::write(packages_dir.join("owner-only/main.mpl"), "").unwrap();

        fs::create_dir_all(packages_dir.join(".hidden/ignored@1.0.0")).unwrap();
        fs::write(
            packages_dir.join(".hidden/ignored@1.0.0/mesh.toml"),
            "[package]\nname = \"ignored\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
        )
        .unwrap();

        let roots = discover_installed_package_roots(&packages_dir).unwrap();
        let relative_roots: Vec<String> = roots
            .iter()
            .map(|path| {
                path.strip_prefix(&packages_dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();

        assert_eq!(relative_roots, vec!["acme/greeter@1.0.0", "flat@1.0.0"]);
    }

    #[test]
    fn test_build_project_discovers_scoped_installed_package_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let package_root = root.join(".mesh/packages/acme/greeter@1.0.0");

        fs::write(
            root.join("main.mpl"),
            "from Support.Message import message\n\nfn main() do\n  println(message())\nend\n",
        )
        .unwrap();
        fs::create_dir_all(package_root.join("support")).unwrap();
        fs::write(
            package_root.join("mesh.toml"),
            "[package]\nname = \"acme/greeter\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
        )
        .unwrap();
        fs::write(package_root.join("main.mpl"), "fn main() do\n  0\nend\n").unwrap();
        fs::write(
            package_root.join("support/message.mpl"),
            "fn message() -> String do\n  \"hello from package\"\nend\n",
        )
        .unwrap();

        let project = build_project(root).unwrap();

        assert!(project.graph.resolve("Support.Message").is_some());
        assert!(project
            .graph
            .resolve("Greeter@1.0.0.Support.Message")
            .is_none());
    }

    #[test]
    fn test_build_project_discovers_path_dependency_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("app");
        let dependency = tmp.path().join("shared");

        fs::create_dir_all(root.clone()).unwrap();
        fs::create_dir_all(dependency.join("support")).unwrap();
        fs::write(
            root.join("mesh.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nshared = { path = \"../shared\" }\n",
        )
        .unwrap();
        fs::write(
            root.join("main.mpl"),
            "from Support.Message import message\n\nfn main() do\n  println(message())\nend\n",
        )
        .unwrap();
        fs::write(
            dependency.join("mesh.toml"),
            "[package]\nname = \"shared\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            dependency.join("support/message.mpl"),
            "pub fn message() -> String do\n  \"hello from path dependency\"\nend\n",
        )
        .unwrap();

        let project = build_project(&root).unwrap();

        assert!(project.graph.resolve("Support.Message").is_some());
    }

    #[test]
    fn test_build_project_discovers_git_dependency_checkouts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("app");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("mesh.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nshared = { git = \"https://example.invalid/shared.git\", tag = \"v1\" }\n",
        )
        .unwrap();
        fs::write(
            root.join("main.mpl"),
            "from Support.Message import message\n\nfn main() do\n  println(message())\nend\n",
        )
        .unwrap();

        let error = build_project(&root)
            .err()
            .expect("unfetched git dependency");
        assert!(error.contains("run `meshc deps`"), "{error}");

        let checkout = root.join(".mesh/deps/shared");
        fs::create_dir_all(checkout.join("support")).unwrap();
        fs::write(
            checkout.join("mesh.toml"),
            "[package]\nname = \"shared\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            checkout.join("support/message.mpl"),
            "pub fn message() -> String do\n  \"hello from git dependency\"\nend\n",
        )
        .unwrap();

        let project = build_project(&root).unwrap();
        assert!(project.graph.resolve("Support.Message").is_some());
    }

    // ── Import extraction tests ─────────────────────────────────────────

    #[test]
    fn test_extract_imports_both_forms() {
        let source = r#"
import Foo.Bar
from Baz.Qux import { name1, name2 }
"#;
        let parse = mesh_parser::parse(source);
        let tree = parse.tree();
        let imports = extract_imports(&tree);
        assert_eq!(imports, vec!["Foo.Bar".to_string(), "Baz.Qux".to_string()]);
    }

    // ── build_module_graph integration tests ────────────────────────────

    #[test]
    fn test_build_module_graph_simple() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.mpl"), "import Utils\n").unwrap();
        fs::write(root.join("utils.mpl"), "fn helper() do\n  1\nend\n").unwrap();

        let (graph, order) = build_module_graph(root).unwrap();
        assert_eq!(graph.module_count(), 2);

        let names: Vec<&str> = order
            .iter()
            .map(|id| graph.get(*id).name.as_str())
            .collect();
        assert_eq!(names, vec!["Utils", "Main"]);
    }

    #[test]
    fn test_build_module_graph_cycle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.mpl"), "fn main() do\n  1\nend\n").unwrap();
        fs::write(root.join("a.mpl"), "import B\n").unwrap();
        fs::write(root.join("b.mpl"), "import A\n").unwrap();

        let result = build_module_graph(root);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Circular dependency"),
            "Expected cycle error, got: {}",
            err
        );
    }

    #[test]
    fn test_build_module_graph_diamond() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.mpl"), "import A\nimport B\n").unwrap();
        fs::write(root.join("a.mpl"), "import C\n").unwrap();
        fs::write(root.join("b.mpl"), "import C\n").unwrap();
        fs::write(root.join("c.mpl"), "fn base() do\n  1\nend\n").unwrap();

        let (graph, order) = build_module_graph(root).unwrap();
        let names: Vec<&str> = order
            .iter()
            .map(|id| graph.get(*id).name.as_str())
            .collect();

        // C first (no deps), then A and B (alphabetical), then Main last.
        assert_eq!(names, vec!["C", "A", "B", "Main"]);
    }

    #[test]
    fn test_build_module_graph_unknown_import_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.mpl"), "import NonExistent\nimport IO\n").unwrap();

        let (graph, order) = build_module_graph(root).unwrap();
        assert_eq!(graph.module_count(), 1);

        let names: Vec<&str> = order
            .iter()
            .map(|id| graph.get(*id).name.as_str())
            .collect();
        assert_eq!(names, vec!["Main"]);
    }

    #[test]
    fn test_build_module_graph_self_import() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.mpl"), "fn main() do\n  1\nend\n").unwrap();
        fs::write(root.join("utils.mpl"), "import Utils\n").unwrap();

        let result = build_module_graph(root);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("cannot import itself"),
            "Expected self-import error, got: {}",
            err
        );
    }

    // ── build_project tests ──────────────────────────────────────────────

    /// Each module is checked with the exports of those it imports, and a
    /// `let` outside every function is an error, named when it has a name.
    #[test]
    fn a_project_is_checked_module_by_module() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("geo.mpl"),
            "pub fn area(r :: Int) -> Int do\n  r * r\nend\n",
        )
        .unwrap();
        fs::write(
            root.join("main.mpl"),
            "import Geo\nimport Missing\n\nlet limit = 3\nlet (a, b) = (1, 2)\n\nfn main() do\n  let inner = Geo.area(2)\n  println(\"#{inner}\")\nend\n",
        )
        .unwrap();
        let project = build_project(root).unwrap();
        let checked = check_project(&project, false);

        let main = project.graph.resolve("Main").unwrap().0 as usize;
        let geo = project.graph.resolve("Geo").unwrap().0 as usize;
        assert!(checked.exports[geo]
            .as_ref()
            .unwrap()
            .functions
            .contains_key("area"));
        let top_level: Vec<&str> = checked.typeck[main]
            .as_ref()
            .unwrap()
            .errors
            .iter()
            .filter_map(|error| match error {
                mesh_typeck::error::TypeError::TopLevelLet { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(top_level, ["limit", "_"]);

        let context = build_import_context(
            &project.graph,
            &checked.exports,
            &project.module_parses[main],
        );
        assert!(context.module_exports.contains_key("Geo"));
        assert!(!context.module_exports.contains_key("Missing"));
        assert!(context.project_modules.iter().any(|name| name == "Geo"));
        assert!(!context.project_modules.iter().any(|name| name == "Main"));
    }

    #[test]
    fn module_names_come_from_the_path() {
        assert_eq!(to_pascal_case("my__cool_"), "MyCool");
        assert_eq!(
            path_to_module_name(Path::new("./net/http_client.mpl")).as_deref(),
            Some("Net.HttpClient")
        );
        assert_eq!(
            path_to_module_name(Path::new("lib/main.mpl")).as_deref(),
            Some("Lib.Main")
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let dir = std::ffi::OsStr::from_bytes(b"\xff");
            assert_eq!(path_to_module_name(&Path::new(dir).join("x.mpl")), None);
        }
    }

    #[test]
    fn an_entrypoint_outside_the_project_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        for entry in ["../main.mpl", "/main.mpl"] {
            let error = build_project_with_entrypoint(tmp.path(), Path::new(entry))
                .err()
                .unwrap();
            assert!(
                error.contains("must stay within project"),
                "{entry}: {error}"
            );
        }
    }

    /// A native binding is a module of its own, unless it is a file of the
    /// project already; one named like another file's module is refused.
    #[test]
    fn native_bindings_join_the_project_once() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("app");
        fs::create_dir_all(root.join("bindings")).unwrap();
        fs::write(root.join("main.mpl"), "fn main() do\n  1\nend\n").unwrap();
        fs::write(
            root.join("bindings/own.mpl"),
            "pub fn own() -> Int do\n  1\nend\n",
        )
        .unwrap();
        let dependency = tmp.path().join("dep");
        fs::create_dir_all(dependency.join("bindings")).unwrap();
        fs::write(
            dependency.join("bindings/own.mpl"),
            "pub fn other() -> Int do\n  2\nend\n",
        )
        .unwrap();
        fs::write(
            dependency.join("bindings/extra.mpl"),
            "pub fn extra() -> Int do\n  3\nend\n",
        )
        .unwrap();
        let binding = |package_root: &Path, relative: &str| ResolvedNativeBinding {
            package: "p".to_string(),
            path: package_root.join(relative),
            relative_path: PathBuf::from(relative),
        };
        let build = |bindings: &[ResolvedNativeBinding]| {
            super::build_project(
                &root,
                Path::new(DEFAULT_ENTRYPOINT),
                bindings,
                false,
                &read_file,
            )
        };

        let project = build(&[
            binding(&root, "bindings/own.mpl"),
            binding(&dependency, "bindings/extra.mpl"),
        ])
        .unwrap();
        let names: Vec<&str> = project
            .graph
            .modules
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, ["Bindings.Own", "Main", "Bindings.Extra"]);

        let error = build(&[binding(&dependency, "bindings/own.mpl")])
            .err()
            .unwrap();
        assert!(
            error.contains("Native binding module `Bindings.Own`"),
            "{error}"
        );
        // A binding at a package's root `main.mpl` names no module.
        fs::write(
            dependency.join("main.mpl"),
            "pub fn m() -> Int do\n  1\nend\n",
        )
        .unwrap();
        let error = build(&[binding(&dependency, "main.mpl")]).err().unwrap();
        assert!(
            error.contains("Cannot determine native binding module name"),
            "{error}"
        );
    }

    #[test]
    fn a_module_block_named_like_a_file_module_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("geo.mpl"), "pub fn area() -> Int do\n  1\nend\n").unwrap();
        fs::write(
            root.join("main.mpl"),
            "module Geo do\nend\n\nfn main() do\n  1\nend\n",
        )
        .unwrap();
        let error = build_project(root).err().unwrap();
        assert!(error.contains("Module `Geo` declared in"), "{error}");
    }

    #[test]
    fn a_private_module_block_is_not_imported_from_another_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("shapes.mpl"),
            "module Inner do\n  pub fn f() -> Int do\n    1\n  end\nend\n",
        )
        .unwrap();
        fs::write(
            root.join("main.mpl"),
            "import Inner\n\nfn main() do\n  1\nend\n",
        )
        .unwrap();
        let error = build_project(root).err().unwrap();
        assert!(error.contains("is private to"), "{error}");
    }

    /// Using an interface adds a dependency on the module defining it,
    /// unless that module already depends on the user: a cycle would
    /// follow, and the order is right already.
    #[test]
    fn using_an_interface_adds_no_edge_that_would_close_a_cycle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("greeting.mpl"),
            "import Numbers\n\npub interface Greet do\n  fn greet(self) -> String\nend\n",
        )
        .unwrap();
        fs::write(
            root.join("numbers.mpl"),
            "impl Greet for Int do\n  fn greet(self) -> String do\n    \"hi\"\n  end\nend\n",
        )
        .unwrap();
        fs::write(root.join("main.mpl"), "fn main() do\n  1\nend\n").unwrap();
        let (graph, order) = build_module_graph(root).unwrap();
        let position = |name: &str| order.iter().position(|id| graph.get(*id).name == name);
        assert!(position("Numbers") < position("Greeting"));
    }

    #[test]
    fn using_an_interface_orders_its_module_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("zeta.mpl"),
            "pub interface Greet do\n  fn greet(self) -> String\nend\n",
        )
        .unwrap();
        fs::write(
            root.join("alpha.mpl"),
            "impl Greet for Int do\n  fn greet(self) -> String do\n    \"hi\"\n  end\nend\n",
        )
        .unwrap();
        fs::write(root.join("main.mpl"), "fn main() do\n  1\nend\n").unwrap();
        let (graph, order) = build_module_graph(root).unwrap();
        let position = |name: &str| order.iter().position(|id| graph.get(*id).name == name);
        // Discovered after `Alpha`, `Zeta` comes first only through the edge.
        assert!(position("Zeta") < position("Alpha"));
    }

    #[test]
    fn test_build_project_simple() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("main.mpl"),
            "import Utils\nfn main() do\n  1\nend\n",
        )
        .unwrap();
        fs::write(root.join("utils.mpl"), "fn helper() do\n  1\nend\n").unwrap();

        let project = build_project(root).unwrap();

        // Graph has 2 modules
        assert_eq!(project.graph.module_count(), 2);

        // Sources and parses are indexed in parallel
        assert_eq!(project.module_sources.len(), 2);
        assert_eq!(project.module_parses.len(), 2);

        // Compilation order: Utils before Main (Main imports Utils)
        let names: Vec<&str> = project
            .compilation_order
            .iter()
            .map(|id| project.graph.get(*id).name.as_str())
            .collect();
        assert_eq!(names, vec!["Utils", "Main"]);

        // Parse results have no errors
        for parse in &project.module_parses {
            assert!(parse.errors().is_empty(), "Expected no parse errors");
        }

        // Sources contain expected text
        let main_id = project.graph.resolve("Main").unwrap();
        let utils_id = project.graph.resolve("Utils").unwrap();
        assert!(project.module_sources[main_id.0 as usize].contains("import Utils"));
        assert!(project.module_sources[utils_id.0 as usize].contains("fn helper()"));
    }

    #[test]
    fn test_build_project_with_entrypoint_override_marks_non_root_entry_without_renaming() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(root.join("main.mpl"), "fn main() do\n  0\nend\n").unwrap();
        fs::write(
            root.join("lib/start.mpl"),
            "from Lib.Support import answer\n\nfn main() do\n  answer()\nend\n",
        )
        .unwrap();
        fs::write(
            root.join("lib/support.mpl"),
            "pub fn answer() -> Int do\n  42\nend\n",
        )
        .unwrap();

        let project = build_project_with_entrypoint(root, Path::new("lib/start.mpl")).unwrap();

        let root_main = project
            .graph
            .resolve("Main")
            .expect("root main should still exist");
        let override_entry = project
            .graph
            .resolve("Lib.Start")
            .expect("override entry should keep its path-derived module name");
        let support = project
            .graph
            .resolve("Lib.Support")
            .expect("support module should be discovered");

        assert!(!project.graph.get(root_main).is_entry);
        assert!(project.graph.get(override_entry).is_entry);
        assert_eq!(
            project.graph.get(override_entry).path,
            PathBuf::from("lib/start.mpl")
        );
        assert_eq!(
            project.graph.get(support).path,
            PathBuf::from("lib/support.mpl")
        );
    }

    #[test]
    fn test_build_project_with_entrypoint_override_wins_when_both_entry_files_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(root.join("main.mpl"), "fn main() do\n  0\nend\n").unwrap();
        fs::write(root.join("lib/start.mpl"), "fn main() do\n  1\nend\n").unwrap();

        let project = build_project_with_entrypoint(root, Path::new("lib/start.mpl")).unwrap();

        let entry_modules: Vec<&str> = project
            .graph
            .modules
            .iter()
            .filter(|module| module.is_entry)
            .map(|module| module.name.as_str())
            .collect();
        assert_eq!(entry_modules, vec!["Lib.Start"]);
    }

    #[test]
    fn test_build_project_with_entrypoint_missing_override_reports_resolved_path() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.mpl"), "fn main() do\n  0\nend\n").unwrap();

        let err = match build_project_with_entrypoint(root, Path::new("lib/start.mpl")) {
            Ok(project) => panic!(
                "expected missing override entrypoint to fail, discovered {} modules",
                project.graph.module_count()
            ),
            Err(err) => err,
        };

        assert!(err.contains("lib/start.mpl"), "unexpected error: {err}");
    }

    #[test]
    fn test_build_project_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.mpl"), "fn main() do\n  42\nend\n").unwrap();

        let project = build_project(root).unwrap();

        assert_eq!(project.graph.module_count(), 1);
        assert_eq!(project.module_sources.len(), 1);
        assert_eq!(project.module_parses.len(), 1);

        // Single entry in compilation order, marked as entry
        assert_eq!(project.compilation_order.len(), 1);
        let entry_id = project.compilation_order[0];
        assert!(project.graph.get(entry_id).is_entry);

        // Parse has no errors
        assert!(project.module_parses[0].errors().is_empty());
    }

    #[test]
    fn test_build_project_parse_error_retained() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.mpl"), "fn main() do\n  1\nend\n").unwrap();
        fs::write(root.join("broken.mpl"), "fn incomplete(\n").unwrap();

        let project = build_project(root).unwrap();

        // build_project succeeds even with parse errors (that is build()'s job to check)
        assert_eq!(project.graph.module_count(), 2);

        let main_id = project.graph.resolve("Main").unwrap();
        let broken_id = project.graph.resolve("Broken").unwrap();

        // Broken module has parse errors
        assert!(
            !project.module_parses[broken_id.0 as usize]
                .errors()
                .is_empty(),
            "Expected parse errors in broken module"
        );

        // Main module has no parse errors
        assert!(
            project.module_parses[main_id.0 as usize]
                .errors()
                .is_empty(),
            "Expected no parse errors in main module"
        );
    }
}
