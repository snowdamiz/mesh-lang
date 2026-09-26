//! Type checker paths the language's ordinary programs rarely take: each test
//! checks a small program and the messages of the errors it gets.

/// The messages of the errors `src` gets from the type checker.
fn errors(src: &str) -> Vec<String> {
    let parse = mesh_parser::parse(src);
    mesh_typeck::check(&parse)
        .errors
        .iter()
        .map(|error| error.to_string())
        .collect()
}

/// Checks that `src` type-checks without errors.
fn assert_clean(src: &str) {
    assert_eq!(errors(src), Vec::<String>::new());
}

// ── Tuple rows ─────────────────────────────────────────────────────────

/// A parameter read by its second and then its first element is one tuple
/// of at least two elements, however the reads are ordered and repeated.
#[test]
fn tuple_rows_of_one_parameter_agree() {
    assert_clean(
        "fn sum(p) do\n  let b = Tuple.second(p)\n  let a = Tuple.first(p)\n  let c = Tuple.first(p)\n  a + b + c\nend\n\
         fn main() do\n  sum((1, 2))\n  sum((1, 2, 3))\nend\n",
    );
}

/// A tuple whose element is not the row's, and a value that is no tuple.
#[test]
fn tuple_rows_reject_what_cannot_be_their_tuple() {
    assert_eq!(
        errors(
            "fn head(p) do\n  Tuple.first(p) + 1\nend\n\
             fn main() do\n  head((\"a\", 1))\n  head(5)\nend\n"
        ),
        [
            "type mismatch: expected `Int`, found `String`",
            "type mismatch: expected `(Int, ..)`, found `Int`",
        ]
    );
}
