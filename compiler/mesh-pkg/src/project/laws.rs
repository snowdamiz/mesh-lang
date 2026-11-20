//! Law discovery shared by verification and source packaging.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::manifest::Manifest;

pub struct LawFiles {
    /// Sorted, unique, package-relative roots. The default is `.`.
    pub roots: Vec<PathBuf>,
    /// Sorted, unique, package-relative regular `.law` files.
    pub paths: Vec<PathBuf>,
}

pub fn discover(package: &Path, manifest: Option<&Manifest>) -> Result<LawFiles, String> {
    let roots = match manifest.and_then(|manifest| manifest.verify.law_roots.as_deref()) {
        None => BTreeSet::from([PathBuf::from(".")]),
        Some([]) => return Err("[verify].law_roots must contain at least one directory".into()),
        Some(roots) => roots
            .iter()
            .map(|root| checked_root(package, root))
            .collect::<Result<BTreeSet<_>, _>>()?,
    };

    let mut paths = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for root in &roots {
        // Use an empty relative path internally so descendants are canonical,
        // including when an explicit root overlaps the package root.
        let dir = if root == Path::new(".") {
            Path::new("")
        } else {
            root
        };
        walk(package, dir, &mut visited, &mut paths)?;
    }
    Ok(LawFiles {
        roots: roots.into_iter().collect(),
        paths: paths.into_iter().collect(),
    })
}

fn checked_root(package: &Path, text: &str) -> Result<PathBuf, String> {
    let fail = |reason| format!("Invalid [verify].law_roots directory `{text}`: {reason}");
    if text.trim().is_empty() || text.contains(['\\', ':']) {
        return Err(fail("expected a package-relative path using '/'"));
    }
    let mut relative = PathBuf::new();
    for component in Path::new(text).components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name)
                if !excluded(
                    name.to_string_lossy().as_ref(),
                    relative.as_os_str().is_empty(),
                ) =>
            {
                relative.push(name);
            }
            Component::Normal(_) => return Err(fail("hidden paths and build output are excluded")),
            _ => return Err(fail("expected a path inside the package")),
        }
        let dir = package.join(&relative);
        let metadata = fs::symlink_metadata(&dir)
            .map_err(|error| format!("{} ({error})", fail("directory could not be opened")))?;
        if metadata.is_symlink() {
            return Err(fail("symbolic links are excluded"));
        }
        if !metadata.is_dir() {
            return Err(fail("expected a directory"));
        }
        if super::is_package_boundary(&dir)
            .map_err(|error| format!("{}: {error}", dir.display()))?
        {
            return Err(fail("nested packages are excluded"));
        }
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Ok(relative)
}

fn excluded(name: &str, at_package_root: bool) -> bool {
    name.starts_with('.') || (at_package_root && matches!(name, "build" | "dist" | "target"))
}

fn walk(
    package: &Path,
    relative: &Path,
    visited: &mut BTreeSet<PathBuf>,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    if !visited.insert(relative.to_path_buf()) {
        return Ok(());
    }
    let dir = package.join(relative);
    let fail = |error| format!("Failed to discover laws in '{}': {error}", dir.display());
    let mut entries = fs::read_dir(&dir)
        .map_err(fail)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(fail)?;
    entries.sort_unstable_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let kind = entry.file_type().map_err(fail)?;
        if excluded(
            name.to_string_lossy().as_ref(),
            relative.as_os_str().is_empty() && (kind.is_dir() || kind.is_symlink()),
        ) {
            continue;
        }
        let path = relative.join(name);
        if kind.is_symlink() {
            return Err(format!(
                "Law discovery path '{}' must not be a symbolic link",
                entry.path().display()
            ));
        }
        if kind.is_dir() && !super::is_package_boundary(&entry.path()).map_err(fail)? {
            walk(package, &path, visited, paths)?;
        } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "law") {
            paths.insert(path);
        }
    }
    Ok(())
}
