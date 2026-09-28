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

/// `send`, `spawn` and `link` cut off before their arguments add no error
/// to their parse error: each was also "expected 2 arguments, found 0".
#[test]
fn actor_primitives_without_arguments_are_skipped() {
    for source in [
        "fn f() do\n  send\nend\n",
        "fn f() do\n  spawn\nend\n",
        "fn f() do\n  link\nend\n",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(!parse.errors().is_empty(), "{source:?}");
        let result = mesh_typeck::check(&parse);
        assert!(result.errors.is_empty(), "{source:?}: {:?}", result.errors);
    }
}

/// Each construct cut off at a part it needs adds no error to its parse
/// error: a pattern, scrutinee, name, type, value, timeout or start
/// missing is left to it.
#[test]
fn constructs_cut_off_at_a_part_add_no_error() {
    for source in [
        "actor a(:: Int) do\n  receive do\n    x -> 1\n  end\nend\n",
        "type A =\n",
        "fn f(x) do\n  case x do\n    as whole -> whole\n  end\nend\n",
        "fn f(x) do\n  case x do\n    -> 1\n  end\nend\n",
        "fn f() do\n  case do\n    x -> 1\n  end\nend\n",
        "fn g() do\n  let h = fn\n    | -> 1\n  end\n  h\nend\n",
        "fn f(xs) do\n  case xs do\n    :: t -> 1\n    _ -> 2\n  end\nend\n",
        "fn f(x :: ) do\n  x\nend\n",
        "fn f(:: Int) do\n  1\nend\n",
        "interface I do\n  fn (self) -> Int\nend\n",
        "interface I do\n  fn f(self, :: Int) -> Int\nend\n",
        "interface I do\n  fn f(self) -> Int\nend\n\nimpl I for do\n  fn f(self) -> Int do\n    1\n  end\nend\n",
        "interface I do\n  fn f(self, x :: Int) -> Int\nend\n\nstruct P do\n  x :: Int\nend\n\nimpl I for P do\n  fn f(self, :: Int) -> Int do\n    1\n  end\nend\n",
        "fn f() do\n  let x :: = 1\n  x\nend\n",
        "struct P do\n  x :: Int\nend\n\nfn f() do\n  P { x: }\nend\n",
        "fn f() do\n  %{ => 1 }\nend\n",
        "fn f() do\n  %{ 1 => }\nend\n",
        "actor a() do\n  receive do\n    x -> 1\n  after -> 2\n  end\nend\n",
        "actor a() do\n  receive do\n    -> 1\n  end\nend\n",
        "actor w() do\n  receive do\n    m -> 1\n  end\nend\n\nsupervisor S do\n  strategy: one_for_one\n  child c do\n    start:\n  end\nend\n",
        "fn f(t) do\n  t.0 + 1\nend\n",
        "struct P do\n  x :: Int\nend\n\nfn f() do\n  %{ | x: 1 }\nend\n",
        "struct P do\n  x :: Int\nend\n\nfn f(p :: P) do\n  %{p | : 1}\nend\n",
        "type T do\n  A\n  (Int)\nend\n",
        "fn f<T>(x :: T) where T do\n  x\nend\n",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(!parse.errors().is_empty(), "{source:?}");
        let result = mesh_typeck::check(&parse);
        assert!(result.errors.is_empty(), "{source:?}: {:?}", result.errors);
        mesh_typeck::collect_exports(&parse, &result);
    }
}

/// A literal or pattern cut off at a part adds no error to its parse
/// error: `P { : 1 }` was "missing field `x`", a cons pattern without its
/// tail made its case non-exhaustive, and a struct pattern cut off inside
/// made its arm "no `->`".
#[test]
fn literals_and_patterns_cut_off_at_a_part_add_no_error() {
    for source in [
        "struct P do\n  x :: Int\nend\n\nfn f() do\n  P { : 1 }\nend\n",
        "struct P do\n  x :: Int\nend\n\nfn f(p :: P) do\n  %{p | : 1}\nend\n",
        "fn f() do\n  %{1 => 2, => 3}\nend\n",
        "fn f() do\n  %{1 => }\nend\n",
        "fn f() do\n  json { a: }\nend\n",
        "struct P do\n  x :: Int\nend\n\nfn f(p :: P) do\n  case p do\n    P { : 1 } -> 1\n  end\nend\n",
        "struct P do\n  x :: Int\nend\n\nfn f(p :: P) do\n  case p do\n    P { x: } -> 1\n  end\nend\n",
        "fn f(xs) do\n  case xs do\n    h :: -> 1\n    _ -> 2\n  end\nend\n",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(!parse.errors().is_empty(), "{source:?}");
        let result = mesh_typeck::check(&parse);
        assert!(result.errors.is_empty(), "{source:?}: {:?}", result.errors);
        assert!(result.warnings.is_empty(), "{source:?}: {:?}", result.warnings);
    }
}

/// Definitions cut off at a part (an impl without its interface, an alias
/// or an associated type without its name, a parameter or return type
/// without its type, a method without its body, an `as` without its
/// name) are checked as far as they go, and add no error of their own.
#[test]
fn definitions_cut_off_at_a_part_add_no_error() {
    for source in [
        "impl for Int do\nend\n",
        "type = Int\n",
        "interface I do\n  fn m(self, x :: ) -> Int\nend\n",
        "struct X do\nend\n\nimpl Display for X do\n  fn to_string(self) -> String\nend\n",
        "fn f() -> do\n  1\nend\n",
        "fn f(0) = 1\nfn f(x :: ) = 2\n",
        "struct P do\n  x :: Int\nend\n\nfn f(p :: P) do\n  %{p | x: }\nend\n",
        "fn f(Some(x) as) do\n  1\nend\n",
        // Types cut off: a tuple of no element, a function type without
        // its result.
        "fn f(x :: (,)) do\n  x\nend\n",
        "fn f(x :: Fun()) do\n  x\nend\n",
        "fn f(x :: Fun(Int) ->) do\n  x\nend\n",
        // A keyword argument list cut off at a positional argument, in a
        // closure.
        "fn f(m) do\n  m\nend\n\nfn main() do\n  let g = fn() -> f(a: 1, 2) end\n  g\nend\n",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(!parse.errors().is_empty(), "{source:?}");
        let result = mesh_typeck::check(&parse);
        assert!(result.errors.is_empty(), "{source:?}: {:?}", result.errors);
    }
    // What the rest of the definition says is still checked.
    for (source, error) in [
        (
            "struct X do\nend\n\nimpl Iterable for X do\n  type = Int\nend\n",
            "impl `Iterable` for `X` is missing method `iter`",
        ),
        (
            "fn f(x :: Map<Int, ) do\n  x\nend\n",
            "`Map` takes 2 type arguments, not 1",
        ),
    ] {
        let parse = mesh_parser::parse(source);
        assert!(!parse.errors().is_empty(), "{source:?}");
        let errors: Vec<String> = mesh_typeck::check(&parse)
            .errors
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(errors.iter().any(|e| e == error), "{source:?}: {errors:?}");
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
