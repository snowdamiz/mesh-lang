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
        // An installed git checkout may be reached through a link.
        let package_root = package_root
            .canonicalize()
            .unwrap_or_else(|_| package_root.to_path_buf());
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

/// A file of a package, named by a path inside it: no `..` or absolute part,
/// no symbolic link on the way (either could lead out of the package), and
/// every component there. The package root is canonical, so the path is too.
pub fn checked_package_file(root: &Path, relative: &Path, kind: &str) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => continue,
            std::path::Component::Normal(segment) => path.push(segment),
            _ => {
                return Err(format!(
                    "{kind} '{}' must be a path inside its package",
                    relative.display()
                ))
            }
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "{kind} '{}' does not exist or cannot be read: {error}",
                path.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "{kind} '{}' must not contain a symbolic link",
                relative.display()
            ));
        }
    }
    if !path.is_file() {
        return Err(format!("{kind} '{}' is not a file", path.display()));
    }
    Ok(path)
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
    use crate::{LockedPackage, Lockfile};
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

    #[test]
    fn resolve_native_bindings_rejects_top_level_test_helpers() {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join("tests")).unwrap();
        fs::write(
            project.path().join("tests/helper.mpl"),
            "@native(\"test_helper\")\npub fn helper() -> Int\n",
        )
        .unwrap();
        fs::write(
            project.path().join("mesh.toml"),
            r#"[package]
name = "test-native-binding"
version = "0.1.0"

[native]
abi = 1
bindings = ["tests/helper.mpl"]
"#,
        )
        .unwrap();

        let error = resolve_native_bindings(project.path()).unwrap_err();
        assert!(
            error.contains("native binding") && error.contains("tests/helper.mpl"),
            "unexpected error: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_native_bindings_rejects_symlink_aliases() {
        use std::os::unix::fs::symlink;

        for target_name in ["tests", "outside"] {
            let temp = tempfile::tempdir().unwrap();
            let project = temp.path().join("project");
            fs::create_dir(&project).unwrap();
            fs::create_dir(project.join("bindings")).unwrap();
            fs::create_dir(project.join("tests")).unwrap();
            fs::write(
                project.join("tests/helper.mpl"),
                "@native(\"test_helper\")\npub fn helper() -> Int\n",
            )
            .unwrap();
            fs::write(
                temp.path().join("outside.mpl"),
                "@native(\"outside_helper\")\npub fn helper() -> Int\n",
            )
            .unwrap();
            let target = if target_name == "tests" {
                project.join("tests/helper.mpl")
            } else {
                temp.path().join("outside.mpl")
            };
            symlink(target, project.join("bindings/helper.mpl")).unwrap();
            fs::write(
                project.join("mesh.toml"),
                r#"[package]
name = "symlinked-native-binding"
version = "0.1.0"

[native]
abi = 1
bindings = ["bindings/helper.mpl"]
"#,
            )
            .unwrap();

            let error = resolve_native_bindings(&project).unwrap_err();
            assert!(
                error.contains("symbolic link") && error.contains("bindings/helper.mpl"),
                "unexpected error: {error}"
            );
        }
    }

    /// A native package reached through the registry or git is read from
    /// its installed copy, and only when mesh.lock pins exactly that copy.
    #[test]
    fn native_dependencies_resolve_only_from_their_pinned_installed_copies() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("app");
        let write_package = |dir: &Path, manifest: &str| {
            fs::create_dir_all(dir.join("bindings")).unwrap();
            fs::write(
                dir.join("bindings/n.mpl"),
                "pub fn n() -> Int do\n  1\nend\n",
            )
            .unwrap();
            fs::write(dir.join("mesh.toml"), manifest).unwrap();
        };
        let native_package = |name: &str| {
            format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\n\n[native]\nabi = 1\nbindings = [\"bindings/n.mpl\"]\n")
        };
        let lock = |packages: &[LockedPackage]| {
            Lockfile::new(packages.to_vec())
                .write(&app.join("mesh.lock"))
                .unwrap()
        };
        let locked =
            |name: &str, version: &str, revision: &str, sha256: Option<&str>| LockedPackage {
                name: name.to_string(),
                version: version.to_string(),
                source: String::new(),
                revision: revision.to_string(),
                sha256: sha256.map(str::to_string),
            };
        let error = || resolve_native_bindings(&app).unwrap_err();

        // Through the registry.
        write_package(
            &app,
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nreg = \"1.0.0\"\n",
        );
        assert!(
            error().contains("requires an exact mesh.lock entry"),
            "{}",
            error()
        );
        lock(&[locked("reg", "1.0.0", "1.0.0", None)]);
        assert!(error().contains("is not checksum-pinned"), "{}", error());
        lock(&[locked("reg", "1.0.0", "1.0.0", Some("ab"))]);
        assert!(error().contains("run `meshpkg install`"), "{}", error());
        write_package(
            &app.join(".mesh/packages/reg@1.0.0"),
            &native_package("reg"),
        );
        let bindings = resolve_native_bindings(&app).unwrap();
        assert_eq!(bindings[0].package, "reg");

        // Through git.
        write_package(
            &app,
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nremote = { git = \"https://example.com/r.git\" }\n",
        );
        lock(&[locked("remote", "", "local", None)]);
        assert!(
            error().contains("is not pinned to an exact revision"),
            "{}",
            error()
        );
        lock(&[locked(
            "remote",
            "",
            "0000000000000000000000000000000000000000",
            None,
        )]);
        assert!(error().contains("run `meshc deps`"), "{}", error());
        let checkout = app.join(".mesh/deps/remote");
        write_package(&checkout, &native_package("remote"));
        let repository = git2::Repository::init(&checkout).unwrap();
        let mut index = repository.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        let tree = repository.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("t", "t@example.com").unwrap();
        let head = repository
            .commit(Some("HEAD"), &signature, &signature, "c", &tree, &[])
            .unwrap();
        assert!(error().contains("revision mismatch"), "{}", error());
        lock(&[locked("remote", "", &head.to_string(), None)]);
        assert_eq!(resolve_native_bindings(&app).unwrap()[0].package, "remote");

        // A binding the package names must be a file in it.
        fs::remove_file(checkout.join("bindings/n.mpl")).unwrap();
        assert!(
            error().contains("does not exist or cannot be read"),
            "{}",
            error()
        );
        fs::create_dir(checkout.join("bindings/n.mpl")).unwrap();
        assert!(error().contains("is not a file"), "{}", error());

        // A checkout without a commit has no revision to compare.
        fs::remove_dir_all(&checkout).unwrap();
        write_package(&checkout, &native_package("remote"));
        git2::Repository::init(&checkout).unwrap();
        assert!(
            error().contains("Failed to read installed `remote` revision"),
            "{}",
            error()
        );
    }

    /// A native path names a file inside its package (`./` allowed, `..` not);
    /// a package two dependencies share resolves once; a path dependency
    /// that is not there is an error.
    #[test]
    fn native_paths_stay_inside_their_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let package = |name: &str, binding: &str, dependencies: &str| {
            let dir = root.join(name);
            fs::create_dir_all(dir.join("bindings")).unwrap();
            fs::write(
                dir.join("bindings/n.mpl"),
                "pub fn n() -> Int do\n  1\nend\n",
            )
            .unwrap();
            fs::write(
                dir.join("mesh.toml"),
                format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\n\n[native]\nabi = 1\nbindings = [\"{binding}\"]\n\n[dependencies]\n{dependencies}"),
            )
            .unwrap();
            dir
        };
        let app = package("app", "./bindings/n.mpl", "");
        assert_eq!(resolve_native_bindings(&app).unwrap().len(), 1);
        // The manifest refuses a path out of the package; the file check does
        // too, should a path reach it some other way.
        package("app", "bindings/../../outside.mpl", "");
        let error = resolve_native_bindings(&app).unwrap_err();
        assert!(
            error.contains("must stay within the package root"),
            "{error}"
        );
        fs::write(root.join("outside.mpl"), "pub fn o() -> Int do\n  1\nend\n").unwrap();
        let error =
            checked_package_file(&app, Path::new("../outside.mpl"), "native binding").unwrap_err();
        assert!(
            error.contains("must be a path inside its package"),
            "{error}"
        );

        package("shared", "bindings/n.mpl", "");
        package(
            "left",
            "bindings/n.mpl",
            "shared = { path = \"../shared\" }\n",
        );
        package(
            "right",
            "bindings/n.mpl",
            "shared = { path = \"../shared\" }\n",
        );
        package(
            "app",
            "bindings/n.mpl",
            "left = { path = \"../left\" }\nright = { path = \"../right\" }\n",
        );
        let packages: Vec<String> = resolve_native_bindings(&app)
            .unwrap()
            .into_iter()
            .map(|binding| binding.package)
            .collect();
        assert_eq!(packages, ["app", "left", "shared", "right"]);

        package("app", "bindings/n.mpl", "gone = { path = \"../gone\" }\n");
        let error = resolve_native_bindings(&app).unwrap_err();
        assert!(
            error.contains("Failed to resolve path dependency `gone`"),
            "{error}"
        );
    }
}
