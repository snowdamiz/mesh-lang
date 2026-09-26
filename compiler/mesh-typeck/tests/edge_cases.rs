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

/// The messages of the errors `src` gets when it may import the module
/// `name` of source `module`, which must check cleanly.
fn errors_importing(name: &str, module: &str, src: &str) -> Vec<String> {
    let module_parse = mesh_parser::parse(module);
    let module_check = mesh_typeck::check(&module_parse);
    assert!(module_check.errors.is_empty(), "{:?}", module_check.errors);
    let exports = mesh_typeck::collect_exports(&module_parse, &module_check);
    let mut imports = mesh_typeck::ImportContext::empty();
    imports.all_trait_impls = exports.trait_impls.clone();
    imports.module_exports.insert(
        name.to_string(),
        mesh_typeck::ModuleExports::new(name.to_string(), &exports),
    );
    mesh_typeck::check_with_imports(&mesh_parser::parse(src), &imports)
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

// ── Error recovery ─────────────────────────────────────────────────────

/// An expression given up on midway does not leave the checker inside it:
/// after a loop whose filter, pattern or condition failed, `break` and
/// `continue` were taken for inside the loop and the loop's names stayed
/// bound.
#[test]
fn a_failed_loop_is_left_behind() {
    assert_eq!(
        errors(
            r#"
fn filtered(xs :: List<Int>) do
  let r = for x in xs when nope(x) do
    x
  end
  break
end

fn destructured(xs :: List<Int>) do
  let r = for (a, b) in xs do
    a
  end
  a
end

fn condition(n :: Int) do
  while nope() do
    1
  end
  continue
end
"#
        ),
        [
            "undefined variable `nope`",
            "`break` outside of loop",
            "type mismatch: expected `(_, _)`, found `Int`",
            "undefined variable `a`",
            "undefined variable `nope`",
            "`continue` outside of loop",
        ]
    );
}

// ── The `?` operator ───────────────────────────────────────────────────

/// `?` on an Option returns its `None` early from a function returning an
/// Option, declared or not, or from a closure whose return type its caller
/// gives; so does `?` on a Result, with its `Err`.
#[test]
fn try_returns_early_from_what_returns_its_kind() {
    assert_clean(
        r#"
fn declared(o :: Option<Int>) -> Option<Int> do
  let v = o?
  Some(v + 1)
end

fn undeclared(o :: Option<Int>) do
  let v = o?
  Some(v * 2)
end

fn in_closures(os :: List<Option<Int>>, rs :: List<Result<Int, String>>) -> Int do
  let tens = List.map(os, fn o -> Some(o? * 10) end)
  let fives = List.map(rs, fn r -> Ok(r? + 5) end)
  List.length(tens) + List.length(fives)
end
"#,
    );
}

/// An operand whose type is not known yet is an Option in a function
/// returning an Option: it was always taken for a Result, so `o?` in such
/// a function could not be written without annotating `o`. Elsewhere it is
/// a Result.
#[test]
fn try_on_an_unknown_value_takes_the_kind_its_function_returns() {
    assert_clean(
        r#"
fn next(o) -> Option<Int> do
  let v = o?
  Some(v + 1)
end

fn bumped(r) do
  let v = r?
  Ok(v + 1)
end

fn main() do
  next(Some(1))
  bumped(Err("no"))
end
"#,
    );
}

/// What `?` cannot do: apply to a value that is neither, return early from
/// a function that returns neither or the other kind, or give a Result's
/// error to a function whose error type has no `From` it.
#[test]
fn try_is_refused_where_its_early_return_cannot_go() {
    assert_eq!(
        errors(
            r#"
fn not_either(xs :: List<Int>) -> Option<Int> do
  let v = xs?
  Some(v)
end

fn returns_int(o :: Option<Int>) -> Int do
  o?
end

fn other_kind(r :: Result<Int, String>) -> Option<Int> do
  Some(r?)
end

fn other_error(r :: Result<Int, Bool>) -> Result<Int, Int> do
  Ok(r?)
end

fn undefined() -> Result<Int, String> do
  Ok(missing?)
end
"#
        ),
        [
            "`?` operator requires `Result` or `Option`, found `List<Int>`",
            "`?` cannot propagate `Option<Int>` from a function returning `Int`",
            "`?` cannot propagate `Result<Int, String>` from a function returning `Option<Int>`",
            "`?` cannot propagate `Result<Int, Bool>` from a function returning `Result<Int, Int>`",
            "undefined variable `missing`",
        ]
    );
}

/// A type error shows the parts of a type not settled yet as `_`, not as
/// the checker's variables (`Result<?40, ?41>`).
#[test]
fn type_errors_show_unsettled_types_as_holes() {
    assert_eq!(
        errors(
            r#"
fn f() do
  let g = fn o -> Some(o? + 1) end
  g
end

fn h() do
  let xs = []
  xs.nope()
end
"#
        ),
        [
            "`?` cannot propagate `Result<_, _>` from a function returning `Option<Int>`",
            "no method `nope` on type `List<_>`",
        ]
    );
}

/// A Result's error converts to the one the function returns through a
/// `From` impl.
#[test]
fn try_converts_an_error_through_from() {
    assert_clean(
        r#"
struct AppError do
  message :: String
end

impl From<String> for AppError do
  fn from(message :: String) -> AppError do
    AppError { message: message }
  end
end

fn parse(r :: Result<Int, String>) -> Result<Int, AppError> do
  Ok(r? + 1)
end
"#,
    );
}

// ── Operators ──────────────────────────────────────────────────────────

/// Negating a value with no `Neg` is reported at the negation (it was
/// reported at the whole file); `!` takes a Bool; a `Neg` impl that leaves
/// out its `Output` negates to its own type.
#[test]
fn unary_operators_check_their_operand() {
    let at = |message: &str, text: &str| (message.to_string(), text.to_string());
    assert_eq!(
        located_errors(
            r#"struct P do
  x :: Int
end

impl Neg for P do
  fn neg(self) -> P do
    self
  end
end

fn negated(p :: P) -> P do
  -p
end

fn text() do
  -"s"
end

fn not_bool() do
  !5
end

fn undefined() do
  -nope
end
"#
        ),
        [
            at(
                "impl `Neg` for `P` is missing associated type `Output`",
                "impl Neg for P do"
            ),
            at("`String` does not implement `Neg`", "-\"s\""),
            at("type mismatch: expected `Bool`, found `Int`", "!"),
            at("undefined variable `nope`", "nope"),
        ]
    );
}

/// An arithmetic operand whose type only the other operand settles is
/// checked once it is known, and an `Add` impl that leaves out its `Output`
/// adds to its own type.
#[test]
fn arithmetic_checks_the_type_the_operands_settle_on() {
    assert_eq!(
        errors(
            r#"struct V do
  x :: Int
end

impl Add for V do
  fn add(self, other :: V) -> V do
    self
  end
end

fn sum(a :: V, b :: V) -> V do
  a + b
end

fn text(x) do
  x + "s"
end
"#
        ),
        [
            "impl `Add` for `V` is missing associated type `Output`",
            "`String` does not implement `Add`",
        ]
    );
}

// ── Fields ─────────────────────────────────────────────────────────────

/// A field read from a value whose type is settled later in its function
/// is checked against that type: only a struct has fields. A tuple or a
/// function was let through, and a generic struct's field reads as the
/// struct's argument.
#[test]
fn fields_read_before_their_value_is_known_are_checked_later() {
    assert_eq!(
        errors(
            r#"
struct Box<T> do
  item :: T
end

fn tuple_field(p) do
  let a = p.x
  let (u, v) = p
  a
end

fn fn_field(f) do
  let a = f.x
  let b = f(1)
  a
end

fn generic_field(b) do
  let i = b.item
  let c :: Box<Int> = b
  i + 1
end

fn int_field(n) do
  let a = n.x
  let m = n + 1
  a
end
"#
        ),
        [
            "type `(_, _)` has no field `x`",
            "type `(Int) -> _` has no field `x`",
            "type `Int` has no field `x`",
        ]
    );
}

// ── Patterns ───────────────────────────────────────────────────────────

/// A variant pattern with too many or too few fields is reported at the
/// pattern, and a field pattern of the wrong type at that field: the first
/// was reported at the whole file, and the language server did not show it.
#[test]
fn variant_pattern_errors_are_reported_at_the_pattern() {
    let at = |message: &str, text: &str| (message.to_string(), text.to_string());
    assert_eq!(
        located_errors(
            r#"fn extra(o :: Option<Int>) -> Int do
  case o do
    Some(1, 2) -> 1
    _ -> 3
  end
end

fn nullary(o :: Option<Int>) -> Int do
  case o do
    None(x) -> 2
    _ -> 3
  end
end

type Count do
  Many(Int)
  Zero
end

fn field(c :: Count) -> Int do
  case c do
    Many("s") -> 1
    _ -> 2
  end
end
"#
        ),
        [
            at("arity mismatch: expected 1 argument, found 2", "Some(1, 2)"),
            at("arity mismatch: expected 0 arguments, found 1", "None(x)"),
            at("type mismatch: expected `Int`, found `String`", "\"s\""),
        ]
    );
}

/// Each alternative of an or-pattern matches the first one's type and binds
/// the same names at the same types, and a mistake is reported at the
/// alternative that makes it.
#[test]
fn or_pattern_alternatives_agree_with_the_first() {
    let at = |message: &str, text: &str| (message.to_string(), text.to_string());
    assert_eq!(
        located_errors(
            r#"type Shape do
  Wide(Int)
  Tall(Int)
  Dot
end

fn both(s :: Shape) -> Int do
  case s do
    Wide(n) | Tall(n) -> n
    Dot -> 0
  end
end

fn unbound(o :: Option<Int>) -> Int do
  case o do
    Some(x) | None -> 1
  end
end

fn kinds(n :: Int) -> Int do
  case n do
    1 | "s" -> 1
    _ -> 2
  end
end

fn names(p :: (Int, String)) -> Int do
  case p do
    (1, x) | (x, "s") -> 1
    _ -> 2
  end
end
"#
        ),
        [
            at(
                "or-pattern binding mismatch: expected [x], found []",
                "Some(x) | None"
            ),
            at("type mismatch: expected `Int`, found `String`", "\"s\""),
            at(
                "type mismatch: expected `String`, found `Int`",
                "(x, \"s\")"
            ),
        ]
    );
}

/// A struct pattern may name the struct through an alias; an alias of a
/// type that is no struct names no struct.
#[test]
fn struct_patterns_name_their_struct_through_an_alias() {
    assert_eq!(
        errors(
            r#"struct P do
  x :: Int
end

type Q = P
type N = Int

fn aliased(q :: Q) -> Int do
  case q do
    Q { x: 1 } -> 1
    _ -> 2
  end
end

fn not_struct(n :: N) -> Int do
  case n do
    N { x: 1 } -> 1
    _ -> 2
  end
end
"#
        ),
        ["`N` is not a struct"]
    );
}

// ── Exhaustiveness ─────────────────────────────────────────────────────

/// `true | false` inside another pattern covers every Bool: the column's
/// type was taken from its patterns without looking into or-patterns, so a
/// Bool there counted as a type of endless values.
#[test]
fn an_or_pattern_of_both_bools_is_exhaustive_inside_another_pattern() {
    assert_clean(
        r#"
fn pair(a :: Bool, b :: Int) -> Int do
  case (a, b) do
    (true | false, _) -> 1
  end
end

fn maybe(o :: Option<Bool>) -> Int do
  case o do
    Some(true | false) -> 1
    None -> 0
  end
end
"#,
    );
}

/// A missing case is named down to the constructors the arms leave out,
/// in columns whose arms left no constructor to take the type from.
#[test]
fn missing_cases_are_named_in_columns_the_arms_leave_open() {
    assert_eq!(
        errors(
            r#"
fn with_list(o :: Option<Int>, xs :: List<Int>) -> Int do
  case (o, xs) do
    (None, []) -> 1
    (Some(1), _) -> 2
  end
end

fn with_option(o :: Option<Int>, p :: Option<Int>) -> Int do
  case (o, p) do
    (None, Some(_)) -> 1
    (Some(1), _) -> 2
  end
end
"#
        ),
        [
            "non-exhaustive match on `(Option<Int>, List<Int>)`: missing patterns [(Some(_), _ :: _)]",
            "non-exhaustive match on `(Option<Int>, Option<Int>)`: missing patterns [(Some(_), Some(_))]",
        ]
    );
}

/// Arms that do not fit the value's type are reported as such, and the
/// check of which cases they cover takes them as they are.
#[test]
fn arms_of_the_wrong_type_are_errors_of_their_own() {
    assert_eq!(
        errors(
            r#"
fn unknown(o :: Option<Int>) -> Int do
  case o do
    Some(Nope.Thing) -> 1
    None -> 0
  end
end

fn literal(o :: Option<Int>) -> Int do
  case o do
    1 -> 1
    None -> 0
  end
end
"#
        ),
        [
            "unknown variant `Nope.Thing`",
            "type mismatch: expected `Option<Int>`, found `Int`",
        ]
    );
}

/// A variant named through the module that exports its type (`Geo.Dot`)
/// belongs to that type inside another pattern too: the module's name was
/// taken for the type's, so `Some(Geo.Dot)` and `Some(Geo.Line(_))` did
/// not cover `Some(_)`.
#[test]
fn variants_named_through_their_module_cover_their_type() {
    let geo = "pub type Shape do\n  Dot\n  Line(Int)\nend\n";
    assert_eq!(
        errors_importing(
            "Geo",
            geo,
            r#"
import Geo

fn every(o :: Option<Geo.Shape>) -> Int do
  case o do
    Some(Geo.Dot) -> 1
    Some(Geo.Line(_)) -> 2
    None -> 3
  end
end

fn some(o :: Option<Geo.Shape>) -> Int do
  case o do
    Some(Geo.Dot) -> 1
    None -> 3
  end
end
"#
        ),
        ["non-exhaustive match on `Option<Shape>`: missing patterns [Some(Line(_))]"]
    );
}

/// An or-pattern arm is redundant only when every alternative is.
#[test]
fn an_or_pattern_arm_is_redundant_when_all_its_alternatives_are() {
    let parse = mesh_parser::parse(
        "fn f(n :: Int) -> Int do\n  case n do\n    1 -> 1\n    1 | 2 -> 2\n    1 | 2 -> 3\n    _ -> 4\n  end\nend\n",
    );
    let result = mesh_typeck::check(&parse);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let warnings: Vec<String> = result.warnings.iter().map(|w| w.to_string()).collect();
    assert_eq!(warnings, ["redundant match arm (arm 3)"]);
}

// ── Imports ────────────────────────────────────────────────────────────

const STORE: &str = r#"
pub struct User do
  name :: String
end deriving(Schema)

pub type Shade = Int

pub type Kind do
  Big
  Small
end

pub interface Labelled do
  fn label(self) -> String
end

fn secret() do
  1
end

pub fn pad(x :: Int) -> Int do
  x
end

pub fn pad(x :: Int, y :: Int) -> Int do
  x + y
end

actor Pinger(n :: Int) do
  receive do
    m -> Pinger(n)
  end
end

service Counter do
  fn init(n :: Int) -> Int do
    n
  end
  call Get() :: Int do |n|
    (n, n)
  end
end
"#;

/// `from Module import ...` brings in each kind of definition a module
/// exports: a struct with its schema functions, an alias, a sum type's
/// variants, an interface, both arities of an overloaded function, an
/// actor and a service. A standard module's functions come in under their
/// own name and with the module's prefix.
#[test]
fn from_import_brings_in_every_kind_of_definition() {
    assert_eq!(
        errors_importing(
            "Store",
            STORE,
            r#"
from Store import User, Shade, Kind, Labelled, pad, Pinger, Counter
from String import length

fn main() do
  let t = User.__table__()
  let c = User.__name_col__()
  let columns = [User.__fields__(), User.__relationships__(), User.__field_types__(), User.__relationship_meta__()]
  let key = User.__primary_key__()
  let s :: Shade = 3
  let k = Big
  let pid = Counter.start(0)
  let n = Counter.get(pid)
  let p = spawn(Pinger, 1)
  pad(1) + pad(1, 2) + length("abc") + string_length("ab")
end
"#
        ),
        Vec::<String>::new()
    );
}

/// What an import cannot bring in: a name a module does not export or
/// keeps private, or anything of a module that does not exist.
#[test]
fn imports_of_what_is_not_there_are_errors() {
    let errors = errors_importing(
        "Store",
        STORE,
        "from Store import secret, Nope\nfrom Nowhere import thing\nimport Elsewhere\n",
    );
    // The names a module does export follow, in no particular order.
    let headlines: Vec<&str> = errors
        .iter()
        .map(|error| error.split("; ").next().unwrap())
        .collect();
    assert_eq!(
        headlines,
        [
            "`secret` is private in module `Store`",
            "`Nope` is not exported by module `Store`",
            "module `Nowhere` not found",
            "module `Elsewhere` not found",
        ]
    );
}

// ── Supervisors ────────────────────────────────────────────────────────

/// A supervisor with the given child specs (each a `child ... end` block).
fn supervisor(children: &str) -> String {
    format!(
        "actor worker() do\n  receive do\n    m -> worker()\n  end\nend\n\
         fn restart() do\n  spawn(worker)\nend\n\
         supervisor Sup do\n  strategy: one_for_one\n{children}end\n"
    )
}

/// A child spec's restart and shutdown values are checked, and its start
/// expression is not read for them: a start function calling one named
/// `restart` was taken for the key. A duplicate child name was reported as
/// an unknown strategy.
#[test]
fn child_spec_values_are_checked_where_they_are_given() {
    assert_clean(&supervisor(
        "  child a do\n    start: fn -> restart() end\n    restart: transient\n    shutdown: 10_000\n  end\n",
    ));
    assert_eq!(
        errors(&supervisor(
            "  child a do\n    start: fn -> spawn(worker) end\n    shutdown: 0\n  end\n\
               child b do\n    start: fn -> spawn(worker) end\n    shutdown: 99999999999999999999\n  end\n\
               child a do\n    start: fn -> spawn(worker) end\n  end\n",
        )),
        [
            "invalid shutdown value `0` for child `a`, expected a positive integer or brutal_kill",
            "invalid shutdown value `99999999999999999999` for child `b`, expected a positive integer or brutal_kill",
            "child `a` is defined twice",
        ]
    );
}

/// A supervisor the parser could not finish is checked as far as it goes.
#[test]
fn an_unfinished_supervisor_is_checked_as_far_as_it_goes() {
    for source in [
        "supervisor Sup do\n  strategy: 5\nend\n",
        "supervisor Sup do\n  child a do\n    start: fn -> nope() end\n    restart: 5\n  end\nend\n",
        "supervisor Sup do\n  child a\nend\n",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(!parse.ok(), "{source}");
        mesh_typeck::check(&parse);
    }
}
