//! End-to-end integration tests for `meshc fmt`.

use std::path::PathBuf;
use std::process::Command;

fn find_meshc() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("cannot find current exe")
        .parent()
        .expect("cannot find parent dir")
        .to_path_buf();
    // Walk up from deps dir to the debug dir.
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("meshc")
}

#[test]
fn fmt_formats_single_file_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.mpl");
    std::fs::write(&file, "fn add(a,b) do\na+b\nend").unwrap();

    let output = Command::new(find_meshc())
        .args(["fmt", file.to_str().unwrap()])
        .output()
        .expect("failed to run meshc fmt");

    assert!(
        output.status.success(),
        "meshc fmt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let contents = std::fs::read_to_string(&file).unwrap();
    assert_eq!(contents, "fn add(a, b) do\n  a + b\nend\n");
}

#[test]
fn fmt_already_formatted_file_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("good.mpl");
    let canonical = "fn add(a, b) do\n  a + b\nend\n";
    std::fs::write(&file, canonical).unwrap();

    let _mtime_before = std::fs::metadata(&file).unwrap().modified().unwrap();

    let output = Command::new(find_meshc())
        .args(["fmt", file.to_str().unwrap()])
        .output()
        .expect("failed to run meshc fmt");

    assert!(output.status.success());

    let contents = std::fs::read_to_string(&file).unwrap();
    assert_eq!(contents, canonical, "File should remain unchanged");

    // The file should not have been rewritten (content identical, skip write).
    // Note: mtime might still match due to OS granularity, so we only check content.
}

#[test]
fn fmt_check_exits_1_on_unformatted() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bad.mpl");
    std::fs::write(&file, "fn bad(a,b) do\na+b\nend").unwrap();

    let output = Command::new(find_meshc())
        .args(["fmt", "--check", file.to_str().unwrap()])
        .output()
        .expect("failed to run meshc fmt --check");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for unformatted file"
    );

    // File should NOT be modified in check mode.
    let contents = std::fs::read_to_string(&file).unwrap();
    assert_eq!(contents, "fn bad(a,b) do\na+b\nend");
}

#[test]
fn fmt_check_exits_0_on_formatted() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("good.mpl");
    std::fs::write(&file, "fn add(a, b) do\n  a + b\nend\n").unwrap();

    let output = Command::new(find_meshc())
        .args(["fmt", "--check", file.to_str().unwrap()])
        .output()
        .expect("failed to run meshc fmt --check");

    assert!(
        output.status.success(),
        "Expected exit 0 for formatted file, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fmt_directory_formats_all_mesh_files() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();

    std::fs::write(dir.path().join("a.mpl"), "let x=1").unwrap();
    std::fs::write(sub.join("b.mpl"), "let y=2").unwrap();
    // Non-.mpl file should be ignored.
    std::fs::write(dir.path().join("readme.txt"), "hello").unwrap();

    let output = Command::new(find_meshc())
        .args(["fmt", dir.path().to_str().unwrap()])
        .output()
        .expect("failed to run meshc fmt on directory");

    assert!(
        output.status.success(),
        "meshc fmt dir failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let a = std::fs::read_to_string(dir.path().join("a.mpl")).unwrap();
    assert_eq!(a, "let x = 1\n");

    let b = std::fs::read_to_string(sub.join("b.mpl")).unwrap();
    assert_eq!(b, "let y = 2\n");

    // Non-.mpl file should be untouched.
    let readme = std::fs::read_to_string(dir.path().join("readme.txt")).unwrap();
    assert_eq!(readme, "hello");
}

#[test]
fn fmt_custom_line_width_and_indent_size() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.mpl");
    std::fs::write(&file, "fn foo(x) do\nlet y = x\ny\nend").unwrap();

    let output = Command::new(find_meshc())
        .args([
            "fmt",
            "--indent-size",
            "4",
            "--line-width",
            "80",
            file.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run meshc fmt with options");

    assert!(output.status.success());

    let contents = std::fs::read_to_string(&file).unwrap();
    // With indent-size 4, body should be indented by 4 spaces.
    assert!(
        contents.contains("    let y = x"),
        "Expected 4-space indent, got:\n{}",
        contents
    );
}
