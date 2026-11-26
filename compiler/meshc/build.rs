use std::collections::HashSet;
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
        watch(&git_marker);
    } else if !git_marker.is_dir() {
        return None;
    }

    let layout = git_output(
        workspace,
        &[
            "rev-parse",
            "--show-prefix",
            "--path-format=absolute",
            "--absolute-git-dir",
            "--git-common-dir",
        ],
    )?;
    let mut paths = layout.lines();
    if paths.next()? != "" {
        return None;
    }
    let git_dir = Path::new(paths.next()?);
    let common_dir = Path::new(paths.next()?);
    // Older Git echoes unrecognized rev-parse flags and can return relative paths.
    if paths.next().is_some() || !git_dir.is_absolute() || !common_dir.is_absolute() {
        return None;
    }
    let head = git_dir.join("HEAD");
    if !head.is_file() {
        return None;
    }
    watch(&head);
    if git_dir != common_dir {
        watch(&git_dir.join("commondir"));
    }

    let shared_tables = common_dir.join("reftable/tables.list");
    if shared_tables.is_file() {
        watch(&shared_tables);
        if git_dir != common_dir {
            watch(&git_dir.join("reftable/tables.list"));
        }
    } else {
        watch_symbolic_refs(workspace, common_dir)?;
    }

    git_output(workspace, &["rev-parse", "--verify", "HEAD^{commit}"]).filter(|revision| {
        matches!(revision.len(), 40 | 64) && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn watch_symbolic_refs(workspace: &Path, common_dir: &Path) -> Option<()> {
    let packed_refs = common_dir.join("packed-refs");
    let shared_refs = common_dir.join("refs");
    let mut seen = HashSet::new();
    let mut reference = "HEAD".to_owned();
    while let Some(target) = git_output(
        workspace,
        &["symbolic-ref", "--quiet", "--no-recurse", &reference],
    ) {
        if !target.starts_with("refs/") || !seen.insert(target.clone()) {
            return None;
        }
        // Git resolves shared and worktree-private refs against different directories.
        let path = git_output(
            workspace,
            &["rev-parse", "--path-format=absolute", "--git-path", &target],
        )?;
        if path.contains('\n') || !Path::new(&path).is_absolute() {
            return None;
        }
        let path = Path::new(&path);
        if !path.exists() && path.starts_with(&shared_refs) && packed_refs.is_file() {
            watch(&packed_refs);
            // A packed branch can gain a loose ref without changing packed-refs.
            // Watch its parent when available so missing paths do not rebuild every time.
            let loose = path
                .parent()
                .filter(|parent| *parent != shared_refs && parent.is_dir());
            watch(loose.unwrap_or(path));
        } else {
            watch(path);
        }
        reference = target;
    }
    Some(())
}

fn watch(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
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
    Some(output.trim_end_matches(['\r', '\n']).to_owned())
}
