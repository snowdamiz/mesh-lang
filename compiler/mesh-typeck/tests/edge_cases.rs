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

/// The messages of the errors `src` gets, each with the source text it is
/// reported at (empty for an error at no one place).
fn located_errors(src: &str) -> Vec<(String, String)> {
    let parse = mesh_parser::parse(src);
    mesh_typeck::check(&parse)
        .errors
        .iter()
        .map(|error| {
            let at = error.span().map_or("", |span| &src[span]);
            (error.to_string(), at.to_string())
        })
        .collect()
}

// ── Tuple rows ─────────────────────────────────────────────────────────

/// A parameter read by its elements is one tuple of at least as many
/// elements, however the reads are ordered and repeated.
#[test]
fn tuple_rows_of_one_parameter_agree() {
    assert_clean(
        r#"
fn sum(p) do
  let b = Tuple.second(p)
  let a = Tuple.first(p)
  let c = Tuple.first(p)
  let d = Tuple.second(p)
  a + b + c + d
end

fn main() do
  sum((1, 2))
  sum((1, 2, 3))
end
"#,
    );
}

/// A tuple whose element is not the row's, and a value that is no tuple.
#[test]
fn tuple_rows_reject_what_cannot_be_their_tuple() {
    assert_eq!(
        errors(
            r#"
fn head(p) do
  Tuple.first(p) + 1
end

fn main() do
  head(("a", 1))
  head(5)
end
"#
        ),
        [
            "type mismatch: expected `Int`, found `String`",
            "type mismatch: expected `(Int, ..)`, found `Int`",
        ]
    );
}

// ── Unification ────────────────────────────────────────────────────────

/// A declared type parameter inside the type it is unified with is a plain
/// mismatch, shown by the parameter's name.
#[test]
fn a_type_parameter_is_not_the_type_that_holds_it() {
    assert_eq!(
        errors("fn wrap<T>(x :: T) -> Option<T> do\n  x\nend\n"),
        ["type mismatch: expected `Option<T>`, found `T`"]
    );
}

/// A generalized closure keeps what its body requires of its parameters
/// (`<>` joins strings or lists) for each use, and leaves the enclosing
/// function's variables, fixed later or not, to that function.
#[test]
fn a_generalized_closure_carries_its_requirements_to_each_use() {
    assert_eq!(
        errors(
            r#"
fn pair_with(y) do
  let pair = fn (x) -> (x, y) end
  let n = y + 1
  pair(n)
end

fn pair_later(y) do
  let pair = fn (x) -> (x, y) end
  pair(1)
end

fn twice(y) do
  let f = fn (a) -> y <> y end
  f(1)
end

fn joins() do
  let join = fn (a, b) -> a <> b end
  join("a", "b") <> join(1, 2)
end
"#
        ),
        [
            "type mismatch: expected `String`, found `Int`",
            "`<>` joins strings or lists, not `Int`",
        ]
    );
}

/// Only an iterator handle is an `Iter`.
#[test]
fn a_value_that_is_no_iterator_is_no_iter() {
    assert_eq!(
        errors("fn f() do\n  let it :: Iter<Int> = 5\n  it\nend\n"),
        ["type mismatch: expected `Iter<Int>`, found `Int`"]
    );
}

// ── Impls ──────────────────────────────────────────────────────────────

const CELSIUS_FROM_INT: &str = r#"
struct Celsius do
  degrees :: Int
end

impl From<Int> for Celsius do
  fn from(n :: Int) -> Celsius do
    Celsius { degrees: n }
  end
end
"#;

/// Two impls of a generic interface for one type are two impls when the
/// interface's arguments differ, and one twice when they do not, even when
/// the second has methods the first lacks.
#[test]
fn impls_of_a_generic_interface_differ_by_its_arguments() {
    assert_clean(&format!(
        "{CELSIUS_FROM_INT}
impl From<Float> for Celsius do
  fn from(f :: Float) -> Celsius do
    Celsius {{ degrees: Float.to_int(f) }}
  end
end
"
    ));
    assert_eq!(
        errors(&format!(
            "{CELSIUS_FROM_INT}
impl From<Int> for Celsius do
  fn from(n :: Int) -> Celsius do
    Celsius {{ degrees: n + 1 }}
  end

  fn extra(n :: Int) -> Int do
    n
  end
end
"
        )),
        ["duplicate impl: `From` is already implemented for `Celsius` (previously defined for `Celsius`)"]
    );
}

/// `T` and `E`, the parameters of the built-in `Option` and `Result`, name
/// no type: `x :: T` without a declared `T` is an unknown type, and an impl
/// for it implements nothing, not every type.
#[test]
fn the_builtin_types_parameters_are_no_types() {
    assert_eq!(
        errors("fn same(x :: T) -> E do\n  x\nend\n"),
        ["unknown type `T`", "unknown type `E`"]
    );
    assert_eq!(
        errors(
            r#"
interface Named do
  fn name(self) -> String
end

impl Named for Int do
  fn name(self) -> String do
    "int"
  end
end

impl Named for T do
  fn name(self) -> String do
    "any"
  end
end
"#
        ),
        ["unknown type `T`"]
    );
}

/// What is wrong with an impl is reported where the impl says it: a missing
/// method or associated type at its header, an extra associated type at its
/// binding, a method unlike the interface's at the method, a duplicate at
/// the second impl's header, and a method's body at its value. They were
/// all reported at the whole file, and the language server showed none.
#[test]
fn impl_errors_are_reported_where_the_impl_says_it() {
    let at = |message: &str, text: &str| (message.to_string(), text.to_string());
    assert_eq!(
        located_errors(
            r#"struct P do
  x :: Int
end

interface Named do
  type Item
  fn name(self) -> String
  fn id(self) -> Int
end

impl Named for P do
  type Extra = Int
  fn name(self, extra :: Int) -> String do
    "p"
  end
end

interface Sized do
  fn size(self) -> Int
end

impl Sized for P do
  fn size(self) -> Int do
    "big"
  end
end

impl Sized for P do
  fn size(self) -> Int do
    2
  end
end
"#
        ),
        [
            at(
                "method `name` in impl `Named` has wrong signature: expected `(Self) -> _`, found `(Self, Int) -> _`",
                "fn name(self, extra :: Int) -> String do\n    \"p\"\n  end"
            ),
            at(
                "impl `Named` for `P` is missing method `id`",
                "impl Named for P do"
            ),
            at(
                "impl `Named` for `P` is missing associated type `Item`",
                "impl Named for P do"
            ),
            at(
                "impl `Named` for `P` provides associated type `Extra` which is not declared by the trait",
                "type Extra = Int"
            ),
            at(
                "duplicate impl: `Sized` is already implemented for `P` (previously defined for `P`)",
                "impl Sized for P do"
            ),
            at("type mismatch: expected `Int`, found `String`", "\"big\""),
        ]
    );
}
