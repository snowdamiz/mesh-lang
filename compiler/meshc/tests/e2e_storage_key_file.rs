//! Actual compiled Mesh persists BR1 resources across two native processes.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::{fs, process::Command};

#[path = "support/test_artifacts.rs"]
mod artifacts;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn file_storage_key_restores_br1_key_and_pending_inverse_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let project = root.join("file-storage-proof");
    fs::create_dir(&project).unwrap();
    let output = root.join("sealed-records");
    fs::create_dir(&output).unwrap();
    let key_directory = root.join("keys");
    let mut context = vec![1u8];
    context.extend_from_slice(&[7; 48]);
    context.extend_from_slice(&[0; 32]);
    context.extend_from_slice(&[8; 32]);
    context.extend_from_slice(&18u16.to_be_bytes());
    context.extend_from_slice(&1u64.to_be_bytes());
    let key_context = hex(&context);
    context[114] = 19;
    let source = fs::read_to_string(artifacts::repo_root().join("tests/e2e/storage_key_file.mpl"))
        .unwrap()
        .replace("__KEY_CONTEXT__", &key_context)
        .replace("__STATE_CONTEXT__", &hex(&context));
    fs::write(project.join("main.mpl"), source).unwrap();
    let build = Command::new(artifacts::meshc_bin())
        .args(["build"])
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        artifacts::command_output_text(&build)
    );
    for mode in ["write", "read"] {
        let result = Command::new(project.join("file-storage-proof"))
            .env("MESH_STORAGE_DIRECTORY", &key_directory)
            .env("MESH_STORAGE_OUTPUT", &output)
            .env("MESH_STORAGE_MODE", mode)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{mode}: {}",
            artifacts::command_output_text(&result)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "storage-file-restart:ok",
            "{mode}: {}",
            artifacts::command_output_text(&result)
        );
        assert!(
            result.stderr.is_empty(),
            "{mode}: {}",
            artifacts::command_output_text(&result)
        );
    }
}
