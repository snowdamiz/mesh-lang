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
            let checked = std::panic::catch_unwind(|| {
                let parse = mesh_parser::parse(cut);
                mesh_typeck::collect_exports(&parse, &mesh_typeck::check(&parse))
            });
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

/// A condition, iterable or guard cut off before its `do` is missing, not
/// the block after the `do`: `if do 1 end` was "expected `Bool`, found
/// `Int`", and a guard without its expression made its clause guarded.
#[test]
fn a_missing_condition_is_not_the_block_after_it() {
    for source in [
        "fn f() do\n  if do\n    1\n  end\nend\n",
        "fn f() do\n  while do\n    1\n  end\nend\n",
        "fn f() do\n  for x in do\n    x\n  end\nend\n",
        "fn f(n) when do\n  1\nend\n",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(!parse.errors().is_empty(), "{source:?}");
        let result = mesh_typeck::check(&parse);
        assert!(result.errors.is_empty(), "{source:?}: {:?}", result.errors);
        assert!(
            result.warnings.is_empty(),
            "{source:?}: {:?}",
            result.warnings
        );
    }
}

/// A body cut off before it begins adds no error to its parse error: it
/// was taken as `()`, "expected `Int`, found `()`" beside the arms, clauses
/// or declared type it did not match.
#[test]
fn a_body_cut_off_before_it_begins_is_not_unit() {
    for source in [
        "fn f(0) = 1\nfn f(n)\n",
        "fn f() -> Int\n",
        "fn f() -> List<Int> do\n  for x in [1]\nend\n",
        "fn f(x) -> Int do\n  case x do\n    1 -> 1\n    _ ->\n  end\nend\n",
        "fn g() do\n  let h = fn 0 -> 1\n    | n ->\n  end\n  h\nend\n",
        "actor a() do\n  receive do\n    1 -> 1\n    x ->\n  end\nend\n",
        "actor a() do\n  receive do\n    x -> 1\n  after 5 ->\n  end\nend\n",
        "service S do\n  fn init() -> Int\nend\n",
        "service S do\n  fn init() -> Int do\n    0\n  end\n  call Get() :: Int\nend\n",
        "service S do\n  fn init() -> Int do\n    0\n  end\n  cast Clear()\nend\n",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(!parse.errors().is_empty(), "{source:?}");
        let result = mesh_typeck::check(&parse);
        assert!(result.errors.is_empty(), "{source:?}: {:?}", result.errors);
    }
}

/// A definition the parser could not name is neither exported nor a
/// private name.
#[test]
fn definitions_without_names_are_not_exported() {
    let parse = mesh_parser::parse("pub fn (x) do\n  x\nend\n\npub struct do\n  x :: Int\nend\n");
    assert!(!parse.errors().is_empty());
    let exports = mesh_typeck::collect_exports(&parse, &mesh_typeck::check(&parse));
    assert!(exports.functions.is_empty(), "{:?}", exports.functions);
    assert!(exports.struct_defs.is_empty(), "{:?}", exports.struct_defs);
    assert!(
        exports.private_names.is_empty(),
        "{:?}",
        exports.private_names
    );
}
