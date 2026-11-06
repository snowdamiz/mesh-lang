//! Deterministic source audit, collected before MIR reachability pruning.
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize)]
pub(crate) struct PlaintextReport {
    schema_version: u8,
    policy: &'static str,
    compiler_version: &'static str,
    modules: Vec<ModuleReport>,
}

#[derive(Serialize)]
struct ModuleReport {
    module: String,
    file: String,
    source_sha256: String,
    sites: Vec<SiteReport>,
}

#[derive(Serialize)]
struct SiteReport {
    kind: String,
    reason: String,
    start: usize,
    end: usize,
    line: usize,
    column: usize,
}

impl PlaintextReport {
    pub(crate) fn collect(
        root: &Path,
        project: &mesh_pkg::project::ProjectData,
        checks: &[mesh_typeck::TypeckResult],
    ) -> Self {
        let mut modules = Vec::new();
        for id in &project.compilation_order {
            let index = id.0 as usize;
            let module = project.graph.get(*id);
            let source = &project.module_sources[index];
            let file = if module.path.is_absolute() {
                module
                    .path
                    .strip_prefix(root)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|_| {
                        PathBuf::from("dependency")
                            .join(&module.name)
                            .join(module.path.file_name().unwrap_or_default())
                    })
            } else {
                module.path.clone()
            };
            let mut sites = checks[index]
                .plaintext
                .sites
                .iter()
                .map(|site| {
                    let start = u32::from(site.span.start()) as usize;
                    let end = u32::from(site.span.end()) as usize;
                    let prefix = &source[..start];
                    SiteReport {
                        kind: site.kind.clone(),
                        reason: site.reason.clone(),
                        start,
                        end,
                        line: prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
                        column: prefix
                            .rsplit('\n')
                            .next()
                            .unwrap_or_default()
                            .chars()
                            .count()
                            + 1,
                    }
                })
                .collect::<Vec<_>>();
            sites.sort_by(|a, b| {
                (a.start, a.end, &a.kind, &a.reason).cmp(&(b.start, b.end, &b.kind, &b.reason))
            });
            modules.push(ModuleReport {
                module: module.name.clone(),
                file: file.to_string_lossy().replace('\\', "/"),
                source_sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
                sites,
            });
        }
        modules.sort_by(|a, b| (&a.module, &a.file).cmp(&(&b.module, &b.file)));
        Self {
            schema_version: 1,
            policy: "plaintext-v1",
            compiler_version: env!("CARGO_PKG_VERSION"),
            modules,
        }
    }

    pub(crate) fn write(&self, path: &Path) -> Result<(), String> {
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("plaintext report encode failed: {error}"))?;
        bytes.push(b'\n');
        std::fs::write(path, bytes)
            .map_err(|error| format!("plaintext report write '{}': {error}", path.display()))
    }
}

pub(crate) fn default_path(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_os_string();
    path.push(".plaintext.json");
    PathBuf::from(path)
}

pub(crate) fn validate_path(report: &Path, output: &Path) -> Result<(), String> {
    fn destination(path: &Path) -> Option<PathBuf> {
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Some(parent.canonicalize().ok()?.join(path.file_name()?))
    }
    if report == output
        || destination(report)
            .zip(destination(output))
            .is_some_and(|(report, output)| report == output)
        || report
            .canonicalize()
            .ok()
            .zip(output.canonicalize().ok())
            .is_some_and(|(report, output)| report == output)
    {
        return Err(
            "plaintext report must have a different path from the compiled artifact".into(),
        );
    }
    if matches!(
        report.extension().and_then(|extension| extension.to_str()),
        Some("mpl" | "toml")
    ) {
        return Err("plaintext report cannot overwrite Mesh source or a package manifest".into());
    }
    Ok(())
}
