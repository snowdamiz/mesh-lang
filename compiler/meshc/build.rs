use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let guide = "../../tools/skill/mesh";
    let manifest_dir =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let package_version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let version = match git_revision(&manifest_dir.join("../..")) {
        Some(revision) => format!("{package_version} (git {revision})"),
        None => package_version,
    };
    println!("cargo:rustc-env=MESHC_LONG_VERSION={version}");

    let root = manifest_dir.join(guide);
    println!("cargo:rerun-if-changed={guide}");

    let index = fs::read_to_string(root.join("SKILL.md")).expect("read the Mesh skill index");
    assert!(index.starts_with("---\nname: mesh\ndescription: "));
    let mut files = vec!["SKILL.md".to_owned()];
    for entry in fs::read_dir(root.join("references")).expect("read the Mesh guide topics") {
        let entry = entry.expect("read guide directory entry");
        assert!(entry.file_type().unwrap().is_file());
        let name = entry.file_name().into_string().expect("UTF-8 topic name");
        let slug = name.strip_suffix(".md").expect("Markdown topic");
        assert!(!slug.is_empty() && slug.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'));
        let path = format!("references/{name}");
        assert!(
            index.contains(&format!("]({path})")),
            "unlisted topic {path}"
        );
        let body = fs::read_to_string(entry.path()).expect("read the Mesh guide topic");
        assert!(body.starts_with("# "), "expected a topic heading in {path}");
        files.push(path);
    }
    for (offset, _) in index.match_indices("](references/") {
        let path = index[offset + 2..].split(')').next().unwrap();
        assert!(
            files.iter().any(|file| file == path),
            "missing topic {path}"
        );
    }
    files.sort_unstable();

    let mut source = String::from("const FILES: &[(&str, &str)] = &[\n");
    for file in files {
        println!("cargo:rerun-if-changed={guide}/{file}");
        let path = format!("/{guide}/{file}");
        writeln!(
            source,
            "    ({file:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {path:?}))),"
        )
        .unwrap();
    }
    source.push_str("];\n");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    fs::write(output.join("guide_files.rs"), source).expect("write the embedded Mesh guide");
}

fn git_revision(workspace: &Path) -> Option<String> {
    let git_marker = workspace.join(".git");
    // A source archive may be unpacked inside a different Git repository.
    if git_marker.is_file() {
        println!("cargo:rerun-if-changed={}", git_marker.display());
    } else if !git_marker.is_dir() {
        return None;
    }

    // Ask Git for the paths so this also works with worktrees and submodules.
    // Watch the loose refs directory for branches that gain a loose ref after packing.
    let paths = git_output(
        workspace,
        &[
            "rev-parse",
            "--show-prefix",
            "--path-format=absolute",
            "--git-path",
            "HEAD",
            "--git-path",
            "refs",
            "--git-path",
            "packed-refs",
            "--git-path",
            "reftable/tables.list",
        ],
    )?;
    let mut paths = paths.lines();
    if !paths.next()?.is_empty() {
        return None;
    }
    for path in paths.filter(|path| Path::new(path).exists()) {
        println!("cargo:rerun-if-changed={path}");
    }

    git_output(workspace, &["rev-parse", "--verify", "HEAD^{commit}"]).filter(|revision| {
        matches!(revision.len(), 40 | 64) && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn git_output(workspace: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    Some(output.trim_end().to_owned())
}
