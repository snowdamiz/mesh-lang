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

/// What a method returns of an associated type is the receiver's, and so,
/// at each call, is what a generic function returns of one: a call taken
/// for another type, directly (`Container.first(box)`) or through the
/// generic function, was let through.
#[test]
fn an_associated_type_returned_through_a_call_is_the_receivers() {
    let at = |message: &str, text: &str| (message.to_string(), text.to_string());
    assert_eq!(
        located_errors(
            r#"interface Container do
  type Item
  fn first(self) -> Self.Item
  fn put(self, item :: Self.Item) -> Self
end

struct Box do
  v :: Int
end

impl Container for Box do
  type Item = Int
  fn first(self) -> Int do
    self.v
  end
  fn put(self, item :: Int) -> Box do
    Box { v: item }
  end
end

fn head<C>(c :: C) where C: Container do
  c.first()
end

fn refill<C>(c :: C, item) where C: Container do
  c.put(item)
end

fn qualified(b :: Box) -> Int do
  Container.first(b)
end

fn qualified_wrong(b :: Box) do
  let s :: String = Container.first(b)
  s
end

fn main() do
  let n = head(Box { v: 1 }) + 1
  let m :: String = head(Box { v: 2 })
  let b = refill(Box { v: 3 }, 4)
  let k = head(Box { v: 5 })
  n
end
"#
        ),
        [
            at(
                "type mismatch: expected `String`, found `Int`",
                "Container.first"
            ),
            at("type mismatch: expected `String`, found `Int`", "head"),
        ]
    );
}

/// What a generic function's body makes of an associated type (`c.first()
/// + 1` makes it an `Int`) each call requires of the receiver it gives.
#[test]
fn a_generic_bodys_use_of_an_associated_type_holds_at_each_call() {
    let at = |message: &str, text: &str| (message.to_string(), text.to_string());
    assert_eq!(
        located_errors(
            r#"interface Container do
  type Item
  fn first(self) -> Self.Item
end

struct Label do
  text :: String
end

impl Container for Label do
  type Item = String
  fn first(self) -> String do
    self.text
  end
end

struct Box do
  v :: Int
end

impl Container for Box do
  type Item = Int
  fn first(self) -> Int do
    self.v
  end
end

fn plus_one<C>(c :: C) -> Int where C: Container do
  c.first() + 1
end

fn main() do
  plus_one(Box { v: 1 })
  plus_one(Label { text: "a" })
end
"#
        ),
        [at(
            "type mismatch: expected `Int`, found `String`",
            "plus_one"
        )]
    );
}

/// A generic alias's arguments are counted as a struct's are, an alias's
/// own definition where it is defined, and a miscounted or bare alias
/// stands for its type with the parameters unknown. `Pair<Int>` was `(Int,
/// B)`, "expected `B`, found `Int`" at its uses, and `Pair<Int, String,
/// Bool>` passed.
#[test]
fn a_generic_aliases_arguments_are_counted() {
    let at = |message: &str, text: &str| (message.to_string(), text.to_string());
    assert_eq!(
        located_errors(
            r#"type Pair<A, B> = (A, B)

type Ints = Pair<Int>

fn first(p :: Pair<Int>) -> Int do
  case p do
    (a, _) -> a
  end
end

fn second(p :: Pair<Int, String, Bool>) -> String do
  case p do
    (_, b) -> b
  end
end

fn third(p :: Ints) -> Int do
  case p do
    (a, _) -> a
  end
end

fn fourth(p :: Pair) -> Int do
  case p do
    (a, _) -> a + 1
  end
end

fn main() do
  first((1, 2))
  second((1, "a"))
  third((1, 2))
  fourth((1, 2))
end
"#
        ),
        [
            at(
                "`Pair` takes 2 type arguments, not 1",
                "type Ints = Pair<Int>"
            ),
            at("`Pair` takes 2 type arguments, not 1", "Pair<Int>"),
            at(
                "`Pair` takes 2 type arguments, not 3",
                "Pair<Int, String, Bool>"
            ),
        ]
    );
}

/// A generic type named with another number of type arguments than it
/// takes is reported where it is named, and taken with the right number:
/// it was an "arity mismatch" at each use of the value, as if a function
/// were called wrongly. A type named in its own definition, or before its
/// own, takes its parameters there too.
#[test]
fn type_arguments_are_counted_where_the_type_is_named() {
    let at = |message: &str, text: &str| (message.to_string(), text.to_string());
    assert_eq!(
        located_errors(
            r#"struct Box<T> do
  item :: T
end

type Tree<T> do
  Leaf
  Node(Tree<T>, T, Tree<T>)
end

struct Holder do
  inner :: Later<Int>
end

struct Later<T> do
  value :: T
end

struct Point do
  x :: Int
end

fn opt(o :: Option<Int, String>) -> Int do
  case o do
    Some(n) -> n
    None -> 0
  end
end

fn boxed(b :: Box<Int, Int>) -> Int do
  b.item
end

fn point(p :: Point<Int>) -> Map<Int> do
  %{}
end

fn nested(xs :: List<Option<Int, Int>>) -> Int do
  0
end

fn chan(c :: Channel<Int, Int>) -> Int do
  0
end

fn main() do
  opt(Some(1))
  boxed(Box { item: 1 })
end
"#
        ),
        [
            at(
                "`Option` takes 1 type argument, not 2",
                "Option<Int, String>"
            ),
            at("`Box` takes 1 type argument, not 2", "Box<Int, Int>"),
            at("`Point` takes 0 type arguments, not 1", "Point<Int>"),
            at("`Map` takes 2 type arguments, not 1", "Map<Int>"),
            at(
                "`Option` takes 1 type argument, not 2",
                "List<Option<Int, Int>>"
            ),
            at(
                "`Channel` takes 1 type argument, not 2",
                "Channel<Int, Int>"
            ),
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

/// An impl method without a return annotation returns what its interface
/// declares, `Self` as the implementing type: `fn name(self) do 5 end` for
/// `fn name(self) -> String` was taken to return an `Int`. An impl for a
/// generic type is not supported.
#[test]
fn impl_methods_return_what_their_interface_declares() {
    assert_eq!(
        errors(
            r#"
interface Named do
  fn name(self) -> String
end

interface Sized do
  fn size(self) -> Int
end

interface Make do
  fn make() -> Self
end

struct Box<T> do
  item :: T
end

struct P do
  x :: Int
end

impl Named for List do
  fn name(self) -> String do
    "list"
  end
end

impl Named for Box do
  fn name(self) -> String do
    "box"
  end
end

impl Sized for P do
  fn size(self) do
    self.x
  end
end

impl Make for P do
  fn make() do
    P { x: 1 }
  end
end

impl Named for P do
  fn name(self) do
    5
  end
end
"#
        ),
        [
            "an `impl` for the generic type `List` is not supported",
            "an `impl` for the generic type `Box` is not supported",
            "type mismatch: expected `String`, found `Int`",
        ]
    );
}

/// A struct derives `Json` only of fields JSON can hold (no function), and
/// an impl binds an associated type to any type, a generic one included.
#[test]
fn derived_json_and_associated_types_take_what_they_can_hold() {
    assert_eq!(
        errors(
            r#"
struct Holder do
  f :: Fun(Int) -> Int
end deriving(Json)

interface Container do
  type Item
  fn items(self) -> Self.Item
end

struct Bag do
  xs :: List<Int>
end

impl Container for Bag do
  type Item = List<Int>
  fn items(self) -> List<Int> do
    self.xs
  end
end

fn bag_items(b :: Bag) -> Int do
  List.length(b.items())
end
"#
        ),
        ["field `f` of type `(Int) -> Int` is not JSON-serializable"]
    );
}

/// A schema reads its options, and its fields may have their names.
#[test]
fn a_schema_reads_its_options_beside_fields_of_their_names() {
    let parse = mesh_parser::parse(
        r#"
struct Seat do
  table "seating"
  primary_key :code
  timestamps true
  table :: Int
  code :: String
end deriving(Schema)

fn main() do
  let seat = Seat { table: 1, code: "a" }
  seat.table + 1
end
"#,
    );
    let result = mesh_typeck::check(&parse);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let schema = result.type_registry.struct_defs["Seat"]
        .schema
        .as_ref()
        .expect("a schema");
    assert_eq!(
        (
            schema.table.as_str(),
            schema.primary_key.as_str(),
            schema.timestamps
        ),
        ("seating", "code", true)
    );
}

/// What a sum type derives is checked: a trait no type derives, any trait
/// for a type that holds a resource, and `Json` for variant fields, named
/// or not, that JSON cannot hold.
#[test]
fn sum_type_derives_are_checked() {
    assert_eq!(
        errors(
            r#"
type Odd do
  A
end deriving(Frobnicate)

type Holder do
  H(PgConn)
end deriving(Eq)

type Msg do
  Ping(to :: Pid<Int>)
  Pong(Pid<Int>)
end deriving(Json)
"#
        ),
        [
            "cannot derive `Frobnicate` for `Odd` -- structs derive Eq, Ord, Display, Debug, Hash, Json, Row, and Schema; sum types all but Row and Schema",
            "resource ownership violation: resource type `Holder` cannot derive `Eq`",
            "field `Ping::to` of type `Pid<Int>` is not JSON-serializable",
            "field `Pong::0` of type `Pid<Int>` is not JSON-serializable",
        ]
    );
}

/// A default method calls the interface's own methods with the arguments
/// they declare, and gets what they return, `Self.Item` included:
/// `self.plus("s")` was let through.
#[test]
fn default_methods_call_their_interfaces_methods_as_declared() {
    assert_eq!(
        errors(
            r#"
interface Counted do
  type Item
  fn count(self) -> Int
  fn plus(self, n :: Int) -> Int
  fn first(self) -> Self.Item
  fn bad(self) -> Int do
    self.plus("s")
  end
  fn fine(self) -> Int do
    self.plus(self.count())
  end
  fn again(self) -> Self.Item do
    self.first()
  end
  fn missing(self) -> Int do
    self.nothing()
  end
  fn shown(self) -> String do
    let n = self.count()
    n.to_string()
  end
end
"#
        ),
        [
            "type mismatch: expected `Int`, found `String`",
            "no method `nothing` on type `Self`",
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

/// An error in any part of a control-flow expression is reported, and the
/// function after it is checked as ever.
#[test]
fn errors_in_each_part_of_control_flow_are_reported() {
    assert_eq!(
        errors(
            r#"
fn if_condition() do
  if nope1() do
    1
  end
end

fn if_not_bool() do
  if 1 do
    1
  end
end

fn else_if() do
  if true do
    1
  else if nope2() do
    2
  end
end

fn branches() do
  if true do
    1
  else
    "s"
  end
end

fn scrutinee() do
  case nope3() do
    _ -> 1
  end
end

fn arm_pattern(n :: Int) do
  case n do
    Some(x) -> 1
    _ -> 2
  end
end

fn arm_guard(n :: Int) do
  case n do
    x when nope4(x) -> 1
    _ -> 2
  end
end

fn arm_body(n :: Int) do
  case n do
    1 -> nope5()
    _ -> 2
  end
end

fn arm_types(n :: Int) do
  case n do
    1 -> 1
    _ -> "s"
  end
end

fn iterable() do
  for x in nope6() do
    x
  end
end

fn filter_type(xs :: List<Int>) do
  for x in xs when x do
    x
  end
end

fn range_ends() do
  for x in 1.."s" do
    x
  end
end

fn not_iterable() do
  for x in 5 do
    x
  end
end

fn while_condition() do
  while nope7() do
    1
  end
end

fn while_not_bool() do
  while 1 do
    1
  end
end

fn unknown_iterable(xs) do
  for x in xs do
    x + 1
  end
end
"#
        ),
        [
            "undefined variable `nope1`",
            "type mismatch: expected `Bool`, found `Int`",
            "undefined variable `nope2`",
            "type mismatch: expected `Int`, found `String`",
            "undefined variable `nope3`",
            "type mismatch: expected `Int`, found `Option<_>`",
            "undefined variable `nope4`",
            "undefined variable `nope5`",
            "type mismatch: expected `Int`, found `String`",
            "undefined variable `nope6`",
            "type mismatch: expected `Bool`, found `Int`",
            "type mismatch: expected `Int`, found `String`",
            "`Int` does not implement `Iterable`",
            "undefined variable `nope7`",
            "type mismatch: expected `Bool`, found `Int`",
        ]
    );
}

/// The same for each part of a `receive`: an arm's pattern, guard and
/// body, and the `after` clause's timeout and body.
#[test]
fn errors_in_each_part_of_a_receive_are_reported() {
    assert_eq!(
        errors(
            r#"
actor pattern_types() do
  receive do
    1 -> 1
    "s" -> 2
  end
end

actor guard_error() do
  receive do
    n when nope(n) -> 1
  end
end

actor body_error() do
  receive do
    n -> nope(n)
  end
end

actor arm_types() do
  receive do
    1 -> 1
    _ -> "s"
  end
end

actor timeout_error() do
  receive do
    n -> 1
  after nope ->
    2
  end
end

actor after_body_error() do
  receive do
    n -> 1
  after 10 ->
    nope()
  end
end

actor after_type() do
  receive do
    n -> 1
  after 10 ->
    "late"
  end
end

actor only_after() do
  receive do
  after 10 ->
    1
  end
end
"#
        ),
        [
            "type mismatch: expected `Int`, found `String`",
            "undefined variable `nope`",
            "undefined variable `nope`",
            "type mismatch: expected `Int`, found `String`",
            "undefined variable `nope`",
            "undefined variable `nope`",
            "type mismatch: expected `Int`, found `String`",
            "cannot tell what type of message `guard_error` receives",
            "cannot tell what type of message `body_error` receives",
        ]
    );
}

/// A receive's timeout is an `Int`, and one of another type is reported at
/// the timeout: it was reported at the whole receive.
#[test]
fn a_receive_timeout_of_another_type_is_reported_at_it() {
    assert_eq!(
        located_errors(
            "actor waiter() do\n  receive do\n    n -> n + 1\n  after \"soon\" ->\n    2\n  end\nend\n"
        ),
        [(
            "type mismatch: expected `Int`, found `String`".to_string(),
            "\"soon\"".to_string()
        )]
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

/// A list's elements, and a map's keys and values, are of the first one's
/// type, and one of another type is reported at it as found where that
/// type was expected: it was the other way round, "from annotation".
#[test]
fn collection_literal_mismatches_are_reported_at_the_element() {
    let at = |message: &str, text: &str| (message.to_string(), text.to_string());
    assert_eq!(
        located_errors(
            r#"fn list() do
  [1, "a"]
end

fn keys() do
  %{"a" => 1, 2 => 3}
end

fn values() do
  %{"a" => 1, "b" => "c"}
end
"#
        ),
        [
            at("type mismatch: expected `Int`, found `String`", "\"a\""),
            at("type mismatch: expected `String`, found `Int`", "2"),
            at("type mismatch: expected `Int`, found `String`", "\"c\""),
        ]
    );
}

/// An interpolated expression is checked, and shown with its `Display`.
#[test]
fn interpolations_are_shown_with_display() {
    assert_eq!(
        errors(
            r##"
struct P do
  x :: Int
end deriving(Eq)

fn undefined() do
  "a #{nope} b"
end

fn not_shown(p :: P) do
  "p is #{p}"
end

fn shown(n :: Int) do
  "n is #{n}"
end
"##
        ),
        [
            "undefined variable `nope`",
            "`P` does not implement `Display`",
        ]
    );
}

/// A `let` that destructures its value checks it: a value with an error
/// leaves the names defined, and one of another shape is a mismatch.
#[test]
fn destructuring_a_let_checks_its_value() {
    assert_eq!(
        errors(
            r#"
fn destructure_error() do
  let (a, b) = nope
  a
end

fn destructure_mismatch() do
  let (a, b) = 5
  a
end
"#
        ),
        [
            "undefined variable `nope`",
            "type mismatch: expected `(_, _)`, found `Int`",
        ]
    );
}

/// A `for` loop's pattern is checked as a `let`'s is, and its errors say
/// so: each was an "invalid let destructuring pattern" of "let binders".
#[test]
fn a_for_loops_pattern_is_checked_as_a_lets() {
    assert_eq!(
        errors(
            r#"
fn repeated() do
  for (a, a) in [(1, 2)] do
    a
  end
end

fn refutable() do
  for (1, b) in [(1, 2)] do
    b
  end
end
"#
        ),
        [
            "invalid destructuring pattern: the names a pattern binds must be unique; `a` is repeated",
            "invalid destructuring pattern: a `let` or `for` pattern must match every value (use `case` to match some)",
        ]
    );
}

/// An alias's target is checked as an annotation's types are: each type in
/// it, the builtins' and the imported ones included. Only its first name
/// was, looked up among a few builtins: `type R = Request` and `type C =
/// Channel<Int>` were "undefined", `type P = Geo.Point` named `Geo`, and
/// `List<Nope>` passed.
#[test]
fn an_aliases_target_is_checked_as_an_annotations_types() {
    assert_eq!(
        errors(
            r#"
type R = Request
type I = Iter<Int>
type J = Json
type C = Channel<Int>
type Pair<A> = (A, A)
type LN = List<Nope>

fn size(l :: LN) -> Int do
  List.length(l)
end
"#
        ),
        ["type alias `LN` references undefined type `Nope`"]
    );
    assert_eq!(
        errors_importing(
            "Geo",
            "pub struct Point do\n  x :: Int\nend\n",
            r#"
import Geo

type P = Geo.Point

fn x_of(p :: P) -> Int do
  p.x
end

fn main() do
  x_of(Geo.Point { x: 1 })
end
"#
        ),
        Vec::<String>::new()
    );
}

/// A value that is no function cannot be called, directly or piped into,
/// though its arguments are checked first; a call of what never returns
/// (`panic(...)`) is anything. A field read from a value nothing gives a
/// type has no struct to come from.
#[test]
fn calls_of_what_is_no_function_are_reported() {
    assert_eq!(
        errors(
            r#"
fn not_fn() do
  let n = 5
  n(1)
end

fn not_fn_arg_error() do
  let n = 5
  n(nope)
end

fn piped_not_fn() do
  let n = 5
  1 |> n
end

fn never_callee() do
  panic("x")(1)
end

fn get_x(p) do
  p.x
end
"#
        ),
        [
            "`Int` is not a function",
            "undefined variable `nope`",
            "`Int` is not a function",
            "cannot tell which type has the field `x`",
        ]
    );
}

/// What goes wrong in a pipe is reported: its value, its function, a
/// clustered route wrapper out of its place, and a slot past the function's
/// arguments, with the function named as written (a qualified one was ``).
#[test]
fn errors_in_each_part_of_a_pipe_are_reported() {
    assert_eq!(
        errors(
            r#"
fn value_error() do
  nope1 |> String.length
end

fn function_error() do
  1 |> nope2
end

fn call_error() do
  1 |> nope3(2)
end

fn clustered(h :: Int) do
  h |> HTTP.clustered(1)
end

fn slot() do
  "s" |3> String.length()
end
"#
        ),
        [
            "undefined variable `nope1`",
            "undefined variable `nope2`",
            "undefined variable `nope3`",
            "HTTP.clustered(...) can only appear in the route handler position of HTTP.route(...) or HTTP.on_*(...)",
            "slot position 3 is out of range: `String.length` takes fewer than 2 arguments; use |> instead",
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

/// A declared type parameter has no fields and no methods but its bounds':
/// each was "cannot tell which type has the field", as if a later line
/// could still say.
#[test]
fn type_parameters_have_only_their_bounds_methods() {
    assert_eq!(
        errors(
            r#"
interface Named do
  fn label(self) -> String
end

fn field_of<T>(x :: T) -> String do
  x.name
end

fn method_of<T>(x :: T) -> Int where T: Named do
  x.count()
end

fn bound_method<T>(x :: T) -> String where T: Named do
  x.label()
end

fn in_closure<T>(x :: T) -> Int do
  let f = fn () -> x.size end
  1
end
"#
        ),
        [
            "type `T` has no field `name`",
            "no method `count` on type `T`",
            "type `T` has no field `size`",
        ]
    );
}

/// `Type.from_json`, `Type.from_row` and the schema functions come from the
/// type's `deriving`: without it the type has no such function, and a sum
/// type has no `from_row` at all.
#[test]
fn a_types_derived_functions_need_its_deriving() {
    assert_eq!(
        errors(
            r#"
struct Plain do
  name :: String
end deriving(Eq)

type Kind do
  Big
  Small
end deriving(Eq)

struct Wrap<T> do
  item :: T
end deriving(Json)

fn wrapped() -> Int do
  case Wrap.from_json("{}") do
    Ok(w) -> w.item + 1
    Err(_) -> 0
  end
end

fn rows() do
  Plain.from_row
end

fn decoded() do
  Plain.from_json("{}")
end

fn table() do
  Plain.__table__()
end

fn sum_rows() do
  Kind.from_row
end
"#
        ),
        [
            "no method `from_row` on type `Plain`",
            "no method `from_json` on type `Plain`",
            "no method `__table__` on type `Plain`",
            "no method `from_row` on type `Kind`",
        ]
    );
}

/// An interface method called bare dispatches on its first argument: to a
/// type parameter's bound, or to the impl of the argument's type, whose
/// other arguments it then checks.
#[test]
fn bare_calls_of_interface_methods_dispatch_on_their_first_argument() {
    assert_eq!(
        errors(
            r#"
interface Shows do
  fn shows(self) -> String
end

interface Sized do
  fn size(self) -> Int
end

interface Scales do
  fn scale(self, by :: Int) -> Int
end

struct P do
  x :: Int
end

impl Shows for P do
  fn shows(self) -> String do
    "p"
  end
end

impl Scales for P do
  fn scale(self, by :: Int) -> Int do
    self.x * by
  end
end

fn both<T>(a :: T) -> String where T: Sized, T: Shows do
  shows(a)
end

fn second<A, B>(a :: A, b :: B) -> String where A: Sized, B: Shows do
  shows(b)
end

fn unbound<A>(a :: A) -> String where A: Sized do
  shows(a)
end

fn scaled(p :: P) -> Int do
  scale(p, 2)
end

fn wrong_arg(p :: P) -> Int do
  scale(p, "two")
end

fn undefined_receiver() do
  shows(nope)
end
"#
        ),
        [
            "type parameter `A` stands for any type, but this function makes it `P`",
            "type mismatch: expected `Int`, found `String`",
            "undefined variable `nope`",
        ]
    );
}

/// An alias's name is a type's, not a value's, alone or before a dot: it
/// was "undefined variable".
#[test]
fn an_alias_names_no_value() {
    assert_eq!(
        errors(
            r#"
type Shade = Int

fn decoded() do
  Shade.from_json("1")
end

fn alone() do
  let s = Shade
  s
end
"#
        ),
        [
            "`Shade` is a type, not a value",
            "`Shade` is a type, not a value",
        ]
    );
}

/// A method call finds its method through the receiver's interfaces, a
/// type parameter's bounds, or the standard module of its type, and says
/// which interfaces make it ambiguous or that none has it.
#[test]
fn methods_are_found_through_interfaces_bounds_and_modules() {
    assert_eq!(
        errors(
            r#"
interface Named do
  fn label(self) -> String
end

interface Titled do
  fn label(self) -> String
end

struct Book do
  pages :: Int
end

impl Named for Book do
  fn label(self) -> String do
    "named"
  end
end

impl Titled for Book do
  fn label(self) -> String do
    "titled"
  end
end

fn ambiguous(b :: Book) do
  b.label()
end

fn second<A, B>(a :: A, b :: B) -> String where A: Named, B: Titled do
  b.label()
end

fn iter_other(xs :: List<Int>) do
  let it = Iter.from(xs)
  it.to_string()
end

fn list_missing(xs :: List<Int>) do
  xs.nonexistent()
end
"#
        ),
        [
            "ambiguous method `label` for type `Book`: candidates from traits [Named, Titled]",
            "no method `to_string` on type `Iter<Int>`",
            "no method `nonexistent` on type `List<Int>`",
        ]
    );
}

/// A struct update's base must be a struct value, and each field it gives
/// one of the struct's, once, of the field's type (a generic struct's
/// field as the base's argument makes it).
#[test]
fn struct_updates_check_their_base_and_fields() {
    assert_eq!(
        errors(
            r#"
struct Point do
  x :: Int
end

struct Box<T> do
  item :: T
end

fn unknown(p :: Point) -> Point do
  %{p | nope: 1}
end

fn not_struct(n :: Int) do
  %{n | x: 1}
end

fn dup(p :: Point) -> Point do
  %{p | x: 1, x: 2}
end

fn bad_value(p :: Point) -> Point do
  %{p | x: "s"}
end

fn value_error(p :: Point) -> Point do
  %{p | x: nope}
end

fn base_error() do
  %{nope2 | x: 1}
end

fn generic(b :: Box<Int>) -> Box<Int> do
  %{b | item: "s"}
end
"#
        ),
        [
            "unknown field `nope` in struct `Point`",
            "`Int` is not a struct",
            "field `x` is given more than once",
            "type mismatch: expected `Int`, found `String`",
            "undefined variable `nope`",
            "undefined variable `nope2`",
            "type mismatch: expected `Int`, found `String`",
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

/// An arm with no `->` stands for its pattern read back as a value, which
/// only a pattern of constructors, literals and bound names is: `_`, a
/// variant with its payload left out, or a tuple is not one.
#[test]
fn pass_through_arms_are_values_of_their_patterns() {
    assert_eq!(
        errors(
            r#"
fn wild(o :: Option<Int>) -> Option<Int> do
  case o do
    Some(n) -> Some(n + 1)
    _
  end
end

fn bare_ctor(o :: Option<Int>) -> Option<Int> do
  case o do
    None
    Some
  end
end

fn tuple(p :: (Int, Int)) -> (Int, Int) do
  case p do
    (a, b)
  end
end

fn nested(r :: Result<Option<Int>, String>) -> Result<Option<Int>, String> do
  case r do
    Ok(Some(n))
    Ok(None)
    Err(e)
  end
end

fn literal(n :: Int) -> Int do
  case n do
    1
    _ -> 0
  end
end
"#
        ),
        [
            "this arm has no `->` and its pattern is not a value: `_` names no value",
            "this arm has no `->` and its pattern is not a value: `Some` alone leaves its payload unnamed",
            "this arm has no `->` and its pattern is not a value: only constructors, literals and the names a pattern binds can stand for the arm's value",
        ]
    );
}

/// Each clause of a function is checked: its patterns against the others',
/// its body, and an early `return` against what the clauses give; the
/// first clause's type parameters and return type hold for all.
#[test]
fn each_clause_of_a_function_is_checked() {
    assert_eq!(
        errors(
            r#"
fn first<T>(x :: T, 0) -> T = x
fn first(x, n) = x

fn kinds(0) = 1
fn kinds("s") = 2

fn body_error(0) = nope
fn body_error(n) = n

fn early(0) do
  return 1
end
fn early(n) = n + 1

fn early_wrong(0) do
  return "s"
end
fn early_wrong(n) = n + 1

fn main() do
  first("a", 0)
end
"#
        ),
        [
            "type mismatch: expected `Int`, found `String`",
            "undefined variable `nope`",
            "type mismatch: expected `Int`, found `String`",
        ]
    );
}

/// Each clause of a closure is checked as a function's is: its patterns,
/// its guard, its body against the others', and an early `return`. A
/// map's keys and values are checked like any expression.
#[test]
fn each_clause_of_a_closure_is_checked() {
    assert_eq!(
        errors(
            r#"
fn kinds() do
  let f = fn 0 -> 1 | "s" -> 2 end
  f
end

fn guard_error() do
  let f = fn n when nope(n) -> 1 | _ -> 2 end
  f
end

fn body_types() do
  let f = fn 0 -> 1 | _ -> "s" end
  f
end

fn returns() do
  let f = fn 0 -> return 1 | n -> n end
  f(2)
end

fn map_errors() do
  let a = %{nope1 => 1}
  let b = %{"a" => nope2}
  a
end
"#
        ),
        [
            "type mismatch: expected `Int`, found `String`",
            "undefined variable `nope`",
            "type mismatch: expected `Int`, found `String`",
            "undefined variable `nope1`",
            "undefined variable `nope2`",
        ]
    );
}

/// A clause that matches anything, `_` included, must be a function's last:
/// the clauses after it are unreachable.
#[test]
fn a_catch_all_clause_comes_last() {
    assert_eq!(
        errors("fn f(_) = 0\nfn f(1) = 1\n\nfn h(x) = x\nfn h(2) = 2\n"),
        [
            "catch-all clause must be the last clause of function `f/1`; clauses after a catch-all are unreachable",
            "catch-all clause must be the last clause of function `h/1`; clauses after a catch-all are unreachable",
        ]
    );
}

// ── Exhaustiveness ─────────────────────────────────────────────────────

/// A match on a value whose type is not settled names it with a hole: it
/// was the checker's own variable, `?8`.
#[test]
fn a_match_on_an_unsettled_type_names_it_with_a_hole() {
    let parse = mesh_parser::parse("fn pick(x) when true = 1\n");
    let warnings: Vec<String> = mesh_typeck::check(&parse)
        .warnings
        .iter()
        .map(|warning| warning.to_string())
        .collect();
    assert_eq!(
        warnings,
        ["clauses do not cover every `_`: missing patterns [_]"]
    );
}

/// Float and string literals have endless values, so arms of them need a
/// catch-all; `nil` has one value; a list pattern covers lists of its
/// length only; a pattern of an unknown variant or struct is reported as
/// that.
#[test]
fn literal_and_list_patterns_cover_what_they_name() {
    assert_eq!(
        errors(
            r#"
fn floats(x :: Float) -> Int do
  case x do
    1.5 -> 1
    -2.5 -> 2
  end
end

fn strings(s :: String) -> Int do
  case s do
    "a" -> 1
    "b" -> 2
  end
end

fn nils(n) -> Int do
  case n do
    nil -> 1
  end
end

fn lists(xs :: List<Int>) -> Int do
  case xs do
    [1, 2] -> 1
    [] -> 0
  end
end

fn unknown_variant(o :: Option<Int>) -> Int do
  case o do
    Nope(n) -> n
    _ -> 0
  end
end

fn unknown_struct(o :: Int) -> Int do
  case o do
    Nope { x } -> x
    _ -> 0
  end
end
"#
        ),
        [
            "non-exhaustive match on `Float`: missing patterns [_]",
            "non-exhaustive match on `String`: missing patterns [_]",
            "non-exhaustive match on `List<Int>`: missing patterns [_ :: _ :: _]",
            "unknown variant `Nope`",
            "unknown type `Nope`",
        ]
    );
}

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

/// `import Module` brings in the module's functions, structs, variants and
/// services under its name, and a standard module needs no import at all.
/// A function the module lacks is named with those it has.
#[test]
fn import_brings_in_a_module_by_its_name() {
    assert_eq!(
        errors_importing(
            "Store",
            STORE,
            r#"
import Store
import String

fn main() do
  let pid = Counter.start(0)
  let n = Counter.get(pid)
  let k = Store.Big
  Store.pad(1) + Store.pad(n, 2) + String.length("abc")
end

fn missing() do
  Store.nope(1)
end
"#
        ),
        ["module `Store` has no function `nope`"]
    );
}

/// What an import cannot bring in: a name a module does not export or
/// keeps private, a function a standard module does not have (it was left
/// out without a word), or anything of a module that does not exist. The
/// names a module has are listed sorted, an overloaded function once.
#[test]
fn imports_of_what_is_not_there_are_errors() {
    let errors = errors_importing(
        "Store",
        STORE,
        "from Store import secret, Nope\nfrom String import length, lenght\nfrom Nowhere import thing\nimport Elsewhere\n",
    );
    let headlines: Vec<&str> = errors
        .iter()
        .map(|error| error.split("; ").next().unwrap())
        .collect();
    assert_eq!(
        headlines,
        [
            "`secret` is private in module `Store`",
            "`Nope` is not exported by module `Store`",
            "module `String` has no function `lenght`",
            "module `Nowhere` not found",
            "module `Elsewhere` not found",
        ]
    );
    assert_eq!(
        errors[1],
        "`Nope` is not exported by module `Store`; available: Counter, Kind, Labelled, Pinger, Shade, User, pad"
    );
}

// ── Actors ─────────────────────────────────────────────────────────────

/// `spawn` needs an actor and `send` a pid and a message; each was accepted
/// without them (`spawn()`, `send(p)`), and `send` to a value that is no pid
/// (`send(5, 1)`) as well.
#[test]
fn spawn_and_send_take_what_they_need() {
    assert_eq!(
        errors(
            r#"
actor counter(n :: Int) do
  receive do
    m -> counter(n + m)
  end
end

fn no_args() do
  spawn()
end

fn one_arg(p :: Pid<Int>) do
  send(p)
end

fn not_pid() do
  send(5, 1)
end

fn wrong_msg(p :: Pid<Int>) do
  send(p, "s")
end

fn not_fn() do
  spawn(5)
end

fn piped(p :: Pid<Int>) do
  "s" |2> send(p)
end

fn piped_arity(p :: Pid<Int>) do
  1 |> send(p, 2)
end
"#
        ),
        [
            "arity mismatch: expected 1 argument, found 0",
            "arity mismatch: expected 2 arguments, found 1",
            "type mismatch: expected `Pid<Int>`, found `Int`",
            "message type mismatch: expected `Int`, found `String`",
            "cannot spawn non-function: found `Int`",
            "message type mismatch: expected `Int`, found `String`",
            "arity mismatch: expected 2 arguments, found 3",
        ]
    );
}

/// `Node.spawn` takes a node, an actor and the actor's arguments, whether
/// it is called or piped into. A pipe's call was not checked at all, and a
/// call without its actor, which code generation cannot compile, was let
/// through.
#[test]
fn node_spawn_takes_a_node_and_an_actor_however_it_is_called() {
    assert_eq!(
        errors(
            r#"
actor worker(n :: Int) do
  receive do
    m -> worker(n + m)
  end
end

fn direct() do
  Node.spawn("n@h", worker, 1)
end

fn piped() do
  "n@h" |> Node.spawn(worker, 1)
end

fn slot(n :: Int) do
  n |3> Node.spawn("n@h", worker)
end

fn too_few() do
  Node.spawn("n@h")
end

fn piped_wrong() do
  "n@h" |> Node.spawn(worker, "s")
end

fn piped_bare() do
  "n@h" |> Node.spawn
end

fn as_value() do
  let f = Node.spawn
  0
end

fn bad_node() do
  Node.spawn_link(1, worker, 1)
end
"#
        ),
        [
            "arity mismatch: expected 2 arguments, found 1",
            "type mismatch: expected `Int`, found `String`",
            "arity mismatch: expected 2 arguments, found 1",
            "arity mismatch: expected 2 arguments, found 0",
            "type mismatch: expected `String`, found `Int`",
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
