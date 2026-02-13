//! Project scaffolding for `meshc init`.
//!
//! Creates the standard Mesh project layout:
//!
//! ```text
//! <name>/
//!   mesh.toml
//!   main.mpl
//! ```

use std::path::Path;

/// Create a new Mesh project with the given name inside the given parent directory.
///
/// Creates `<dir>/<name>/` containing:
/// - `mesh.toml` with package metadata and empty dependencies
/// - `main.mpl` with a minimal hello-world program
///
/// Returns an error if the target directory already exists.
pub fn scaffold_project(name: &str, dir: &Path) -> Result<(), String> {
    let project_dir = dir.join(name);

    if project_dir.exists() {
        return Err(format!("Directory '{}' already exists", name));
    }

    std::fs::create_dir_all(&project_dir)
        .map_err(|e| format!("Failed to create directory '{}': {}", name, e))?;

    // Write mesh.toml
    let manifest = format!(
        r#"[package]
name = "{}"
version = "0.1.0"

[dependencies]
"#,
        name
    );
    std::fs::write(project_dir.join("mesh.toml"), manifest)
        .map_err(|e| format!("Failed to write mesh.toml: {}", e))?;

    // Write main.mpl
    let main_mesh = r#"fn main() do
  IO.puts("Hello from Mesh!")
end
"#;
    std::fs::write(project_dir.join("main.mpl"), main_mesh)
        .map_err(|e| format!("Failed to write main.mpl: {}", e))?;

    println!("Created project '{}'", name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use tempfile::TempDir;

    #[test]
    fn scaffold_creates_directory_structure() {
        let tmp = TempDir::new().unwrap();
        scaffold_project("my-app", tmp.path()).unwrap();

        let project_dir = tmp.path().join("my-app");
        assert!(project_dir.exists(), "Project directory should exist");
        assert!(project_dir.is_dir(), "Project path should be a directory");
        assert!(
            project_dir.join("mesh.toml").exists(),
            "mesh.toml should exist"
        );
        assert!(
            project_dir.join("main.mpl").exists(),
            "main.mpl should exist"
        );
    }

    #[test]
    fn scaffold_mesh_toml_is_valid() {
        let tmp = TempDir::new().unwrap();
        scaffold_project("test-project", tmp.path()).unwrap();

        let toml_path = tmp.path().join("test-project").join("mesh.toml");
        let content = std::fs::read_to_string(&toml_path).unwrap();
        let manifest = Manifest::from_str(&content).unwrap();

        assert_eq!(manifest.package.name, "test-project");
        assert_eq!(manifest.package.version, "0.1.0");
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn scaffold_main_mesh_content() {
        let tmp = TempDir::new().unwrap();
        scaffold_project("hello", tmp.path()).unwrap();

        let main_path = tmp.path().join("hello").join("main.mpl");
        let content = std::fs::read_to_string(&main_path).unwrap();
        assert!(content.contains("fn main()"), "Should have main function");
        assert!(
            content.contains("IO.puts"),
            "Should have IO.puts call"
        );
    }

    #[test]
    fn scaffold_error_when_directory_exists() {
        let tmp = TempDir::new().unwrap();
        let existing = tmp.path().join("existing");
        std::fs::create_dir_all(&existing).unwrap();

        let result = scaffold_project("existing", tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("already exists"),
            "Error should mention 'already exists', got: {}",
            err
        );
    }
}
