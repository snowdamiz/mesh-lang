use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/skill/mesh");
    println!("cargo:rerun-if-changed={}", root.display());

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
        let path = root.join(&file);
        println!("cargo:rerun-if-changed={}", path.display());
        writeln!(source, "    ({file:?}, include_str!({path:?})),").unwrap();
    }
    source.push_str("];\n");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    fs::write(output.join("guide_files.rs"), source).expect("write the embedded Mesh guide");
}
