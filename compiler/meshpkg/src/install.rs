use std::io::Read as IoRead;
use std::path::Path;

use colored::Colorize;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive;

use mesh_pkg::{LockedPackage, Lockfile, Manifest};

const LOCKFILE_NAME: &str = "mesh.lock";

fn dependency_declaration_snippet(name: &str, version: &str) -> String {
    format!("\"{}\" = \"{}\"", name, version)
}

fn named_install_follow_up(name: &str, version: &str) -> String {
    format!(
        "Named install does not edit mesh.toml; add {} if you want to declare this dependency.",
        dependency_declaration_snippet(name, version)
    )
}

fn named_install_result_json(name: &str, version: &str) -> String {
    serde_json::json!({
        "status": "ok",
        "name": name,
        "version": version,
        "lockfile": LOCKFILE_NAME,
        "manifest_changed": false,
    })
    .to_string()
}

pub fn run(
    project_dir: &Path,
    package_name: Option<&str>,
    registry: &str,
    json_mode: bool,
) -> Result<(), String> {
    match package_name {
        Some(name) => install_named(project_dir, name, registry, json_mode),
        None => install_all(project_dir, registry, json_mode),
    }
}

/// Install all registry dependencies declared in mesh.toml.
/// Uses mesh.lock exact pins if it exists; otherwise resolves from registry.
fn install_all(project_dir: &Path, registry: &str, json_mode: bool) -> Result<(), String> {
    let manifest_path = project_dir.join("mesh.toml");
    let manifest = Manifest::from_file(&manifest_path)?;

    let lock_path = project_dir.join(LOCKFILE_NAME);
    let existing_lock = if lock_path.exists() {
        Some(Lockfile::read(&lock_path)?)
    } else {
        None
    };

    let mut locked_packages: Vec<LockedPackage> = Vec::new();

    for (name, dep) in &manifest.dependencies {
        let version = match dep.registry_version() {
            Some(v) => v,
            None => continue, // skip git/path deps (handled by meshc deps)
        };

        // Versions are exact: a lock entry for another version is stale.
        let pinned = existing_lock.as_ref().and_then(|lock| {
            lock.packages.iter().find_map(|p| {
                (p.name == *name && p.version == version)
                    .then(|| p.sha256.clone())
                    .flatten()
            })
        });
        let sha256 = match pinned {
            Some(sha256) => sha256,
            None => resolve_version(name, version, registry)?,
        };
        let locked = fetch_package(project_dir, name, version, &sha256, registry, json_mode)?;
        locked_packages.push(locked);

        if !json_mode {
            println!("{} Installed {}@{}", "✓".green().bold(), name, version);
        }
    }

    // Write lockfile
    if !locked_packages.is_empty() || existing_lock.is_none() {
        // Merge with existing non-registry entries if lockfile existed
        let mut all_packages = locked_packages;
        if let Some(ref lock) = existing_lock {
            for pkg in &lock.packages {
                // Keep existing non-registry (git/path) entries
                if pkg.sha256.is_none() && !all_packages.iter().any(|p| p.name == pkg.name) {
                    all_packages.push(pkg.clone());
                }
            }
        }
        let lockfile = Lockfile::new(all_packages);
        lockfile.write(&lock_path)?;

        if json_mode {
            println!(
                "{}",
                serde_json::json!({"status": "ok", "lockfile": LOCKFILE_NAME})
            );
        } else {
            println!("{} Updated {}", "✓".green().bold(), LOCKFILE_NAME);
        }
    }

    Ok(())
}

/// Fetch and lock the latest release of a single named package.
/// Named install does not edit mesh.toml; it only downloads the package and updates mesh.lock.
fn install_named(
    project_dir: &Path,
    name: &str,
    registry: &str,
    json_mode: bool,
) -> Result<(), String> {
    let (version, sha256) = resolve_latest(name, registry)?;
    let locked = fetch_package(project_dir, name, &version, &sha256, registry, json_mode)?;

    // Update mesh.lock
    let lock_path = project_dir.join(LOCKFILE_NAME);
    let mut packages = if lock_path.exists() {
        Lockfile::read(&lock_path)?.packages
    } else {
        Vec::new()
    };

    // Replace or add the entry
    packages.retain(|p| p.name != name);
    packages.push(locked);
    Lockfile::new(packages).write(&lock_path)?;

    if json_mode {
        println!("{}", named_install_result_json(name, &version));
    } else {
        println!("{} Installed {}@{}", "✓".green().bold(), name, version);
        println!("  Updated {}", LOCKFILE_NAME);
        println!("  {}", named_install_follow_up(name, &version));
    }

    Ok(())
}

/// Download a package tarball from the registry. Returns (bytes, sha256_hex).
fn download_tarball(
    name: &str,
    version: &str,
    registry: &str,
) -> Result<(Vec<u8>, String), String> {
    let url = format!("{}/api/v1/packages/{}/{}/download", registry, name, version);
    let agent = ureq::Agent::new_with_defaults();
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|e| format!("Failed to download {}@{}: {}", name, version, e))?;

    let mut buf = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    let sha256: String = Sha256::digest(&buf)
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    Ok((buf, sha256))
}

/// Download `name@version`, check it is the release `sha256` pins, and
/// unpack it: its lock entry.
fn fetch_package(
    project_dir: &Path,
    name: &str,
    version: &str,
    sha256: &str,
    registry: &str,
    json_mode: bool,
) -> Result<LockedPackage, String> {
    let msg = format!("Downloading {}@{}...", name, version);
    let (tarball_bytes, actual_sha256) = crate::publish::with_spinner(&msg, json_mode, || {
        download_tarball(name, version, registry)
    })?;
    if sha256 != actual_sha256 {
        return Err(format!(
            "SHA-256 mismatch for {}@{}: expected {}, got {}",
            name, version, sha256, actual_sha256
        ));
    }
    extract_tarball(
        &tarball_bytes,
        &package_install_dir(project_dir, name, version)?,
    )?;
    Ok(LockedPackage {
        name: name.to_string(),
        version: version.to_string(),
        source: format!("{registry}/api/v1/packages/{name}/{version}/download"),
        revision: version.to_string(),
        sha256: Some(actual_sha256),
    })
}

/// The registry's answer about `package` (a name, or `name/version`).
fn registry_json(registry: &str, package: &str) -> Result<serde_json::Value, String> {
    let url = format!("{registry}/api/v1/packages/{package}");
    let body = ureq::Agent::new_with_defaults()
        .get(&url)
        .call()
        .and_then(|mut response| response.body_mut().read_to_string())
        .map_err(|e| format!("Failed to query registry for {package}: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("Failed to parse registry response: {e}"))
}

/// The string at `pointer` in a registry answer about `package`.
fn registry_field(
    json: &serde_json::Value,
    pointer: &str,
    package: &str,
) -> Result<String, String> {
    json.pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("Registry response missing {} for {package}", &pointer[1..]))
}

/// The checksum of an exact version (the only constraint there is).
fn resolve_version(name: &str, version: &str, registry: &str) -> Result<String, String> {
    let json = registry_json(registry, &format!("{name}/{version}"))?;
    registry_field(&json, "/sha256", &format!("{name}@{version}"))
}

/// The latest release of a package: its version and checksum.
fn resolve_latest(name: &str, registry: &str) -> Result<(String, String), String> {
    let json = registry_json(registry, name)?;
    Ok((
        registry_field(&json, "/latest/version", name)?,
        registry_field(&json, "/latest/sha256", name)?,
    ))
}

/// Create `.mesh/packages/<name>@<version>/` and remove any other installed
/// version of the package: a build compiles every installed package, and two
/// versions of one would define the same modules twice.
fn package_install_dir(
    project_dir: &Path,
    name: &str,
    version: &str,
) -> Result<std::path::PathBuf, String> {
    let install_dir = project_dir
        .join(".mesh")
        .join("packages")
        .join(format!("{}@{}", name, version));
    let parent = install_dir
        .parent()
        .expect("package directory has a parent");
    let leaf = name.rsplit('/').next().unwrap_or(name);
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if file_name.starts_with(&format!("{leaf}@")) && entry.path() != install_dir {
                std::fs::remove_dir_all(entry.path())
                    .map_err(|e| format!("Failed to remove {}: {}", entry.path().display(), e))?;
            }
        }
    }
    std::fs::create_dir_all(&install_dir)
        .map_err(|e| format!("Failed to create {}: {}", install_dir.display(), e))?;
    Ok(install_dir)
}

/// Extract a .tar.gz tarball to the given directory.
fn extract_tarball(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let dec = GzDecoder::new(bytes);
    let mut archive = Archive::new(dec);
    archive
        .unpack(dest)
        .map_err(|e| format!("Failed to extract package to {}: {}", dest.display(), e))
}

#[cfg(test)]
mod tests {
    use super::{
        dependency_declaration_snippet, named_install_follow_up, named_install_result_json,
        package_install_dir,
    };

    #[test]
    fn installing_a_version_removes_the_other_installed_versions() {
        let project = tempfile::tempdir().unwrap();
        let packages = project.path().join(".mesh/packages");
        for dir in [
            "acme/widget@1.0.0",
            "acme/widget-extra@1.0.0",
            "widget@1.0.0",
        ] {
            std::fs::create_dir_all(packages.join(dir)).unwrap();
        }

        let dir = package_install_dir(project.path(), "acme/widget", "1.1.0").unwrap();

        assert_eq!(dir, packages.join("acme/widget@1.1.0"));
        assert!(dir.is_dir());
        assert!(!packages.join("acme/widget@1.0.0").exists());
        assert!(packages.join("acme/widget-extra@1.0.0").is_dir());
        assert!(packages.join("widget@1.0.0").is_dir());
    }

    #[test]
    fn named_install_json_reports_lockfile_and_manifest_stability() {
        let json: serde_json::Value =
            serde_json::from_str(&named_install_result_json("acme/widget", "1.2.3"))
                .expect("named install JSON should parse");

        assert_eq!(json["status"], "ok");
        assert_eq!(json["name"], "acme/widget");
        assert_eq!(json["version"], "1.2.3");
        assert_eq!(json["lockfile"], "mesh.lock");
        assert_eq!(json["manifest_changed"], false);
    }

    #[test]
    fn named_install_follow_up_quotes_scoped_dependency_keys() {
        let declaration = dependency_declaration_snippet("acme/widget", "1.2.3");
        let follow_up = named_install_follow_up("acme/widget", "1.2.3");

        assert_eq!(declaration, "\"acme/widget\" = \"1.2.3\"");
        assert!(follow_up.contains("does not edit mesh.toml"));
        assert!(follow_up.contains(&declaration));
    }
}
