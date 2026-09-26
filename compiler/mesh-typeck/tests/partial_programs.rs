//! The language server checks a document at every keystroke, so the type
//! checker sees programs the parser could not finish: every construct cut
//! off partway. Checking one must never fail.

use std::path::Path;

/// Every end-to-end fixture, cut off after every fourth line, type-checks
/// without panicking.
#[test]
fn programs_cut_off_partway_are_checked_without_panicking() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/e2e");
    let mut files: Vec<_> = std::fs::read_dir(&fixtures)
        .expect("the e2e fixtures")
        .map(|entry| entry.expect("a fixture").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "mpl"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no fixtures in {}", fixtures.display());
    for file in files {
        let source = std::fs::read_to_string(&file).expect("a readable fixture");
        for (line, (end, _)) in source.match_indices('\n').enumerate().step_by(4) {
            let cut = &source[..end];
            let checked = std::panic::catch_unwind(|| mesh_typeck::check(&mesh_parser::parse(cut)));
            assert!(
                checked.is_ok(),
                "checking {} cut after line {} panicked",
                file.display(),
                line + 1
            );
        }
    }
}

/// An import cut off before its module names nothing, and adds no error to
/// its parse error.
#[test]
fn imports_without_a_module_are_skipped() {
    for source in ["import\n", "from\n", "from import length\n"] {
        let parse = mesh_parser::parse(source);
        assert!(!parse.errors().is_empty(), "{source:?}");
        let result = mesh_typeck::check(&parse);
        assert!(result.errors.is_empty(), "{source:?}: {:?}", result.errors);
    }
}
