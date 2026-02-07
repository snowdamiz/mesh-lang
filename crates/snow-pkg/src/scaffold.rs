//! Project scaffolding for `snowc init`.
//!
//! Creates the standard Snow project layout:
//!
//! ```text
//! <name>/
//!   snow.toml
//!   main.snow
//! ```

use std::path::Path;

/// Create a new Snow project with the given name inside the given parent directory.
///
/// Creates `<dir>/<name>/` containing:
/// - `snow.toml` with package metadata and empty dependencies
/// - `main.snow` with a minimal hello-world program
///
/// Returns an error if the target directory already exists.
pub fn scaffold_project(name: &str, dir: &Path) -> Result<(), String> {
    let project_dir = dir.join(name);

    if project_dir.exists() {
        return Err(format!("Directory '{}' already exists", name));
    }

    std::fs::create_dir_all(&project_dir)
        .map_err(|e| format!("Failed to create directory '{}': {}", name, e))?;

    // Write snow.toml
    let manifest = format!(
        r#"[package]
name = "{}"
version = "0.1.0"

[dependencies]
"#,
        name
    );
    std::fs::write(project_dir.join("snow.toml"), manifest)
        .map_err(|e| format!("Failed to write snow.toml: {}", e))?;

    // Write main.snow
    let main_snow = r#"fn main() do
  IO.puts("Hello from Snow!")
end
"#;
    std::fs::write(project_dir.join("main.snow"), main_snow)
        .map_err(|e| format!("Failed to write main.snow: {}", e))?;

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
            project_dir.join("snow.toml").exists(),
            "snow.toml should exist"
        );
        assert!(
            project_dir.join("main.snow").exists(),
            "main.snow should exist"
        );
    }

    #[test]
    fn scaffold_snow_toml_is_valid() {
        let tmp = TempDir::new().unwrap();
        scaffold_project("test-project", tmp.path()).unwrap();

        let toml_path = tmp.path().join("test-project").join("snow.toml");
        let content = std::fs::read_to_string(&toml_path).unwrap();
        let manifest = Manifest::from_str(&content).unwrap();

        assert_eq!(manifest.package.name, "test-project");
        assert_eq!(manifest.package.version, "0.1.0");
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn scaffold_main_snow_content() {
        let tmp = TempDir::new().unwrap();
        scaffold_project("hello", tmp.path()).unwrap();

        let main_path = tmp.path().join("hello").join("main.snow");
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
