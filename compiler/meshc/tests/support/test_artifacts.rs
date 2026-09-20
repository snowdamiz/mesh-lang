#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::any::Any;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuildOutputMetadata {
    pub source_package_dir: PathBuf,
    pub binary_path: PathBuf,
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("meshc should live under compiler/")
        .parent()
        .expect("workspace root should contain compiler/")
        .to_path_buf()
}

pub fn meshc_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_meshc"))
}

pub fn ensure_mesh_rt_staticlib() {
    static BUILD_ONCE: OnceLock<()> = OnceLock::new();
    BUILD_ONCE.get_or_init(|| {
        let output = Command::new("cargo")
            .current_dir(repo_root())
            .args(["build", "-p", "mesh-rt"])
            .output()
            .expect("failed to invoke cargo build -p mesh-rt");
        assert!(
            output.status.success(),
            "cargo build -p mesh-rt failed:\n{}",
            command_output_text(&output)
        );
    });
}

pub fn command_output_text(output: &Output) -> String {
    format!(
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

pub fn panic_payload_to_string(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

pub fn artifact_dir(bucket: &str, test_name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    let dir = repo_root()
        .join(".tmp")
        .join(bucket)
        .join(format!("{test_name}-{stamp}"));
    fs::create_dir_all(&dir).expect("failed to create e2e artifact dir");
    dir
}

pub fn write_artifact(path: &Path, contents: impl AsRef<str>) {
    fs::write(path, contents.as_ref())
        .unwrap_or_else(|error| panic!("failed to write artifact {}: {error}", path.display()));
}

pub fn write_json_artifact(path: &Path, value: &impl Serialize) {
    write_artifact(
        path,
        serde_json::to_string_pretty(value).expect("json pretty print failed"),
    );
}

pub fn archive_directory_tree(source_dir: &Path, artifact_dir: &Path) {
    assert!(
        source_dir.is_dir(),
        "expected {} to be a directory before archiving",
        source_dir.display()
    );
    fs::create_dir_all(artifact_dir).unwrap_or_else(|error| {
        panic!(
            "failed to create archive directory {}: {error}",
            artifact_dir.display()
        )
    });

    for entry in fs::read_dir(source_dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", source_dir.display()))
    {
        let entry = entry
            .unwrap_or_else(|error| panic!("failed to iterate {}: {error}", source_dir.display()));
        let source_path = entry.path();
        let artifact_path = artifact_dir.join(entry.file_name());
        if source_path.is_dir() {
            archive_directory_tree(&source_path, &artifact_path);
        } else {
            if let Some(parent) = artifact_path.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|error| {
                    panic!("failed to create {}: {error}", parent.display())
                });
            }
            fs::copy(&source_path, &artifact_path).unwrap_or_else(|error| {
                panic!(
                    "failed to archive {} -> {}: {error}",
                    source_path.display(),
                    artifact_path.display()
                )
            });
        }
    }
}
