//! Real compiled recovery-code ABI and affine ownership checks.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "support/test_artifacts.rs"]
mod artifacts;

fn compile(
    compiler: &Path,
    project: &Path,
    optimization: &str,
    binary: &Path,
) -> std::process::Output {
    let child = Command::new(compiler)
        .args(["build", "--opt-level", optimization, "--output"])
        .arg(binary)
        .arg(project)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    artifacts::wait_with_timeout(child, Duration::from_secs(120)).unwrap()
}

#[test]
fn recovery_code_compiles_and_restores_after_original_owner_is_destroyed() {
    let compiler = artifacts::meshc_bin();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/e2e/recovery_code.mpl");
    for optimization in ["0", "2"] {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("recovery-code");
        fs::create_dir(&project).unwrap();
        fs::copy(&fixture, project.join("main.mpl")).unwrap();
        let binary = project.join(if cfg!(windows) { "proof.exe" } else { "proof" });
        let build = compile(&compiler, &project, optimization, &binary);
        assert!(
            build.status.success(),
            "{}",
            artifacts::command_output_text(&build)
        );
        let child = Command::new(&binary)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let run = artifacts::wait_with_timeout(child, Duration::from_secs(30)).unwrap();
        assert!(run.status.success(), "compiled recovery program failed");
        // A failure diagnostic must not echo a randomly generated recovery code.
        assert!(
            run.stderr.is_empty(),
            "unexpected recovery program diagnostic"
        );
        assert!(
            run.stdout == b"recovery-code:ok\n",
            "recovery agreement failed"
        );
    }
}

#[test]
fn recovery_code_generated_and_imported_secrets_cannot_be_used_after_destroy() {
    let compiler = artifacts::meshc_bin();
    for initializer in [
        "let (key, code) = RecoveryCode.generate()?",
        "let key = RecoveryCode.parse(\"MRC1-00010203-04050607-08090A0B-0C0D0E0F-10111213-14151617-18191A1B-1C1D1E1F-3A37D678\")?",
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("invalid-recovery-owner");
        fs::create_dir(&project).unwrap();
        fs::write(project.join("main.mpl"), format!(
            "fn misuse() -> Bytes!CryptoError do\n  {initializer}\n  Secret.destroy(key)\n  Crypto.hmac_sha256_tag(key, Bytes.from_utf8(\"public\"))\nend\nfn main() do nil end\n"
        )).unwrap();
        let binary = project.join("must-not-exist");
        let build = compile(&compiler, &project, "2", &binary);
        assert!(!build.status.success(), "use after destroy compiled");
        assert!(!binary.exists(), "invalid ownership produced an executable");
        assert!(artifacts::command_output_text(&build).contains("E0053"),
            "expected ownership diagnostic: {}", artifacts::command_output_text(&build));
    }
}
