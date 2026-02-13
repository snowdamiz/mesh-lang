use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Represents a parsed mesh.toml manifest file.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub package: Package,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
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
}

/// A dependency specification -- either git-based or path-based.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    Git {
        git: String,
        #[serde(default)]
        rev: Option<String>,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        tag: Option<String>,
    },
    Path {
        path: String,
    },
}

impl Manifest {
    /// Read and parse a mesh.toml manifest from a file path.
    pub fn from_file(path: &Path) -> Result<Manifest, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        Self::from_str(&content)
    }

    /// Parse a mesh.toml manifest from a string.
    pub fn from_str(content: &str) -> Result<Manifest, String> {
        toml::from_str(content).map_err(|e| format!("Failed to parse manifest: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(manifest.package.description.as_deref(), Some("A test project"));
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
        assert!(manifest.dependencies.is_empty());
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
            Dependency::Git { git, rev, branch, tag } => {
                assert_eq!(git, "https://example.com/lib.git");
                assert!(rev.is_none());
                assert!(branch.is_none());
                assert!(tag.is_none());
            }
            _ => panic!("Expected git dependency"),
        }
    }
}
