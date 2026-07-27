use crate::{Dependency, Lockfile, Manifest};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNativeArchive {
    pub package: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNativeBinding {
    pub package: String,
    pub path: PathBuf,
    pub relative_path: PathBuf,
}

/// Resolve and verify the native archives reachable from one project.
///
/// Registry and git packages must already be installed and pinned by
/// `mesh.lock`; this function never fetches code while compiling.
pub fn resolve_native_archives(
    project_root: &Path,
    target: &str,
) -> Result<Vec<ResolvedNativeArchive>, String> {
    resolve_native(project_root, Some(target)).map(|resolution| resolution.archives)
}

pub fn resolve_native_bindings(project_root: &Path) -> Result<Vec<ResolvedNativeBinding>, String> {
    resolve_native(project_root, None).map(|resolution| resolution.bindings)
}

fn resolve_native(project_root: &Path, target: Option<&str>) -> Result<NativeResolution, String> {
    let project_root = project_root
        .canonicalize()
        .map_err(|error| format!("Failed to resolve '{}': {error}", project_root.display()))?;
    let manifest = Manifest::from_file(&project_root.join("mesh.toml"))?;
    let lock_path = project_root.join("mesh.lock");
    let lockfile = lock_path
        .exists()
        .then(|| Lockfile::read(&lock_path))
        .transpose()?;
    let mut context = ResolveContext {
        project_root: &project_root,
        target,
        lockfile: lockfile.as_ref(),
        visited: BTreeSet::new(),
        archives: Vec::new(),
        bindings: Vec::new(),
    };
    context.visit(&project_root, &manifest)?;
    Ok(NativeResolution {
        archives: context.archives,
        bindings: context.bindings,
    })
}

struct NativeResolution {
    archives: Vec<ResolvedNativeArchive>,
    bindings: Vec<ResolvedNativeBinding>,
}

struct ResolveContext<'a> {
    project_root: &'a Path,
    target: Option<&'a str>,
    lockfile: Option<&'a Lockfile>,
    visited: BTreeSet<PathBuf>,
    archives: Vec<ResolvedNativeArchive>,
    bindings: Vec<ResolvedNativeBinding>,
}

impl ResolveContext<'_> {
    fn visit(&mut self, package_root: &Path, manifest: &Manifest) -> Result<(), String> {
        let package_root = package_root.canonicalize().map_err(|error| {
            format!(
                "Failed to resolve native package root '{}': {error}",
                package_root.display()
            )
        })?;
        if !self.visited.insert(package_root.clone()) {
            return Ok(());
        }

        if let Some(native) = &manifest.native {
            for binding in &native.bindings {
                self.bindings.push(ResolvedNativeBinding {
                    package: manifest.package.name.clone(),
                    path: checked_package_file(&package_root, binding, "native binding")?,
                    relative_path: binding.clone(),
                });
            }

            if let Some(target) = self.target {
                let library = native
                    .libraries
                    .iter()
                    .find(|library| library.target == target)
                    .ok_or_else(|| {
                        let available = native
                            .libraries
                            .iter()
                            .map(|library| library.target.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!(
                            "Native package `{}` has no archive for target `{}` (available: {})",
                            manifest.package.name, target, available
                        )
                    })?;
                let path =
                    checked_package_file(&package_root, &library.path, "native static archive")?;
                let actual = sha256_file(&path)?;
                if actual != library.sha256 {
                    return Err(format!(
                        "SHA-256 mismatch for native archive '{}' in package `{}`: expected {}, got {}",
                        library.path.display(),
                        manifest.package.name,
                        library.sha256,
                        actual
                    ));
                }
                self.archives.push(ResolvedNativeArchive {
                    package: manifest.package.name.clone(),
                    path,
                });
            }
        }

        for (name, dependency) in &manifest.dependencies {
            let dependency_root = self.dependency_root(&package_root, name, dependency)?;
            let dependency_manifest = Manifest::from_file(&dependency_root.join("mesh.toml"))?;
            self.visit(&dependency_root, &dependency_manifest)?;
        }

        Ok(())
    }

    fn dependency_root(
        &self,
        package_root: &Path,
        name: &str,
        dependency: &Dependency,
    ) -> Result<PathBuf, String> {
        match dependency {
            Dependency::Path { path } => package_root.join(path).canonicalize().map_err(|error| {
                format!("Failed to resolve path dependency `{name}` ({path}): {error}")
            }),
            Dependency::Git { .. } => {
                let locked = self.locked_package(name)?;
                if locked.revision == "local" {
                    return Err(format!(
                        "Git dependency `{name}` is not pinned to an exact revision in mesh.lock"
                    ));
                }
                let root = self.project_root.join(".mesh").join("deps").join(name);
                let repository = git2::Repository::open(&root).map_err(|error| {
                    format!(
                        "Git dependency `{name}` is not installed at '{}': {error}; run `meshc deps`",
                        root.display()
                    )
                })?;
                let head = repository
                    .head()
                    .and_then(|head| head.peel_to_commit())
                    .map_err(|error| {
                        format!("Failed to read installed `{name}` revision: {error}")
                    })?
                    .id()
                    .to_string();
                if head != locked.revision {
                    return Err(format!(
                        "Git dependency `{name}` revision mismatch: mesh.lock pins {}, installed checkout is {}",
                        locked.revision, head
                    ));
                }
                Ok(root)
            }
            Dependency::RegistryShorthand(version) | Dependency::Registry { version } => {
                let locked = self.locked_package(name)?;
                if locked.version != *version || locked.sha256.is_none() {
                    return Err(format!(
                        "Registry dependency `{name}` is not checksum-pinned at version `{version}` in mesh.lock"
                    ));
                }
                let root = self
                    .project_root
                    .join(".mesh")
                    .join("packages")
                    .join(format!("{name}@{}", locked.version));
                if !root.join("mesh.toml").is_file() {
                    return Err(format!(
                        "Registry dependency `{name}` is not installed at '{}'; run `meshpkg install`",
                        root.display()
                    ));
                }
                Ok(root)
            }
        }
    }

    fn locked_package(&self, name: &str) -> Result<&crate::LockedPackage, String> {
        self.lockfile
            .and_then(|lockfile| lockfile.packages.iter().find(|package| package.name == name))
            .ok_or_else(|| {
                format!(
                    "Native dependency `{name}` requires an exact mesh.lock entry; run the package resolver first"
                )
            })
    }
}

fn checked_package_file(root: &Path, relative: &Path, kind: &str) -> Result<PathBuf, String> {
    let joined = root.join(relative);
    let canonical = joined.canonicalize().map_err(|error| {
        format!(
            "{kind} '{}' does not exist or cannot be read: {error}",
            joined.display()
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(format!(
            "{kind} '{}' resolves outside package root '{}'",
            relative.display(),
            root.display()
        ));
    }
    if !canonical.is_file() {
        return Err(format!("{kind} '{}' is not a file", joined.display()));
    }
    Ok(joined)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("Failed to read '{}': {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Failed to read '{}': {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fs;

    #[test]
    fn resolve_native_archive_is_target_exact_and_checksum_verified() {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir_all(project.path().join("bindings")).unwrap();
        fs::create_dir_all(project.path().join("native/aarch64-apple-darwin")).unwrap();
        fs::write(
            project.path().join("bindings/math.mpl"),
            "@native(\"mesh_math_add\")\npub fn add(a :: Int, b :: Int) -> Int\n",
        )
        .unwrap();
        let archive_path = project.path().join("native/aarch64-apple-darwin/libmath.a");
        fs::write(&archive_path, b"archive-v1").unwrap();
        let sha256 = format!("{:x}", Sha256::digest(b"archive-v1"));
        fs::write(
            project.path().join("mesh.toml"),
            format!(
                r#"[package]
name = "native-math"
version = "0.1.0"

[native]
abi = 1
bindings = ["bindings/math.mpl"]

[[native.libraries]]
target = "aarch64-apple-darwin"
path = "native/aarch64-apple-darwin/libmath.a"
sha256 = "{sha256}"
"#
            ),
        )
        .unwrap();

        let resolved = resolve_native_archives(project.path(), "aarch64-apple-darwin").unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].package, "native-math");
        assert_eq!(resolved[0].path, archive_path.canonicalize().unwrap());
        let bindings = resolve_native_bindings(project.path()).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            bindings[0].path,
            project
                .path()
                .join("bindings/math.mpl")
                .canonicalize()
                .unwrap()
        );

        let target_error =
            resolve_native_archives(project.path(), "x86_64-unknown-linux-gnu").unwrap_err();
        assert!(
            target_error.contains("x86_64-unknown-linux-gnu"),
            "unexpected error: {target_error}"
        );

        fs::write(&archive_path, b"tampered").unwrap();
        let hash_error =
            resolve_native_archives(project.path(), "aarch64-apple-darwin").unwrap_err();
        assert!(
            hash_error.contains("SHA-256 mismatch"),
            "unexpected error: {hash_error}"
        );
    }
}
