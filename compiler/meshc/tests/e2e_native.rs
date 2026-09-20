#![cfg(unix)]

use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn meshc_bin() -> PathBuf {
    let mut path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join("meshc")
}

#[test]
fn package_native_binding_links_and_runs() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("native-project");
    let package = temp.path().join("native-math");
    let native_dir = package.join("native");
    fs::create_dir_all(project.as_path()).unwrap();
    fs::create_dir_all(package.join("bindings")).unwrap();
    fs::create_dir_all(&native_dir).unwrap();

    fs::write(
        native_dir.join("math.c"),
        r#"#include <stdint.h>
#include <string.h>

typedef struct {
    uint8_t tag;
    void *value;
} MeshResult;

void *mesh_gc_alloc_actor(uint64_t size, uint64_t align);
void *mesh_string_new(const uint8_t *data, uint64_t length);

int64_t mesh_math_add(int64_t left, int64_t right) {
    return left + right;
}

MeshResult mesh_math_double(int64_t value) {
    if (value < 0) {
        const char *message = "negative";
        return (MeshResult){1, mesh_string_new((const uint8_t *)message, strlen(message))};
    }
    int64_t *result = mesh_gc_alloc_actor(sizeof(int64_t), _Alignof(int64_t));
    *result = value * 2;
    return (MeshResult){0, result};
}
"#,
    )
    .unwrap();
    assert!(Command::new("cc")
        .args(["-c", "math.c", "-o", "math.o"])
        .current_dir(&native_dir)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("ar")
        .args(["rcs", "libmath.a", "math.o"])
        .current_dir(&native_dir)
        .status()
        .unwrap()
        .success());

    let archive = fs::read(native_dir.join("libmath.a")).unwrap();
    let sha256 = format!("{:x}", Sha256::digest(&archive));
    let target = mesh_codegen::link::effective_target_triple(None).unwrap();
    fs::write(
        package.join("mesh.toml"),
        format!(
            r#"[package]
name = "native-math"
version = "0.1.0"

[native]
abi = 1
bindings = ["bindings/math.mpl"]

[[native.libraries]]
target = "{target}"
path = "native/libmath.a"
sha256 = "{sha256}"
"#
        ),
    )
    .unwrap();
    fs::write(
        package.join("bindings/math.mpl"),
        "@native(\"mesh_math_add\")\npub fn add(left :: Int, right :: Int) -> Int\n\n@native(\"mesh_math_double\")\npub fn double(value :: Int) -> Int!String\n",
    )
    .unwrap();
    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"native-project\"\nversion = \"0.1.0\"\n\n[dependencies]\nnative-math = { path = \"../native-math\" }\n",
    )
    .unwrap();
    fs::write(
        project.join("main.mpl"),
        r##"from Bindings.Math import add, double

fn print_double(value :: Int) do
  case double(value) do
    Ok(result) -> println("#{result}")
    Err(error) -> println(error)
  end
end

fn main() do
  20 |> add(1) |> print_double()
  print_double(-1)
end
"##,
    )
    .unwrap();

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("native-project"))
        .output()
        .unwrap();
    assert!(run.status.success(), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "42\nnegative\n");

    fs::write(
        project.join("main.mpl"),
        "@native(\"mesh_math_add\")\npub fn undeclared(left :: Int, right :: Int) -> Int\n\nfn main() -> Int do\n  undeclared(1, 2)\nend\n",
    )
    .unwrap();
    let rejected = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("outside a manifest-declared native binding"),
        "unexpected compiler error:\n{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
}
