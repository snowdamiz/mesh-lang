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
    let dir = tempfile::tempdir().expect("temp dir");
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).expect("project dir");
    std::fs::write(project.join("main.mpl"), source).expect("main.mpl");
    let mut cmd = Command::new(find_meshc());
    cmd.arg("build").arg(&project).arg("--no-color");
    if json {
        cmd.arg("--json");
    }
    let output = cmd.output().expect("meshc");
    Build {
        dir,
        ok: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// Build and run; the program must build, run, and exit 0. Returns stdout.
fn run(source: &str) -> String {
    let built = build(source, false);
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
        "3 5050\ntrue false\nCons(1, Cons(2, Nil))\nCons(...)\n2 1\n"
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
