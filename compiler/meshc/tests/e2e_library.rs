use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[test]
#[cfg(unix)]
fn builds_hosted_dynamic_and_static_libraries() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library");
    let temp = tempfile::tempdir().expect("temporary artifact directory");
    let dynamic = temp.path().join(if cfg!(target_os = "macos") {
        "libmesh_library.dylib"
    } else {
        "libmesh_library.so"
    });

    assert_success(build(&fixture, &dynamic, "cdylib"), "dynamic build");
    for extension in ["h", "swift", "kt", "jni.c", "ts", "abi.json"] {
        assert!(
            dynamic.with_extension(extension).is_file(),
            "missing generated {extension} binding"
        );
    }

    let host = temp.path().join("mesh-library-host");
    let mut cc = Command::new("cc");
    cc.arg(fixture.join("host.c"))
        .arg("-I")
        .arg(temp.path())
        .arg("-L")
        .arg(temp.path())
        .arg("-lmesh_library")
        .arg(format!("-Wl,-rpath,{}", temp.path().display()))
        .arg("-o")
        .arg(&host);
    if cfg!(target_os = "macos") {
        cc.args(["-framework", "Security", "-framework", "CoreFoundation"]);
    }
    assert_success(cc.output().expect("C host compiler"), "C host link");
    let run = Command::new(&host).output().expect("C host run");
    assert_success(run, "C host run");

    let static_library = temp.path().join("libmesh_library.a");
    assert_success(
        build(&fixture, &static_library, "staticlib"),
        "static build",
    );
    assert!(
        static_library.metadata().expect("static artifact").len() > 0,
        "static library is empty"
    );
}

fn build(fixture: &Path, output: &Path, artifact: &str) -> Output {
    Command::new(PathBuf::from(env!("CARGO_BIN_EXE_meshc")))
        .args(["build", fixture.to_str().unwrap(), "--artifact", artifact])
        .arg("--output")
        .arg(output)
        .output()
        .expect("meshc build")
}

fn assert_success(output: Output, operation: &str) {
    assert!(
        output.status.success(),
        "{operation} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
