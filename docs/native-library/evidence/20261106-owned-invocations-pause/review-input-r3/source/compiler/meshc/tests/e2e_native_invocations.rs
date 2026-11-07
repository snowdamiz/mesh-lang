//! Direct generated companions: actual held HTTP and all retained authority cases.
#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
#[path = "support/test_artifacts.rs"]
mod artifacts;

fn build(fixture: &str, directory: &Path, name: &str) -> PathBuf {
    let library = directory.join(format!(
        "lib{name}.{}",
        if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        }
    ));
    let output = Command::new(artifacts::meshc_bin())
        .arg("build")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture),
        )
        .args([
            "--artifact",
            "cdylib",
            "--opt-level",
            "0",
            "--no-color",
            "-o",
        ])
        .arg(&library)
        .output()
        .unwrap();
    artifacts::write_artifact(
        &directory.join(format!("{name}-build.log")),
        artifacts::command_output_text(&output),
    );
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    library
}

fn run(mut command: Command, log: &Path) {
    let child = command
        .env("MESH_SCHEDULER_MIN_WORKERS", "1")
        .env("MESH_SCHEDULER_MAX_WORKERS", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = artifacts::wait_with_timeout(child, Duration::from_secs(45)).unwrap();
    artifacts::write_artifact(log, artifacts::command_output_text(&output));
    assert!(
        output.status.success(),
        "{}: {}",
        log.display(),
        artifacts::command_output_text(&output)
    );
}

#[test]
fn generated_owned_invocations_progress_and_preserve_original_effect_authority() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let directory = artifacts::artifact_dir("native-owned-invocations", "generated-abi");
    let progress = build("library_invocations", &directory, "progress");
    for (name, error) in [("success", false), ("error", true)] {
        let mut command = Command::new("python3");
        command
            .arg(fixtures.join("library_invocations/check.py"))
            .arg(&progress)
            .arg(directory.join(name));
        if error {
            command.arg("--error");
        }
        run(command, &directory.join(format!("progress-{name}.log")));
    }
    for (name, includes) in [
        (
            "legacy-first",
            "#include \"libprogress.h\"\n#include \"libprogress.invocations.h\"\n",
        ),
        (
            "extension-first",
            "#include \"libprogress.invocations.h\"\n#include \"libprogress.h\"\n",
        ),
    ] {
        let source = directory.join(format!("{name}.c"));
        std::fs::write(&source, includes).unwrap();
        for language in ["c", "c++"] {
            let mut command = Command::new("cc");
            command
                .args(["-Werror", "-fsyntax-only", "-x", language])
                .arg(&source);
            run(command, &directory.join(format!("{name}-{language}.log")));
        }
    }
    let bounds = build("library", &directory, "mesh_library");
    let mut command = Command::new("python3");
    command
        .arg(fixtures.join("library_invocations/bounds.py"))
        .arg(bounds);
    run(command, &directory.join("bounds-state.log"));
    let authority = build("library_authority", &directory, "authority");
    let probe = directory.join("stack-probe.so");
    let mut command = Command::new("cc");
    command
        .arg(fixtures.join("library_authority/stack_probe.c"))
        .arg("-I")
        .arg(&directory)
        .args(["-shared", "-fPIC", "-pthread", "-o"])
        .arg(&probe);
    run(command, &directory.join("stack-probe-build.log"));
    for scheduled in [false, true] {
        for case in [
            "control",
            "write",
            "query-write",
            "schema",
            "commit",
            "raw-commit",
            "release",
            "accepted-commit",
            "http",
            "store",
            "accepted-store",
            "nonce",
            "accepted-nonce",
            "actor",
            "timer",
            "missing",
            "error",
            "unsupported",
            "generations",
            "registration",
            "hot-journal",
            "terminate",
            "v1-6",
            "v1-8",
            "v1-output",
            "v1-missing",
            "ancestry",
            "gc",
            "panic",
            "request-refusal",
            "stream-refusal",
        ] {
            let mut command = Command::new("python3");
            command
                .arg(fixtures.join("library_authority/check.py"))
                .arg(&authority)
                .arg(case)
                .arg(format!("--stack-probe={}", probe.display()));
            if scheduled {
                command.arg("--scheduled");
            }
            run(
                command,
                &directory.join(format!(
                    "{}-{case}.log",
                    if scheduled { "scheduled" } else { "legacy" }
                )),
            );
        }
    }
}

#[test]
fn generated_companion_names_cannot_alias_legacy_or_runtime_symbols() {
    let directory = artifacts::artifact_dir("native-owned-invocations", "symbol-collisions");
    for (name, first, second) in [
        ("companion", "export_one", "export_one_submit_v2"),
        ("entry", "export_one", "__mesh_library_entry_export_one"),
        ("runtime", "export_one", "mesh_library_invocation_take_v2"),
    ] {
        let fixture = directory.join(name);
        std::fs::create_dir(&fixture).unwrap();
        std::fs::write(
            fixture.join("mesh.toml"),
            "[package]\nname = \"collision\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        std::fs::write(fixture.join("main.mpl"), format!(
            "@export(\"{first}\")\npub fn first(request :: Bytes) -> Bytes!String do Ok(request) end\n@export(\"{second}\")\npub fn second(request :: Bytes) -> Bytes!String do Ok(request) end\n"
        )).unwrap();
        let output = Command::new(artifacts::meshc_bin())
            .arg("build")
            .arg(&fixture)
            .args([
                "--artifact",
                "cdylib",
                "--opt-level",
                "0",
                "--no-color",
                "-o",
            ])
            .arg(fixture.join("libcollision.so"))
            .output()
            .unwrap();
        let text = artifacts::command_output_text(&output);
        artifacts::write_artifact(&fixture.join("build.log"), &text);
        let expected = if name == "runtime" {
            "exported symbol is reserved"
        } else {
            "conflicts with another generated or runtime symbol"
        };
        assert!(
            !output.status.success() && text.contains(expected),
            "{text}"
        );
    }
}
