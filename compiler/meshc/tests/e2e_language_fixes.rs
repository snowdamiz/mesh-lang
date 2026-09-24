//! Regression tests for compiler and language fixes: each test is a program
//! that once miscompiled, crashed, or was wrongly rejected.

use std::path::PathBuf;
use std::process::Command;

fn find_meshc() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("cannot find current exe")
        .parent()
        .expect("cannot find parent dir")
        .to_path_buf();
    if path.file_name().is_some_and(|n| n == "deps") {
        path = path.parent().unwrap().to_path_buf();
    }
    let meshc = path.join("meshc");
    assert!(
        meshc.exists(),
        "meshc binary not found at {}",
        meshc.display()
    );
    meshc
}

struct Build {
    dir: tempfile::TempDir,
    ok: bool,
    stderr: String,
}

fn build(source: &str, json: bool) -> Build {
    build_with_args(source, if json { &["--json"] } else { &[] })
}

fn build_with_args(source: &str, args: &[&str]) -> Build {
    let dir = tempfile::tempdir().expect("temp dir");
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).expect("project dir");
    std::fs::write(project.join("main.mpl"), source).expect("main.mpl");
    let output = Command::new(find_meshc())
        .arg("build")
        .arg(&project)
        .arg("--no-color")
        .args(args)
        .output()
        .expect("meshc");
    Build {
        dir,
        ok: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// Build and run; the program must build, run, and exit 0. Returns stdout.
fn run(source: &str) -> String {
    run_built(&build(source, false))
}

/// Run a program built by `build_with_args`; it must exit 0. Returns stdout.
fn run_built(built: &Build) -> String {
    assert!(built.ok, "build failed:\n{}", built.stderr);
    let output = Command::new(built.dir.path().join("project/project"))
        .output()
        .expect("run binary");
    assert!(
        output.status.success(),
        "binary failed with {:?}:\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Build and run, also returning the compiler's stderr (warnings).
fn run_with_build_stderr(source: &str) -> (String, String) {
    let built = build(source, false);
    assert!(built.ok, "build failed:\n{}", built.stderr);
    let output = Command::new(built.dir.path().join("project/project"))
        .output()
        .expect("run binary");
    assert!(output.status.success(), "binary failed: {:?}", output);
    (
        built.stderr,
        String::from_utf8_lossy(&output.stdout).to_string(),
    )
}

fn build_error(source: &str) -> String {
    let built = build(source, false);
    assert!(!built.ok, "expected the build to fail:\n{}", built.stderr);
    built.stderr
}

fn json_diagnostics(source: &str) -> Vec<serde_json::Value> {
    let built = build(source, true);
    built
        .stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("diagnostic line is not one JSON object: {e}\n{line}"))
        })
        .collect()
}

// ── Diagnostics ────────────────────────────────────────────────────────

#[test]
fn diagnostic_spans_are_source_offsets_despite_whitespace() {
    let source = "fn main() do\n    let   a     =    1\n    println(zzz)\nend\n";
    let diags = json_diagnostics(source);
    let unbound = diags
        .iter()
        .find(|d| d["code"] == "E0004")
        .expect("unbound variable diagnostic");
    let span = &unbound["spans"][0];
    let start = source.find("zzz").unwrap();
    assert_eq!(span["start"], start);
    assert_eq!(span["end"], start + 3);
}

#[test]
fn json_diagnostics_are_one_object_per_line() {
    let source = "fn main() do\n  println(aaa)\n  println(bbb)\nend\n";
    let diags = json_diagnostics(source);
    let codes: Vec<&str> = diags.iter().filter_map(|d| d["code"].as_str()).collect();
    assert_eq!(codes, ["E0004", "E0004", "C0001"]);
}

#[test]
fn not_on_an_int_reports_the_operator() {
    let source = "fn main() do\n  println(\"#{not 1 == 2}\")\nend\n";
    let diags = json_diagnostics(source);
    let mismatch = &diags[0];
    assert_eq!(mismatch["code"], "E0001");
    assert!(mismatch["message"]
        .as_str()
        .unwrap()
        .contains("expected `Bool`, found `Int`"));
    assert_eq!(mismatch["spans"][0]["start"], source.find("not").unwrap());
}

// ── Pattern matching ───────────────────────────────────────────────────

#[test]
fn tuple_of_bool_and_int_match_is_exhaustive() {
    let source = r##"
fn boolpat(b :: Bool, n :: Int) -> String do
  case (b, n) do
    (true, 0) -> "t0"
    (true, _) -> "tN"
    (false, 0) -> "f0"
    (false, _) -> "fN"
  end
end

fn tup(t :: (Int, (String, Bool))) -> String do
  case t do
    (0, (s, true)) -> "zero true #{s}"
    (0, (s, false)) -> "zero false #{s}"
    (n, (s, _)) when n < 0 -> "neg #{n} #{s}"
    (n, (_, b)) -> "pos #{n} #{b}"
  end
end

fn main() do
  println(boolpat(true, 0) <> boolpat(true, 1) <> boolpat(false, 0) <> boolpat(false, 2))
  println(tup((0, ("s", true))) <> "," <> tup((0, ("s", false))) <> "," <> tup((-3, ("n", true))) <> "," <> tup((7, ("p", false))))
end
"##;
    let (warnings, out) = run_with_build_stderr(source);
    assert_eq!(
        out,
        "t0tNf0fN\nzero true s,zero false s,neg -3 n,pos 7 false\n"
    );
    assert!(
        !warnings.contains("redundant"),
        "spurious warning:\n{warnings}"
    );
}

#[test]
fn guarded_arm_does_not_shadow_its_unguarded_twin() {
    let source = r##"
fn nested(v :: Option<Option<Int>>) -> String do
  case v do
    Some(Some(n)) when n > 10 -> "big #{n}"
    Some(Some(n)) -> "small #{n}"
    Some(None) -> "inner none"
    None -> "outer none"
  end
end

fn main() do
  println(nested(Some(Some(42))) <> "," <> nested(Some(Some(1))) <> "," <> nested(Some(None)) <> "," <> nested(None))
end
"##;
    let (warnings, out) = run_with_build_stderr(source);
    assert_eq!(out, "big 42,small 1,inner none,outer none\n");
    assert!(
        !warnings.contains("redundant"),
        "spurious warning:\n{warnings}"
    );
}

#[test]
fn cons_pattern_is_the_list_constructor_and_parens_group() {
    let source = r##"
fn listpat(xs :: List<Int>) -> String do
  case xs do
    h :: (h2 :: t) -> "two+ #{h} #{h2} rest=#{List.length(t)}"
    h :: t -> "one #{h} rest=#{List.length(t)}"
    _ -> "empty"
  end
end

fn main() do
  println(listpat([1, 2, 3]) <> "," <> listpat([1]) <> "," <> listpat([]))
end
"##;
    let (warnings, out) = run_with_build_stderr(source);
    assert_eq!(out, "two+ 1 2 rest=1,one 1 rest=0,empty\n");
    assert!(
        !warnings.contains("redundant"),
        "spurious warning:\n{warnings}"
    );
}

#[test]
fn cons_pattern_alone_is_not_exhaustive() {
    let err = build_error("fn f(xs :: List<Int>) -> Int do\n  case xs do\n    h :: _ -> h\n  end\nend\nfn main() do\n  println(\"#{f([1])}\")\nend\n");
    assert!(err.contains("non-exhaustive"), "{err}");
    assert!(
        err.contains("[]"),
        "witness should be the empty list:\n{err}"
    );
}

#[test]
fn a_pattern_alone_is_an_arm_that_rebuilds_its_value() {
    let source = r##"
type Step<T> do
  Done(T)
  Failed(String)
end

fn check(n :: Int) -> Result<Int, String> do
  if n > 0 do
    Ok(n)
  else
    Err("not positive")
  end
end

fn measured(n :: Int) -> Result<Int, Int> do
  case check(n) do
    Ok(value)
    Err(message) -> Err(String.length(message))
  end
end

fn widen<T>(r :: Result<T, String>) -> Result<T, Int> do
  case r do
    Ok(value)
    Err(message) -> Err(String.length(message))
  end
end

fn nested(o :: Option<Result<Int, String>>) -> Option<Result<Int, Int>> do
  case o do
    Some(Ok(value))
    Some(Err(message)) -> Some(Err(String.length(message)))
    None
  end
end

fn label(s :: Step<Int>) -> Step<String> do
  case s do
    Done(n) -> Done("step #{n}")
    Failed(reason)
  end
end

fn positive(r :: Result<Int, String>) -> Result<Int, String> do
  case r do
    Ok(v) when v > 0
    Ok(v) -> Err("#{v} is not positive")
    Err(e)
  end
end

fn show(r :: Result<Int, Int>) -> String do
  case r do
    Ok(v) -> "ok #{v}"
    Err(e) -> "err #{e}"
  end
end

fn main() do
  println(show(measured(5)) <> "," <> show(measured(-1)))
  let widened = case widen(Ok("text")) do
    Ok(s) -> s
    Err(e) -> "err #{e}"
  end
  println(widened <> "," <> show(widen(Err("four"))))
  let inner = case nested(Some(Err("four"))) do
    Some(Ok(v)) -> "ok #{v}"
    Some(Err(e)) -> "err #{e}"
    None -> "none"
  end
  let outer = case nested(Some(Ok(7))) do
    Some(Ok(v)) -> "ok #{v}"
    Some(Err(e)) -> "err #{e}"
    None -> "none"
  end
  let missing = case nested(None) do
    Some(_) -> "some"
    None -> "none"
  end
  println(inner <> "," <> outer <> "," <> missing)
  let steps = [label(Done(3)), label(Failed("boom"))]
  println(String.join(List.map(steps, fn s -> case s do
    Done(text) -> text
    Failed(reason) -> "failed #{reason}"
  end end), ","))
  let checked = case positive(Ok(0)) do
    Ok(v) -> "ok #{v}"
    Err(e) -> e
  end
  let kept = case positive(Ok(2)) do
    Ok(v) -> "ok #{v}"
    Err(e) -> e
  end
  println(checked <> "," <> kept)
end
"##;
    assert_eq!(
        run(source),
        "ok 5,err 12\ntext,err 4\nerr 4,ok 7,none\nstep 3,failed boom\n0 is not positive,ok 2\n"
    );

    for (arm, reason) in [
        ("_", "`_` names no value"),
        ("Ok", "`Ok` alone leaves its payload unnamed"),
    ] {
        let err = build_error(&format!(
            "fn f(r :: Result<Int, String>) -> Result<Int, String> do\n  case r do\n    Err(e) -> Err(e)\n    {arm}\n  end\nend\nfn main() do\n  println(\"x\")\nend\n"
        ));
        assert!(
            err.contains("E0056") && err.contains(reason),
            "{arm}:\n{err}"
        );
    }
}

#[test]
fn negative_and_string_literal_patterns_match_their_own_value() {
    let source = r##"
fn describe(x :: Int) -> String do
  case x do
    -1 -> "minus one"
    1 -> "one"
    _ -> "other"
  end
end

fn half(x :: Float) -> String do
  case x do
    -0.5 -> "minus half"
    _ -> "other"
  end
end

fn pair(-1, "x") = "minus one and x"
fn pair(n, "x") = "#{n} and x"
fn pair(_, _) = "neither"

fn checked(r :: Result<Bool, String>) -> Result<Bool, Int> do
  case r do
    Ok(true)
    Ok(false) -> Err(0)
    Err(e) -> Err(String.length(e))
  end
end

fn main() do
  println(describe(-1) <> "," <> describe(1) <> "," <> describe(5))
  println(half(-0.5) <> "," <> half(0.5))
  println(pair(-1, "x") <> "," <> pair(1, "x") <> "," <> pair(-1, "y"))
  let kept = case checked(Ok(true)) do
    Ok(b) -> "ok #{b}"
    Err(e) -> "err #{e}"
  end
  println(kept)
end
"##;
    let (warnings, out) = run_with_build_stderr(source);
    assert_eq!(
        out,
        "minus one,one,other\nminus half,other\nminus one and x,1 and x,neither\nok true\n"
    );
    assert!(
        !warnings.contains("redundant"),
        "`1` is not `-1`:\n{warnings}"
    );
}

#[test]
fn as_pattern_binds_the_matched_value() {
    let source = r##"
fn aspat(t :: (Int, Int)) -> String do
  case t do
    (x, y) as whole when x == y -> "diag #{x} #{Tuple.first(whole)}"
    (x, _) as whole -> "off #{x} #{Tuple.second(whole)}"
  end
end

fn main() do
  println(aspat((2, 2)) <> "," <> aspat((2, 3)))
  let r = case Some(4) do
    Some(n) as w -> case w do
      Some(m) -> "#{n} #{m}"
      None -> "none"
    end
    None -> "outer"
  end
  println(r)
end
"##;
    assert_eq!(run(source), "diag 2 2,off 2 3\n4 4\n");
}

// ── Grammar ────────────────────────────────────────────────────────────

#[test]
fn closure_can_be_a_statement_and_a_tail_expression() {
    let source = r##"
fn compose(f :: Fun(Int) -> Int, g :: Fun(Int) -> Int) -> Fun(Int) -> Int do
  fn x -> g(f(x)) end
end

fn main() do
  let inc = fn x -> x + 1 end
  println("#{compose(inc, inc)(1)}")
end
"##;
    assert_eq!(run(source), "3\n");
}

#[test]
fn match_arm_body_may_start_on_the_next_line() {
    let source = r##"
fn f(o :: Option<Int>) -> Int do
  case o do
    Some(x) ->
      let y = x * 2
      y + 1
    None -> -1
  end
end

fn g(o :: Option<Int>) -> Int do
  case o do
    Some(x) ->
      if x > 0 do
        x
      else
        0
      end
    None -> -1
  end
end

fn main() do
  println("#{f(Some(3))} #{f(None)} #{g(Some(3))} #{g(None)}")
end
"##;
    assert_eq!(run(source), "7 -1 3 -1\n");
}

#[test]
fn empty_match_arm_body_is_rejected() {
    let err = build_error("fn main() do\n  case 1 do\n    1 ->\n  end\nend\n");
    assert!(err.contains("expected expression after `->`"), "{err}");
}

#[test]
fn return_is_an_expression() {
    let source = r##"
fn f(o :: Option<Int>) -> Int do
  let v = case o do
    Some(x) -> x
    None -> return -1
  end
  v * 2
end

fn main() do
  println("#{f(Some(3))} #{f(None)}")
end
"##;
    assert_eq!(run(source), "6 -1\n");
}

#[test]
fn function_ending_in_let_has_unit_type() {
    let source =
        "fn f() do\n  let x = 5\nend\n\nfn main() do\n  f()\n  println(\"ok\")\n  let y = 6\nend\n";
    assert_eq!(run(source), "ok\n");
}

#[test]
fn heredoc_keeps_lines_around_interpolations() {
    // A heredoc ends at its last content line (no trailing newline).
    let source = "fn main() do\n  let a = \"\"\"\n    line one\n    #{1 + 2}\n    line three\n    \"\"\"\n  println(a)\n  let b = \"\"\"\n    x #{1 + 2} y\n    \"\"\"\n  println(b)\nend\n";
    assert_eq!(run(source), "line one\n3\nline three\nx 3 y\n");
}

// ── Closures and inference ─────────────────────────────────────────────

#[test]
fn closure_parameter_annotations_keep_generic_arguments() {
    let source = r##"
fn main() do
  let len = fn(xs :: List<Int>) -> List.length(xs) end
  let show = fn(o :: Option<Int>) -> case o do Some(n) -> "S#{n}" None -> "N" end end
  println("#{len([1, 2])} #{show(Some(1))}")
end
"##;
    assert_eq!(run(source), "2 S1\n");
}

#[test]
fn let_bound_closure_takes_the_type_of_its_use() {
    let source = r##"
fn main() do
  let show = fn o -> "#{o}" end
  let pick = fn o -> case o do
    Some(n) -> "S#{n}"
    None -> "N"
  end end
  let pair = fn p -> case p do (a, b) -> "#{a}#{b}" end end
  println(show(1) <> " " <> pick(Some(2)) <> " " <> pair((3, 4)))
end
"##;
    assert_eq!(run(source), "1 S2 34\n");
}

/// A let-bound closure is polymorphic like a named function: each use type
/// gets its own compiled copy, captures included.
#[test]
fn let_bound_closure_is_specialized_per_use_type() {
    let source = r##"
fn main() do
  let suffix = "!"
  let id = fn x -> x end
  let tag = fn x -> "#{x}#{suffix}" end
  let twice = fn f, x -> f(f(x)) end
  println("#{id(1)} #{id("s")} #{id(2.5)} #{tag(1)} #{tag("s")} #{twice(fn n -> n + 1 end, 1)}")
  println("#{List.map([1, 2], id)} #{List.map(["a"], id)} #{id([true])}")
end
"##;
    assert_eq!(run(source), "1 s 2.5 1! s! 3\n[1, 2] [a] [true]\n");
}

#[test]
fn trait_bound_holds_inside_closures_of_a_generic_function() {
    let source = r##"
interface Describe do
  fn describe(self) -> String
end
impl Describe for Int do
  fn describe(self) -> String do "int #{self}" end
end
impl Describe for String do
  fn describe(self) -> String do "string '#{self}'" end
end
impl Describe for Bool do
  fn describe(self) -> String do if self do "yes" else "no" end end
end
fn show_all<T>(items :: List<T>) -> List<String> where T: Describe do
  List.map(items, fn i -> i.describe() end)
end
fn each<T>(items :: List<T>) -> List<String> where T: Describe do
  for i in items do
    i.describe()
  end
end
fn twice<T>(item :: T) -> String where T: Describe do
  let f = fn -> item.describe() end
  f() <> f()
end
fn main() do
  println("#{show_all([1, 2])} #{show_all(["x"])} #{show_all([true, false])}")
  println("#{each([3])} #{twice(4)}")
end
"##;
    let (warnings, out) = run_with_build_stderr(source);
    assert_eq!(
        out,
        "[int 1, int 2] [string 'x'] [yes, no]\n[int 3] int 4int 4\n"
    );
    assert!(!warnings.contains("type checker bug"), "{warnings}");
}

#[test]
fn generic_function_specializes_per_source_type() {
    let source = r##"
fn each<T>(xs :: List<T>) -> List<String> where T: Display do
  List.map(xs, fn x -> x.to_string() end)
end
fn main() do
  println("#{each([1, 2])} #{each(["a"])} #{each([true])}")
end
"##;
    assert_eq!(run(source), "[1, 2] [a] [true]\n");
}

#[test]
fn pattern_bound_names_interpolate_inside_closures() {
    let source = "fn main() do\n  let r = List.map([Some(1), None], fn o -> case o do Some(n) -> \"#{n}\" None -> \"-\" end end)\n  println(\"#{r}\")\nend\n";
    assert_eq!(run(source), "[1, -]\n");
}

// ── Interfaces ─────────────────────────────────────────────────────────

#[test]
fn default_interface_methods_run_for_each_implementing_type() {
    let source = r##"
interface Shape do
  fn area(self) -> Float
  fn name(self) -> String
  fn describe(self) -> String do
    "#{self.name()} with area #{self.area()}"
  end
end

struct Circle do
  r :: Float
end

struct Sq do
  side :: Float
end

impl Shape for Circle do
  fn area(self) -> Float do 3.0 * self.r * self.r end
  fn name(self) -> String do "circle" end
end

impl Shape for Sq do
  fn area(self) -> Float do self.side * self.side end
  fn name(self) -> String do "square" end
  fn describe(self) -> String do "custom square #{self.side}" end
end

fn main() do
  let c = Circle { r: 1.0 }
  println(c.describe())
  println(Sq { side: 2.0 }.describe())
end
"##;
    assert_eq!(run(source), "circle with area 3.0\ncustom square 2.0\n");
}

#[test]
fn static_interface_methods_are_called_through_the_type() {
    let source = r##"
interface Counter do
  fn zero() -> Int
  fn bump(self, n :: Int) -> Int
end
struct Tally do
  count :: Int
end
impl Counter for Tally do
  fn zero() -> Int do 0 end
  fn bump(self, n :: Int) -> Int do self.count + n end
end
fn main() do
  println("#{Tally.zero()} #{Tally { count: 5 }.bump(3)}")
end
"##;
    assert_eq!(run(source), "0 8\n");
}

// ── Sum types ──────────────────────────────────────────────────────────

#[test]
fn recursive_sum_types_construct_match_and_derive() {
    let source = r##"
type IntList do
  Nil
  Cons(Int, IntList)
end deriving(Eq, Display, Debug)

type Tree<T> do
  Leaf
  Node(Tree<T>, T, Tree<T>)
end

fn sum(l :: IntList) -> Int do
  case l do
    Nil -> 0
    Cons(h, t) -> h + sum(t)
  end
end

fn build(n :: Int) -> IntList do
  if n == 0 do Nil else Cons(n, build(n - 1)) end
end

fn size<T>(t :: Tree<T>) -> Int do
  case t do
    Leaf -> 0
    Node(l, _, r) -> 1 + size(l) + size(r)
  end
end

fn main() do
  let l = Cons(1, Cons(2, Nil))
  println("#{sum(l)} #{sum(build(100))}")
  println("#{l == Cons(1, Cons(2, Nil))} #{l == Cons(1, Nil)}")
  println("#{l}")
  println(l.inspect())
  println("#{size(Node(Node(Leaf, 1, Leaf), 2, Leaf))} #{size(Node(Leaf, "s", Leaf))}")
end
"##;
    assert_eq!(
        run(source),
        "3 5050\ntrue false\nCons(1, Cons(2, Nil))\nCons(1, Cons(2, Nil))\n2 1\n"
    );
}

#[test]
fn debug_inspect_on_sum_types() {
    let source = "type Level do\n  Low\n  Mid\n  High\nend deriving(Eq, Ord, Display, Debug, Hash)\nfn main() do\n  println(\"#{Mid.inspect()} #{Low < High} #{Mid} #{High == High}\")\nend\n";
    assert_eq!(run(source), "Mid true Mid true\n");
}

#[test]
fn lists_of_structs_and_sum_types_display_and_sort() {
    let source = r##"
struct Money do
  cents :: Int
end deriving(Eq, Display)
type Level do
  Low
  Mid
  High
end deriving(Eq, Ord, Display)
fn main() do
  println("#{[Money { cents: 1 }, Money { cents: 2 }]}")
  println("#{[High, Low]}")
  println("#{List.sort([High, Low, Mid], fn x, y -> if x < y do -1 else 1 end end)}")
  println("#{List.reduce([High, Low, Mid], High, fn acc, x -> if x < acc do x else acc end end)}")
end
"##;
    assert_eq!(
        run(source),
        "[Money(1), Money(2)]\n[High, Low]\n[Low, Mid, High]\nLow\n"
    );
}

// ── Runtime values ─────────────────────────────────────────────────────

#[test]
fn runtime_option_passed_directly_to_functions_and_closures() {
    let source = r##"
fn show(o :: Option<Int>) -> String do
  case o do
    Some(n) -> "S#{n}"
    None -> "N"
  end
end
fn main() do
  let pick = fn o -> case o do Some(n) -> "s#{n}" None -> "n" end end
  println(show(String.to_int("42")) <> show(String.to_int("x")) <> pick(String.to_int("7")))
end
"##;
    assert_eq!(run(source), "S42Ns7\n");
}

#[test]
fn list_find_on_scalars_matches() {
    let source = r##"
fn main() do
  let xs = [3, 1, 2]
  let found = case List.find(xs, fn x -> x == 2 end) do
    Some(v) -> "found #{v}"
    None -> "none"
  end
  let missing = case List.find(xs, fn x -> x == 9 end) do
    Some(v) -> "found #{v}"
    None -> "none"
  end
  let s = case List.find(["a", "bb"], fn s -> String.length(s) == 2 end) do
    Some(v) -> v
    None -> "none"
  end
  println("#{found} #{missing} #{s}")
end
"##;
    assert_eq!(run(source), "found 2 none bb\n");
}

#[test]
fn for_over_a_list_of_tuples() {
    let source = "fn main() do\n  let mixed = [(1, \"a\"), (2, \"b\")]\n  for pr in mixed do\n    let n = Tuple.first(pr)\n    let ch = Tuple.second(pr)\n    print(\"#{n}#{ch} \")\n  end\n  println(\"\")\nend\n";
    assert_eq!(run(source), "1a 2b \n");
}

#[test]
fn for_body_ending_in_if_without_else() {
    let source = r##"
fn first_even(xs :: List<Int>) -> Int do
  for x in xs do
    if x % 2 == 0 do
      return x
    end
  end
  -1
end
fn main() do
  println("#{first_even([1, 4])} #{first_even([1])}")
  for x in [1, 2] do
    if x == 1 do
      println("one")
    end
  end
end
"##;
    assert_eq!(run(source), "4 -1\none\n");
}

#[test]
fn queue_pop_destructures_into_front_and_rest() {
    let source = "fn main() do\n  let q = Queue.push(Queue.push(Queue.new(), 1), 2)\n  let (front, rest) = Queue.pop(q)\n  println(\"#{front} #{Queue.size(rest)} #{Tuple.first(Queue.pop(rest))}\")\nend\n";
    assert_eq!(run(source), "1 1 2\n");
}

// ── Structural equality, ordering and display ──────────────────────────

#[test]
fn tuples_maps_sets_unit_and_ordering_compare_by_contents() {
    let source = r##"
struct Point do
  x :: Int
  y :: Int
end

type Shape do
  Circle(Int)
  Poly(List<Int>)
end

fn main() do
  println("#{(1, 2) == (1, 2)} #{(1, "a") == (1, "b")} #{(1, (2, 3)) == (1, (2, 3))}")
  println("#{%{"a" => 1, "b" => 2} == %{"b" => 2, "a" => 1}} #{%{"a" => 1} == %{"a" => 2}}")
  println("#{Set.from_list([1, 2]) == Set.from_list([2, 1])} #{Set.from_list([1]) == Set.from_list([2])}")
  let u = ()
  println("#{u == u} #{compare(1, 2) == Less} #{compare(2, 1)} #{Less < Greater}")
  println("#{Some(1) == Some(1)} #{Some(1) == None} #{Ok(1) == Err("x")} #{Some([1, 2]) == Some([1, 2])}")
  println("#{[Some(1), None] == [Some(1), None]} #{[(1, 2)] == [(1, 2)]} #{[1.0, 2.5] == [1.0, 2.5]} #{[1.0] == [2.0]}")
  println("#{Poly([1, 2]) == Poly([1, 2])} #{Circle(1) == Poly([1])} #{Point { x: 1, y: 2 } == Point { x: 1, y: 2 }}")
  println("#{(1, 2) < (1, 3)} #{(2, 0) > (1, 9)} #{[-1.0] < [1.0]} #{["a"] < ["b"]} #{Some(1) < Some(2)}")
end
"##;
    assert_eq!(
        run(source),
        "true false true\ntrue false\ntrue false\ntrue true Greater true\ntrue false false true\ntrue true true false\ntrue false true\ntrue true true true true\n"
    );
}

#[test]
fn tuples_options_results_and_floats_display_everywhere() {
    let source = r##"
struct Pt do
  x :: Int
end

impl Display for Pt do
  fn to_string(self) do
    "P#{self.x}"
  end
end

fn main() do
  println("#{(1, "a")} #{(1, (2.5, true))} #{[(1, "a"), (2, "b")]}")
  println("#{Some(1)} #{None} #{Some("s")} #{Ok(1)} #{Err("bad")} #{[Some(1), None]} #{[Some("x")]}")
  println("#{Some(1).inspect()} #{Some("x").inspect()} #{(1, "a").inspect()} #{[Some(1)].inspect()}")
  println("#{Some(1).to_string()} #{(1, 2).to_string()} #{()}")
  let p = Pt { x: 1 }
  println("#{p} #{p.to_string()} #{[p]} #{Some(p)} #{(p, 4)} #{%{"k" => (1, 2)}} #{%{1 => Some(2)}}")
  println("#{6.0} #{Float.from(42)} #{1.5 * 2.0} #{[1.0, 2.5]} #{-3.0} #{10.0 / 4.0} #{(1.0, 2.0)} #{Some(2.0)}")
end
"##;
    assert_eq!(
        run(source),
        "(1, a) (1, (2.5, true)) [(1, a), (2, b)]\n\
         Some(1) None Some(s) Ok(1) Err(bad) [Some(1), None] [Some(x)]\n\
         Some(1) Some(\"x\") (1, \"a\") [Some(1)]\n\
         Some(1) (1, 2) ()\n\
         P1 P1 [P1] Some(P1) (P1, 4) %{k => (1, 2)} %{1 => Some(2)}\n\
         6.0 42.0 3.0 [1.0, 2.5] -3.0 2.5 (1.0, 2.0) Some(2.0)\n"
    );
}

#[test]
fn single_letter_type_names_are_types_not_type_parameters() {
    let source = r##"
struct P do
  x :: Int
end deriving(Eq, Display)

type A do
  B
  C(Int)
end deriving(Eq, Display)

fn main() do
  println("#{P { x: 1 } == P { x: 1 }} #{P { x: 2 }} #{C(3) == C(3)} #{B}")
end
"##;
    assert_eq!(run(source), "true P(2) true B\n");
}

// ── Loops, calls and closures ──────────────────────────────────────────

#[test]
fn for_loops_destructure_tuple_patterns() {
    let source = r##"
fn main() do
  let pairs = [(1, "a"), (2, "b"), (3, "c")]
  for (n, s) in pairs when n != 2 do
    println("#{n}: #{s}")
  end
  let squares = for (n, _) in pairs do n * n end
  println("#{squares}")
  for (k, v) in %{"x" => 1} do
    println("#{k}=#{v}")
  end
  for (a, (b, c)) in [(1, (2, 3))] do
    println("#{a + b + c}")
  end
  for (i, x) in List.enumerate(["p", "q"]) do
    println("#{i} #{x}")
  end
end
"##;
    assert_eq!(run(source), "1: a\n3: c\n[1, 4, 9]\nx=1\n6\n0 p\n1 q\n");
}

#[test]
fn function_typed_struct_fields_are_callable_with_dot_syntax() {
    let source = r##"
struct Op do
  run :: Fun(Int) -> Int
end

fn main() do
  let op = Op { run: fn x -> x * 2 end }
  println("#{op.run(10)} #{(op.run)(21)}")
end
"##;
    assert_eq!(run(source), "20 42\n");
}

#[test]
fn ranges_are_values_outside_for_headers() {
    let source = "fn main() do\n  let r = 1..5\n  println(\"#{Range.to_list(r)} #{Range.to_list(3..4)}\")\nend\n";
    assert_eq!(run(source), "[1, 2, 3, 4] [3]\n");
}

#[test]
fn iter_find_returns_a_typed_option() {
    let source = r##"
fn main() do
  let found = Iter.find(Iter.from([1, 2, 3]), fn x -> x > 1 end)
  case found do
    Some(v) -> println("found #{v}")
    None -> println("none")
  end
  case Iter.find(Iter.from(["a"]), fn s -> s == "z" end) do
    Some(v) -> println("found #{v}")
    None -> println("none")
  end
end
"##;
    assert_eq!(run(source), "found 2\nnone\n");
}

#[test]
fn impl_methods_without_return_annotations_return_their_body_type() {
    let source = r##"
interface Named do
  fn label(self) -> String
end

struct User do
  name :: String
end

impl Named for User do
  fn label(self) do
    "user:#{self.name}"
  end
end

fn main() do
  let u = User { name: "ann" }
  println(u.label())
end
"##;
    assert_eq!(run(source), "user:ann\n");
}

// ── Lists ──────────────────────────────────────────────────────────────

#[test]
fn list_tail_is_a_view_so_cons_recursion_is_linear() {
    // 30k elements: with a copying tail this took seconds; a view makes each
    // step constant, and views behave as lists everywhere they flow.
    let source = r##"
fn sum(xs :: List<Int>, acc :: Int) -> Int do
  case xs do
    h :: t -> sum(t, acc + h)
    [] -> acc
  end
end

fn main() do
  let xs = Range.to_list(1..30001)
  println("#{sum(xs, 0)}")
  let t = List.tail([1, 2, 3])
  println("#{t} #{List.length(t)} #{List.drop(xs, 29998)} #{List.append(t, 9)} #{List.tail(t) ++ [7]}")
  println("#{List.tail([1, 2]) == [2]} #{String.join(List.tail(["a", "b", "c"]), "-")} #{Set.size(Set.from_list(List.tail([1, 1, 2])))}")
end
"##;
    let started = std::time::Instant::now();
    assert_eq!(
        run(source),
        "450015000\n[2, 3] 2 [29999, 30000] [2, 3, 9] [3, 7]\ntrue b-c 2\n"
    );
    // Under MESH_GC_STRESS every allocation collects, which is quadratic
    // here by design; the bound is about the copy that used to happen.
    if std::env::var_os("MESH_GC_STRESS").is_none() {
        assert!(started.elapsed() < std::time::Duration::from_secs(60));
    }
}

#[test]
fn list_tail_views_cross_actor_boundaries() {
    let source = r##"
actor printer() do
  receive do
    xs -> println("#{xs} #{List.length(xs)}")
  end
end

fn main() do
  let p = spawn(printer)
  send(p, List.tail([1, 2, 3]))
  Timer.sleep(200)
end
"##;
    assert_eq!(run(source), "[2, 3] 2\n");
}

#[test]
fn list_literal_patterns_match_fixed_lengths() {
    let source = r##"
fn describe(xs :: List<Int>) -> String do
  case xs do
    [] -> "empty"
    [x] -> "one #{x}"
    [x, y] -> "two #{x} #{y}"
    h :: t -> "many #{h} +#{List.length(t)}"
  end
end

fn sum(xs :: List<Int>) -> Int do
  case xs do
    [] -> 0
    h :: t -> h + sum(t)
  end
end

fn pairs(xs :: List<(Int, String)>) -> String do
  case xs do
    [(1, s), _] -> "first is one: #{s}"
    _ -> "other"
  end
end

fn main() do
  println("#{describe([])} #{describe([7])} #{describe([1, 2])} #{describe([1, 2, 3])}")
  println("#{sum([1, 2, 3, 4])} #{pairs([(1, "a"), (2, "b")])} #{pairs([(2, "a"), (1, "b")])}")
  println("#{for l in [[1], [], [2, 3]] do describe(l) end}")
end
"##;
    assert_eq!(
        run(source),
        "empty one 7 two 1 2 many 1 +2\n10 first is one: a other\n[one 1, empty, two 2 3]\n"
    );
    let err = build_error(
        "fn f(xs :: List<Int>) -> Int do\n  case xs do\n    [] -> 0\n    [x] -> x\n  end\nend\nfn main() do\n  println(\"#{f([1])}\")\nend\n",
    );
    assert!(
        err.contains("_ :: _ :: _"),
        "witness should name lists of two or more:\n{err}"
    );
}

// ── Recursive types ────────────────────────────────────────────────────

#[test]
fn mutually_recursive_and_tuple_payload_sum_types() {
    let source = r##"
type Expr do
  Num(Int)
  Block(Stmt)
end

type Stmt do
  Ret(Expr)
  Nop
end

type Chain do
  Leaf
  Node((Int, Chain))
end

fn eval(e :: Expr) -> Int do
  case e do
    Num(n) -> n
    Block(s) -> run(s)
  end
end

fn run(s :: Stmt) -> Int do
  case s do
    Ret(e) -> eval(e)
    Nop -> 0
  end
end

fn depth(c :: Chain) -> Int do
  case c do
    Leaf -> 0
    Node((n, rest)) -> n + depth(rest)
  end
end

fn main() do
  println("#{eval(Block(Ret(Num(7))))} #{run(Nop)} #{depth(Node((1, Node((2, Leaf)))))}")
end
"##;
    assert_eq!(run(source), "7 0 3\n");
}

// ── Branches that never return ─────────────────────────────────────────

#[test]
fn a_returning_first_branch_does_not_type_the_whole_expression() {
    let source = r##"
fn f(x :: Int) -> Int do
  let y = case x do
    0 -> return 5
    _ -> x * 3
  end
  let z = if x > 10 do
    return 7
  else
    y + 1
  end
  z
end

fn main() do
  println("#{f(0)} #{f(2)} #{f(20)}")
end
"##;
    assert_eq!(run(source), "5 7 7\n");
}

// ── Actors ─────────────────────────────────────────────────────────────

/// Lines of output, sorted: separate actors print in no fixed order.
fn sorted_lines(output: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort_unstable();
    lines
}

#[test]
fn receive_matches_arms_by_pattern_and_guard() {
    let source = r##"
struct Job do
  id :: Int
  name :: String
end

fn is_stop(j :: Job) -> Bool do
  j.id == 0
end

actor numbers() do
  receive do
    0 -> println("zero")
    1 -> println("one")
    n ->
      println("other #{n}")
      numbers()
  end
end

actor tuples(seen :: Int) do
  receive do
    (0, label) -> println("zero-tagged #{label} seen=#{seen}")
    (n, label) when n > 100 ->
      let big = n * 2
      println("big #{label} #{big}")
      tuples(seen + 1)
    (n, label) ->
      println("tuple #{n} #{label}")
      tuples(seen + 1)
  after 500 ->
    println("idle after #{seen}")
  end
end

actor jobs() do
  receive do
    j when is_stop(j) -> println("stop job #{j.name}")
    j ->
      println("job #{j.id}: #{j.name}")
      jobs()
  end
end

actor strings() do
  receive do
    "quit" -> println("quit")
    s ->
      println("string #{s}")
      strings()
  end
end

actor floats(total :: Float) do
  let next = receive do
    x when x < 0.0 -> -1.0
    x -> total + x
  end
  if next < 0.0 do
    println("float total #{total}")
  else
    floats(next)
  end
end

fn main() do
  let n = spawn(numbers)
  send(n, 7)
  send(n, 1)
  let t = spawn(tuples, 0)
  send(t, (5, "five"))
  send(t, (500, "huge"))
  send(t, (0, "end"))
  let j = spawn(jobs)
  send(j, Job { id: 3, name: "build" })
  send(j, Job { id: 0, name: "halt" })
  let s = spawn(strings)
  send(s, "héllo")
  send(s, "quit")
  let f = spawn(floats, 0.5)
  send(f, 1.25)
  send(f, 2.0)
  send(f, -3.0)
  let idle = spawn(tuples, 7)
  Timer.sleep(800)
end
"##;
    let expected = [
        "big huge 1000",
        "float total 3.75",
        "idle after 7",
        "job 3: build",
        "one",
        "other 7",
        "quit",
        "stop job halt",
        "string héllo",
        "tuple 5 five",
        "zero-tagged end seen=2",
    ];
    for opt in ["0", "2"] {
        let built = build_with_args(source, &["--opt-level", opt]);
        assert_eq!(
            sorted_lines(&run_built(&built)),
            expected,
            "--opt-level {opt}"
        );
    }
}

#[test]
fn an_actor_arm_may_stop_instead_of_calling_the_actor_again() {
    let source = r##"
type Msg do
  Add(n :: Int)
  Print
  Stop
end

actor acc(total :: Int) do
  receive do
    Add(n) -> acc(total + n)
    Print ->
      println("total=#{total}")
      acc(total)
    Stop -> println("stopping at #{total}")
  end
end

actor looper(n :: Int) do
  receive do
    m ->
      println("got #{m} at #{n}")
      if m > 0 do
        looper(n + 1)
      else
        println("done")
      end
  end
end

fn main() do
  let pid = spawn(acc, 0)
  send(pid, Add(5))
  send(pid, Add(7))
  send(pid, Print)
  send(pid, Stop)
  Timer.sleep(100)
  let l = spawn(looper, 0)
  send(l, 1)
  send(l, 2)
  send(l, 0)
  Timer.sleep(100)
end
"##;
    assert_eq!(
        run(source),
        "total=12\nstopping at 12\ngot 1 at 0\ngot 2 at 1\ngot 0 at 2\ndone\n"
    );
}

#[test]
fn an_actor_calling_itself_outside_tail_position_runs_its_body() {
    let source = r##"
actor nested(depth :: Int) do
  receive do
    m ->
      println("got #{m} at #{depth}")
      if m > 0 do
        nested(depth + 1)
        println("back at #{depth}")
      end
  end
end

fn main() do
  let p = spawn(nested, 0)
  send(p, 1)
  send(p, 0)
  Timer.sleep(100)
end
"##;
    assert_eq!(run(source), "got 1 at 0\ngot 0 at 1\nback at 0\n");
}

#[test]
fn an_actor_message_type_is_inferred_from_what_is_sent_to_it() {
    let source = r##"
actor echo() do
  receive do
    m -> println("echo got #{m}")
  end
end

actor ring(n :: Int) do
  if n > 0 do
    let child = spawn(ring, n - 1)
    send(child, n)
  end
  receive do
    m -> println("ring got #{m}")
  end
end

fn main() do
  let p = spawn(echo)
  send(p, 100)
  let r = spawn(ring, 2)
  send(r, 50)
  Timer.sleep(100)
end
"##;
    assert_eq!(
        sorted_lines(&run(source)),
        ["echo got 100", "ring got 1", "ring got 2", "ring got 50"]
    );
    let err = build_error(
        "actor echo() do\n  receive do\n    m -> println(\"#{m}\")\n  end\nend\nfn main() do\n  let p = spawn(echo)\n  send(p, 1)\n  send(p, \"two\")\nend\n",
    );
    assert!(
        err.contains("message type mismatch: expected Int, found String"),
        "mixed message types:\n{err}"
    );
}

#[test]
fn receive_arms_must_cover_the_message_type() {
    let err = build_error(
        "actor num() do\n  receive do\n    0 -> println(\"zero\")\n    1 -> println(\"one\")\n  end\nend\nfn main() do\n  let pid = spawn(num)\n  send(pid, 1)\nend\n",
    );
    assert!(err.contains("non-exhaustive match on `Int`"), "{err}");
    let (warnings, out) = run_with_build_stderr(
        "actor num() do\n  receive do\n    n -> println(\"any #{n}\")\n    0 -> println(\"zero\")\n  end\nend\nfn main() do\n  let pid = spawn(num)\n  send(pid, 1)\n  Timer.sleep(50)\nend\n",
    );
    assert!(warnings.contains("redundant match arm"), "{warnings}");
    assert_eq!(out, "any 1\n");
}

// ── Numbers ────────────────────────────────────────────────────────────

/// Build and run a program that may fail; returns (exit code, stdout, stderr).
fn run_status(source: &str, args: &[&str]) -> (Option<i32>, String, String) {
    let built = build_with_args(source, args);
    assert!(built.ok, "build failed:\n{}", built.stderr);
    let output = Command::new(built.dir.path().join("project/project"))
        .output()
        .expect("run binary");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn integer_division_by_zero_panics_and_min_divided_by_minus_one_wraps() {
    let wraps = r##"
fn div(a :: Int, b :: Int) -> Int = a / b
fn rem(a :: Int, b :: Int) -> Int = a % b

fn main() do
  let min = 0 - 9223372036854775807 - 1
  println("#{div(min, -1)} #{rem(min, -1)} #{div(7, -1)} #{rem(7, -2)} #{div(-7, 2)}")
end
"##;
    let panics = r##"
fn main() do
  let zero = 0
  println("before")
  println("#{10 % zero}")
  println("after")
end
"##;
    for opt in ["0", "2"] {
        let (code, out, _) = run_status(wraps, &["--opt-level", opt]);
        assert_eq!(code, Some(0));
        assert_eq!(out, "-9223372036854775808 0 -7 1 -3\n", "--opt-level {opt}");
        // The panic ends the process cleanly instead of aborting.
        let (code, out, err) = run_status(panics, &["--opt-level", opt]);
        assert_eq!(code, Some(101), "--opt-level {opt}\n{err}");
        assert_eq!(out, "before\n");
        assert!(err.contains("division by zero"), "{err}");
        assert!(!err.contains("failed to initiate panic"), "{err}");
    }
}

#[test]
fn nan_is_unequal_to_itself_and_float_to_int_saturates() {
    let source = r##"
fn main() do
  let nan = 0.0 / 0.0
  let big = 1.0e20
  println("#{nan != nan} #{not (nan == nan)} #{[nan] != [nan]} #{1.0 != 1.0}")
  println("#{Float.to_int(big)} #{Float.to_int(0.0 - big)} #{Float.to_int(nan)}")
  println("#{Math.floor(big)} #{Math.ceil(nan)} #{Math.round(0.0 - big)} #{Float.to_int(2.9)}")
end
"##;
    for opt in ["0", "2"] {
        let (code, out, err) = run_status(source, &["--opt-level", opt]);
        assert_eq!(code, Some(0), "{err}");
        assert_eq!(
            out,
            "true true true false\n9223372036854775807 -9223372036854775808 0\n9223372036854775807 0 -9223372036854775808 2\n",
            "--opt-level {opt}"
        );
    }
}

// ── Diagnostic locations ───────────────────────────────────────────────

#[test]
fn a_service_handler_type_error_points_at_the_handler() {
    let source = "service Registry do\n  fn init() -> Int do\n    0\n  end\n\n  call Get() :: String do |n|\n    (n, n)\n  end\nend\n\nfn main() do\n  let r = Registry.start()\n  println(Registry.get(r))\nend\n";
    let diags = json_diagnostics(source);
    let codes: Vec<&str> = diags.iter().filter_map(|d| d["code"].as_str()).collect();
    // Only the handler's error: the service is still defined for `main`.
    assert_eq!(codes, ["E0001", "C0001"], "{diags:?}");
    assert_eq!(
        diags[0]["message"],
        "type mismatch: expected `String`, found `Int`"
    );
    let start = source.find("(n, n)").unwrap();
    assert_eq!(diags[0]["spans"][0]["start"], start);
}

#[test]
fn an_undefined_module_is_reported_once() {
    let diags = json_diagnostics("fn main() do\n  let r = Nope.start()\n  println(\"x\")\nend\n");
    let codes: Vec<&str> = diags.iter().filter_map(|d| d["code"].as_str()).collect();
    assert_eq!(codes, ["E0004", "C0001"], "{diags:?}");
}

// ── Pattern matching: guards, nesting, scoping ─────────────────────────

#[test]
fn a_failed_guard_tests_the_next_arms_patterns() {
    let source = r##"
fn classify(x :: Int) -> String do
  case x do
    n when n > 7 -> "big"
    0 -> "zero"
    _ -> "other"
  end
end

fn clause(n) when n > 7 = "big"
fn clause(0) = "zero"
fn clause(_) = "other"

fn never(o :: Option<Int>) -> Bool = false

fn main() do
  let closure = fn n when n > 7 -> "big" | 0 -> "zero" | _ -> "other" end
  println("#{classify(5)} #{classify(0)} #{classify(9)} #{clause(5)} #{closure(5)}")
  let r = case Some(1) do
    o when never(o) -> "never"
    None -> "none"
    Some(_) -> "some"
  end
  println(r)
  case 5 do
    a when a > 7 -> println("big #{a}")
    b -> println("other #{b}")
  end
end
"##;
    assert_eq!(run(source), "other zero big other other\nsome\nother 5\n");
}

#[test]
fn nested_or_patterns_match_only_their_alternatives() {
    let source = r##"
type Letter do
  A
  B
  C
end

fn describe(o :: Option<Int>) -> String do
  case o do
    Some(1 | 2) -> "small"
    Some(n) -> "n #{n}"
    None -> "none"
  end
end

fn main() do
  println("#{describe(Some(2))} #{describe(Some(3))} #{describe(None)}")
  let t = case (3, 0) do
    (1 | 2, _) -> "small"
    (n, _) -> "n #{n}"
  end
  let l = case Some(C) do
    Some(A | B) -> "a or b"
    _ -> "other"
  end
  println("#{t} #{l}")
end
"##;
    assert_eq!(run(source), "small n 3 none\nn 3 other\n");
}

#[test]
fn case_arm_bindings_do_not_leak_into_the_enclosing_scope() {
    let source = r##"
fn main() do
  let x = 1
  case Some(2) do
    Some(x) -> println("arm #{x}")
    None -> println("none")
  end
  println("after #{x}")
  let b = "outer"
  let r = case Some(5) do
    Some(b) -> b
    None -> 0
  end
  println("#{r} #{b}")
  for i in [1, 2] do
    case Some(i * 10) do
      Some(i) -> println("inner #{i}")
      None -> println("none")
    end
    println("loop #{i}")
  end
end
"##;
    assert_eq!(
        run(source),
        "arm 2\nafter 1\n5 outer\ninner 10\nloop 1\ninner 20\nloop 2\n"
    );
}

#[test]
fn list_patterns_read_aggregate_elements() {
    let source = r##"
struct P do
  x :: Int
end

type Sh do
  C(r :: Int)
  S(w :: Int)
end

fn first_ok(xs :: List<Int!String>) -> Int do
  case xs do
    Ok(a) :: _ -> a
    _ -> 0
  end
end

fn main() do
  case [P { x: 42 }] do
    h :: _ -> println("#{h.x}")
    [] -> println("empty")
  end
  case [Some(7)] do
    [Some(a)] -> println("got #{a}")
    _ -> println("no match")
  end
  case [S(8)] do
    h :: _ -> case h do
      C(a) -> println("C #{a}")
      S(a) -> println("S #{a}")
    end
    [] -> println("empty")
  end
  case [fn n -> n * 3 end] do
    f :: _ -> println("#{f(10)}")
    [] -> println("none")
  end
  case [Some(9)] do
    Some(a) :: _ -> println("head #{a}")
    _ -> println("no")
  end
  println("#{first_ok([Ok(7)])}")
end
"##;
    for opt in ["0", "2"] {
        let (code, out, err) = run_status(source, &["--opt-level", opt]);
        assert_eq!(code, Some(0), "{err}");
        assert_eq!(out, "42\ngot 7\nS 8\n30\nhead 9\n7\n", "--opt-level {opt}");
    }
}

#[test]
fn a_tuple_literal_scrutinee_is_matched_by_columns() {
    let source = r##"
type Sh do
  C(r :: Int)
  S(w :: Int)
end

fn classify(a :: Int, b :: String, s :: Sh) -> String do
  case (a, b, s) do
    (0, "x", _) -> "zero-x"
    (n, _, C(r)) when n > r -> "big C #{n} #{r}"
    (1 | 2, t, S(w)) -> "small S #{t} #{w}"
    (n, t, _) -> "other #{n} #{t}"
  end
end

fn main() do
  println(classify(0, "x", C(1)))
  println(classify(5, "y", C(2)))
  println(classify(2, "z", S(9)))
  println(classify(1, "q", C(4)))
end
"##;
    assert_eq!(run(source), "zero-x\nbig C 5 2\nsmall S z 9\nother 1 q\n");
}

// ── Function clauses ───────────────────────────────────────────────────

#[test]
fn clause_parameters_match_constructors_tuples_and_or_patterns() {
    let source = r##"
type Sh do
  C(r :: Int)
  S(w :: Int)
end

type Color do
  Red
  Green
  Blue
end

fn f(C(_), n) = "C #{n}"
fn f(_, n) = "other #{n}"

fn g((0, _), n) = "zero-first #{n}"
fn g(_, n) = "other #{n}"

fn opt_add(Some(a), Some(b)) = Some(a + b)
fn opt_add(_, _) = None

fn pick((a, _), true) = a
fn pick((_, b), false) = b

fn small(1 | 2, label) = "small #{label}"
fn small(_, label) = "big #{label}"

fn name(Red) = "red"
fn name(Green) = "green"
fn name(Blue) = "blue"

fn len([]) = 0
fn len(_ :: t) = 1 + len(t)

fn max(a, b) when a > b = a
fn max(_, b) = b

fn fact(0) do
  1
end
fn fact(n) do
  n * fact(n - 1)
end

fn unwrap(Some(v)) = v
fn unwrap(None) = 0

fn first(x, _) = x

fn main() do
  println("#{f(S(1), 2)} #{f(C(1), 3)} #{g((5, 1), 2)} #{g((0, 1), 4)}")
  println("#{opt_add(Some(1), Some(2))} #{opt_add(None, Some(2))} #{pick((1, 2), true)} #{pick((1, 2), false)}")
  println("#{small(2, "a")} #{small(3, "b")} #{name(Green)} #{name(Blue)}")
  println("#{len([1, 2, 3])} #{max(3, 2)} #{max(2, 3)} #{fact(5)} #{unwrap(Some(3))} #{unwrap(None)}")
  println("#{first(1, "a")} #{first("s", 2)}")
end
"##;
    assert_eq!(
        run(source),
        "other 2 C 3 other 2 zero-first 4\nSome(3) None 1 2\nsmall a big b green blue\n3 3 3 120 3 0\n1 s\n"
    );
}

#[test]
fn closure_clauses_match_their_parameters() {
    let source = r##"
type Sh do
  C(r :: Int)
  S(w :: Int)
end

type Wrap do
  W(Int)
end

fn main() do
  let f = fn C(_), n -> "C #{n}" | _, n -> "other #{n}" end
  let some = fn Some(x) -> x * 2 | None -> 0 end
  let unwrap = fn W(v) -> v end
  let flag = fn true -> "T" | false -> "F" end
  println("#{f(S(1), 2)} #{f(C(1), 3)} #{some(Some(4))} #{some(None)} #{unwrap(W(3))} #{flag(false)}")
end
"##;
    assert_eq!(run(source), "other 2 C 3 8 0 3 F\n");
}

#[test]
fn clauses_that_do_not_cover_every_argument_warn_and_panic_when_missed() {
    for (source, missing) in [
        ("fn f(\"a\") = 1\nfn main() do\n  println(\"#{f(\"zzz\")}\")\nend\n", "missing: _"),
        ("fn pos(n) when n > 0 = n\nfn main() do\n  println(\"#{pos(-5)}\")\nend\n", "missing: _"),
        ("fn f(0, n) = n\nfn f(1, n) = n + 1\nfn main() do\n  println(\"#{f(5, 4)}\")\nend\n", "missing: (_, _)"),
        ("type Color do\n  Red\n  Green\nend\nfn f(Red) = \"red\"\nfn main() do\n  println(f(Green))\nend\n", "missing: Green"),
        ("fn main() do\n  let t = fn true -> \"T\" end\n  println(t(false))\nend\n", "missing: false"),
    ] {
        // A clause group that misses values is a warning, as documented; a
        // call no clause matches panics instead of returning garbage.
        let built = build(source, false);
        assert!(built.ok, "{source}\n{}", built.stderr);
        assert!(built.stderr.contains("clauses do not cover every"), "{source}\n{}", built.stderr);
        assert!(built.stderr.contains(missing), "{source}\n{}", built.stderr);
        assert!(built.stderr.contains("Warning"), "{source}\n{}", built.stderr);
        let output = Command::new(built.dir.path().join("project/project"))
            .output()
            .expect("run binary");
        assert_eq!(output.status.code(), Some(101), "{source}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("non-exhaustive match"),
            "{source}"
        );
        assert!(output.stdout.is_empty(), "{source}");
    }
}

// ── Tuple values ───────────────────────────────────────────────────────

#[test]
fn tuple_values_keep_their_elements_wherever_they_come_from() {
    let source = r##"
struct P do
  n :: Int
  f :: (String, Int)
end deriving(Eq)

struct Box<T> do
  value :: T
end

fn pair() -> (Int, Int) = (1, 2)

fn fst(t :: (String, Int)) -> String do
  let (s, _) = t
  s
end

fn main() do
  let z = pair()
  println("#{z} #{z == (1, 2)} #{Tuple.first(z)}")
  let w = if true do (1, "a") else (2, "b") end
  let c = case 2 do
    1 -> (0, 0)
    _ -> (3, 4)
  end
  println("#{w} #{c}")
  let p = fn (a :: Int) -> (a, a * 10) end
  let t = p(7)
  println("#{p(1)} #{Tuple.second(t)} #{[p(2)]}")
  let a = P { n: 1, f: ("x", 2) }
  let b = P { n: 1, f: ("x", 2) }
  let d = P { n: 1, f: ("y", 9) }
  println("#{a.f} #{fst(a.f)} #{a == b} #{a == d} #{Box { value: (5, 6) }.value}")
  let m = %{"b" => (2, 3)}
  let v = Map.get(m, "b")
  println("#{v} #{Tuple.first(v)}")
  for {k, pv} in m do
    println("#{k} #{pv}")
  end
  for (k, (x, y)) in %{1 => (10, "s")} do
    println("#{k} #{x} #{y}")
  end
end
"##;
    for opt in ["0", "2"] {
        let (code, out, err) = run_status(source, &["--opt-level", opt]);
        assert_eq!(code, Some(0), "{err}");
        assert_eq!(
            out,
            "(1, 2) true 1\n(1, a) (3, 4)\n(1, 10) 70 [(2, 20)]\n(x, 2) x true false (5, 6)\n(2, 3) 2\nb (2, 3)\n1 10 s\n",
            "--opt-level {opt}"
        );
    }
}

// ── Runtime ────────────────────────────────────────────────────────────

#[test]
fn list_collect_and_flat_map_keep_their_results_alive_across_collections() {
    let source = r##"
fn check(ws :: List<String>, i :: Int, bad :: Int) -> Int do
  case ws do
    w :: rest -> check(rest, i + 1, if w == "w#{i}" do bad else bad + 1 end)
    _ -> bad
  end
end

fn main() do
  let xs = for i in 0..20000 do
    i
  end
  let lazy :: List<String> = Iter.from(xs) |> Iter.map(fn (x :: Int) -> "w#{x}" end) |> List.collect()
  let fm = List.flat_map(xs, fn x -> ["w#{x}"] end)
  println("#{check(lazy, 0, 0)} #{check(fm, 0, 0)} #{List.length(fm)}")
end
"##;
    assert_eq!(run(source), "0 0 20000\n");
}

#[cfg(unix)]
#[test]
fn a_closed_stdout_ends_the_program_quietly() {
    // The runtime ignores SIGPIPE so a server outlives a client that hung up
    // (e2e_http_crash_isolation covers a closed stderr); `println` to a
    // closed stdout still ends the program the way SIGPIPE would.
    let built = build(
        "fn main() do\n  for i in 0..100000 do\n    println(\"line #{i}\")\n  end\nend\n",
        false,
    );
    assert!(built.ok, "{}", built.stderr);
    let mut child = Command::new(built.dir.path().join("project/project"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("run binary");
    let mut first = [0u8; 6];
    std::io::Read::read_exact(child.stdout.as_mut().unwrap(), &mut first).unwrap();
    drop(child.stdout.take());
    let status = child.wait().unwrap();
    // It ends quietly, as SIGPIPE ends a C program (`prog | head`).
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(13), "{status:?}");
}

// ── Collections compare by Eq ──────────────────────────────────────────

#[test]
fn list_contains_compares_elements_by_their_eq() {
    let source = r##"
struct P do
  x :: Int
end

type Color do
  Red
  Green
end

fn main() do
  println("#{List.contains([(2, "b")], (2, "b"))} #{List.contains([Some(3)], Some(3))} #{List.contains([[3]], [3])}")
  println("#{List.contains([P { x: 1 }], P { x: 1 })} #{List.contains([Red], Red)} #{List.contains([Red], Green)}")
  println("#{[(1, 2)] |> List.contains((1, 2))} #{List.contains([0.0], -0.0)} #{List.contains([None], None)}")
  let nan = 0.0 / 0.0
  println("#{List.contains([nan], nan)} #{List.contains([1, 2], 2)} #{List.contains(["a"], "a")}")
end
"##;
    assert_eq!(
        run(source),
        "true true true\ntrue true false\ntrue true true\nfalse true true\n"
    );
}

#[test]
fn maps_compare_compound_keys_by_their_eq() {
    // `struct K` also checks that the built-in `Map<K, V>` impls do not take
    // a user type named K for their type parameter.
    let source = r##"
struct K do
  id :: Int
  name :: String
end

fn main() do
  let m = Map.put(Map.new(), (1, 2), "a")
  let m = Map.put(m, (1, 2), "b")
  println("#{Map.size(m)} #{Map.has_key(m, (1, 2))} #{Map.get(m, (1, 2))} #{Map.has_key(m, (2, 1))}")
  let ks = Map.put(Map.new(), K { id: 1, name: "x" }, 10)
  let ks = Map.put(ks, K { id: 1, name: "x" }, 11)
  println("#{Map.size(ks)} #{Map.get(ks, K { id: 1, name: "x" })} #{Map.has_key(ks, K { id: 2, name: "x" })}")
  let ls = Map.delete(Map.put(Map.put(Map.new(), [1], "one"), [2], "two"), [1])
  println("#{Map.size(ls)} #{Map.keys(ls)}")
  let fl = Map.from_list([("Al", 1), ("Bo", 2)])
  println("#{fl} #{Map.has_key(fl, "Al")} #{Map.get(fl, "Bo")}")
  let pairs = [(1, "a"), (2, "b"), (3, "c")]
  println("#{Map.from_list(List.drop(pairs, 1))}")
  let tl = Map.from_list([((1, 1), "x"), ((1, 1), "y")])
  println("#{Map.size(tl)} #{Map.get(tl, (1, 1))}")
  let zm = Iter.from(["a", "b"]) |> Iter.zip(Iter.from([1, 2])) |> Map.collect()
  println("#{Map.has_key(zm, "a")} #{Map.get(zm, "b")}")
  let lit = %{(1, "a") => 1, (1, "a") => 2}
  println("#{Map.size(lit)} #{Map.get(lit, (1, "a"))} #{lit == %{(1, "a") => 2}} #{lit == %{(1, "a") => 3}}")
  let mg = Map.merge(%{(1, 1) => 1}, %{(1, 1) => 2, (2, 2) => 3})
  println("#{Map.size(mg)} #{Map.get(mg, (1, 1))}")
  let sk = %{Some(1) => "s", None => "n"}
  println("#{Map.get(sk, Some(1))} #{Map.get(sk, None)}")
end
"##;
    assert_eq!(
        run(source),
        "1 true b false\n1 11 false\n1 [[2]]\n%{Al => 1, Bo => 2} true 2\n%{2 => b, 3 => c}\n1 y\ntrue 2\n1 2 true false\n2 2\ns n\n"
    );
}

#[test]
fn iterators_carry_their_element_types() {
    let source = r##"
struct P do
  x :: Int
end

fn main() do
  let words = Iter.from(["a", "bb", "ccc"]) |> Iter.map(fn w -> String.length(w) end) |> List.collect()
  let floats = Iter.from([1.5, 2.5]) |> Iter.map(fn f -> f * 2.0 end) |> List.collect()
  let xs = Iter.from([P { x: 1 }, P { x: 2 }]) |> Iter.filter(fn p -> p.x > 1 end) |> List.collect()
  let pairs = Iter.from(["a", "b"]) |> Iter.enumerate() |> List.collect()
  let found = Iter.from(["x", "yy"]) |> Iter.find(fn s -> String.length(s) == 2 end)
  println("#{words} #{floats} #{List.length(xs)} #{pairs} #{found}")
end
"##;
    assert_eq!(
        run(source),
        "[1, 2, 3] [3.0, 5.0] 1 [(0, a), (1, b)] Some(yy)\n"
    );
    for (bad, code) in [
        ("fn main() do\n  let bad = Iter.from([\"a\"]) |> Iter.map(fn x -> x + 1 end) |> List.collect()\n  println(\"#{bad}\")\nend\n", "E0006"),
        ("fn main() do\n  println(\"#{Iter.from([\"x\"]) |> Iter.sum()}\")\nend\n", "E0001"),
        ("fn main() do\n  let r :: List<String> = Iter.from([1, 2]) |> List.collect()\n  println(\"#{r}\")\nend\n", "E0001"),
    ] {
        let err = build_error(bad);
        assert!(err.contains(code), "{bad}\n{err}");
        assert!(!err.contains("E0004"), "no cascade:\n{err}");
    }
}

#[test]
fn values_of_a_type_nothing_fixed_compare_equal() {
    let source = "fn main() do\n  println(\"#{None == None} #{Ok(1) == Ok(1)} #{Ok(1) == Ok(2)} #{[None] == [None]}\")\nend\n";
    assert_eq!(run(source), "true true false true\n");
}

// ── Type-checker soundness ─────────────────────────────────────────────

#[test]
fn a_failed_let_does_not_make_its_name_undefined() {
    let diags = json_diagnostics(
        "fn main() do\n  let v = \"a\" <> nope\n  let w :: Int = \"s\"\n  println(v)\n  println(\"#{w}\")\nend\n",
    );
    let codes: Vec<&str> = diags.iter().filter_map(|d| d["code"].as_str()).collect();
    assert_eq!(codes, ["E0004", "E0001", "C0001"], "{diags:?}");
    // The annotation is what was expected.
    assert_eq!(
        diags[1]["message"],
        "type mismatch: expected `Int`, found `String`"
    );
}

#[test]
fn ill_typed_programs_that_used_to_build_are_rejected() {
    for (source, message) in [
        (
            "fn f(n :: Int) -> Int do\n  if n > 0 do\n    return \"x\"\n  end\n  0\nend\nfn main() do\n  println(\"#{f(1)}\")\nend\n",
            "expected Int, found String",
        ),
        (
            "fn f(n :: Int) -> Int do\n  if n > 0 do\n    return\n  end\n  0\nend\nfn main() do\n  println(\"#{f(1)}\")\nend\n",
            "expected Int, found ()",
        ),
        (
            "fn f(b :: Bool) -> Int do\n  if b do\n    1\n  end\nend\nfn main() do\n  println(\"#{f(false)}\")\nend\n",
            "expected Int, found ()",
        ),
        (
            "fn double(n :: Int) -> Int = n * 2\nfn main() do\n  case (5, \"hello\") do\n    (a, \"x\") | (5, a) -> println(\"#{double(a)}\")\n    _ -> println(\"y\")\n  end\nend\n",
            "expected Int, found String",
        ),
        (
            "fn main() do\n  case (1, 2) do\n    (a, a) -> println(\"#{a}\")\n  end\nend\n",
            "`a` is bound twice in one pattern",
        ),
        (
            "fn main() do\n  for x in \"hello\" do\n    println(x)\n  end\nend\n",
            "String does not implement Iterable",
        ),
        (
            "fn main() do\n  for x in 5 do\n    println(\"#{x}\")\n  end\nend\n",
            "Int does not implement Iterable",
        ),
    ] {
        let err = build_error(source);
        assert!(err.contains(message), "{source}\n{err}");
    }
}

#[test]
fn returns_are_checked_and_joined_without_a_declared_type() {
    let source = r##"
fn first_even(xs) do
  for x in xs do
    if x % 2 == 0 do
      return x
    end
  end
  -1
end

fn classify(v :: Int?) -> Int do
  case v do
    Some(v) -> v
    None -> 0
  end
end

fn main() do
  let f = fn n -> if n > 0 do
    return "pos"
  else
    "non-pos"
  end end
  println("#{first_even([1, 3, 4, 5])} #{first_even([1])} #{f(1)} #{f(0)} #{classify(Some(3))}")
end
"##;
    assert_eq!(run(source), "4 -1 pos non-pos 3\n");
}

// ── Generics and interfaces ────────────────────────────────────────────

#[test]
fn generic_functions_calling_generic_functions_are_specialized_for_each_use() {
    let source = r##"
fn ident(a) do
  a
end

fn wrap(a) do
  ident(a)
end

fn show(a) do
  "<${a}>"
end

fn relay(a) do
  show(a)
end

fn size(xs) do
  List.length(xs)
end

fn main() do
  println("#{wrap(5)} #{wrap("five")} #{show(7)} #{relay("hello")} #{relay(2.5)}")
  println("#{size([1])} #{size(["a"])} #{size([])}")
end
"##;
    assert_eq!(run(source), "5 five <7> <hello> <2.5>\n1 1 0\n");
}

#[test]
fn methods_of_bounded_type_parameters_have_their_interface_types() {
    let source = r##"
interface Sized do
  fn size(self) -> Int
end

interface Container do
  type Item
  fn first(self) -> Self.Item
end

struct Sq do
  s :: Int
end

struct IntBox do
  value :: Int
end

struct StrBox do
  s :: String
end

impl Sized for Sq do
  fn size(self) -> Int do
    self.s
  end
end

impl Container for IntBox do
  type Item = Int
  fn first(self) -> Int do
    self.value
  end
end

impl Container for StrBox do
  type Item = String
  fn first(self) -> String do
    self.s
  end
end

fn report<T>(x :: T) -> String where T: Sized do
  let n = x.size()
  "${n} ${x.size() + 1}"
end

fn show_first<T>(c :: T) -> String where T: Container do
  let x = c.first()
  "${x}"
end

fn get<T>(c :: T) where T: Container do
  c.first()
end

fn main() do
  let v = get(IntBox { value: 5 })
  println("#{report(Sq { s: 3 })} #{show_first(IntBox { value: 5 })} #{show_first(StrBox { s: "hi" })} #{v + 1}")
end
"##;
    assert_eq!(run(source), "3 4 5 hi 6\n");
    // `T.Item` fixed to Int by the body does not hold for StrBox.
    let err = build_error(&source.replace(
        "fn main() do",
        "fn plus<T>(c :: T) -> Int where T: Container do\n  c.first() + 1\nend\n\nfn main() do\n  println(\"#{plus(StrBox { s: \"x\" })}\")",
    ));
    assert!(err.contains("expected Int, found String"), "{err}");
}

#[test]
fn method_arguments_and_impl_signatures_are_checked_against_the_interface() {
    let interface = "interface Scale do\n  fn scale(self, k :: Int) -> String\nend\n\nstruct Cat do\n  n :: String\nend\n\n";
    let err = build_error(&format!(
        "{interface}impl Scale for Cat do\n  fn scale(self, k :: Int) -> String do\n    \"${{k}}\"\n  end\nend\n\nfn main() do\n  println(Cat {{ n: \"a\" }}.scale(\"oops\"))\nend\n"
    ));
    assert!(err.contains("expected Int, found String"), "{err}");
    for (method, found) in [
        (
            "fn scale(self, k :: String) -> String do\n    k\n  end",
            "(Self, String)",
        ),
        ("fn scale(self) -> String do\n    \"x\"\n  end", "(Self)"),
        ("fn scale(k :: Int) -> String do\n    \"x\"\n  end", "(Int)"),
    ] {
        let err = build_error(&format!(
            "{interface}impl Scale for Cat do\n  {method}\nend\n\nfn main() do\n  println(Cat {{ n: \"a\" }}.scale(1))\nend\n"
        ));
        assert!(
            err.contains("E0008") && err.contains(found),
            "{method}\n{err}"
        );
        // Located at the impl method, not the whole file.
        assert!(err.contains(":10:3"), "{method}\n{err}");
    }
    let err = build_error(
        "struct V do\n  x :: Int\nend\n\nimpl Mul for V do\n  type Output = Int\n  fn mul(self, other :: V) -> V do\n    V { x: self.x * other.x }\n  end\nend\n\nfn main() do\n  println(\"#{(V { x: 2 } * V { x: 3 }).x}\")\nend\n",
    );
    assert!(err.contains("expected Int, found V"), "{err}");
}

// ── Derived traits follow the source types ─────────────────────────────

#[test]
fn generic_struct_instantiations_derive_by_their_type_arguments() {
    // Box<List<Int>> and Box<List<String>> share a layout; their derived
    // helpers must not be shared.
    let source = r##"
struct Box<T> do
  value :: T
end deriving(Eq, Ord, Display, Debug, Hash)

fn main() do
  let a = Box { value: [1, 2] }
  let b = Box { value: ["x", "y"] }
  println("#{a} #{b} #{b.to_string()}")
  println("#{b == Box { value: ["x", "y"] }} #{a == Box { value: [1, 3] }}")
  println("#{a.inspect()} #{b.inspect()}")
  println("#{a < Box { value: [1, 3] }} #{b > Box { value: ["x"] }} #{compare(a, Box { value: [0] })}")
  println("#{b.hash() == Box { value: ["x", "y"] }.hash()} #{Box { value: 1 }.hash() == Box { value: 2 }.hash()}")
end
"##;
    assert_eq!(
        run(source),
        "Box([1, 2]) Box([x, y]) Box([x, y])\ntrue false\nBox { value: [1, 2] } Box { value: [\"x\", \"y\"] }\ntrue true Greater\ntrue false\n"
    );
}

#[test]
fn derived_traits_handle_collection_tuple_option_and_bool_fields() {
    let source = r##"
struct P do
  name :: String
  tags :: List<String>
  ok :: Bool
  pair :: (Int, String)
  opt :: Option<List<Int>>
end deriving(Eq, Ord, Display, Debug, Hash)

type Shape do
  Circle(Float)
  Poly(List<Int>)
  Named(String, Bool)
end deriving(Eq, Ord, Display, Debug, Hash)

type Chain do
  Link(Int, Chain)
  End
end deriving(Eq, Ord, Display, Debug)

fn main() do
  let p = P { name: "n", tags: ["a", "b"], ok: true, pair: (1, "z"), opt: Some([3]) }
  let q = P { name: "n", tags: ["a", "b"], ok: false, pair: (1, "z"), opt: None }
  println("#{p}")
  println(p.inspect())
  println("#{p == q} #{q < p} #{p < q} #{compare(p, p)}")
  println("#{Poly([1, 2])} #{Named("s", true).inspect()} #{Circle(1.5) < Poly([])} #{Poly([1]) < Poly([2])} #{Named("a", false) < Named("a", true)}")
  let c = Link(1, Link(2, End))
  println("#{c.inspect()} #{c < Link(1, Link(3, End))} #{End < c} #{compare(Circle(2.0), Circle(1.0))}")
  let tags = ["a"] ++ ["b"]
  println("#{p.hash() == P { name: "n", tags: tags, ok: true, pair: (1, "z"), opt: Some([3]) }.hash()} #{Poly([1, 2]).hash() == Poly([1] ++ [2]).hash()}")
end
"##;
    assert_eq!(
        run(source),
        "P(n, [a, b], true, (1, z), Some([3]))\n\
         P { name: \"n\", tags: [\"a\", \"b\"], ok: true, pair: (1, \"z\"), opt: Some([3]) }\n\
         false true false Equal\n\
         Poly([1, 2]) Named(\"s\", true) true true true\n\
         Link(1, Link(2, End)) true false Greater\n\
         true true\n"
    );
}

#[test]
fn compare_and_inspect_work_on_any_ordered_or_shown_type() {
    let source = r##"
fn main() do
  println("#{compare(1, 2)} #{compare("b", "a")} #{compare([1], [2])} #{compare((1, 2), (1, 2))}")
  let o = compare("b", "a")
  println("#{o == Greater} #{o.to_string()} #{[1].compare([0])}")
  println("#{["x", "y"].inspect()} #{[(1, "a")].inspect()}")
end
"##;
    assert_eq!(
        run(source),
        "Less Greater Less Equal\ntrue Greater Greater\n[\"x\", \"y\"] [(1, \"a\")]\n"
    );
}

#[test]
fn stdlib_functions_called_as_methods_lower_like_module_calls() {
    // `xs.contains(x)` compares by Eq like `List.contains(xs, x)`, and map
    // methods reach the map functions.
    let source = r##"
struct K do
  a :: Int
  b :: String
end deriving(Eq, Hash)

fn main() do
  let m = %{(1, "a") => 10}
  println("#{m.get((1, "a"))} #{m.has_key((1, "a"))} #{Map.has_key(m, (1, "a"))}")
  let m2 = m.put((1, "a"), 20)
  println("#{m2.size()} #{m2.get((1, "a"))}")
  let xs = [(1, "a")]
  println("#{xs.contains((1, "a"))} #{[K { a: 1, b: "q" }].contains(K { a: 1, b: "q" })}")
  let s = %{"k" => 1}
  println("#{s.get("k")} #{s.has_key("k" <> "")}")
end
"##;
    assert_eq!(run(source), "10 true true\n1 20\ntrue true\n1 true\n");
}

#[test]
fn bools_order_and_negative_zero_hashes_like_zero() {
    let source = r##"
struct S do
  s :: String
  b :: Bool
end deriving(Eq, Ord)

struct F do
  v :: Float
end deriving(Eq, Hash)

fn main() do
  println("#{false < true} #{true <= false} #{compare(true, false)} #{S { s: "a", b: false } < S { s: "a", b: true }}")
  println("#{F { v: 0.0 } == F { v: -0.0 }} #{F { v: 0.0 }.hash() == F { v: -0.0 }.hash()}")
end
"##;
    assert_eq!(run(source), "true false Greater true\ntrue true\n");
}

#[test]
fn inspect_quotes_and_escapes_strings_at_every_level() {
    let source = r##"
struct S do
  s :: String
end deriving(Debug)

fn main() do
  println("a\"b\\c\nd".inspect())
  println(("x", ["y"], Some("z")).inspect())
  println(S { s: "w\t" }.inspect())
  println(%{"k" => "v"}.inspect())
end
"##;
    assert_eq!(
        run(source),
        "\"a\\\"b\\\\c\\nd\"\n(\"x\", [\"y\"], Some(\"z\"))\nS { s: \"w\\t\" }\n%{\"k\" => \"v\"}\n"
    );
}

#[test]
fn qualified_variant_constructors_work_in_expressions() {
    let source = r##"
type Color do
  Red
  Rgb(n :: Int)
end

fn main() do
  let c = Color.Rgb(1)
  case c do
    Red -> println("r")
    Rgb(n) -> println("rgb #{n}")
  end
  println("#{Color.Red == Red} #{Result.Ok(1) == Ok(1)} #{Option.Some(2)}")
end
"##;
    assert_eq!(run(source), "rgb 1\ntrue true Some(2)\n");
}

#[test]
fn derived_json_handles_every_serializable_field_type() {
    let source = r##"
struct A do
  s :: Option<String>
  i :: Option<Int>
  f :: Option<Float>
end deriving(Json)

struct Outer do
  inner :: Inner
  kids :: List<Node>
end deriving(Json)

struct Inner do
  n :: List<List<Int>>
  o :: Option<List<Int>>
  m :: Map<String, List<Int>>
  p :: (Int, String)
end deriving(Json)

struct Node do
  kids :: List<Node>
end deriving(Json)

type T do
  V(xs :: List<Int>)
  W(m :: Map<String, Int>)
end deriving(Json)

struct Box<Item> do
  value :: Item
end deriving(Json)

fn show(r :: Result<A, String>) -> String do
  case r do
    Ok(a) -> Json.encode(a)
    Err(e) -> "err #{e}"
  end
end

fn main() do
  println(show(A.from_json("{\"s\":\"x\",\"i\":7,\"f\":1.5}")))
  println(show(A.from_json("{\"s\":null,\"i\":null,\"f\":null}")))
  println(show(A.from_json("{\"s\":null,\"i\":1.9,\"f\":null}")))
  let inner = Inner { n: [[1, 2], []], o: Some([3]), m: %{"a" => [1]}, p: (1, "z") }
  let enc = Json.encode(Outer { inner: inner, kids: [Node { kids: [] }] })
  println(enc)
  case Outer.from_json(enc) do
    Ok(o) -> println("#{Json.encode(o) == enc} #{Map.get(o.inner.m, "a")}")
    Err(e) -> println("err #{e}")
  end
  println("#{Json.encode(W(%{"a" => 1}))} #{Json.encode(V([10, 20]))}")
  case T.from_json("{\"tag\":\"V\",\"fields\":[[1,2]]}") do
    Ok(v) -> println(Json.encode(v))
    Err(e) -> println("err #{e}")
  end
  let r :: Result<Box<Int>, String> = Box.from_json("{\"value\":2}")
  case r do
    Ok(b) -> println("#{b.value + 1}")
    Err(e) -> println("err #{e}")
  end
  println("#{Json.encode(Box { value: ["x"] })} #{Json.encode(Box { value: %{"a" => 1} })}")
end
"##;
    assert_eq!(
        run(source),
        "{\"f\":1.5,\"i\":7,\"s\":\"x\"}\n\
         {\"f\":null,\"i\":null,\"s\":null}\n\
         err expected Int\n\
         {\"inner\":{\"m\":{\"a\":[1]},\"n\":[[1,2],[]],\"o\":[3],\"p\":[1,\"z\"]},\"kids\":[{\"kids\":[]}]}\n\
         true [1]\n\
         {\"fields\":[{\"a\":1}],\"tag\":\"W\"} {\"fields\":[[10,20]],\"tag\":\"V\"}\n\
         {\"fields\":[[1,2]],\"tag\":\"V\"}\n\
         3\n\
         {\"value\":[\"x\"]} {\"value\":{\"a\":1}}\n"
    );
}

#[test]
fn derive_errors_point_at_the_field_or_the_deriving_clause() {
    let err = build_error(
        "struct X do\n  n :: Int\nend\n\nstruct W do\n  x :: X\nend deriving(Json)\n\nfn main() do\n  println(\"x\")\nend\n",
    );
    assert!(err.contains("E0038") && err.contains(":6:3"), "{err}");
    let err =
        build_error("type T do\n  A\nend deriving(Ord)\n\nfn main() do\n  println(\"x\")\nend\n");
    assert!(err.contains("E0029") && err.contains(":3:5"), "{err}");
}

#[test]
fn values_whose_type_parameter_is_never_fixed_compare_and_order() {
    // `Ok(1)`'s error type is never fixed; it holds no values to tell apart.
    let source = r##"
type Outcome<T> do
  Pending
  Done(T)
end deriving(Eq, Ord)

fn main() do
  let r = Ok(1)
  println("#{r == Ok(1)} #{None == None} #{Pending == Pending} #{Err("x") == Err("x")}")
  println("#{r < Ok(2)} #{compare(None, None)} #{Pending < Pending} #{Err("a") < Err("b")}")
end
"##;
    assert_eq!(run(source), "true true true true\ntrue Equal false true\n");
}

// ── Struct literals, updates, aliases and variants ─────────────────────

#[test]
fn aliases_expand_through_other_aliases() {
    let source = r##"
type Pair<A, B> = (A, B)
type Twin<T> = Pair<T, T>
type Id<T> = T
type Twice<T> = Id<Id<T>>
type Nested<T> = List<Pair<T, T>>

fn main() do
  let n :: Twin<Int> = (1, 2)
  let i :: Twice<Int> = 3
  let l :: Nested<String> = [("a", "b")]
  println("#{n} #{i} #{l}")
end
"##;
    assert_eq!(run(source), "(1, 2) 3 [(a, b)]\n");
}

#[test]
fn struct_literals_through_aliases() {
    let source = r##"
struct Point do
  x :: Int
end

struct Box<T> do
  value :: T
end

type P = Point
type IntBox = Box<Int>

fn main() do
  let q = P { x: 4 }
  let b = IntBox { value: 5 }
  println("#{q.x} #{b.value + 1}")
end
"##;
    assert_eq!(run(source), "4 6\n");
}

#[test]
fn a_struct_literal_must_name_a_struct() {
    let err = build_error("fn main() do\n  let q = Nope { x: 4 }\n  println(\"x\")\nend\n");
    assert!(err.contains("E0059") && err.contains(":2:11"), "{err}");
}

#[test]
fn a_struct_update_needs_a_struct_value() {
    let err = build_error(
        "fn main() do\n  let m = %{\"a\" => 1}\n  let t = %{m | a: 3}\n  println(\"#{t}\")\nend\n",
    );
    assert!(
        err.contains("E0059") && err.contains("`Map<String, Int>` is not a struct"),
        "{err}"
    );
    let err = build_error("fn main() do\n  let t = %{(1, 2) | a: 3}\n  println(\"#{t}\")\nend\n");
    assert!(
        err.contains("E0059") && !err.contains("<struct update>"),
        "{err}"
    );
}

#[test]
fn a_field_is_given_once() {
    let err = build_error(
        "struct S do\n  a :: Int\n  b :: Int\nend\n\nfn main() do\n  let s = S { a: 1, a: 2, b: 3 }\n  println(\"#{s.a}\")\nend\n",
    );
    assert!(err.contains("E0058") && err.contains(":7:21"), "{err}");
    let err = build_error(
        "struct S do\n  a :: Int\nend\n\nfn main() do\n  let s = S { a: 1 }\n  let t = %{s | a: 2, a: 3}\n  println(\"#{t.a}\")\nend\n",
    );
    assert!(err.contains("E0058"), "{err}");
}

#[test]
fn a_function_field_takes_no_value_derives() {
    // No deriving clause: the struct works, but has no Eq to call.
    let source = "struct Op do\n  run :: Fun(Int) -> Int\nend\n\nfn main() do\n  let o = Op { run: fn x -> x * 2 end }\n  println(\"#{o.run(10)}\")\nend\n";
    assert_eq!(run(source), "20\n");
    let err = build_error(&source.replace("#{o.run(10)}", "#{o == o}"));
    assert!(err.contains("does not implement Eq"), "{err}");
    // Deriving one explicitly is an error at the field.
    let err = build_error(&source.replace("end\n\nfn main", "end deriving(Eq)\n\nfn main"));
    assert!(err.contains("E0060") && err.contains(":2:3"), "{err}");
}

#[test]
fn a_variant_name_belongs_to_one_type_per_module() {
    let err = build_error(
        "type A do\n  Same(n :: Int)\nend\n\ntype B do\n  Same(s :: String)\nend\n\nfn main() do\n  println(\"x\")\nend\n",
    );
    assert!(err.contains("E0061") && err.contains(":6:3"), "{err}");
    // Only the one error: `Same` stays A's.
    assert_eq!(err.matches("Error:").count(), 1, "{err}");
}

#[test]
fn an_alias_that_refers_to_itself_is_reported() {
    let err = build_error(
        "type A = B\ntype B = A\ntype L<T> = List<L<T>>\n\nfn main() do\n  println(\"x\")\nend\n",
    );
    for alias in ["`A`", "`B`", "`L`"] {
        assert!(
            err.contains(&format!("type alias {alias} refers to itself")),
            "{err}"
        );
    }
}

#[test]
fn methods_are_callable_before_their_impl_and_from_sibling_methods() {
    let source = r##"
struct Dog do
  n :: Int
end

fn main() do
  let d = Dog { n: 1 }
  println("#{d.name()} #{d.label()} #{d.tag()}")
end

interface Named do
  fn name(self) -> Int
  fn label(self) -> Int
  fn tag(self) -> String do
    "t#{self.name()}"
  end
end

impl Named for Dog do
  fn name(self) -> Int do
    self.n
  end
  fn label(self) -> Int do
    self.name() + 1
  end
end
"##;
    assert_eq!(run(source), "1 2 t1\n");
}

#[test]
fn let_bound_closures_with_operators_work_at_each_type() {
    let source = r##"
fn main() do
  let dbl = fn x -> x * x end
  let add = fn a, b -> a + b end
  let eq = fn a, b -> a == b end
  let lt = fn a, b -> a < b end
  println("#{dbl(3)} #{dbl(1.5)} #{add(1, 2)} #{eq(1, 1)} #{lt(1, 2)} #{lt("b", "a")}")
  println("#{List.map([1, 2], dbl)}")
end
"##;
    assert_eq!(run(source), "9 2.25 3 true true false\n[1, 4]\n");
}

#[test]
fn an_operator_on_a_type_parameter_needs_its_bound() {
    let err = build_error(
        "fn less<T>(a :: T, b :: T) -> Bool do\n  a < b\nend\n\nfn main() do\n  println(\"#{less(1, 2)}\")\nend\n",
    );
    assert!(
        err.contains("E0063") && err.contains("where T: Ord") && err.contains(":2:3"),
        "{err}"
    );
    // With the bound (Ord includes Eq) it builds.
    let source = "fn less<T>(a :: T, b :: T) -> Bool where T: Ord do\n  a < b or a == b\nend\n\nfn main() do\n  println(\"#{less(1, 2)}\")\nend\n";
    assert_eq!(run(source), "true\n");
}

#[test]
fn default_needs_a_known_type_with_a_default() {
    let err = build_error("fn main() do\n  let x = default()\n  println(\"#{x}\")\nend\n");
    assert!(err.contains("E0064") && err.contains(":2:11"), "{err}");
    let err = build_error(
        "struct NoDef do\n  n :: Int\nend\n\nfn main() do\n  let x :: NoDef = default()\n  println(\"#{x.n}\")\nend\n",
    );
    assert!(err.contains("NoDef does not implement Default"), "{err}");
    let source = r##"
struct Cfg do
  n :: Int
end

impl Default for Cfg do
  fn default() -> Cfg do
    Cfg { n: 7 }
  end
end

fn make<T>() -> T where T: Default do
  default()
end

fn main() do
  let a :: Int = default()
  let c :: Cfg = default()
  let d :: Cfg = make()
  println("#{a} #{c.n} #{d.n}")
end
"##;
    assert_eq!(run(source), "0 7 7\n");
}

#[test]
fn impls_of_generic_interfaces_are_called_by_their_result_type() {
    // The reference's example: one impl, with a default method.
    let source = r##"
pub interface Container<T> do
  type Item
  fn first(self) -> Self.Item
  fn label(self) -> String do
    "container"
  end
end

struct IntBox do
  value :: Int
end

impl Container<Int> for IntBox do
  type Item = Int
  fn first(self) -> Int do
    self.value
  end
end

fn main() do
  let b = IntBox { value: 3 }
  println("#{b.first()} #{b.label()}")
end
"##;
    assert_eq!(run(source), "3 container\n");
    // Two impls of one generic interface: the annotation picks.
    let source = r##"
interface Convert<T> do
  fn convert(self) -> T
end

struct Meters do
  v :: Int
end

impl Convert<String> for Meters do
  fn convert(self) -> String do
    "#{self.v}m"
  end
end

impl Convert<Int> for Meters do
  fn convert(self) -> Int do
    self.v * 100
  end
end

fn main() do
  let s :: String = Meters { v: 3 }.convert()
  let c :: Int = Meters { v: 3 }.convert()
  println("#{s} #{c}")
end
"##;
    assert_eq!(run(source), "3m 300\n");
}

#[test]
fn into_and_try_into_go_to_the_from_impl_the_context_asks_for() {
    let source = r##"
struct Wrapper do
  value :: Int
end

impl From<Int> for Wrapper do
  fn from(n :: Int) -> Wrapper do
    Wrapper { value: n * 2 }
  end
end

impl From<String> for Wrapper do
  fn from(s :: String) -> Wrapper do
    Wrapper { value: String.length(s) }
  end
end

struct Meters do
  v :: Int
end

struct Feet do
  v :: Int
end

impl From<Meters> for Feet do
  fn from(m :: Meters) -> Feet do
    Feet { v: m.v * 3 }
  end
end

fn main() do
  let w :: Wrapper = 21.into()
  let f :: Float = 2.into()
  let s :: String = 7.into()
  let w4 :: Wrapper = "abc".into()
  let ft :: Feet = Meters { v: 2 }.into()
  println("#{w.value} #{f} #{s} #{w4.value} #{ft.v}")
end
"##;
    assert_eq!(run(source), "42 2.0 7 3 6\n");
    let source = r##"
struct Pos do
  v :: Int
end

struct Even do
  v :: Int
end

impl TryFrom<Int> for Pos do
  fn try_from(n :: Int) -> Result<Pos, String> do
    if n > 0 do
      Ok(Pos { v: n })
    else
      Err("not positive")
    end
  end
end

impl TryFrom<Int> for Even do
  fn try_from(n :: Int) -> Result<Even, String> do
    if n % 2 == 0 do
      Ok(Even { v: n })
    else
      Err("odd")
    end
  end
end

fn main() do
  let p :: Result<Pos, String> = 3.try_into()
  let e :: Result<Even, String> = 3.try_into()
  let q :: Result<Pos, String> = Pos.try_from(-1)
  case p do
    Ok(x) -> println("pos #{x.v}")
    Err(m) -> println(m)
  end
  case e do
    Ok(x) -> println("even #{x.v}")
    Err(m) -> println(m)
  end
  case q do
    Ok(x) -> println("pos #{x.v}")
    Err(m) -> println(m)
  end
end
"##;
    assert_eq!(run(source), "pos 3\nodd\nnot positive\n");
    let err = build_error("fn main() do\n  let x = 5.into()\n  println(\"#{x}\")\nend\n");
    assert!(err.contains("E0065") && err.contains(":2:11"), "{err}");
}

#[test]
fn interface_methods_are_called_by_argument_type_or_by_interface() {
    // `A.hello(x)` names the interface; a bare `hello(dog)` dispatches by
    // its argument like `dog.hello()`.
    let source = r##"
interface A do
  fn hello(self) -> String
end

interface B do
  fn hello(self) -> String
end

struct Cat do
  n :: Int
end

struct Dog do
  n :: Int
end

impl A for Cat do
  fn hello(self) -> String do
    "A"
  end
end

impl B for Cat do
  fn hello(self) -> String do
    "B"
  end
end

impl A for Dog do
  fn hello(self) -> String do
    "dog"
  end
end

fn main() do
  println(A.hello(Cat { n: 1 }))
  println(B.hello(Cat { n: 1 }))
  println(hello(Dog { n: 5 }))
  println(Dog { n: 5 }.hello())
end
"##;
    assert_eq!(run(source), "A\nB\ndog\ndog\n");
    // Two interfaces giving Cat a `hello`: a bare call is ambiguous.
    let err = build_error(&source.replace("A.hello(Cat", "hello(Cat"));
    assert!(
        err.contains("E0027") && err.contains("A.hello(value)"),
        "{err}"
    );
}

#[test]
fn static_interface_methods_are_called_on_types_and_type_parameters() {
    let source = r##"
interface Versioned do
  fn version() -> Int
end

struct A do
  n :: Int
end

struct B do
  n :: Int
end

impl Versioned for A do
  fn version() -> Int do
    1
  end
end

impl Versioned for B do
  fn version() -> Int do
    2
  end
end

fn ver_of<T>(x :: T) -> Int where T: Versioned do
  T.version()
end

fn main() do
  println("#{A.version()} #{B.version()} #{ver_of(A { n: 0 })} #{ver_of(B { n: 0 })}")
end
"##;
    assert_eq!(run(source), "1 2 1 2\n");
    // A stdlib module name as the type: the impl's method, not a module function.
    let source = r##"
interface Named do
  fn tag() -> String
end

impl Named for Int do
  fn tag() -> String do
    "int"
  end
end

fn main() do
  println(Int.tag())
end
"##;
    assert_eq!(run(source), "int\n");
    // Bare: fine with one impl, ambiguous with several.
    let source = r##"
interface Versioned do
  fn version() -> Int
end

struct A do
  n :: Int
end

impl Versioned for A do
  fn version() -> Int do
    1
  end
end

fn main() do
  println("#{version()}")
end
"##;
    assert_eq!(run(source), "1\n");
    let err = build_error(&open_s02());
    assert!(
        err.contains("E0066") && err.contains("A.version()"),
        "{err}"
    );
}

fn open_s02() -> String {
    r##"
interface Versioned do
  fn version() -> Int
end

struct A do
  n :: Int
end

struct B do
  n :: Int
end

impl Versioned for A do
  fn version() -> Int do
    1
  end
end

impl Versioned for B do
  fn version() -> Int do
    2
  end
end

fn ver_of<T>(x :: T) -> Int where T: Versioned do
  T.version()
end

fn main() do
  println("#{version()}")
end
"##
    .to_string()
}

#[test]
fn non_ascii_text_does_not_move_later_diagnostics() {
    // Spans are byte offsets; ariadne counted them as characters, so every
    // multi-byte character before an error moved it further right.
    let source = "# コメント コメント コメント\nfn main() do\n  let n = 1 + \"x\"\n  println(\"#{n}\")\nend\n";
    let err = build_error(source);
    assert!(err.contains("main.mpl:3:11"), "{err}");
}

fn mismatches(source: &str) -> Vec<(String, usize)> {
    json_diagnostics(source)
        .iter()
        .filter(|d| d["code"] == "E0001")
        .map(|d| {
            (
                d["spans"][0]["label"].as_str().unwrap_or("").to_string(),
                d["spans"][0]["start"].as_u64().unwrap_or(0) as usize,
            )
        })
        .collect()
}

#[test]
fn mismatches_name_the_expected_type_first_at_the_offending_expression() {
    let source = r##"
fn g() -> Int do
  "str"
end

fn check(f :: Fun(Int) -> Bool) -> Bool do
  f(1)
end

fn main() do
  while 1 do
    println("loop")
  end
  println("#{check(fn (x :: Int) -> x + 1 end)}")
end
"##;
    let found = mismatches(source);
    assert_eq!(
        found,
        [
            (
                "expected Int, found String".to_string(),
                source.find("\"str\"").unwrap()
            ),
            (
                "expected Bool, found Int".to_string(),
                source.find("while 1").unwrap() + 6
            ),
            (
                "expected Bool, found Int".to_string(),
                source.find("x + 1").unwrap()
            ),
        ]
    );
    let err = build_error("fn main() do\n  let y :: String = 5\nend\n");
    assert!(err.contains("expected String, found Int"), "{err}");
}

#[test]
fn a_failed_tail_expression_is_reported_once() {
    // The body's type is unknown after its tail fails; it used to become
    // `()` and add "expected Int, found ()" over the whole file.
    let source = "fn g() -> Int do\n  1 + \"y\"\nend\n\nfn main() do\n  println(\"x\")\nend\n";
    let found = mismatches(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].1, source.find("1 + ").unwrap());
}

#[test]
fn an_operator_on_a_type_without_it_names_the_left_operand() {
    // It said "expected Option<Int>, found Int", blaming the `1`.
    let err =
        build_error("fn main() do\n  let v :: Int? = Some(3)\n  println(\"#{v + 1}\")\nend\n");
    assert!(err.contains("Option<Int> does not implement Add"), "{err}");
}

#[test]
fn question_mark_needs_a_function_that_can_return_its_error() {
    // `main` returns `()`: an `Err` cannot leave it (this failed the LLVM
    // verifier before).
    let helper = "fn helper(n :: Int) -> Int!String do\n  if n > 0 do\n    Ok(n)\n  else\n    Err(\"neg\")\n  end\nend\n\n";
    let err = build_error(&format!(
        "{helper}fn main() do\n  let v = helper(1)?\n  println(\"#{{v}}\")\nend\n"
    ));
    assert!(err.contains("E0036"), "{err}");
    // A function without a declared return type returns the `Err` early,
    // so its other results must be Results too.
    let source = format!(
        "{helper}fn go(n :: Int) do\n  let v = helper(n)?\n  v + 1\nend\n\nfn main() do\n  println(\"#{{go(1)}}\")\nend\n"
    );
    let diags = json_diagnostics(&source);
    let try_err = diags.iter().find(|d| d["code"] == "E0036").expect("E0036");
    assert_eq!(
        try_err["spans"][0]["start"].as_u64().unwrap() as usize,
        source.find("helper(n)?").unwrap()
    );
}

#[test]
fn question_mark_in_unannotated_functions_and_closures_returns_early() {
    let source = r##"
fn helper(n :: Int) -> Int!String do
  if n > 0 do
    Ok(n)
  else
    Err("neg")
  end
end

fn double(n :: Int) do
  let v = helper(n)?
  Ok(v * 2)
end

fn main() do
  let triple = fn (n :: Int) do
    let v = helper(n)?
    Ok(v * 3)
  end
  case double(3) do
    Ok(v) -> println("ok #{v}")
    Err(e) -> println("err #{e}")
  end
  case triple(-1) do
    Ok(v) -> println("ok #{v}")
    Err(e) -> println("err #{e}")
  end
end
"##;
    assert_eq!(run(source), "ok 6\nerr neg\n");
}

#[test]
fn a_conflicting_early_return_is_reported_where_it_is() {
    let source = "fn pick(n :: Int) do\n  if n > 0 do\n    return \"pos\"\n  end\n  n\nend\n\nfn main() do\n  println(\"#{pick(1)}\")\nend\n";
    let diags = json_diagnostics(source);
    let mismatch = diags.iter().find(|d| d["code"] == "E0001").expect("E0001");
    assert_eq!(
        mismatch["spans"][0]["start"].as_u64().unwrap() as usize,
        source.find("return").unwrap()
    );
}

#[test]
fn bare_method_call_on_a_bounded_type_parameter_works_for_every_impl() {
    // `show_me(x)` fixed `T` to the first type with an impl (Int), so
    // `show(P { .. })` was rejected.
    let source = r##"
interface Printable do
  fn show_me(self) -> String
end

impl Printable for Int do
  fn show_me(self) -> String do
    "int"
  end
end

struct P do
  n :: Int
end

impl Printable for P do
  fn show_me(self) -> String do
    "p #{self.n}"
  end
end

fn show<T>(x :: T) -> String where T: Printable do
  show_me(x)
end

fn main() do
  println(show(1))
  println(show(P { n: 2 }))
end
"##;
    assert_eq!(run(source), "int\np 2\n");
}

#[test]
fn a_generic_function_must_keep_its_type_parameters_generic() {
    let err =
        build_error("fn bad<T>(x :: T) -> T do\n  5\nend\n\nfn main() do\n  println(\"x\")\nend\n");
    assert!(
        err.contains("E0067")
            && err.contains(
                "type parameter `T` stands for any type, but this function makes it `Int`"
            ),
        "{err}"
    );
    let err = build_error(
        "fn pair<A, B>(a :: A, b :: B) -> A do\n  b\nend\n\nfn main() do\n  println(\"x\")\nend\n",
    );
    assert!(
        err.contains("makes it `B`") || err.contains("makes it `A`"),
        "{err}"
    );
    // A parameter against a type built from it is a plain mismatch, named
    // by the parameter, not an "infinite type".
    let err = build_error(
        "fn first<T>(xs :: List<T>) -> Option<T> do\n  List.head(xs)\nend\n\nfn unwrap<T>(xs :: List<T>) -> T do\n  xs\nend\n\nfn main() do\n  println(\"x\")\nend\n",
    );
    assert!(err.contains("expected Option<T>, found T"), "{err}");
    assert!(err.contains("expected T, found List<T>"), "{err}");
    assert!(!err.contains("infinite type"), "{err}");
}

#[test]
fn duplicate_definitions_are_rejected() {
    let source = r##"
struct A do
  x :: Int
  x :: String
end

type C do
  R
  R
end

struct A do
  y :: Int
end

fn f(x :: Int, x :: String) -> String do
  x
end

fn main() do
  println(f(1, "dup"))
end
"##;
    let diags = json_diagnostics(source);
    let mut dups: Vec<(&str, usize)> = diags
        .iter()
        .filter(|d| d["code"] == "E0068")
        .map(|d| {
            (
                d["message"].as_str().unwrap(),
                d["spans"][0]["start"].as_u64().unwrap() as usize,
            )
        })
        .collect();
    dups.sort_by_key(|(_, start)| *start);
    assert_eq!(
        dups,
        [
            (
                "field `x` is defined twice",
                source.find("x :: String").unwrap()
            ),
            (
                "variant `R` is defined twice",
                source.rfind("  R\n").unwrap() + 2
            ),
            ("type `A` is defined twice", source.rfind("A do").unwrap()),
            (
                "parameter `x` is defined twice",
                source.find("x :: String)").unwrap()
            ),
        ]
    );
}

#[test]
fn annotations_that_start_with_a_type_parameter_keep_the_rest() {
    // `-> (A, B)` was read as `-> A` and `:: T?` as `:: T`.
    let source = r##"
fn pair<A, B>(a :: A, b :: B) -> (A, B) do
  (a, b)
end

fn maybe<T>(x :: T, keep :: Bool) -> T? do
  if keep do
    Some(x)
  else
    None
  end
end

fn unwrap_or<T>(x :: T?, d :: T) -> T do
  case x do
    Some(v) -> v
    None -> d
  end
end

fn swap<A, B>(p :: (A, B)) -> (B, A) do
  let (a, b) = p
  (b, a)
end

fn main() do
  let (a, b) = pair(1, "two")
  println("#{a} #{b}")
  println("#{unwrap_or(maybe(5, true), 0)} #{unwrap_or(maybe(5, false), 0)}")
  let (c, d) = swap((1, "x"))
  println("#{c} #{d}")
end
"##;
    assert_eq!(run(source), "1 two\n5 0\nx 1\n");
}

#[test]
fn tuple_types_take_option_and_result_sugar() {
    let source = r##"
fn h(x :: Int) -> (Int, String)? do
  if x > 0 do
    Some((x, "a"))
  else
    None
  end
end

fn k(x :: (Int, Int)!String) -> Int do
  case x do
    Ok((a, b)) -> a + b
    Err(_) -> 0
  end
end

fn main() do
  case h(1) do
    Some((a, b)) -> println("#{a} #{b}")
    None -> println("none")
  end
  println("#{k(Ok((1, 2)))} #{k(Err("e"))}")
end
"##;
    assert_eq!(run(source), "1 a\n3 0\n");
}

#[test]
fn unknown_type_names_are_rejected_where_they_are_written() {
    let source = r##"
fn f(x :: Strng) -> Int do
  1
end

struct S do
  field :: Nonexistent
end

type Shape do
  Circle(Flot)
  Box(w :: Int, h :: Integer)
end

fn g(xs :: List<Foo>, cb :: Fun(Intt) -> Bool) -> (Int, Strin)? do
  None
end

fn main() do
  let c = fn (q :: Quux) -> 1 end
  println("x")
end
"##;
    let diags = json_diagnostics(source);
    let unknown: Vec<(&str, usize)> = diags
        .iter()
        .filter(|d| d["code"] == "E0069")
        .map(|d| {
            (
                d["message"].as_str().unwrap(),
                d["spans"][0]["start"].as_u64().unwrap() as usize,
            )
        })
        .collect();
    let at = |name: &str| source.find(name).unwrap();
    assert_eq!(
        unknown,
        [
            ("unknown type `Strng`", at("Strng")),
            ("unknown type `Nonexistent`", at("Nonexistent")),
            ("unknown type `Flot`", at("Flot")),
            ("unknown type `Integer`", at("Integer")),
            ("unknown type `Foo`", at("Foo")),
            ("unknown type `Intt`", at("Intt")),
            ("unknown type `Strin`", at("Strin")),
            ("unknown type `Quux`", at("Quux")),
        ]
    );
}

#[test]
fn known_type_names_in_annotations_are_accepted() {
    let source = r##"
type Pair<A, B> = (A, B)
type Id = Int

struct Box<T> do
  value :: T
  items :: List<T>
end

type Tree<T> do
  Leaf
  Node(Tree<T>, T, Tree<T>)
end

interface Container<T> do
  type Item
  fn first(self) -> Self.Item
  fn wrap(self, x :: T) -> Self
end

fn swap<A, B>(p :: Pair<A, B>) -> Pair<B, A> do
  let (a, b) = p
  (b, a)
end

fn apply(f :: Fun(Int) -> Int, x :: Id) -> Int do
  f(x)
end

fn main() do
  let b :: Box<Int> = Box { value: 1, items: [2] }
  let t :: Tree<Int> = Leaf
  let m :: Map<String, Set<Int>> = Map.new()
  let o :: Option<Json>? = None
  let p :: Pid<Int>? = None
  let r :: Result<Bytes, CryptoError>? = None
  println("#{apply(fn (n :: Int) -> n + 1 end, 2)} #{b.value}")
  let (x, y) = swap((1, "a"))
  println("#{x} #{y}")
end
"##;
    assert_eq!(run(source), "3 1\na 1\n");
}

#[test]
fn a_value_from_a_function_defined_later_has_one_type() {
    // The function's type was still a placeholder when the `let` was
    // checked, and the `let` generalized it: `r` was accepted both as an
    // Int and as a String.
    for later in [
        "fn make(n) do\n  n * 2\nend\n",
        "fn make(0) = 1\nfn make(n) = n * make(n - 1)\n",
    ] {
        let source = format!(
            "fn main() do\n  let r = make(5)\n  let a :: Int = r\n  let b :: String = r\n  println(\"#{{a}} #{{b}}\")\nend\n\n{later}"
        );
        let err = build_error(&source);
        assert!(
            err.contains("expected String, found Int"),
            "{source}\n{err}"
        );
    }
    let source = "fn main() do\n  let r = make(5)\n  println(\"#{r + 1} #{fact(5)}\")\nend\n\nfn make(n) do\n  n * 2\nend\n\nfn fact(0) = 1\nfn fact(n) = n * fact(n - 1)\n";
    assert_eq!(run(source), "11 120\n");
}

#[test]
fn a_call_before_an_annotated_definition_has_its_declared_types() {
    // The call saw only a placeholder, which `?` took for a Result.
    let source = r##"
fn first() -> Int? do
  let v = maybe_later(3)?
  Some(v + 1)
end

fn maybe_later(n :: Int) -> Int? do
  Some(n)
end

fn main() do
  println("#{first()}")
end
"##;
    assert_eq!(run(source), "Some(4)\n");
}

#[test]
fn fields_of_values_typed_later_in_the_function() {
    // A closure's parameter is typed by the call after it, and a value by
    // a function defined later: codegen saw `Unit` ("Field access on
    // non-struct type").
    let source = r##"
struct A do
  x :: Int
  tags :: List<String>
end

fn from_later() -> Int!String do
  let a = later(4)?
  Ok(a.x + List.length(a.tags))
end

fn later(n) do
  Ok(A { x: n, tags: ["t"] })
end

fn main() do
  let f = fn p -> p.x end
  let a = A { x: 5, tags: ["t"] }
  let g = fn p -> List.length(p.tags) + p.x end
  println("#{f(a)} #{g(a)} #{List.map([a], fn p -> p.x * 2 end)}")
  case from_later() do
    Ok(n) -> println("#{n}")
    Err(e) -> println(e)
  end
end
"##;
    assert_eq!(run(source), "5 6 [10]\n5\n");
    // Nothing fixes the type of `p`: its layout is unknown.
    let err = build_error("fn get_x(p) do\n  p.x\nend\n\nfn main() do\n  println(\"x\")\nend\n");
    assert!(
        err.contains("E0070") && err.contains("cannot tell which type has the field `x`"),
        "{err}"
    );
}

#[test]
fn trailing_closures_are_the_last_argument() {
    // The docs' `with_value(10) do |value| ... end` was "expected 2
    // argument(s), found 1": the closure was parsed and then ignored.
    let source = r##"
fn with_value(value :: Int, block :: Fun(Int) -> Int) -> Int do
  block(value)
end

fn twice(block :: Fun() -> Int) -> Int do
  block() + block()
end

fn main() do
  let result = with_value(10) do |value|
    value * 2
  end
  let base = 5
  let total = twice() do
    base + 1
  end
  let lengths = List.map(["a", "bb"]) do |w|
    String.length(w)
  end
  let piped = 3 |> with_value() do |n|
    n * 10
  end
  println("#{result} #{total} #{lengths} #{piped}")
end
"##;
    assert_eq!(run(source), "20 12 [1, 2] 30\n");
    // A closure passed both ways is one argument too many; the trailing
    // one was dropped without a word.
    let err = build_error(
        "fn with_value(value :: Int, block :: Fun(Int) -> Int) -> Int do\n  block(value)\nend\n\nfn main() do\n  let r = with_value(10, fn x -> x + 1 end) do |v|\n    v\n  end\n  println(\"#{r}\")\nend\n",
    );
    assert!(err.contains("expected 2 argument(s), found 3"), "{err}");
}

#[test]
fn items_can_be_used_before_their_declaration() {
    // A generic function called before its definition was pinned to its
    // first use ("expected Int, found String" at the second), and an actor
    // or service used above its declaration was an undefined variable.
    let source = r##"
fn main() do
  println("#{List.length(twice(1))} #{List.length(twice("a"))}")
  let p = spawn(echo)
  send(p, 1)
  let c = Counter.start(0)
  Counter.inc(c)
  println("#{Counter.get(c)} #{area(Sq(3))}")
  Timer.sleep(50)
end

fn twice(x) do
  [x, x]
end

fn area(s :: Shape) -> Int do
  case s do
    Sq(n) -> n * n
  end
end

type Shape do
  Sq(Int)
end

actor echo() do
  receive do
    m -> println("echo #{m}")
  end
end

service Counter do
  fn init(n :: Int) -> Int do
    n
  end

  call Get() :: Int do |s|
    (s, s)
  end

  cast Inc() do |s|
    s + 1
  end
end
"##;
    let output = run(source);
    assert!(output.starts_with("2 2\n"), "{output}");
    assert!(
        output.contains("echo 1\n") && output.contains("1 9\n"),
        "{output}"
    );
}

#[test]
fn a_let_naming_a_generic_function_is_generic() {
    // `let id = ident` used at two types compiled one copy of `ident` for
    // both, and the LLVM verifier rejected the call.
    let source = r##"
fn ident(x) do
  x
end

fn pair(a, b) do
  (a, b)
end

fn main() do
  let id = ident
  let again = id
  println("#{id(1)} #{id("two")} #{again(3.5)}")
  let p = pair
  let (a, b) = p(1, "x")
  let (c, d) = p("y", 2)
  println("#{a} #{b} #{c} #{d}")
end
"##;
    assert_eq!(run(source), "1 two 3.5\n1 x y 2\n");
}

#[test]
fn an_impl_must_name_an_existing_interface_and_type() {
    // Both impls were accepted; the first one's methods even worked.
    let source = r##"
struct C do
  n :: Int
end

impl Dispaly for C do
  fn to_string(self) -> String do
    "c"
  end
end

impl Display for Nope do
  fn to_string(self) -> String do
    "x"
  end
end

fn main() do
  println("x")
end
"##;
    let err = build_error(source);
    assert!(
        err.contains("[E0071] Error: unknown interface `Dispaly`"),
        "{err}"
    );
    assert!(err.contains("[E0069] Error: unknown type `Nope`"), "{err}");
}

#[test]
fn a_pipe_can_feed_send() {
    // `5 |2> send(p)` was "expected (), found (Int) -> ?14" over the whole
    // file: `send` is not a function value.
    let source = r##"
actor sink() do
  receive do
    m -> println("got #{m}")
  end
  sink()
end

fn main() do
  let p = spawn(sink)
  5 |2> send(p)
  p |> send(6)
  Timer.sleep(100)
end
"##;
    assert_eq!(run(source), "got 5\ngot 6\n");
}

#[test]
fn numeric_literals_are_checked() {
    // Each of these compiled to 0; the smallest Int could not be written.
    let source = r##"
fn main() do
  println("#{-9223372036854775808}")
  let r = case -9223372036854775808 do
    -9223372036854775808 -> "INT_MIN"
    _ -> "other"
  end
  println("#{r} #{0x7fffffffffffffff} #{1_000} #{0b101} #{0o17} #{1.5e3}")
end
"##;
    assert_eq!(
        run(source),
        "-9223372036854775808\nINT_MIN 9223372036854775807 1000 5 15 1500.0\n"
    );
    let bad = "fn main() do\n  let a = 1e\n  let b = 2e+\n  let c = 0x\n  let d = 9223372036854775808\n  let e = 0xffffffffffffffff\n  let f = 1e999\n  let g = case 0 do\n    99999999999999999999 -> 1\n    _ -> 0\n  end\n  println(\"#{a} #{b} #{c} #{d} #{e} #{f} #{g}\")\nend\n";
    let err = build_error(bad);
    assert_eq!(err.matches("E0072").count(), 7, "{err}");
    assert!(err.contains("expected digits after the exponent"), "{err}");
    assert!(err.contains("expected digits after `0x`"), "{err}");
    assert!(
        err.contains("integer literal `9223372036854775808` is out of range"),
        "{err}"
    );
    assert!(
        err.contains("float literal `1e999` is out of range"),
        "{err}"
    );
}

#[test]
fn concatenation_needs_strings_or_lists() {
    // `1 <> 2` aborted at runtime, `1.5 <> 2.5` failed the LLVM verifier,
    // `true ++ false` hung and `(1, 2) <> (3, 4)` printed garbage.
    let source = r##"
fn cat<T>(a :: T, b :: T) -> T do
  a <> b
end

fn main() do
  let v = 1 <> 2
  let w = 1.5 <> 2.5
  let x = true ++ false
  let y = (1, 2) <> (3, 4)
  println("done")
end
"##;
    let err = build_error(source);
    for ty in ["T", "Int", "Float", "Bool", "(Int, Int)"] {
        assert!(
            err.contains(&format!("joins strings or lists, not `{ty}`")),
            "{ty}\n{err}"
        );
    }
    let ok = "fn main() do\n  println(\"a\" <> \"b\")\n  println(\"#{[1] ++ [2]} #{[3] <> [4]}\")\n  println(\"c\" ++ \"d\")\nend\n";
    assert_eq!(run(ok), "ab\n[1, 2] [3, 4]\ncd\n");
}

#[test]
fn operators_in_unannotated_functions_are_checked_at_their_calls() {
    // `add("x", "y")` reached codegen ("Unsupported binop type: String",
    // no location), and `join(1, 2)` aborted at runtime.
    let source = r##"
fn add(a, b) do
  a + b
end

fn pass(x, y) do
  add(x, y)
end

fn join(a, b) do
  a <> b
end

fn main() do
  println("#{add(1, 2)} #{add(1.5, 2.0)} #{pass(3, 4)} #{join("a", "b")} #{join([1], [2])}")
  println(pass("x", "y"))
  println("#{join(1, 2)}")
end
"##;
    let err = build_error(source);
    assert!(err.contains("String does not implement Add"), "{err}");
    assert!(
        err.contains("`<>` joins strings or lists, not `Int`"),
        "{err}"
    );
    let ok = source
        .replace("  println(pass(\"x\", \"y\"))\n", "")
        .replace("  println(\"#{join(1, 2)}\")\n", "");
    assert_eq!(run(&ok), "3 3.5 7 ab [1, 2]\n");
}

#[test]
fn string_from_shows_any_value_with_display() {
    // Anything but Int, Float and Bool printed a pointer.
    let source = r##"
struct P do
  x :: Int
end deriving(Display)

fn main() do
  println(String.from(1))
  println(String.from(2.5))
  println(String.from("s"))
  println(String.from([1, 2]))
  println(String.from(Some(3)))
  println(String.from(P { x: 4 }))
  println(5 |> String.from())
end
"##;
    assert_eq!(run(source), "1\n2.5\ns\n[1, 2]\nSome(3)\nP(4)\n5\n");
    let err = build_error(
        "struct Q do\n  x :: Int\nend\n\nfn main() do\n  println(String.from(Q { x: 1 }))\nend\n",
    );
    assert!(err.contains("Main.Q does not implement Display"), "{err}");
}

#[test]
fn builtins_expanded_inline_can_be_used_as_values() {
    // "Undefined variable 'mesh_math_sqrt'": these have no function of
    // their own to point at.
    let source = r##"
fn apply(f :: Fun(Float) -> Float, x :: Float) -> Float do
  f(x)
end

fn main() do
  let f = Math.sqrt
  println("#{f(4.0)} #{apply(Math.sqrt, 16.0)}")
  println("#{[1, 2] |> List.map(Int.to_float)} #{List.reduce([3, 9, 2], 0, Math.max)}")
  println("#{List.map([1.5, 2.5], Float.to_int)} #{List.map([1, 2], String.from)}")
end
"##;
    assert_eq!(run(source), "2.0 4.0\n[1.0, 2.0] 9\n[1, 2] [1, 2]\n");
}

#[test]
fn a_heredoc_pattern_matches_the_same_heredoc_value() {
    // The pattern kept its raw text: newline, indentation and all.
    let source = r##"
fn classify(s :: String) -> String do
  case s do
    """
    a
    """ -> "heredoc pattern"
    _ -> "other"
  end
end

fn main() do
  let h = """
    a
    """
  println("#{classify(h)} #{classify("\n    a\n    ")}")
end
"##;
    assert_eq!(run(source), "heredoc pattern other\n");
}

#[test]
fn heredocs_in_crlf_files_end_their_lines_with_newlines() {
    // The value was "a\r\nb\r": the `\r` before the closing indent stayed.
    let source = "fn main() do\r\n  let h = \"\"\"\r\n    a\r\n    b\r\n    \"\"\"\r\n  println(\"#{String.length(h)} #{h == \"a\\nb\"}\")\r\nend\r\n";
    assert_eq!(run(source), "3 true\n");
}

#[test]
fn long_expressions_do_not_overflow_the_compilers_stack() {
    // 200 interpolations or 500 chained `<>`/`+` crashed the compiler with
    // a stack overflow.
    let interpolations = "#{x},".repeat(1000);
    let joined = vec!["\"a\""; 800].join(" <> ");
    let sum = vec!["1"; 800].join(" + ");
    let source = format!(
        "fn main() do\n  let x = 7\n  let s = \"{interpolations}\"\n  println(\"#{{String.length(s)}} #{{String.length({joined})}} #{{{sum}}}\")\nend\n"
    );
    assert_eq!(run(&source), "2000 800 800\n");
}

#[test]
fn a_parse_error_at_the_end_of_the_file_is_reported() {
    // With byte offsets, the one-past-the-end span of an error at the end
    // of the file made ariadne panic.
    for source in [
        "fn main() do\n  let s = \"abc\n  println(s)\nend\n",
        "fn main() do\n  println(\"a\n",
        "fn main() do\n  println(\"é",
    ] {
        let err = build_error(source);
        assert!(
            err.contains("Parse error") && !err.contains("panicked"),
            "{err}"
        );
    }
}

#[test]
fn an_unterminated_string_is_one_error_at_its_opening_quote() {
    // It was "Parse error" with an empty message at the end of the file,
    // two or three times.
    let err = build_error("fn main() do\n  let s = \"abc\n  println(s)\nend\n");
    assert_eq!(err.matches("Parse error").count(), 1, "{err}");
    assert!(
        err.contains("unterminated string: no closing `\"`"),
        "{err}"
    );
    assert!(err.contains("main.mpl:2:11"), "{err}");
}

#[test]
fn parse_errors_name_tokens_as_written() {
    // "expected R_PAREN", "expected DO_KW", "expected IDENT".
    let err = build_error("fn main() do\n  println(1\nend\n");
    assert!(err.contains("expected `)`"), "{err}");
    assert!(!err.contains("R_PAREN"), "{err}");
}

#[test]
fn a_parse_error_is_not_repeated_at_the_same_place() {
    // An unclosed interpolation gave four errors on one column.
    let err = build_error("fn main() do\n  let x = 1\n  println(\"a#{x\")\nend\n");
    assert_eq!(err.matches("Parse error").count(), 1, "{err}");
    assert!(
        err.contains("expected `}` to close the interpolation"),
        "{err}"
    );
}

#[test]
fn a_pattern_of_the_wrong_type_is_reported_at_the_pattern() {
    // It spanned the whole function (and `main`), said "expected String,
    // found Int" the wrong way round, and suggested to_string().
    let source = "fn f(x :: Int) -> String do\n  case x do\n    \"a\" -> \"str\"\n    _ -> \"other\"\n  end\nend\n\nfn main() do\n  println(f(1))\nend\n";
    let diags = json_diagnostics(source);
    let mismatch = diags.iter().find(|d| d["code"] == "E0001").expect("E0001");
    assert_eq!(
        mismatch["spans"][0]["start"].as_u64().unwrap() as usize,
        source.find("\"a\"").unwrap()
    );
    assert_eq!(mismatch["fix"], serde_json::Value::Null);
    let err = build_error(source);
    assert!(
        err.contains("this pattern matches String, but the value is Int"),
        "{err}"
    );
    assert_eq!(err.matches("E0001").count(), 1, "{err}");
}

#[test]
fn types_inference_has_not_determined_show_as_holes() {
    // "expected Int, found Option<?9>" leaked an inference variable.
    let err = build_error(
        "fn f(x :: Int) -> String do\n  case x do\n    Some(_) -> \"opt\"\n    _ -> \"other\"\n  end\nend\n\nfn main() do\n  println(f(1))\nend\n",
    );
    assert!(err.contains("expected Int, found Option<_>"), "{err}");
    assert!(!err.contains("?"), "{err}");
}

#[test]
fn a_function_a_module_lacks_is_reported_as_such() {
    // "undefined variable: Math", although `Math` exists.
    let err = build_error(
        "fn main() do\n  println(\"#{Math.log(2.0)}\")\n  println(String.upcase(\"s\"))\nend\n",
    );
    assert!(err.contains("module `Math` has no function `log`"), "{err}");
    assert!(err.contains("`Math` has abs, ceil, floor"), "{err}");
    assert!(
        err.contains("module `String` has no function `upcase`"),
        "{err}"
    );
    assert!(!err.contains("undefined variable"), "{err}");
}

#[test]
fn float_has_to_string_like_int() {
    // It was "expected 1 argument(s), found 2".
    assert_eq!(
        run("fn main() do\n  println(Float.to_string(1.5) <> \" \" <> Int.to_string(2))\nend\n"),
        "1.5 2\n"
    );
}

#[test]
fn an_error_is_reported_once() {
    // A failed callee was inferred again as a method call and reported twice.
    let err = build_error("fn main() do\n  println(Foo.bar(1))\nend\n");
    assert_eq!(err.matches("undefined variable: Foo").count(), 1, "{err}");
}

#[test]
fn string_escapes_are_checked_and_unicode_escapes_work() {
    // An unknown escape `\q` became `q`, and `\u{e9}` printed `u{e9}`.
    let source = r##"
fn main() do
  let x = 1
  println("a\u{e9}b \u{1F389} [\t] [\"] [\\] [\${x}] [\#{x}]")
end
"##;
    assert_eq!(
        run(source),
        "a\u{e9}b \u{1F389} [\t] [\"] [\\] [${x}] [#{x}]\n"
    );
    let err = build_error("fn main() do\n  println(\"a\\qb\")\n  println(\"c\\u{zz}d\")\nend\n");
    assert!(err.contains("unknown escape `\\q`"), "{err}");
    assert!(err.contains("invalid unicode escape"), "{err}");
    assert!(
        err.contains("main.mpl:2:13") && err.contains("main.mpl:3:13"),
        "{err}"
    );
}

#[test]
fn a_heredoc_can_end_with_a_quote() {
    // `""""` closed the heredoc at the first three quotes and left a
    // stray `"`, a parse error.
    let source = "fn main() do\n  println(\"\"\"ends with quote\"\"\"\")\n  println(\"\"\"say \"hi\" twice\"\"\"\"\"\")\nend\n";
    assert_eq!(run(source), "ends with quote\"\nsay \"hi\" twice\"\"\"\n");
}

#[test]
fn a_bad_digit_in_a_radix_literal_is_named() {
    // `0b102` lexed as `0b10` then `2`: "expected a newline or `;`".
    let err = build_error("fn main() do\n  let b = 0b102\n  let c = 0o8\n  let d = 0xfg\n  println(\"#{b} #{c} #{d}\")\nend\n");
    assert!(
        err.contains("invalid digit `2` in binary literal `0b102`"),
        "{err}"
    );
    assert!(
        err.contains("invalid digit `8` in octal literal `0o8`"),
        "{err}"
    );
    assert!(
        err.contains("invalid digit `g` in hexadecimal literal `0xfg`"),
        "{err}"
    );
}

#[test]
fn return_of_a_tail_call_is_a_tail_call() {
    // "Terminator found in the middle of a basic block": the `ret` stayed
    // after the tail call's jump.
    let source = r##"
fn count(n :: Int, acc :: Int) -> Int do
  if n == 0 do
    return acc
  end
  return count(n - 1, acc + 1)
end

fn count2(n :: Int, acc :: Int) -> Int do
  if n == 0 do
    acc
  else
    return count2(n - 1, acc + 1)
  end
end

fn main() do
  println("#{count(10, 0)} #{count2(1000000, 0)}")
end
"##;
    assert_eq!(run(source), "10 1000000\n");
}

#[test]
fn a_stack_overflow_is_reported() {
    // Deep non-tail recursion died with a bare "Segmentation fault: 11".
    let recurse = "fn depth(n :: Int) -> Int do\n  if n == 0 do\n    0\n  else\n    1 + depth(n - 1)\n  end\nend\n\n";
    for main in [
        "fn main() do\n  println(\"#{depth(100000000)}\")\nend\n",
        "actor deep() do\n  receive do\n    n -> println(\"#{depth(n)}\")\n  end\nend\n\nfn main() do\n  let p = spawn(deep)\n  send(p, 100000000)\n  Timer.sleep(2000)\nend\n",
    ] {
        let (code, _stdout, stderr) = run_status(&format!("{recurse}{main}"), &[]);
        assert_ne!(code, Some(0), "{stderr}");
        assert!(stderr.contains("error: stack overflow"), "{stderr}");
    }
}

#[test]
fn a_name_can_be_defined_at_several_arities() {
    // Each arity is its own function. `= expr` bodies and private fns were
    // rejected ("expected 2 argument(s), found 1"), and clauses of one arity
    // beside another arity failed to link.
    let out = run(r##"fn area(r) = r * r * 3
fn area(w, h) = w * h

fn fact(0) = 1
fn fact(n) = fact(n, 1)
fn fact(0, acc) = acc
fn fact(n, acc) = fact(n - 1, acc * n)

fn scale(x :: Int) -> Int do
  x * 10
end

fn scale(x :: Int, by :: Int) -> Int do
  x * by
end

fn first(xs) = List.head(xs)
fn first(xs, fallback) = if List.length(xs) == 0 do fallback else List.head(xs) end

fn main() do
  println("#{area(2)} #{area(3, 4)}")
  println("#{fact(5)} #{fact(4, 1)}")
  println("#{3 |> scale} #{3 |> scale(4)} #{4 |2> scale(5)}")
  println("#{first([1, 2])} #{first(["a"])} #{first([], 7)} #{first([], "z")}")
  let scale = fn x -> x + 1 end
  println("#{scale(1)}")
end
"##);
    assert_eq!(out, "12 12\n120 24\n30 12 20\n1 a 7 z\n2\n");
}

#[test]
fn a_name_defined_at_several_arities_is_not_a_value() {
    let err = build_error(
        "fn area(r) = r * r * 3\nfn area(w, h) = w * h\n\nfn main() do\n  let f = area\n  println(\"#{area(1, 2, 3)}\")\nend\n",
    );
    assert_eq!(err.matches("[E0075]").count(), 2, "{err}");
    assert!(
        err.contains("`area` is defined at arities 1 and 2"),
        "{err}"
    );
    assert!(err.contains("fn a0 -> area(a0) end"), "{err}");
}

#[test]
fn a_runtime_error_is_a_mesh_panic_that_ends_only_its_actor() {
    // `List.get` past the end, in an actor, aborted the whole process ("panic
    // in a function that cannot unwind", then Rust's backtrace): runtime
    // functions could not unwind. It now ends the actor alone, reported as
    // the one-line Mesh panic.
    let (code, out, err) = run_status(
        r##"actor worker(n :: Int) do
  receive do
    i -> println("got #{List.get([1], i)}")
  end
end

fn main() do
  let pid = spawn(worker, 0)
  send(pid, 5)
  Timer.sleep(200)
  println("main still running")
end
"##,
        &[],
    );
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(out, "main still running\n");
    assert_eq!(
        err,
        "Mesh panic: List.get: index 5 is out of bounds for a list of length 1\n"
    );
    // On the main thread it ends the program with status 101.
    for (value, message) in [
        ("List.head(List.tail([1]))", "List.head: the list is empty"),
        (
            "DateTime.to_unix_ms(DateTime.add(dt, 1, :year))",
            "DateTime.add: unknown unit \"year\"",
        ),
        (
            "DateTime.to_unix_ms(DateTime.add(dt, 9223372036854775807, :day))",
            "DateTime.add: adding 9223372036854775807 day to 0 ms is out of the supported range",
        ),
    ] {
        let source = format!(
            "fn main() do\n  case DateTime.from_unix_ms(0) do\n    Ok(dt) -> println(\"#{{{value}}}\")\n    Err(e) -> println(e)\n  end\nend\n"
        );
        let (code, out, err) = run_status(&source, &[]);
        assert_eq!(code, Some(101), "{value}\n{err}");
        assert_eq!(out, "", "{value}");
        assert!(
            err.starts_with(&format!("Mesh panic: {message}")),
            "{value}\n{err}"
        );
        assert_eq!(err.lines().count(), 1, "{value}\n{err}");
    }
}

#[test]
fn a_failed_match_names_its_function() {
    // It said "Mesh panic in <unknown>".
    let (code, out, err) = run_status(
        "fn f(0) = \"zero\"\n\nfn main() do\n  println(f(0))\n  println(f(5))\nend\n",
        &[],
    );
    assert_eq!(code, Some(101), "{err}");
    assert_eq!(out, "zero\n");
    assert_eq!(err, "Mesh panic in Main__f: non-exhaustive match\n");
}

#[test]
fn break_outside_a_loop_names_both_loops() {
    // It said `break` belongs inside a `while` loop; `for` loops take it too.
    let err = build_error("fn main() do\n  break\nend\n");
    assert!(err.contains("inside a `while` or `for` loop"), "{err}");
}

#[test]
fn tuples_of_different_lengths_are_a_type_mismatch() {
    // `let (a, b) = (1, 2, 3)` said "expected 2 argument(s), found 3".
    let err = build_error("fn main() do\n  let (a, b) = (1, 2, 3)\n  println(\"#{a}\")\nend\n");
    assert!(
        err.contains("expected (_, _), found (Int, Int, Int)"),
        "{err}"
    );
    assert!(!err.contains("argument"), "{err}");
}

#[test]
fn a_guard_may_call_a_module_qualified_function() {
    // Guards allow named function calls, but `String.length(s)` was
    // rejected as not one.
    let out = run(
        "fn main() do\n  let r = case \"hello\" do\n    s when String.length(s) > 3 -> \"long\"\n    _ -> \"short\"\n  end\n  println(r)\nend\n",
    );
    assert_eq!(out, "long\n");
}

#[test]
fn a_misspelled_name_suggests_the_one_in_scope() {
    // The "did you mean" help existed, but the compiler never gave it the
    // names in scope.
    let err = build_error(
        "fn main() do\n  let count = 3\n  println(\"#{cuont}\")\n  printn(\"x\")\nend\n",
    );
    assert!(err.contains("did you mean `count`?"), "{err}");
    assert!(err.contains("did you mean `println`?"), "{err}");
    let err = build_error(
        "fn main() do\n  let r = case Some(1) do\n    Sme(n) -> n\n    None -> 0\n  end\n  println(\"#{r}\")\nend\n",
    );
    assert!(err.contains("unknown variant: Sme"), "{err}");
    assert!(err.contains("did you mean `Some`?"), "{err}");
}
