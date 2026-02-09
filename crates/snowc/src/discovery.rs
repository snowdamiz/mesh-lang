//! File discovery and path-to-module-name mapping for Snow projects.
//!
//! Provides utilities to recursively discover `.snow` files in a project
//! directory and convert file paths to PascalCase module names.

use std::path::{Component, Path, PathBuf};

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
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + chars.as_str()
                }
                None => String::new(),
            }
        })
        .collect()
}

/// Convert a relative file path to a PascalCase module name.
///
/// Returns `None` for `main.snow` in the project root (the entry point).
///
/// # Convention
///
/// - `math/vector.snow` -> `Some("Math.Vector")`
/// - `utils.snow` -> `Some("Utils")`
/// - `math/linear_algebra.snow` -> `Some("Math.LinearAlgebra")`
/// - `a/b/c/d.snow` -> `Some("A.B.C.D")`
/// - `main.snow` -> `None`
pub fn path_to_module_name(relative_path: &Path) -> Option<String> {
    let stem = relative_path.file_stem()?.to_str()?;
    let parent = relative_path.parent();

    // Check if this is main.snow at the project root
    let parent_is_empty = match parent {
        None => true,
        Some(p) => p.as_os_str().is_empty() || p == Path::new("."),
    };

    if stem == "main" && parent_is_empty {
        return None;
    }

    // Collect directory components
    let mut parts: Vec<String> = Vec::new();

    if let Some(parent_path) = parent {
        for component in parent_path.components() {
            if let Component::Normal(os_str) = component {
                if let Some(s) = os_str.to_str() {
                    parts.push(to_pascal_case(s));
                }
            }
        }
    }

    // Add the file stem
    parts.push(to_pascal_case(stem));

    Some(parts.join("."))
}

/// Recursively discover all `.snow` files in a project directory.
///
/// Returns paths relative to `project_root`, sorted alphabetically for
/// determinism. Hidden directories (names starting with `.`) are skipped.
pub fn discover_snow_files(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    discover_recursive(project_root, project_root, &mut files)
        .map_err(|e| format!("Failed to walk directory '{}': {}", project_root.display(), e))?;
    files.sort();
    Ok(files)
}

/// Internal recursive walker that collects `.snow` files as relative paths.
fn discover_recursive(
    root: &Path,
    dir: &Path,
    files: &mut Vec<PathBuf>,
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

        if entry_path.is_dir() {
            discover_recursive(root, &entry_path, files)?;
        } else if entry_path.extension().and_then(|e| e.to_str()) == Some("snow") {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("vector"), "Vector");
        assert_eq!(to_pascal_case("linear_algebra"), "LinearAlgebra");
        assert_eq!(to_pascal_case("a"), "A");
        assert_eq!(to_pascal_case("already_long_name"), "AlreadyLongName");
    }

    #[test]
    fn test_path_to_module_name_simple() {
        let path = Path::new("utils.snow");
        assert_eq!(path_to_module_name(path), Some("Utils".to_string()));
    }

    #[test]
    fn test_path_to_module_name_nested() {
        let path = Path::new("math/vector.snow");
        assert_eq!(path_to_module_name(path), Some("Math.Vector".to_string()));
    }

    #[test]
    fn test_path_to_module_name_snake_case() {
        let path = Path::new("math/linear_algebra.snow");
        assert_eq!(
            path_to_module_name(path),
            Some("Math.LinearAlgebra".to_string())
        );
    }

    #[test]
    fn test_path_to_module_name_deeply_nested() {
        let path = Path::new("a/b/c/d.snow");
        assert_eq!(path_to_module_name(path), Some("A.B.C.D".to_string()));
    }

    #[test]
    fn test_path_to_module_name_main() {
        let path = Path::new("main.snow");
        assert_eq!(path_to_module_name(path), None);
    }

    #[test]
    fn test_discover_snow_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create test files
        fs::write(root.join("main.snow"), "").unwrap();
        fs::create_dir_all(root.join("math")).unwrap();
        fs::write(root.join("math/vector.snow"), "").unwrap();
        fs::write(root.join("utils.snow"), "").unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join(".hidden/secret.snow"), "").unwrap();

        let files = discover_snow_files(root).unwrap();
        let file_strs: Vec<&str> = files.iter().map(|p| p.to_str().unwrap()).collect();

        assert_eq!(file_strs, vec!["main.snow", "math/vector.snow", "utils.snow"]);
    }
}
